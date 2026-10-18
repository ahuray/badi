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

# Installed beside this CLI; in the checkout, beside it in scripts/.
import badi_install

LIBRARY = Path(__file__).resolve().parent
ROOT = LIBRARY.parent
APPS = ("xournalpp", "omawrite")
APP_IDS = {"omawrite": "omawrite", "xournalpp": "com.github.xournalpp.xournalpp"}
APP_ADAPTERS = {"obsidian": "obsidian", "bash": "shell", "nvim": "neovim", "libreoffice-writer": "libreoffice"}
SERVICE = "badi-broker.service"
ACCESSIBILITY_SERVICE = "badi-accessibility.service"
SESSION_SERVICE = "badi-session.service"
BATTERY_TEXT = {
    "normal": "On battery, automatic suggestions behave as on mains power.",
    "quiet": ("On battery or in the power-saver profile, the model starts only after a typing pause. "
              "Explicit requests are unchanged."),
    "explicit": ("On battery or in the power-saver profile, suggestions appear only when you ask "
                 "(Ctrl+Shift+Space, Tab in manual fields). Explicit requests are unchanged."),
}
DESKTOP_AUTOMATIC = {"normal": "as on mains", "deferred": "after the typing pause",
                     "explicit_only": "only when asked"}
HELP = """Badi — local writing controls

  badi status [--json]          Model health and counters
  badi settings                Open the Omarchy settings panel
  badi settings --json          Read the current settings document
  badi pause | resume          Persistently pause or resume predictions
  badi pause toggle            Pause if running, resume if paused (the
                               badi-session shortcut runs this)
  badi battery quiet|normal|explicit
                               Automatic suggestions on battery or in the
                               power-saver profile: start the model only
                               after a typing pause (default), as on mains,
                               or only when asked. Needs badi-session
  badi mid-line on|off         Suggestions with a word after the caret (off
                               by default). Punctuation can still finish a word
  badi escape-quiet SECONDS|off How long a dismissal keeps its field quiet
                               (1-3600 seconds, across edits; off keeps quiet
                               only until the text changes, the default)
  badi style on|off|status      Opt-in style learning: keep the sentences you
                               write before accepting a suggestion, sealed
                               with a key in your keyring; no prompt uses
                               them yet
  badi style site ORIGIN on [DAYS]|off
                               Learn on one allowed site, kept DAYS (1-90,
                               default 30)
  badi style export | forget   Print the samples as JSON, or delete them and
                               the keyring key
  badi model [--json]          Writing models: the one this device runs, the
                               recommendation for it, and every choice
  badi model use ID            Download (pinned, hash-checked) and switch to
                               a model, for example gemma-3-1b
  badi model auto              Return to the device recommendation
  badi spelling [LANG on|off] [--json]
                               Spelling languages (en de fr es it nl pl) and
                               their dictionaries; English is built in
  badi suggest [--json]        Suggestion languages and their healing
  badi suggest LANG on|off     Suggest words in a language (en de fa; fr es
                               are experimental)
  badi suggest LANG healing on|off|auto
                               Heal a trailing space in that language, keep
                               it, or follow the model's reviewed default
  badi language APP en|de|fa|fr|es|reset
                               Override one app's writing language; reset
                               restores automatic language selection
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
  badi doctor [--json]         Inspect service, model, install identity and
                               no-suggestion reasons; --json is for tools
  badi debug on [--text] | off  Trace every suggestion, key decision and ghost
                               display for 15 minutes; --text also records
                               context and suggestion text (RAM only)
  badi debug watch | log [--json] | status
                               Follow the trace live, print it, or summarize
  badi logs                    Print the last 60 service log entries
  badi launch omawrite|xournalpp
                               Open a supported editor

Keys: Tab accepts the next word of a visible suggestion and otherwise stays
Tab; Ctrl+Right accepts all of it; typing its next letters keeps the rest;
Escape dismisses; Ctrl+Shift+Space requests. Suggestions appear on their own in
Omawrite, Telegram and the IME-parity apps ({IME_APPS}),
each with an app or site grant; one site grant covers all three browsers.
IME-parity accepts once, append-only, like typing: undo may merge it with
earlier typing.
Xournal++ text cells: Tab requests, Tab again accepts a word.
Obsidian: Tab accepts a word, Ctrl/Command+Right all.
Bash: Ctrl-X then Tab requests/accepts, Ctrl-X then Escape dismisses.
Neovim (badi app nvim on): prose buffers suggest after a typing pause; Tab
accepts a word, Ctrl+Right accepts all, leaving Insert mode dismisses.
LibreOffice Writer spelling extension (badi app libreoffice-writer on): Alt+F7
shows a correction for the word before the caret, Alt+F7 again replaces it in
one undo step, Escape dismisses. badi app libreoffice covers the grey preview.
For protocol commands: badictl --help
"""
# Apps whose fields work today. Discord still needs a private composer trial
# with desktop-wide accessibility; README "Not yet" is the same list.
WORKING_IME_APPS = (
    "Chromium", "Brave and its web apps", "Zen", "Codex", "VS Code", "Cursor",
    "Grok Bot", "LibreOffice Writer",
)
IME_APPS = ", ".join(WORKING_IME_APPS)
HELP = HELP.format(IME_APPS=IME_APPS)
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
    "request_abstained": "the field language is missing or unsupported, the caret splits a word, a word follows the caret while mid-sentence suggestions are off, nothing but spaces (or one unfinished English word) precedes it, or a Persian joiner is not yet between two letters",
    "budget_prefill": "the writing budget (550 ms while typing, 1.2 s after Tab) ended before the model started answering (prompt prefill)",
    "budget_stream": "the writing budget (550 ms while typing, 1.2 s after Tab) ended before a complete word arrived",
    "model_abstained": "the model finished without a complete word",
    "output_rejected": "the output failed language, dictionary, number, shape or safety checks",
    "stale": "the text, focus or pause state changed before display",
    "timeout": "the broker deadline (600 ms while typing, 1.25 s after Tab) expired",
    "provider_error": "the local model runtime failed or answered malformed",
}


def cli_path():
    companion = LIBRARY / "badictl"
    if companion.is_file():
        return companion
    installed = Path.home() / ".local/lib/badi/badictl"
    return installed if installed.is_file() else ROOT / "target/release/badictl"


def control(arguments, endpoint=None, *, input_text=None):
    endpoint = endpoint or runtime_directory() / "broker.sock"
    result = subprocess.run([str(cli_path()), "--socket", str(endpoint), *arguments],
                            input=input_text, capture_output=True, text=True, timeout=8)
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
            if line.startswith("error_code=control_plane:"):
                name = line.removeprefix("error_code=control_plane:").strip()
                if name in {"unsafe_file", "unsafe_directory", "json", "validation", "missing_home"}:
                    return {"code": "settings_" + name,
                            "message": "Badi's private settings cannot be used, so the service has stopped instead of retrying.",
                            "action": "Repair or replace ~/.config/badi/settings.json and its directory permissions, then badi service restart."}
                return {"code": "control_plane",
                        "message": "The broker could not open its private settings.",
                        "action": "Run badi logs, then badi service restart if the settings directory is intact."}
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
    path = next((candidate for candidate in (
        LIBRARY / "accessibility/health.py", ROOT / "adapters/accessibility/health.py",
        Path.home() / ".local/lib/badi/accessibility/health.py",
    ) if candidate.is_file()), None)
    if path is None:
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


def session_helper_state():
    """The desktop session helper's unit; it reports to the broker itself."""
    try:
        fields = badi_install.unit_properties(SESSION_SERVICE, "LoadState", "ActiveState", timeout=4)
    except (RuntimeError, OSError, ValueError, subprocess.SubprocessError):
        return {"loaded": None, "active": "unknown"}
    return {"loaded": fields.get("LoadState") == "loaded", "active": fields.get("ActiveState", "unknown")}


def desktop_text(desktop, settings=None):
    """One content-free line about the session helper's report, or None."""
    if not isinstance(desktop, dict):
        return None
    if not desktop.get("observed"):
        return "Desktop: no session helper (suspend and battery modes need badi-session)"
    mode = (settings or {}).get("battery_mode", "quiet")
    power = "on battery or power saver" if desktop.get("low_power") else "on mains power"
    automatic = DESKTOP_AUTOMATIC.get(desktop.get("automatic"), "unknown")
    return f"Desktop: session {desktop.get('session', 'unknown')} · {power} · battery mode {mode} · automatic {automatic}"


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
    report["notes"].append(f"{IME_APPS} fields use IME-parity: with the field observer and an app or site grant, one append-only acceptance behaves like typed text, so undo may merge it with earlier typing and page script may redirect it. Discord still needs a private composer trial with desktop-wide accessibility.")
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
    helper = report["session_helper"] = session_helper_state()
    desktop = broker.get("desktop") if broker else None
    if broker and helper.get("loaded") is False:
        report["notes"].append("The desktop session helper (badi-session) is not installed, so suspend, session switches and "
                               "battery modes are not reported to the broker. Adapters keep their own focus, lock and expiry checks.")
    elif isinstance(desktop, dict) and desktop.get("session") in ("inactive", "sleeping"):
        report["notes"].append("The session helper reports this login session as not in the foreground, so every grant is held "
                               "until it returns.")
    elif isinstance(desktop, dict) and desktop.get("automatic") in ("deferred", "explicit_only"):
        report["notes"].append("On battery or in the power-saver profile, automatic suggestions start "
                               + DESKTOP_AUTOMATIC[desktop["automatic"]] + " (badi battery normal restores mains behavior).")
    if report["native"].get("addon_update_pending"):
        report["notes"].append("Fcitx is still using the previous addon build. Apply the native update after normal desktop unlock.")
    if broker.get("paused"):
        report["notes"].append("Predictions are paused. Run badi resume when you want suggestions again.")
    elif broker and broker.get("metrics", {}).get("provider_calls") == 0:
        report["notes"].append("The model is ready but has received no prediction requests since startup. Run badi debug on and badi debug watch, then type in a supported field. Native manual fields require Tab; observed fields need matching accessibility and Fcitx context.")
    settings = web_settings() if broker else {}
    if settings.get("all_web_origins") is True:
        report["notes"].append(ALL_SITES_NOTE)
    if settings.get("style_learning") is True:
        try:
            style = json.loads(control(["style", "status"]))
        except (RuntimeError, OSError, ValueError, subprocess.SubprocessError):
            style = {}
        if style.get("key") != "ready" or style.get("available") is not True:
            report["notes"].append("Style learning is on, but the broker cannot seal samples (its keyring key is "
                                   "missing, locked or different, or the store is unreadable), so nothing is kept. "
                                   "Unlock the keyring or run badi style on; badi style status explains which.")
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


def doctor_text(report):
    """Render the same diagnosis as JSON without dumping internal identifiers."""
    service = report.get("service", {})
    broker = report.get("broker", {})
    native = report.get("native", {})
    observer = report.get("accessibility", {})
    problems = report["problems"]
    model = ("paused" if broker.get("paused") else "ready") if broker else "unreachable"
    addon = {True: "loaded", False: "not loaded"}.get(native.get("addon_loaded"), "unknown")
    lines = ["Badi doctor: " + (f"{len(problems)} problem(s)" if problems else "no problems found"),
             f"  Service: {service.get('active', 'unknown')} ({service.get('state', 'unknown')})",
             f"  Model: {model}" + (f" · {broker['provider']}" if broker.get("provider") else ""),
             f"  Fcitx addon: {addon}",
             "  Field observer: " + ("ready" if observer.get("ready") else "unavailable"),
             "  Session helper: " + {True: report.get("session_helper", {}).get("active", "unknown"),
                                      False: "not installed"}.get(report.get("session_helper", {}).get("loaded"), "unknown")]
    installed = report.get("install", {}).get("desktop") or {}
    if installed.get("commit"):
        lines.append(f"  Installed source: {installed['commit']}" + (" (modified)" if installed.get("dirty") else ""))
    for problem in problems:
        lines.extend(["", f"{problem['code']}: {problem['message']}", f"  Next: {problem['action']}"])
    for note in report["notes"]:
        lines.extend(["", note])
    return "\n".join(lines)


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
    quiet = (settings or {}).get("escape_quiet_seconds")
    quiet_text = f"{quiet}s" if type(quiet) is int and quiet > 0 else "until edit"
    return (f"Badi: {state} · {health['provider']}\n"
            f"Requests: {counters['provider_calls']} · Suggestions: {counters['suggestions_shown']} · Errors: {counters['provider_errors']} · {misses}\n"
            "Native Fcitx: automatic in Omawrite; Tab request/accept in the Xournal++ Text tool\n"
            "Editors: Obsidian automatic/Tab · Bash Ctrl-X then Tab\n"
            f"Observed fields: automatic for Omawrite, Telegram and IME-parity apps ({IME_APPS}); Tab accepts a visible suggestion's next word, otherwise stays Tab; Ctrl+Right accepts all of it; Ctrl+Shift+Space requests\n"
            + f"Apps: {mode_text((settings or {}).get('all_linux_apps'), 'app')}\n"
            + f"Web sites: {mode_text((settings or {}).get('all_web_origins'), 'site')}\n"
            + f"Escape: dismiss (quiet: {quiet_text}) · Why nothing appeared: badi doctor; badi debug on; badi debug watch"
            + "".join(f"\n{line}" for line in [desktop_text(health.get("desktop"), settings)] if line))


MODEL_DIRECTORY = Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share") / "badi/models"
# Models that are not Apache-2.0 come with terms a person accepts once.
MODEL_LICENSES = {"Gemma": "the Gemma Terms of Use, https://ai.google.dev/gemma/terms",
                  "Llama-3.2-Community": "the Llama 3.2 Community License, https://www.llama.com/llama3_2/license/"}


def model_advice():
    """The broker's pinned writing catalog for this device (badi.model-advice.v3)."""
    result = subprocess.run([str(cli_path()), "models", "writing", "--json"],
                            capture_output=True, text=True, timeout=15)
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or "badictl models failed")
    return json.loads(result.stdout)


def active_model_file():
    """The model file the running broker loaded, from its content-free startup line."""
    try:
        identifier = badi_install.unit_value(SERVICE, "InvocationID")
        if not re.fullmatch(r"[a-f0-9]{32}", identifier):
            return None
        journal = subprocess.run(["journalctl", "--user", f"_SYSTEMD_INVOCATION_ID={identifier}",
                                  "--no-pager", "--output=cat", "--lines=200"],
                                 capture_output=True, text=True, timeout=3)
    except (OSError, subprocess.SubprocessError, RuntimeError):
        return None
    for line in reversed(journal.stdout.splitlines()):
        found = re.match(r"provider=local_model model=([A-Za-z0-9_.-]{1,120}\.gguf) ", line)
        if found:
            return found[1]
    return None


def model_report(settings):
    advice = model_advice()
    active = active_model_file()
    recommended = (advice.get("recommended") or {}).get("id")
    models = [{"id": model["id"], "name": model["name"], "bytes": model["download_bytes"],
               "license": model["license"], "languages": model["languages"],
               "screen_useful": model["screen_useful"], "fits": model["fits"],
               "installed": (MODEL_DIRECTORY / model["filename"]).is_file(),
               "active": model["filename"] == active, "recommended": model["id"] == recommended,
               "chosen": model["id"] == settings.get("writing_model")}
              for model in advice["catalog"]]
    return {"schema": "badi.models.v1", "recommended": recommended,
            "chosen": settings.get("writing_model"), "memory_mib": advice["hardware"]["memory"],
            "rationale": advice["rationale"], "models": models}


def print_models(report):
    active = next((model for model in report["models"] if model["active"]), None)
    print("Badi writes with " + (active["name"] if active else "no model yet (is the service running?)") + ".")
    best = next((model for model in report["models"] if model["recommended"]), None)
    if best:
        print(f"Recommended for this device: {best['name']}.")
    for model in report["models"]:
        mark = "●" if model["active"] else "★" if model["recommended"] else "·"
        state = ("in use" if model["active"] else "installed" if model["installed"]
                 else "too large for this device" if not model["fits"] else "not installed")
        score = f"{model['screen_useful']}/135 useful" if model["screen_useful"] is not None else "unscreened"
        print(f"  {mark} {model['id']:13} {model['name']:13} {model['bytes'] / 1e9:4.2f} GB  "
              f"{score:15} {model['license']:20} {state}")
    print("Scores are blind-reviewed useful suggestions in Badi's 2026-10-17 development screen, "
          "not a qualification. Switch with: badi model use ID")


def download_model(model):
    """The pinned file at its pinned revision, checked by size and SHA-256
    before it becomes visible to the broker."""
    import hashlib
    import urllib.request
    MODEL_DIRECTORY.mkdir(parents=True, exist_ok=True, mode=0o700)
    target = MODEL_DIRECTORY / model["filename"]
    partial = target.with_name(target.name + ".part")
    url = f"https://huggingface.co/{model['repository']}/resolve/{model['revision']}/{model['filename']}"
    digest = hashlib.sha256()
    received = 0
    with urllib.request.urlopen(url, timeout=60) as response, open(partial, "wb") as out:
        os.fchmod(out.fileno(), 0o600)
        while chunk := response.read(1 << 20):
            out.write(chunk)
            digest.update(chunk)
            received += len(chunk)
            if received > model["download_bytes"]:
                break
            if sys.stdout.isatty():
                print(f"\rDownloading {model['name']}: {100 * received // model['download_bytes']}%", end="", flush=True)
    if sys.stdout.isatty():
        print()
    if received != model["download_bytes"] or digest.hexdigest() != model["sha256"]:
        partial.unlink(missing_ok=True)
        raise RuntimeError(f"{model['filename']} did not match its pinned size and hash; nothing was installed.")
    partial.rename(target)


def use_model(model_id, accept_license):
    advice = model_advice()
    model = next((entry for entry in advice["catalog"] if entry["id"] == model_id), None)
    if model is None:
        raise RuntimeError("Unknown model. Choose one of: " + " ".join(entry["id"] for entry in advice["catalog"]))
    if not model["fits"]:
        raise RuntimeError(f"{model['name']} does not fit this device's memory with Badi's safety reserve.")
    terms = MODEL_LICENSES.get(model["license"])
    if terms and not accept_license:
        if not sys.stdin.isatty():
            raise RuntimeError(f"{model['name']} is provided under {terms}. Add --accept-license to agree.")
        if input(f"{model['name']} is provided under {terms}. Do you accept it? [y/N] ").strip().lower() not in ("y", "yes"):
            raise RuntimeError("Nothing changed.")
    if not (MODEL_DIRECTORY / model["filename"]).is_file():
        download_model(model)
    update_settings(lambda document: document.update(writing_model=model["id"]))
    if not restart_service():
        raise RuntimeError("The model service did not answer after the restart. Run badi doctor.")
    return model


def restart_service(wait=60):
    """Restart the broker and wait until it answers, up to `wait` seconds."""
    if service_state()["active"] == "failed":
        subprocess.run(["systemctl", "--user", "reset-failed", SERVICE], check=True, timeout=4)
    subprocess.run(["systemctl", "--user", "restart", SERVICE], check=True, timeout=30)
    deadline = time.monotonic() + wait
    while time.monotonic() < deadline:
        try:
            control(["status"])
            return True
        except (RuntimeError, subprocess.SubprocessError):
            time.sleep(0.5)
    return False


def update_settings(change):
    document = json.loads(control(["settings", "show", "--json"]))
    revision = document["revision"]
    change(document)
    document["revision"] = revision + 1
    # Let the broker reject concurrent changes; never overwrite newer settings.
    return control(["settings", "replace", "--if-revision", str(revision),
                    "--json", "-"], input_text=json.dumps(document))


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


def set_escape_quiet(document, seconds):
    # Absent is the canonical until-edit state; exact rules are left untouched.
    if seconds:
        document["escape_quiet_seconds"] = seconds
    else:
        document.pop("escape_quiet_seconds", None)


STYLE_ON = ("Style learning is on. Badi keeps the sentence you wrote before each accepted suggestion, "
            "only on sites you allow with: badi style site ORIGIN on. Samples are sealed with a key in your "
            "keyring and no prompt uses them yet; badi style export shows them, badi style forget deletes them.")


def style_keyring():
    """The session helper's keyring module, installed beside it or in the checkout."""
    path = next((candidate for candidate in (LIBRARY / "session/style_key.py",
                                              ROOT / "adapters/session/style_key.py") if candidate.is_file()), None)
    if path is None:
        raise RuntimeError("The keyring module is not installed. Run scripts/install-desktop.py.")
    spec = importlib.util.spec_from_file_location("badi_style_key", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def set_style_learning(document, enabled):
    # Absent is the canonical off state.
    if enabled:
        document["style_learning"] = True
    else:
        document.pop("style_learning", None)


def set_site_learning(document, value, days):
    """Learning for one site that is already allowed; `days` None turns it off."""
    identity = site_identity(value)
    subject = next((item for item in document["subjects"] if item["identity"] == identity), None)
    if days is not None and (subject is None or subject["permissions"]["suggest"] != "allow"):
        raise RuntimeError(f"Allow the site first: badi site {value} on")
    if subject is None:
        return
    permissions = subject["permissions"]
    if days is None:
        permissions.update(learn="block", retention={"mode": "none"})
    else:
        permissions.update(learn="allow", retention={"mode": "bounded", "days": days})


def style_command(arguments):
    if arguments == ["on"]:
        try:
            key_id, key = style_keyring().ensure()
        except Exception as error:  # the module's KeyringError, or no Secret Service
            raise RuntimeError(f"The keyring could not provide a style key ({error}); style learning stays off.") from None
        control(["style", "key", "-"], input_text=f"{key_id} {key}\n")
        update_settings(lambda document: set_style_learning(document, True))
        print(STYLE_ON)
    elif arguments == ["off"]:
        update_settings(lambda document: set_style_learning(document, False))
        print("Style learning is off; nothing new is kept. Kept samples expire with each site's retention, "
              "or delete them now with badi style forget.")
    elif arguments == ["status"]:
        status = json.loads(control(["style", "status"]))
        document = json.loads(control(["settings", "show", "--json"]))
        sites = [origin_text(item["identity"]) + f" ({item['permissions']['retention']['days']} days)"
                 for item in document["subjects"] if item["permissions"].get("learn") == "allow"]
        key = {"ready": "ready", "absent": "not delivered (badi style on)",
               "mismatch": "different from the one that sealed the samples (badi style forget)"}[status["key"]]
        print(f"Style learning: {'on' if status['enabled'] else 'off'} · samples: {status['samples']} "
              f"from {status['subjects']} site(s) · key: {key}"
              + ("" if status["available"] else " · store unreadable (badi style forget)")
              + f"\nLearning sites: {', '.join(sites) if sites else 'none'}")
    elif len(arguments) in (3, 4) and arguments[0] == "site" and arguments[2] in ("on", "off"):
        origin = arguments[1]
        if arguments[2] == "off" and len(arguments) == 3:
            days = None
        elif arguments[2] == "on" and (len(arguments) == 3 or (arguments[3].isascii() and arguments[3].isdigit()
                                                                and 1 <= int(arguments[3]) <= 90)):
            days = int(arguments[3]) if len(arguments) == 4 else 30
        else:
            raise RuntimeError("Usage: badi style site ORIGIN on [DAYS]|off (DAYS is 1-90)")
        update_settings(lambda document: set_site_learning(document, origin, days))
        print(f"{origin}: style samples kept for {days} days." if days else
              f"{origin}: no style samples; any kept from it are deleted.")
    elif arguments == ["export"]:
        if json.loads(control(["style", "status"]))["key"] != "ready":
            raise RuntimeError("The style key is not available to the broker. Run badi style on, or badi style status.")
        print(control(["style", "export"]), end="")
    elif arguments == ["forget"]:
        control(["style", "forget"])
        try:
            removed = style_keyring().delete()
        except Exception as error:
            raise RuntimeError(f"Samples were deleted, but the keyring key was not ({error}).") from None
        print("All style samples were deleted" + (" and the keyring key removed." if removed else "."))
    else:
        raise RuntimeError("Usage: badi style on|off|status|export|forget, or badi style site ORIGIN on [DAYS]|off")


def set_battery_mode(document, mode):
    # Absent is the canonical quiet state, the default.
    if mode == "quiet":
        document.pop("battery_mode", None)
    else:
        document["battery_mode"] = mode


def set_language(document, app, language):
    app_id = app_identity(app)["app_id"]
    if app_id == "chromium-browser":
        app_id = "chromium"
    if language not in (*WRITING, "reset"):
        raise RuntimeError("Use " + ", ".join(WRITING) + " or reset.")
    overrides = document.setdefault("language_overrides", {})
    if language == "reset":
        overrides.pop(app_id, None)
    else:
        overrides[app_id] = language
    if not overrides:
        document.pop("language_overrides", None)


# The broker's PredictionLanguage order (broker/src/settings/document.rs).
WRITING = {"en": "English", "de": "German", "fa": "Persian", "fr": "French", "es": "Spanish"}
DEFAULT_WRITING = ["en", "de", "fa"]
EXPERIMENTAL_WRITING = ("fr", "es")


def model_healing():
    """The running model's name and the languages it heals by default (its
    catalog `healed_languages`), from the broker's advice."""
    advice = model_advice()
    active = active_model_file()
    catalog = advice.get("catalog") or []
    model = next((entry for entry in catalog if entry.get("filename") == active), None) \
        or advice.get("recommended") or {}
    return model.get("name"), model.get("healed_languages", ["en", "fa"])


def writing_report(settings, model_default):
    enabled = settings.get("writing_languages", DEFAULT_WRITING)
    choices = settings.get("trailing_space_healing", {})
    report = []
    for code, name in WRITING.items():
        choice = {True: "on", False: "off"}.get(choices.get(code), "auto")
        report.append({"language": code, "name": name, "enabled": code in enabled,
                       "experimental": code in EXPERIMENTAL_WRITING, "healing": choice,
                       "healed": choices.get(code, code in model_default)})
    return report


def set_writing(document, language, enabled):
    if language not in WRITING:
        raise RuntimeError("Use one of: " + " ".join(WRITING))
    current = set(document.get("writing_languages", DEFAULT_WRITING))
    current = current | {language} if enabled else current - {language}
    ordered = [code for code in WRITING if code in current]
    if ordered == DEFAULT_WRITING:
        document.pop("writing_languages", None)
    else:
        document["writing_languages"] = ordered


def set_healing(document, language, choice):
    if language not in WRITING or choice not in ("on", "off", "auto"):
        raise RuntimeError("Usage: badi suggest LANG healing on|off|auto (LANG: " + " ".join(WRITING) + ")")
    choices = document.setdefault("trailing_space_healing", {})
    if choice == "auto":
        choices.pop(language, None)
    else:
        choices[language] = choice == "on"
    if not choices:
        document.pop("trailing_space_healing", None)


def print_writing(report, model_name):
    on = [entry["name"] for entry in report if entry["enabled"]]
    print("Suggestions: " + (", ".join(on) if on else "none (every language is off)"))
    for entry in report:
        mark = "✓" if entry["enabled"] else "·"
        heal = "heals a trailing space" if entry["healed"] else "keeps a trailing space (shows nothing after one)"
        source = f"auto, as {model_name} was reviewed" if entry["healing"] == "auto" and model_name else entry["healing"]
        note = "experimental · " if entry["experimental"] else ""
        print(f"  {mark} {entry['name']:<8} {'on ' if entry['enabled'] else 'off'}  {note}{heal} ({source})")
    print("Turn one on or off: badi suggest LANG on|off  (" + " ".join(WRITING) + ")")
    print("Healing lets the model write the space after your last word itself, so the next word")
    print("starts naturally: badi suggest LANG healing on|off|auto")


# The broker's SpellingLanguage order (broker/src/spelling.rs): its dictionary
# names, most common first, and the Arch package that installs them.
SPELLING = {
    "en": ("English", (), None),
    "de": ("German", ("de_DE", "de_AT", "de_CH"), "hunspell-de"),
    "fr": ("French", ("fr_FR", "fr"), "hunspell-fr-comprehensive"),
    "es": ("Spanish", ("es_ES", "es_ANY", "es_MX"), "hunspell-es_es"),
    "it": ("Italian", ("it_IT",), "hunspell-it"),
    "nl": ("Dutch", ("nl_NL",), "hunspell-nl"),
    "pl": ("Polish", ("pl_PL",), "hunspell-pl"),
}
# The broker's dictionary_directories(): the user's own, then the system's.
DICTIONARY_DIRECTORIES = (
    Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share") / "hunspell",
    Path("/usr/share/hunspell"), Path("/usr/share/myspell/dicts"))


def installed_dictionary(language):
    """The installed dictionary name for `language`, "built in" for English, or None."""
    _name, names, _package = SPELLING[language]
    if not names:
        return "built in"
    return next((name for name in names for directory in DICTIONARY_DIRECTORIES
                 if (directory / f"{name}.aff").is_file() and (directory / f"{name}.dic").is_file()), None)


def spelling_report(settings):
    enabled = settings.get("spelling_languages", ["en"])
    return [{"language": code, "name": name, "enabled": code in enabled,
             "dictionary": installed_dictionary(code), "package": package}
            for code, (name, _names, package) in SPELLING.items()]


def set_spelling(document, language, enabled):
    if language not in SPELLING:
        raise RuntimeError("Use one of: " + " ".join(SPELLING))
    current = set(document.get("spelling_languages", ["en"]))
    current = current | {language} if enabled else current - {language}
    ordered = [code for code in SPELLING if code in current]
    if ordered == ["en"]:
        document.pop("spelling_languages", None)
    else:
        document["spelling_languages"] = ordered


def print_spelling(report):
    on = [entry["name"] for entry in report if entry["enabled"]]
    print("Spelling: corrects " + (", ".join(on) if on else "nothing (every language is off)"))
    for entry in report:
        ready = entry["dictionary"] is not None
        mark = "✓" if entry["enabled"] and ready else "!" if entry["enabled"] else "·"
        detail = (entry["dictionary"] if entry["dictionary"] == "built in"
                  else f"dictionary {entry['dictionary']}" if ready
                  else f"needs: sudo pacman -S {entry['package']} (or its .aff/.dic in ~/.local/share/hunspell)")
        print(f"  {mark} {entry['name']:<8} {'on ' if entry['enabled'] else 'off'}  {detail}")
    print("Turn one on or off: badi spelling LANG on|off  (" + " ".join(SPELLING) + ")")


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
        for adapter in ("obsidian", "terminal", "neovim"):
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


TRACE_FILE = "debug-trace.jsonl"
# Trace fields that hold typed or suggested text; present only in a --text run.
TRACE_TEXT_FIELDS = ("before", "after", "typed", "text", "accept_word", "replace_before")
PREVIEW_LAYER = "badi-writing-preview"


def private_directory():
    directory = runtime_directory()
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    info = directory.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
        raise RuntimeError("Badi runtime directory must be private and owned by this user")
    return directory


def debug_mode(mode, text=False):
    if mode in ("on", "off"):
        directory = private_directory()
        for name in ("debug-control.json", "debug-native.json", TRACE_FILE,
                     *(f"debug-{adapter}.json" for adapter in ("obsidian", "terminal", "neovim"))):
            (directory / name).unlink(missing_ok=True)
        if mode == "on":
            # The trace exists before the control names the run, so no writer
            # sees a run without its file; writers never create it.
            descriptor = os.open(directory / TRACE_FILE, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            os.close(descriptor)
            marker = {"id": str(uuid.uuid4()), "expires_at": int(time.time()) + 900, "text": text}
            badi_install.atomic_write(directory / "debug-control.json", json.dumps(marker).encode())
            print("Debug trace on for 15 minutes"
                  + (", including context and suggestion text. It stays in memory"
                     " ($XDG_RUNTIME_DIR) and is deleted by badi debug off or logout."
                     if text else "; no typed text is stored (add --text for it).")
                  + " Follow it with badi debug watch.")
        else:
            print("Debug disabled and its trace and snapshots removed.")
        return
    if mode == "watch":
        return debug_watch()
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
    if mode == "status":
        print(json.dumps(report, indent=2), flush=True)
    else:
        print(debug_line(report), flush=True)


def trace_entries(lines):
    """Trace entries from complete lines; a partial or malformed line is skipped."""
    for line in lines:
        if not line.endswith("\n"):
            continue
        try:
            entry = json.loads(line)
        except ValueError:
            continue
        if isinstance(entry, dict):
            yield entry


def format_trace(entry):
    """One readable line: time, source, event, facts, then any text quoted."""
    at = entry.get("t")
    stamp = (time.strftime("%H:%M:%S", time.localtime(at / 1000)) + f".{int(at) % 1000:03d}"
             if isinstance(at, (int, float)) else "--:--:--.---")
    facts = []
    for key, value in entry.items():
        if key in ("t", "src", "ev") or key in TRACE_TEXT_FIELDS:
            continue
        facts.append(f"{key}={json.dumps(value, ensure_ascii=False, separators=(',', ':')) if isinstance(value, (dict, list)) else value}")
    for key in TRACE_TEXT_FIELDS:
        if isinstance(entry.get(key), str):
            facts.append(f"{key}={json.dumps(entry[key], ensure_ascii=False)}")
    return f"{stamp}  {str(entry.get('src', '?')):<10} {str(entry.get('ev', '?')):<24} {' '.join(facts)}".rstrip()


def open_trace(directory):
    descriptor = os.open(directory / TRACE_FILE, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    info = os.fstat(descriptor)
    if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
        os.close(descriptor)
        raise RuntimeError("The debug trace must be a private, owned regular file")
    return os.fdopen(descriptor, encoding="utf-8", errors="replace")


def append_trace(directory, entry):
    """The controller's own lines, under the same rules as every writer."""
    try:
        descriptor = os.open(directory / TRACE_FILE, os.O_WRONLY | os.O_APPEND | os.O_NOFOLLOW | os.O_CLOEXEC)
    except OSError:
        return
    try:
        info = os.fstat(descriptor)
        if stat.S_ISREG(info.st_mode) and info.st_uid == os.getuid() and not info.st_mode & 0o077:
            os.write(descriptor, (json.dumps(entry, ensure_ascii=False) + "\n").encode())
    finally:
        os.close(descriptor)


def preview_layers(run=subprocess.run):
    """The grey preview's layer surfaces as Hyprland maps them now, or None if unknown."""
    try:
        result = run(["hyprctl", "-j", "layers"], capture_output=True, text=True, timeout=2, check=False)
        monitors = json.loads(result.stdout)
    except (OSError, subprocess.SubprocessError, ValueError):
        return None
    if result.returncode != 0 or not isinstance(monitors, dict):
        return None
    return sorted((name, layer.get("x"), layer.get("y"), layer.get("w"), layer.get("h"))
                  for name, monitor in monitors.items() if isinstance(monitor, dict)
                  for level in (monitor.get("levels") or {}).values() if isinstance(level, list)
                  for layer in level if isinstance(layer, dict) and layer.get("namespace") == PREVIEW_LAYER)


def compositor_entry(layers):
    entry = {"t": int(time.time() * 1000), "src": "compositor"}
    if not layers:
        return {**entry, "ev": "preview not on screen"}
    name, x, y, width, height = layers[0]
    return {**entry, "ev": "preview on screen", "monitor": name, "x": x, "y": y, "width": width,
            "height": height, "surfaces": len(layers)}


def print_trace(as_json=False):
    with open_trace(private_directory()) as stream:
        for entry in trace_entries(stream):
            print(json.dumps(entry, ensure_ascii=False) if as_json else format_trace(entry), flush=True)


def debug_watch(poll=0.15):
    """Follows the trace and records what Hyprland shows, until the run ends."""
    directory = private_directory()
    report = debug_activity()
    if not report["enabled"]:
        raise RuntimeError("Debug is off; run badi debug on (or on --text) first")
    print(f"Following {directory / TRACE_FILE}; Ctrl+C stops. The compositor lines say whether"
          " the grey preview is really on screen.", flush=True)
    layers = None
    pending = ""
    checked = time.monotonic()
    with open_trace(directory) as stream:
        while True:
            lines = (pending + stream.read()).splitlines(keepends=True)
            # A writer's line may arrive in two reads; keep its start.
            pending = lines.pop() if lines and not lines[-1].endswith("\n") else ""
            for entry in trace_entries(lines):
                print(format_trace(entry), flush=True)
            now = preview_layers()
            if now is not None and now != layers:
                if layers is not None or now:
                    append_trace(directory, compositor_entry(now))
                layers = now
            if time.monotonic() - checked >= 2:
                checked = time.monotonic()
                if not debug_activity()["enabled"]:
                    print("Debug run ended.", flush=True)
                    return
            time.sleep(poll)


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
    if arguments[:1] == ["debug"] and arguments[1:] in (["on"], ["on", "--text"], ["off"], ["status"], ["watch"]):
        try:
            debug_mode(arguments[1], text=arguments[2:] == ["--text"])
        except KeyboardInterrupt:
            pass
        return
    if arguments[:2] == ["debug", "log"] and arguments[2:] in ([], ["--json"]):
        print_trace(as_json=arguments[2:] == ["--json"])
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
    if arguments == ["pause", "toggle"]:
        outcome = {}

        def toggle(document):
            document["paused"] = outcome["paused"] = not document["paused"]
        update_settings(toggle)
        print("Predictions paused." if outcome["paused"] else "Predictions resumed.")
        return
    if arguments[:1] == ["style"]:
        style_command(arguments[1:])
        return
    if len(arguments) == 2 and arguments[0] == "battery" and arguments[1] in BATTERY_TEXT:
        update_settings(lambda document: set_battery_mode(document, arguments[1]))
        print(BATTERY_TEXT[arguments[1]])
        return
    if arguments in (["pause"], ["resume"]):
        paused = arguments == ["pause"]
        update_settings(lambda document: document.update(paused=paused))
        print("Predictions paused." if paused else "Predictions resumed.")
        return
    if arguments in (["mid-line", "on"], ["mid-line", "off"]):
        enabled = arguments[1] == "on"
        update_settings(lambda document: set_mode(document, "mid_line", enabled))
        print("Mid-sentence suggestions are on." if enabled
              else "Mid-sentence suggestions are off. A word after the caret asks for nothing.")
        return
    if arguments in (["model"], ["model", "--json"], ["models"]):
        report = model_report(json.loads(control(["settings", "show", "--json"])))
        print(json.dumps(report, indent=2) if arguments[1:] == ["--json"] else "", end="")
        if arguments[1:] != ["--json"]:
            print_models(report)
        return
    if arguments[:2] == ["model", "use"] and len(arguments) in (3, 4) and arguments[3:] in ([], ["--accept-license"]):
        model = use_model(arguments[2], arguments[3:] == ["--accept-license"])
        loaded = active_model_file()
        print(f"Badi now writes with {model['name']}." if loaded == model["filename"]
              else f"{model['name']} is chosen, but the broker loaded {loaded or 'no model'}; badi model explains why.")
        return
    if arguments == ["model", "auto"]:
        update_settings(lambda document: document.pop("writing_model", None))
        if not restart_service():
            raise RuntimeError("The model service did not answer after the restart. Run badi doctor.")
        print("Badi follows the device recommendation again.")
        return
    if arguments[:1] == ["spelling"] and arguments[1:] in ([], ["--json"]):
        report = spelling_report(json.loads(control(["settings", "show", "--json"])))
        if arguments[1:]:
            print(json.dumps({"schema": "badi.spelling.v1", "languages": report}, indent=2))
        else:
            print_spelling(report)
        return
    if len(arguments) == 3 and arguments[0] == "spelling" and arguments[2] in ("on", "off"):
        language = arguments[1]
        update_settings(lambda document: set_spelling(document, language, arguments[2] == "on"))
        print_spelling(spelling_report(json.loads(control(["settings", "show", "--json"]))))
        return
    if arguments[:1] == ["suggest"] and arguments[1:] in ([], ["--json"]):
        name, model_default = model_healing()
        report = writing_report(json.loads(control(["settings", "show", "--json"])), model_default)
        if arguments[1:]:
            print(json.dumps({"schema": "badi.writing-languages.v1", "model": name, "languages": report}, indent=2))
        else:
            print_writing(report, name)
        return
    if len(arguments) == 3 and arguments[0] == "suggest" and arguments[2] in ("on", "off"):
        language = arguments[1]
        update_settings(lambda document: set_writing(document, language, arguments[2] == "on"))
        name, model_default = model_healing()
        print_writing(writing_report(json.loads(control(["settings", "show", "--json"])), model_default), name)
        return
    if len(arguments) == 4 and arguments[0] == "suggest" and arguments[2] == "healing":
        language, choice = arguments[1], arguments[3]
        update_settings(lambda document: set_healing(document, language, choice))
        name, model_default = model_healing()
        print_writing(writing_report(json.loads(control(["settings", "show", "--json"])), model_default), name)
        return
    if len(arguments) == 3 and arguments[0] == "language":
        app, language = arguments[1:]
        update_settings(lambda document: set_language(document, app, language))
        print(f"{app}: " + ("automatic language selection restored." if language == "reset"
                            else f"writing language is {language}. App and site permissions still apply."))
        return
    if len(arguments) == 2 and arguments[0] == "escape-quiet":
        value = arguments[1]
        if value == "off":
            seconds = 0
        elif value.isascii() and value.isdigit() and 1 <= int(value) <= 3600:
            seconds = int(value)
        else:
            raise RuntimeError("Usage: badi escape-quiet SECONDS|off (SECONDS is 1-3600)")
        update_settings(lambda document: set_escape_quiet(document, seconds))
        print(f"Escape keeps a field quiet for {seconds} seconds after a dismissal. "
              "Asking explicitly ends it sooner." if seconds
              else "Escape quiet is off. A dismissal lasts only until the text changes.")
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
    if arguments in (["doctor"], ["doctor", "--json"]):
        report = health_report()
        print(json.dumps(report, indent=2) if arguments[-1] == "--json" else doctor_text(report))
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
        misses = no_suggestion_summary(counters)
        if misses and misses["last"]:
            misses["last_meaning"] = NO_SUGGESTION[misses["last"]]
        overview["desktop"] = {
            "requests": counters["provider_calls"], "suggestions": counters["suggestions_shown"],
            "errors": counters["provider_errors"],
            "accepted": counters.get("commits_prepared"),
            "word_savings": counters.get("word_savings"),
            "no_suggestion": misses,
            "spelling": spelling_report(overview.get("settings") or {}),
            "activity": debug_activity(),
            # The file the running broker loaded; the advice's catalog names it.
            "model_file": active_model_file(),
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
