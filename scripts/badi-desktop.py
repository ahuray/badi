#!/usr/bin/env python3
"""Control the persistent desktop broker or an explicitly selected native trial."""

import importlib.util
import json
import os
import re
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import time
import uuid
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
APPS = ("xournalpp", "omawrite")
APP_IDS = {"omawrite": "omawrite", "xournalpp": "com.github.xournalpp.xournalpp"}
APP_ADAPTERS = {"obsidian": "obsidian", "bash": "shell"}
SERVICE = "badi-broker.service"
ACCESSIBILITY_SERVICE = "badi-accessibility.service"
HELP = """Badi — local writing controls

  badi status [--json]          Model health and counters
  badi settings                Open the Omarchy settings panel
  badi settings --json          Read the current settings document
  badi pause | resume          Persistently pause or resume predictions
  badi app APP_ID on|off
                               Allow or block predictions in one app
  badi site ORIGIN on|off       Allow or block one exact browser origin
  badi service start|stop|restart|status
                               Manage the local model process
  badi autostart on|off         Start with the graphical session (next login)
  badi doctor                  Inspect service and model health as JSON
  badi debug on|off|status|watch
                               Trace activity for 15 minutes, without typed text
  badi logs                    Print the last 60 service log entries
  badi launch omawrite|xournalpp
                               Open a supported editor

Native manual fields: Tab requests words; Tab again accepts.
Observed fields: automatic suggestions after exact field and app/site checks.
Web extension/Obsidian: Tab accepts a word, Ctrl/Command+Right all.
Bash: Ctrl-X then Tab requests/accepts. Escape dismisses (Bash: Ctrl-X then Escape).
Native tested applications: Omawrite and Xournal++ text cells.
For advanced protocol commands: badictl --help
"""


def cli_path():
    installed = Path.home() / ".local/lib/badi/badictl"
    return installed if installed.is_file() else ROOT / "target/release/badictl"


def control(arguments, endpoint=None):
    endpoint = endpoint or registry_directory().parent / "broker.sock"
    result = subprocess.run([str(cli_path()), "--socket", str(endpoint), *arguments],
                            capture_output=True, text=True, timeout=8)
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or "Badi is offline. Run: badi service start")
    return result.stdout


def service_state():
    result = subprocess.run([
        "systemctl", "--user", "show", SERVICE,
        "--property=LoadState,ActiveState,SubState,UnitFileState",
    ], capture_output=True, text=True, timeout=4)
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or "Cannot reach the user service manager")
    fields = dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)
    return {"loaded": fields.get("LoadState") == "loaded",
            "active": fields.get("ActiveState", "unknown"),
            "state": fields.get("SubState", "unknown"),
            "autostart": fields.get("UnitFileState") == "enabled"}


def native_state():
    result = subprocess.run([
        "systemctl", "--user", "show", "omarchy-fcitx5.service", "--property=ActiveState,MainPID",
    ], capture_output=True, text=True, timeout=4)
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or "Cannot inspect the native input service")
    fields = dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)
    pid = fields.get("MainPID", "0")
    loaded = False
    update_pending = False
    if pid.isdigit() and int(pid) > 0:
        # Confirm the running process maps the addon; an installed file alone
        # does not mean Fcitx loaded it. Never include process maps in output.
        try:
            mappings = Path(f"/proc/{pid}/maps").read_text().splitlines()
            update_pending = any(line.rstrip().endswith("/libbadi-fcitx5.so (deleted)") for line in mappings)
            loaded = update_pending or any(line.rstrip().endswith("/libbadi-fcitx5.so") for line in mappings)
        except OSError:
            return {"active": fields.get("ActiveState", "unknown"), "addon_loaded": None,
                    "addon_update_pending": None, "inspection_error": "native_process_uninspectable"}
    return {"active": fields.get("ActiveState", "unknown"), "addon_loaded": loaded,
            "addon_update_pending": update_pending}


def startup_problem():
    """Classify this service invocation's startup error without returning logs."""
    try:
        invocation = subprocess.run([
            "systemctl", "--user", "show", SERVICE, "--property=InvocationID", "--value",
        ], capture_output=True, text=True, timeout=3)
        identifier = invocation.stdout.strip()
        if invocation.returncode or not re.fullmatch(r"[a-f0-9]{32}", identifier):
            return None
        journal = subprocess.run([
            "journalctl", "--user", f"_SYSTEMD_INVOCATION_ID={identifier}",
            "--no-pager", "--output=cat", "--lines=40",
        ], capture_output=True, text=True, timeout=3)
        if journal.returncode:
            return None
        # Only translate known metadata errors. Never pass arbitrary journal
        # lines (or a historical invocation's failure) into the health report.
        for line in reversed(journal.stdout.splitlines()):
            if not line.startswith("error_code=local_model:"):
                continue
            if line.removeprefix("error_code=local_model:").strip() == "runtime_process_exited":
                return {"code": "runtime_process_exited", "message": "The local inference process stopped unexpectedly.",
                        "action": "The desktop service retries automatically. If retries stop, inspect badi logs and available memory, then run badi service restart."}
            missing = re.search(r"not installed \(([A-Za-z0-9_.-]{1,100}\.gguf)\)", line)
            if missing:
                return {"code": "model_not_installed",
                        "message": f"The broker selected {missing[1]}, but its model/runtime files are missing.",
                        "action": "Run badictl models writing for the pinned model download plan; then badi service restart."}
            if "no installed writing model" in line or "no writing model fits" in line:
                return {"code": "no_usable_model", "message": "No installed writing model fits the current resources.",
                        "action": "Run badictl hardware and badictl models writing; check available memory and installed model files."}
            if "verification failed" in line:
                return {"code": "model_verification_failed", "message": "The installed model or runtime failed integrity verification.",
                        "action": "Restore the pinned model/runtime artifacts before restarting Badi."}
            if "requires Linux" in line:
                return {"code": "runtime_unsupported", "message": "This machine cannot execute the installed inference runtime.",
                        "action": "Run badictl hardware and check the runtime requirements in the Badi runbook."}
            return {"code": "model_startup_failed", "message": "The local inference runtime failed to start.",
                    "action": "Run badi logs to inspect the local startup error, then badi service restart."}
    except (OSError, subprocess.SubprocessError):
        pass
    return None


def observer_service_state():
    result = subprocess.run([
        "systemctl", "--user", "show", ACCESSIBILITY_SERVICE,
        "--property=LoadState,ActiveState,MainPID",
    ], capture_output=True, text=True, timeout=3)
    if result.returncode:
        raise RuntimeError("observer_service_uninspectable")
    fields = dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)
    pid = fields.get("MainPID", "0")
    return {"loaded": fields.get("LoadState") == "loaded",
            "active": fields.get("ActiveState", "unknown"),
            "process_id": int(pid) if pid.isdigit() else 0}


def observer_probe(endpoint, expected_pid):
    path = ROOT / "adapters/accessibility/health.py"
    if not path.is_file():
        path = Path.home() / ".local/lib/badi/accessibility/health.py"
    if not path.is_file():
        return {"ready": False, "error": "health_module_missing"}
    spec = importlib.util.spec_from_file_location("badi_accessibility_health", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    try:
        return module.probe(endpoint, expected_pid)
    except module.ProbeError as error:
        return {"ready": False, "error": str(error)}


def accessibility_state():
    result = {"loaded": None, "active": "unknown", "ready": False, "bus_enabled": None}
    try:
        service = observer_service_state()
        result.update(service)
        if service["active"] == "active" and service["process_id"] > 0:
            endpoint = registry_directory().parent / "accessibility.sock"
            result.update(observer_probe(endpoint, service["process_id"]))
            if observer_service_state() != service:
                result.update(ready=False, error="service_changed")
        bridge = subprocess.run([
            "busctl", "--user", "--timeout=1s", "get-property", "org.a11y.Bus",
            "/org/a11y/bus", "org.a11y.Status", "IsEnabled",
        ], capture_output=True, text=True, timeout=2)
        if bridge.returncode == 0 and bridge.stdout.strip() in ("b true", "b false"):
            result["bus_enabled"] = bridge.stdout.strip() == "b true"
    except (RuntimeError, OSError, ValueError, subprocess.SubprocessError):
        result.update(ready=False, error="observer_uninspectable")
    return result


def health_report():
    report = {"schema": "badi.desktop-health.v1", "service": service_state(),
              "native_apps": list(APPS), "problems": [], "notes": []}
    report["notes"].append("Native browser/Codex writing is disabled: the external input path cannot guarantee the intended field and caret. App/site permissions and model readiness cannot enable this unsupported path.")
    problems = report["problems"]
    observer = report["accessibility"] = accessibility_state()
    if observer.get("loaded"):
        if not observer["ready"]:
            problems.append({"code": "accessibility_unavailable",
                             "message": "The extension-free field observer is unavailable.",
                             "action": "Inspect systemctl --user status badi-accessibility.service and the accessibility runbook."})
        elif observer.get("bus_enabled") is False:
            problems.append({"code": "accessibility_disabled",
                             "message": "Desktop accessibility is disabled, so applications may expose no fields.",
                             "action": "Complete the accessibility setup and relaunch the target application with its supported accessibility/input-method flags."})
    else:
        report["notes"].append("Extension-free browser/Codex observation is not installed or could not be inspected. Model readiness alone does not establish those integrations.")
    try:
        report["native"] = native_state()
    except (RuntimeError, OSError, ValueError, subprocess.SubprocessError):
        report["native"] = {"active": "unknown", "addon_loaded": None,
                            "addon_update_pending": None, "inspection_error": "native_service_uninspectable"}
    try:
        report["broker"] = json.loads(control(["status"]))
    except (RuntimeError, OSError, ValueError, subprocess.SubprocessError):
        report["error"] = "The desktop prediction broker is unreachable."
        startup = startup_problem() if report["service"]["active"] == "failed" else None
        if startup:
            problems.append(startup)
        problems.append({"code": "broker_unreachable", "message": report["error"],
                         "action": "Run badi service start, then badi doctor. Model startup takes a few seconds."})
    if report["service"]["active"] == "failed":
        problems.append({"code": "service_failed", "message": "The Badi service stopped after a startup failure.",
                         "action": "Resolve the startup problem, then run badi service restart to clear the restart limit."})
    if report.get("broker", {}).get("control_plane_degraded"):
        problems.append({"code": "settings_degraded", "message": "The broker cannot use its persistent settings safely.",
                         "action": "Inspect badi logs and restore valid private settings before enabling predictions."})
    if report["native"].get("inspection_error"):
        problems.append({"code": "native_status_unknown", "message": "Native input integration could not be inspected.",
                         "action": "Inspect the Fcitx user service from the graphical session. Browser and editor broker health is reported separately."})
    elif report["native"]["addon_loaded"] is False:
        problems.append({"code": "native_addon_unavailable", "message": "The running Fcitx service has not loaded Badi.",
                         "action": "Check the native installation in the Badi runbook. Browser/Codex native writing remains unavailable independently of addon or observer health."})
    broker = report.get("broker", {})
    if report["native"].get("addon_update_pending"):
        report["notes"].append("Fcitx is still using the previous addon build. Apply the native update after normal desktop unlock.")
    if broker.get("paused"):
        report["notes"].append("Predictions are paused. Run badi resume when you want suggestions again.")
    elif broker and broker.get("metrics", {}).get("provider_calls") == 0:
        report["notes"].append("The model is ready but has received no prediction requests since startup. Run badi debug on and badi debug watch, then type in a supported field. Native manual fields require Tab; observed fields need matching accessibility and Fcitx context.")
    return report


def update_settings(change):
    document = json.loads(control(["settings", "show", "--json"]))
    revision = document["revision"]
    change(document)
    document["revision"] = revision + 1
    # Let the broker reject concurrent changes; never overwrite newer settings.
    return control(["settings", "replace", "--if-revision", str(revision),
                    "--json", json.dumps(document)])


def set_app(document, app, enabled):
    app_id = APP_IDS.get(app, app)
    if len(app_id) > 128 or not re.fullmatch(r"[a-z][a-z0-9_-]*(\.[a-z][a-z0-9_-]*)*", app_id):
        raise RuntimeError("Use the exact canonical app_id from badi debug status")
    identity = {"kind": "linux_app", "adapter": APP_ADAPTERS.get(app_id, "fcitx"), "app_id": app_id}
    subject = next((item for item in document["subjects"] if item["identity"] == identity), None)
    if subject is None:
        subject = {"identity": identity}
        document["subjects"].append(subject)
    decision = "allow" if enabled else "block"
    subject["permissions"] = {"context_read": decision, "display": decision, "suggest": decision,
                              "learn": "block", "retention": {"mode": "none"}}
    document["subjects"].sort(key=lambda item: identity_key(item["identity"]))


def set_site(document, value, enabled):
    parsed = urlsplit(value)
    if parsed.scheme not in ("http", "https") or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment or parsed.path not in ("", "/"):
        raise RuntimeError("Use an exact http(s) origin without a path or credentials")
    identity = {"kind": "browser_origin", "adapter": "chromium", "scheme": parsed.scheme,
                "host": parsed.hostname.encode("idna").decode("ascii"),
                "port": parsed.port if parsed.port is not None else (443 if parsed.scheme == "https" else 80)}
    if not 1 <= identity["port"] <= 65535:
        raise RuntimeError("Invalid origin port")
    subject = next((item for item in document["subjects"] if item["identity"] == identity), None)
    if subject is None:
        subject = {"identity": identity}
        document["subjects"].append(subject)
    decision = "allow" if enabled else "block"
    subject["permissions"] = {"context_read": decision, "display": decision, "suggest": decision,
                              "learn": "block", "retention": {"mode": "none"}}
    document["subjects"].sort(key=lambda item: identity_key(item["identity"]))


def identity_key(identity):
    if identity["kind"] == "browser_origin":
        return (0, identity["adapter"], identity["scheme"], identity["host"], identity["port"])
    return (1, identity["adapter"], identity["app_id"])


def registry_directory():
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    if not runtime or not Path(runtime).is_absolute():
        raise RuntimeError("Open Badi from your graphical desktop session")
    return Path(runtime) / "badi/native-trials"


def private_json(path, limit=8192):
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    with os.fdopen(descriptor) as stream:
        info = os.fstat(stream.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077 or info.st_size > limit:
            raise RuntimeError("Debug data must be a private, owned, bounded regular file")
        return json.load(stream)


def debug_activity():
    directory = registry_directory().parent
    report = {"enabled": False, "reason": "debug_off", "counts": {}}
    try:
        control = private_json(directory / "debug-control.json", 512)
        if not time.time() < control["expires_at"] <= time.time() + 900:
            return report
        report.update(enabled=True, expires_at=control["expires_at"], reason="no_input_events")
        report["editors"] = {}
        for adapter in ("obsidian", "terminal"):
            try:
                editor = private_json(directory / f"debug-{adapter}.json")
                if editor.get("id") == control["id"] and editor.get("schema") == "badi.editor-activity.v1":
                    report["editors"][adapter] = editor
                    report["reason"] = editor["reason"]
            except FileNotFoundError:
                pass
        snapshot = private_json(directory / "debug-native.json")
        if snapshot.get("id") == control["id"] and snapshot.get("schema") == "badi.native-activity.v1":
            report["native"] = snapshot
            report["reason"] = snapshot["reason"]
            report["counts"] = snapshot["counts"]
            report["age_seconds"] = max(0, int(time.time() - snapshot["at"]))
    except FileNotFoundError:
        pass
    except (ValueError, TypeError, KeyError, RuntimeError, OSError):
        report["reason"] = "invalid_debug_data"
    return report


def debug_line(report):
    model = report.get("model", {})
    metrics = model.get("metrics", {})
    native = report.get("native", {})
    adapters = ["native=" + native.get("reason", "no_events")]
    for name, activity in report.get("editors", {}).items():
        reads = activity.get("reason_counts", {}).get("sent context.changed", 0)
        adapters.append(f"{name}={activity.get('reason', 'unknown')}({reads} contexts)")
    return (f"{time.strftime('%H:%M:%S')}  {'debug' if report['enabled'] else 'debug off'}  "
            f"model={model.get('provider', 'offline')}  "
            f"input={native.get('counts', {}).get('input', 0)}  "
            f"requests={metrics.get('provider_calls', 0)}  displayed={metrics.get('suggestions_shown', 0)}  "
            f"errors={metrics.get('provider_errors', 0)}  " + "  ".join(adapters))


def debug_mode(mode):
    directory = registry_directory().parent
    if mode in ("on", "off"):
        directory.mkdir(mode=0o700, parents=True, exist_ok=True)
        info = directory.lstat()
        if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
            raise RuntimeError("Badi runtime directory must be private and owned by this user")
        if mode == "off":
            (directory / "debug-control.json").unlink(missing_ok=True)
            (directory / "debug-native.json").unlink(missing_ok=True)
            for adapter in ("obsidian", "terminal"):
                (directory / f"debug-{adapter}.json").unlink(missing_ok=True)
        else:
            with tempfile.NamedTemporaryFile(mode="w", dir=directory, delete=False) as stream:
                temporary = Path(stream.name)
                json.dump({"id": str(uuid.uuid4()), "expires_at": int(time.time()) + 900}, stream)
            temporary.replace(directory / "debug-control.json")
        print("Activity debug enabled for 15 minutes; no typed text is stored."
              if mode == "on" else "Activity debug disabled and its snapshot removed.")
        return
    while True:
        report = {"schema": "badi.debug.v1", **debug_activity()}
        try:
            health = json.loads(control(["status"]))
            report["model"] = {key: health[key] for key in ("provider", "paused", "sessions", "metrics")}
        except RuntimeError as error:
            report["model_error"] = str(error)
        print(json.dumps(report, indent=2) if mode == "status" else debug_line(report), flush=True)
        if mode != "watch" or not report["enabled"]:
            return
        time.sleep(1)


def sessions(directory):
    available = []
    for path in directory.glob("*.json"):
        try:
            info = path.lstat()
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077 or info.st_size > 4096:
                continue
            entry = json.loads(path.read_text())
            if entry["id"] != path.stem or entry["app"] not in APPS:
                continue
            socket = Path(entry["socket"])
            metadata = socket.lstat()
            if not socket.is_absolute() or not stat.S_ISSOCK(metadata.st_mode) or metadata.st_uid != os.getuid() or metadata.st_mode & 0o077:
                continue
            available.append((info.st_mtime_ns, entry))
        except (OSError, ValueError, KeyError, TypeError):
            continue
    entries = [entry for _, entry in sorted(available, key=lambda item: item[0], reverse=True)]
    endpoint = directory.parent / "broker.sock"
    try:
        metadata = endpoint.lstat()
        if stat.S_ISSOCK(metadata.st_mode) and metadata.st_uid == os.getuid() and not metadata.st_mode & 0o077:
            entries.insert(0, {"id": "desktop", "app": "desktop", "label": "Desktop writing", "socket": str(endpoint)})
    except OSError:
        pass
    return entries


def main(arguments):
    if arguments in ([], ["--help"], ["-h"]):
        print(HELP, end="")
        return
    if arguments == ["settings"]:
        subprocess.run(["omarchy-shell", "badi-writing", "toggle"], check=True, timeout=4)
        return
    if len(arguments) == 2 and arguments[0] == "debug" and arguments[1] in ("on", "off", "status", "watch"):
        debug_mode(arguments[1])
        return
    if arguments == ["settings", "--json"]:
        print(control(["settings", "show", "--json"]), end="")
        return
    if arguments in (["status"], ["status", "--json"]):
        health = json.loads(control(["status"]))
        if arguments[-1] == "--json":
            print(json.dumps(health))
        else:
            state = "Paused" if health["paused"] else "Model ready"
            counters = health["metrics"]
            print(f"Badi: {state} · {health['provider']}\n"
                  f"Requests: {counters['provider_calls']} · Suggestions: {counters['suggestions_shown']} · Errors: {counters['provider_errors']}\n"
                  "Adapters: Fcitx + optional field observer · Obsidian · Bash · optional web extension\n"
                  "Observed fields: automatic · Native manual: Tab request/accept · Editor plugins: Tab next word\n"
                  "Escape: dismiss · Bash: Ctrl-X then Tab\n"
                  "Input diagnostics: badi debug on; badi debug watch")
        return
    if arguments in (["pause"], ["resume"]):
        paused = arguments == ["pause"]
        update_settings(lambda document: document.update(paused=paused))
        print("Predictions paused." if paused else "Predictions resumed.")
        return
    if len(arguments) == 3 and arguments[0] == "app" and arguments[2] in ("on", "off"):
        update_settings(lambda document: set_app(document, arguments[1], arguments[2] == "on"))
        print(f"{arguments[1]} predictions {arguments[2]}.")
        return
    if len(arguments) == 3 and arguments[0] == "site" and arguments[2] in ("on", "off"):
        update_settings(lambda document: set_site(document, arguments[1], arguments[2] == "on"))
        print(f"{arguments[1]} predictions {arguments[2]}. The exact-origin rule applies to Badi's browser integrations.")
        return
    if arguments == ["service", "status"]:
        state = service_state()
        if state["active"] == "failed":
            state["startup_problem"] = startup_problem()
        print(json.dumps(state))
        return
    if len(arguments) == 2 and arguments[0] == "service" and arguments[1] in ("start", "stop", "restart"):
        if arguments[1] != "stop" and service_state()["active"] == "failed":
            subprocess.run(["systemctl", "--user", "reset-failed", SERVICE], check=True, timeout=4)
        subprocess.run(["systemctl", "--user", arguments[1], SERVICE], check=True, timeout=15)
        print(json.dumps(service_state()))
        return
    if len(arguments) == 2 and arguments[0] == "autostart" and arguments[1] in ("on", "off"):
        subprocess.run(["systemctl", "--user", "enable" if arguments[1] == "on" else "disable", SERVICE],
                       check=True, capture_output=True, text=True, timeout=8)
        print(json.dumps(service_state()))
        return
    if arguments == ["logs"]:
        subprocess.run(["journalctl", "--user", "-u", SERVICE, "-n", "60", "--no-pager"], check=True, timeout=5)
        return
    if arguments == ["doctor"]:
        report = health_report()
        print(json.dumps(report, indent=2))
        if report["problems"]:
            raise SystemExit(1)
        return
    if len(arguments) == 2 and arguments[0] == "launch" and arguments[1] in APPS:
        app = arguments[1]
        subprocess.run(["systemctl", "--user", "start", "badi-broker.service"], check=True, timeout=10)
        subprocess.run([
            "systemd-run", "--user", "--collect", "--quiet",
            f"--unit=badi-editor-{uuid.uuid4().hex}", "--property=Type=exec",
            f"--setenv=WAYLAND_DISPLAY={os.environ.get('WAYLAND_DISPLAY', '')}",
            f"--setenv=XDG_RUNTIME_DIR={os.environ.get('XDG_RUNTIME_DIR', '')}",
            "--setenv=GTK_IM_MODULE=fcitx", "--setenv=QT_IM_MODULE=fcitx",
            app,
        ], check=True, timeout=10)
        return
    if not arguments or arguments[0] != "ctl":
        raise RuntimeError("Unknown command. Run: badi --help")
    arguments = arguments[1:]
    trial_id = None
    if arguments[:1] == ["--trial"] and len(arguments) >= 3:
        trial_id, arguments = arguments[1], arguments[2:]
    available = sessions(registry_directory())
    entry = next((item for item in available if trial_id is None or item["id"] == trial_id), None)
    if entry is None:
        raise RuntimeError("The Badi model is offline. Open a supported editor below to start the service.")
    if arguments not in (["overview", "--json"], ["status"]) and trial_id is None:
        raise RuntimeError("Refresh Badi settings before changing a writing session")
    output = control(arguments, entry["socket"])
    if arguments == ["overview", "--json"]:
        overview = json.loads(output)
        counters = json.loads(control(["status"], entry["socket"]))["metrics"]
        overview["desktop"] = {
            "id": entry["id"], "app": entry["app"],
            "sessions": [{"id": item["id"], "label": item.get("label", item["app"])} for item in available],
            "requests": counters["provider_calls"], "suggestions": counters["suggestions_shown"],
            "errors": counters["provider_errors"],
            "activity": debug_activity(),
        }
        print(json.dumps(overview))
    else:
        print(output, end="")


if __name__ == "__main__":
    try:
        main(sys.argv[1:])
    except KeyboardInterrupt:
        sys.exit(130)
    except (RuntimeError, OSError, ValueError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
