#!/usr/bin/env python3
"""Control the persistent desktop broker, its settings and its diagnostics."""

import importlib.util
import json
import os
import re
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import time
import uuid
from urllib.parse import urlsplit

# Installed beside this CLI (~/.local/lib/badi); in the checkout, beside it in scripts/.
import badi_install

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
  badi apps allowlist|blocklist  Apps: only allowed apps (default), or every
                               supported app except blocked ones
  badi sites allowlist|blocklist Sites: only allowed sites (default), or every
                               http(s) site except blocked ones; covers
                               Chromium/Brave/Zen fields
  badi app APP_ID on|off|reset  Allow or block one app; reset follows the mode
  badi site ORIGIN on|off|reset Allow or block one exact browser origin
  badi app list | site list     Show the mode and every app or site rule
  badi app all on|off           Same as badi apps blocklist|allowlist
  badi site all on|off          Same as badi sites blocklist|allowlist
  badi service start|stop|restart|status
                               Manage the local model process
  badi autostart on|off         Start with the graphical session (next login)
  badi doctor                  Inspect service, model, install identity and
                               no-suggestion reasons as JSON
  badi debug on|off|status|watch
                               Trace activity for 15 minutes, without typed text
  badi logs                    Print the last 60 service log entries
  badi launch omawrite|xournalpp
                               Open a supported editor

Keys: Tab accepts a visible suggestion and otherwise stays Tab; Escape
dismisses; Ctrl+Shift+Space requests. Suggestions appear on their own in
Omawrite, Telegram and the IME-parity apps (Chromium, Brave and its web
apps, Zen, Codex, VS Code, Cursor, Discord, Grok Bot, LibreOffice Writer),
each with an app or site grant; one site grant covers all three browsers.
IME-parity accepts once, append-only, like typing: undo may merge it with
earlier typing.
Xournal++ text cells: Tab requests, Tab again accepts.
Obsidian: Tab accepts a word, Ctrl/Command+Right all.
Bash: Ctrl-X then Tab requests/accepts, Ctrl-X then Escape dismisses.
For protocol commands: badictl --help
"""
VSCODE_SETTINGS = Path(".config/Code/User/settings.json")
CLOSING = re.compile(r"\s*[}\]]")
ALL_SITES_NOTE = ("Every http(s) site is allowed for predictions unless its exact site rule blocks it "
                  "(badi site all off to return to listed sites). This includes "
                  "Chromium/Brave/Zen fields, which have no second site gate and cannot exclude private "
                  "windows. Sensitive fields stay denied.")
ALL_APPS_NOTE = ("Apps use the blocklist: every app Badi supports is allowed unless its rule blocks it "
                 "(badi apps allowlist to return to allowed apps only). The default opens only fields the "
                 "accessibility observer corroborates; a manual Tab-request app still needs badi app APP on.")
INSTALLED_BROKER = ".local/lib/badi/badi-broker"
# Broker no-suggestion classes; counters never include typed text.
NO_SUGGESTION = {
    "request_abstained": "the field language is missing or unsupported, text follows the caret, nothing but spaces (or one unfinished English word) precedes it, or a Persian joiner is not yet between two letters",
    "budget_prefill": "the writing budget (550 ms while typing, 1.2 s after Tab) ended before the model started answering (prompt prefill)",
    "budget_stream": "the writing budget (550 ms while typing, 1.2 s after Tab) ended before a complete word arrived",
    "model_abstained": "the model finished without a complete word",
    "output_rejected": "the output failed language, dictionary, number, shape or safety checks",
    "stale": "the text, focus or pause state changed before display",
    "timeout": "the broker deadline (600 ms while typing, 1.25 s after Tab) expired",
    "provider_error": "the local model runtime failed or answered malformed",
}


def cli_path():
    installed = Path.home() / ".local/lib/badi/badictl"
    return installed if installed.is_file() else ROOT / "target/release/badictl"


def control(arguments, endpoint=None):
    endpoint = endpoint or runtime_directory() / "broker.sock"
    result = subprocess.run([str(cli_path()), "--socket", str(endpoint), *arguments],
                            capture_output=True, text=True, timeout=8)
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or "Badi is offline. Run: badi service start")
    return result.stdout


def service_state():
    fields = badi_install.unit_properties(SERVICE, "LoadState", "ActiveState", "SubState", "UnitFileState", timeout=4)
    return {"loaded": fields.get("LoadState") == "loaded",
            "active": fields.get("ActiveState", "unknown"),
            "state": fields.get("SubState", "unknown"),
            "autostart": fields.get("UnitFileState") == "enabled"}


def native_state():
    fields = badi_install.unit_properties("omarchy-fcitx5.service", "ActiveState", "MainPID", timeout=4)
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
        identifier = badi_install.unit_value(SERVICE, "InvocationID")
        if not re.fullmatch(r"[a-f0-9]{32}", identifier):
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
                        "action": "The desktop service restarts it automatically with backoff. If it keeps stopping, inspect badi logs and available memory."}
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
    except (RuntimeError, OSError, subprocess.SubprocessError):
        pass
    return None


def observer_service_state():
    fields = badi_install.unit_properties(ACCESSIBILITY_SERVICE, "LoadState", "ActiveState", "MainPID")
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
            endpoint = runtime_directory() / "accessibility.sock"
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


def no_suggestion_summary(metrics):
    """Summarize content-free no-suggestion classes; None for older brokers."""
    breakdown = metrics.get("no_suggestion") if isinstance(metrics, dict) else None
    if not isinstance(breakdown, dict):
        return None
    counts = {reason: breakdown[reason] for reason in NO_SUGGESTION
              if isinstance(breakdown.get(reason), int) and breakdown[reason] > 0}
    last = breakdown.get("last") if breakdown.get("last") in NO_SUGGESTION else None
    return {"total": sum(counts.values()), "counts": counts, "last": last}


def receipt(name):
    """Read one private install receipt; None when that installer never ran."""
    try:
        document = private_json(badi_install.receipt_path(Path.home(), name), 1 << 20)
    except FileNotFoundError:
        return None
    except (RuntimeError, ValueError, OSError):
        return {"error": "receipt_unreadable"}
    if not isinstance(document, dict) or document.get("schema") != badi_install.SCHEMA \
            or not isinstance(document.get("source"), dict) or not isinstance(document.get("files"), dict):
        return {"error": "receipt_invalid"}
    return document


def running_broker_identity():
    """Hash and version the exact executable image of the running service."""
    def main_pid():
        try:
            value = badi_install.unit_value(SERVICE, "MainPID")
        except RuntimeError:
            return 0
        return int(value) if value.isdigit() else 0

    pid = main_pid()
    if pid <= 0:
        return None
    process = Path(f"/proc/{pid}")
    try:
        if process.stat().st_uid != os.getuid():
            return {"error": "running_broker_uninspectable"}
        digest = badi_install.file_sha256(process / "exe")
        # /proc/PID/exe runs the image in memory even after an update replaced
        # the installed file, so this is the running build, not the file on disk.
        version = subprocess.run([str(process / "exe"), "--version"], capture_output=True, text=True, timeout=3)
    except (OSError, subprocess.SubprocessError):
        return {"error": "running_broker_uninspectable"}
    if main_pid() != pid:
        return {"error": "service_changed"}
    line = version.stdout.strip() if version.returncode == 0 else ""
    return {"version": line if badi_install.VERSION_LINE.fullmatch(line) else None, "sha256": digest}


def install_state():
    state = {}
    expected = None
    for name in ("desktop", "editors"):
        document = receipt(name)
        if document is None or "error" in document:
            state[name] = document
            continue
        state[name] = {"installed_at": document.get("installed_at"), **document["source"]}
        if name == "desktop":
            expected = document["files"].get(INSTALLED_BROKER)
            state[name]["broker_version"] = expected.get("version") if isinstance(expected, dict) else None
    try:
        running = running_broker_identity()
    except (OSError, ValueError, subprocess.SubprocessError):
        running = {"error": "running_broker_uninspectable"}
    if running and "error" not in running and isinstance(expected, dict):
        running["version_matches_receipt"] = running["version"] is not None and running["version"] == expected.get("version")
        running["sha256_matches_receipt"] = running["sha256"] == expected.get("sha256")
    state["running_broker"] = running
    return state


def jsonc_object(text):
    """Decode JSON with comments and trailing commas, as VS Code settings allow."""
    def scan(source, skipped):
        kept, index, quoted = [], 0, False
        while index < len(source):
            character = source[index]
            if quoted or character == '"':
                kept.append(character)
                if quoted and character == "\\":
                    kept.append(source[index + 1:index + 2])
                    index += 2
                    continue
                quoted = character != '"' if quoted else True
                index += 1
                continue
            length = skipped(source, index)
            kept.append(" " if length else character)
            index += length or 1
        return "".join(kept)

    def comment(source, index):
        if source.startswith("//", index):
            end = source.find("\n", index)
            return (len(source) if end < 0 else end) - index
        if source.startswith("/*", index):
            end = source.find("*/", index + 2)
            if end < 0:
                raise ValueError("unterminated comment")
            return end + 2 - index
        return 0

    def trailing_comma(source, index):
        return int(source[index] == "," and CLOSING.match(source, index + 1) is not None)

    return json.loads(scan(scan(text, comment), trailing_comma))


def vscode_edit_context_note():
    """Read-only check of VS Code's EditContext switch; never returns file text."""
    path = Path.home() / VSCODE_SETTINGS
    if not (shutil.which("code") or path.parents[1].is_dir()):
        return None
    fix = 'add "editor.editContext": false to ~/.config/Code/User/settings.json and reload VS Code'
    try:
        with open(path, "rb") as stream:
            raw = stream.read((1 << 20) + 1)
        if len(raw) > 1 << 20:
            raise ValueError("settings too large")
        # Like VS Code: an optional BOM, and blank means {}.
        text = raw.decode("utf-8-sig")
        settings = jsonc_object(text) if text.strip() else {}
        if not isinstance(settings, dict):
            raise ValueError("settings are not an object")
    except FileNotFoundError:
        settings = {}
    except (OSError, ValueError, UnicodeDecodeError):
        return ("VS Code is installed, but Badi could not read its user settings to check EditContext. "
                f"If Badi misreads text in VS Code, {fix}.")
    if settings.get("editor.editContext") is False:
        return None
    return ("VS Code is installed with EditContext on (its default). EditContext sends Fcitx corrupted "
            f"surrounding text, so Badi cannot read the text before the caret there; {fix}.")


def health_report():
    report = {"schema": "badi.desktop-health.v1", "service": service_state(),
              "native_apps": list(APPS), "problems": [], "notes": []}
    report["notes"].append("Chromium, Brave and its web apps, Zen, Codex, VS Code, Cursor, Discord, Grok Bot and LibreOffice Writer fields use IME-parity: with the field observer and an app or site grant, one append-only acceptance behaves like typed text, so undo may merge it with earlier typing and page script may redirect it.")
    problems = report["problems"]
    observer = report["accessibility"] = accessibility_state()
    if observer.get("loaded"):
        if not observer["ready"]:
            problems.append({"code": "accessibility_unavailable",
                             "message": "The accessibility field observer is unavailable.",
                             "action": "Inspect systemctl --user status badi-accessibility.service and the accessibility runbook."})
        elif observer.get("bus_enabled") is False:
            problems.append({"code": "accessibility_disabled",
                             "message": "Desktop accessibility is disabled, so applications may expose no fields.",
                             "action": "Complete the accessibility setup and relaunch the target application with its supported accessibility/input-method flags."})
    else:
        report["notes"].append("The accessibility field observer is not installed or could not be inspected, so IME-parity apps and observed fields get no suggestions. Model readiness alone does not establish those integrations.")
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
        problems.append({"code": "service_failed", "message": "The Badi service stopped on a startup error that retrying cannot fix.",
                         "action": "Resolve the startup problem, then run badi service restart."})
    if report.get("broker", {}).get("control_plane_degraded"):
        problems.append({"code": "settings_degraded", "message": "The broker cannot use its persistent settings safely.",
                         "action": "Inspect badi logs and restore valid private settings before enabling predictions."})
    if report["native"].get("inspection_error"):
        problems.append({"code": "native_status_unknown", "message": "Native input integration could not be inspected.",
                         "action": "Inspect the Fcitx user service from the graphical session. Browser and editor broker health is reported separately."})
    elif report["native"]["addon_loaded"] is False:
        problems.append({"code": "native_addon_unavailable", "message": "The running Fcitx service has not loaded Badi.",
                         "action": "Check the native installation in the Badi runbook. Native manual, observed and IME-parity fields all need the loaded addon."})
    install = report["install"] = install_state()
    running = install.get("running_broker") or {}
    if install.get("desktop") is None:
        report["notes"].append("No desktop install receipt exists, so the installed files cannot be matched to a source commit. The next scripts/install-desktop.py run records one.")
    elif "error" in install["desktop"]:
        report["notes"].append("The desktop install receipt is unreadable or not private; installed files cannot be matched to a source commit.")
    elif running.get("sha256_matches_receipt") is False or running.get("version_matches_receipt") is False:
        problems.append({"code": "broker_build_mismatch", "message": "The running broker is not the build recorded by the last desktop installation.",
                         "action": "Run badi service restart after an installation; otherwise reinstall with scripts/install-desktop.py."})
    broker = report.get("broker", {})
    if report["native"].get("addon_update_pending"):
        report["notes"].append("Fcitx is still using the previous addon build. Apply the native update after normal desktop unlock.")
    if broker.get("paused"):
        report["notes"].append("Predictions are paused. Run badi resume when you want suggestions again.")
    elif broker and broker.get("metrics", {}).get("provider_calls") == 0:
        report["notes"].append("The model is ready but has received no prediction requests since startup. Run badi debug on and badi debug watch, then type in a supported field. Native manual fields require Tab; observed fields need matching accessibility and Fcitx context.")
    settings = web_settings() if broker else {}
    if settings.get("all_web_origins") is True:
        report["notes"].append(ALL_SITES_NOTE)
    if settings.get("all_linux_apps") is True:
        report["notes"].append(ALL_APPS_NOTE)
    editor = vscode_edit_context_note()
    if editor:
        report["notes"].append(editor)
    if broker:
        summary = no_suggestion_summary(broker.get("metrics", {}))
        if summary is None:
            report["notes"].append("The running broker predates no-suggestion reason counters. Install and restart the current build to see why suggestions are missing.")
        elif summary["total"]:
            detail = "; ".join(f"{reason}={count}: {NO_SUGGESTION[reason]}"
                               for reason, count in sorted(summary["counts"].items(), key=lambda item: -item[1]))
            report["notes"].append(f"{summary['total']} model request(s) since broker start showed no suggestion. {detail}. Last: {summary['last']}.")
    return report


def web_settings():
    """The current settings document, or {} when it cannot be read."""
    try:
        document = json.loads(control(["settings", "show", "--json"]))
    except (RuntimeError, OSError, ValueError, subprocess.SubprocessError):
        return {}
    return document if isinstance(document, dict) else {}


def status_text(health, settings=None):
    state = "Paused" if health["paused"] else "Model ready"
    counters = health["metrics"]
    summary = no_suggestion_summary(counters)
    if summary is None:
        misses = "No-suggestion reasons: not reported by this broker"
    elif summary["total"]:
        misses = f"No suggestion: {summary['total']} (last: {summary['last']})"
    else:
        misses = "No suggestion: 0"
    return (f"Badi: {state} · {health['provider']}\n"
            f"Requests: {counters['provider_calls']} · Suggestions: {counters['suggestions_shown']} · Errors: {counters['provider_errors']} · {misses}\n"
            "Native Fcitx: automatic in Omawrite; Tab request/accept in the Xournal++ Text tool\n"
            "Editors: Obsidian automatic/Tab · Bash Ctrl-X then Tab\n"
            "Observed fields: automatic for Omawrite, Telegram and IME-parity apps (Chromium, Brave and its web apps, Zen, Codex, VS Code, Cursor, Discord, Grok Bot, LibreOffice Writer); Tab accepts a visible suggestion, otherwise stays Tab; Ctrl+Shift+Space requests\n"
            + f"Apps: {mode_text((settings or {}).get('all_linux_apps'), 'app')}\n"
            + f"Web sites: {mode_text((settings or {}).get('all_web_origins'), 'site')}\n"
            + "Escape: dismiss · Why nothing appeared: badi doctor; badi debug on; badi debug watch")


def update_settings(change):
    document = json.loads(control(["settings", "show", "--json"]))
    revision = document["revision"]
    change(document)
    document["revision"] = revision + 1
    # Let the broker reject concurrent changes; never overwrite newer settings.
    return control(["settings", "replace", "--if-revision", str(revision),
                    "--json", json.dumps(document)])


def set_app(document, app, enabled):
    identity = app_identity(app)
    subject = next((item for item in document["subjects"] if item["identity"] == identity), None)
    if subject is None:
        subject = {"identity": identity}
        document["subjects"].append(subject)
    subject["permissions"] = badi_install.grant("allow" if enabled else "block")
    document["subjects"].sort(key=lambda item: identity_key(item["identity"]))


def set_mode(document, key, blocklist):
    # Absent is the canonical allowlist state; exact rules are left untouched.
    if blocklist:
        document[key] = True
    else:
        document.pop(key, None)


def mode_text(blocklist, kind):
    if blocklist is True:
        return ("blocklist: every supported app except blocked ones" if kind == "app"
                else "blocklist: every http(s) site except blocked ones")
    return "allowlist: only allowed apps" if kind == "app" else "allowlist: only allowed sites"


def remove_rule(document, identity):
    document["subjects"] = [item for item in document["subjects"] if item["identity"] != identity]


def app_identity(app):
    app_id = APP_IDS.get(app, app)
    if len(app_id) > 128 or not re.fullmatch(r"[a-z][a-z0-9_-]*(\.[a-z][a-z0-9_-]*)*", app_id):
        raise RuntimeError("Use the exact canonical app_id from badi debug status")
    return {"kind": "linux_app", "adapter": APP_ADAPTERS.get(app_id, "fcitx"), "app_id": app_id}


def rule_state(permissions):
    decisions = {permissions.get(key) for key in ("context_read", "display", "suggest")}
    return "allowed" if decisions == {"allow"} else "blocked" if decisions == {"block"} else "limited"


def origin_text(identity):
    default = 443 if identity["scheme"] == "https" else 80
    return f"{identity['scheme']}://{identity['host']}" + ("" if identity["port"] == default else f":{identity['port']}")


def rule_list(document, kind):
    key, label = ("all_linux_apps", "app") if kind == "app" else ("all_web_origins", "site")
    lines = [("Apps" if kind == "app" else "Sites") + ": " + mode_text(document.get(key), label)]
    for item in document.get("subjects", []):
        identity = item["identity"]
        if kind == "app" and identity["kind"] == "linux_app":
            lines.append(f"  {rule_state(item['permissions']):8} {identity['app_id']}")
        elif kind == "site" and identity["kind"] == "browser_origin":
            lines.append(f"  {rule_state(item['permissions']):8} {origin_text(identity)}")
    if len(lines) == 1:
        lines.append("  (no rules)")
    return "\n".join(lines)


def site_identity(value):
    parsed = urlsplit(value)
    if parsed.scheme not in ("http", "https") or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment or parsed.path not in ("", "/"):
        raise RuntimeError("Use an exact http(s) origin without a path or credentials")
    identity = {"kind": "browser_origin", "adapter": "chromium", "scheme": parsed.scheme,
                "host": parsed.hostname.encode("idna").decode("ascii"),
                "port": parsed.port if parsed.port is not None else (443 if parsed.scheme == "https" else 80)}
    if not 1 <= identity["port"] <= 65535:
        raise RuntimeError("Invalid origin port")
    return identity


def set_site(document, value, enabled):
    identity = site_identity(value)
    subject = next((item for item in document["subjects"] if item["identity"] == identity), None)
    if subject is None:
        subject = {"identity": identity}
        document["subjects"].append(subject)
    subject["permissions"] = badi_install.grant("allow" if enabled else "block")
    document["subjects"].sort(key=lambda item: identity_key(item["identity"]))


def identity_key(identity):
    if identity["kind"] == "browser_origin":
        return (0, identity["adapter"], identity["scheme"], identity["host"], identity["port"])
    return (1, identity["adapter"], identity["app_id"])


def runtime_directory():
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    if not runtime or not Path(runtime).is_absolute():
        raise RuntimeError("Open Badi from your graphical desktop session")
    return Path(runtime) / "badi"


def private_json(path, limit=8192):
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    with os.fdopen(descriptor) as stream:
        info = os.fstat(stream.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077 or info.st_size > limit:
            raise RuntimeError("Debug data must be a private, owned, bounded regular file")
        return json.load(stream)


def debug_activity():
    directory = runtime_directory()
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
    summary = no_suggestion_summary(metrics)
    if summary and summary["total"]:
        adapters.insert(0, f"no_show={summary['total']}(last={summary['last']})")
    return (f"{time.strftime('%H:%M:%S')}  {'debug' if report['enabled'] else 'debug off'}  "
            f"model={model.get('provider', 'offline')}  "
            f"input={native.get('counts', {}).get('input', 0)}  "
            f"requests={metrics.get('provider_calls', 0)}  displayed={metrics.get('suggestions_shown', 0)}  "
            f"errors={metrics.get('provider_errors', 0)}  " + "  ".join(adapters))


def debug_mode(mode):
    directory = runtime_directory()
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
            marker = {"id": str(uuid.uuid4()), "expires_at": int(time.time()) + 900}
            badi_install.atomic_write(directory / "debug-control.json", json.dumps(marker).encode())
        print("Activity debug enabled for 15 minutes; no typed text is stored."
              if mode == "on" else "Activity debug disabled and its snapshot removed.")
        return
    while True:
        report = {"schema": "badi.debug.v1", **debug_activity()}
        try:
            health = json.loads(control(["status"]))
            report["model"] = {key: health[key] for key in ("provider", "paused", "sessions", "metrics")}
            summary = no_suggestion_summary(health["metrics"])
            if summary and summary["last"]:
                summary["last_meaning"] = NO_SUGGESTION[summary["last"]]
            report["model"]["no_suggestion"] = summary
        except RuntimeError as error:
            report["model_error"] = str(error)
        print(json.dumps(report, indent=2) if mode == "status" else debug_line(report), flush=True)
        if mode != "watch" or not report["enabled"]:
            return
        time.sleep(1)


def desktop_socket():
    """The persistent broker's private socket, or None while it is offline."""
    endpoint = runtime_directory() / "broker.sock"
    try:
        metadata = endpoint.lstat()
    except OSError:
        return None
    if stat.S_ISSOCK(metadata.st_mode) and metadata.st_uid == os.getuid() and not metadata.st_mode & 0o077:
        return endpoint
    return None


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
            print(status_text(health, web_settings()))
        return
    if arguments in (["pause"], ["resume"]):
        paused = arguments == ["pause"]
        update_settings(lambda document: document.update(paused=paused))
        print("Predictions paused." if paused else "Predictions resumed.")
        return
    modes = {("apps", "allowlist"): ("all_linux_apps", False), ("apps", "blocklist"): ("all_linux_apps", True),
             ("sites", "allowlist"): ("all_web_origins", False), ("sites", "blocklist"): ("all_web_origins", True),
             ("app", "all", "off"): ("all_linux_apps", False), ("app", "all", "on"): ("all_linux_apps", True),
             ("site", "all", "off"): ("all_web_origins", False), ("site", "all", "on"): ("all_web_origins", True)}
    if tuple(arguments) in modes:
        key, blocklist = modes[tuple(arguments)]
        update_settings(lambda document: set_mode(document, key, blocklist))
        if key == "all_web_origins":
            print(ALL_SITES_NOTE if blocklist else "Sites use the allowlist: only sites with their own allow rule.")
        else:
            print(ALL_APPS_NOTE if blocklist else "Apps use the allowlist: only apps with their own allow rule.")
        return
    if arguments in (["app", "list"], ["site", "list"]):
        print(rule_list(json.loads(control(["settings", "show", "--json"])), arguments[0]))
        return
    if len(arguments) == 3 and arguments[0] == "app" and arguments[2] in ("on", "off"):
        update_settings(lambda document: set_app(document, arguments[1], arguments[2] == "on"))
        print(f"{arguments[1]} predictions {arguments[2]}.")
        return
    if len(arguments) == 3 and arguments[0] == "site" and arguments[2] in ("on", "off"):
        update_settings(lambda document: set_site(document, arguments[1], arguments[2] == "on"))
        print(f"{arguments[1]} predictions {arguments[2]}. The exact-origin rule applies to Badi's browser integrations.")
        return
    if len(arguments) == 3 and arguments[0] in ("app", "site") and arguments[2] == "reset":
        identity = app_identity(arguments[1]) if arguments[0] == "app" else site_identity(arguments[1])
        update_settings(lambda document: remove_rule(document, identity))
        print(f"{arguments[1]} has no rule now and follows the {arguments[0]} list mode.")
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
    endpoint = desktop_socket()
    if endpoint is None:
        raise RuntimeError("The Badi model is offline. Open a supported editor below to start the service.")
    output = control(arguments, endpoint)
    if arguments == ["overview", "--json"]:
        overview = json.loads(output)
        counters = json.loads(control(["status"], endpoint))["metrics"]
        overview["desktop"] = {
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
