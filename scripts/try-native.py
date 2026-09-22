#!/usr/bin/env python3
"""Run a private Badi/Fcitx trial without replacing the desktop input method."""

import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time



ROOT = Path(__file__).resolve().parents[1]
APPS = {"xournalpp": "com.github.xournalpp.xournalpp", "omawrite": "omawrite"}


def run(command, **kwargs):
    return subprocess.run(command, check=True, **kwargs)


def stop(process):
    if process is None or process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=8)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=3)
        raise RuntimeError("A trial process required forced shutdown; inspect the retained logs")


def private_file(path, text):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    with path.open("x") as stream:
        os.chmod(path, 0o600)
        stream.write(text)


def session(args):
    """Executed inside dbus-run-session with an isolated HOME and XDG roots."""
    trial = Path(args.session_root)
    broker = fcitx = app = None
    registration = None
    socket = Path(os.environ["XDG_RUNTIME_DIR"]) / "badi/broker.sock"
    cli = ROOT / "target/release/badictl"
    with (trial / "broker.log").open("w") as broker_log, (trial / "fcitx.log").open("w") as fcitx_log:
        try:
            broker = subprocess.Popen(
                [str(ROOT / "target/release/badi-broker"), "--model-directory", args.models],
                stdout=broker_log, stderr=broker_log,
            )
            deadline = time.monotonic() + 60
            while not socket.exists():
                if broker.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError(f"Model startup failed; inspect {trial / 'broker.log'}")
                time.sleep(0.1)
            status = json.loads(run([str(cli), "status"], capture_output=True, text=True, timeout=5).stdout)
            if status["provider"] != "local_model":
                raise RuntimeError("The trial did not activate the local LLM")
            # Toolkit clients use this private bus. Disable compositor-wide and
            # X11 input-method frontends so the existing desktop Fcitx keeps them.
            fcitx = subprocess.Popen(
                ["fcitx5", "-D", "--disable=waylandim,xim,notificationitem", "--ui=classicui"],
                stdout=fcitx_log, stderr=fcitx_log,
            )
            deadline = time.monotonic() + 10
            while True:
                probe = subprocess.run(
                    ["gdbus", "call", "--session", "--dest", "org.freedesktop.DBus",
                     "--object-path", "/org/freedesktop/DBus", "--method",
                     "org.freedesktop.DBus.NameHasOwner", "org.fcitx.Fcitx5"],
                    capture_output=True, text=True, timeout=2,
                )
                if probe.returncode == 0 and "true" in probe.stdout:
                    break
                if fcitx.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError(f"Private Fcitx startup failed; inspect {trial / 'fcitx.log'}")
                time.sleep(0.1)
            if "Loaded addon badi" not in (trial / "fcitx.log").read_text():
                raise RuntimeError(f"Badi addon did not load; inspect {trial / 'fcitx.log'}")
            if args.check:
                try:
                    run([str(ROOT / "adapters/fcitx5/build/badi-fcitx5-broker-smoke"),
                         str(socket), APPS[args.app], "Please find attached the", " latest version of the"],
                        timeout=10)
                finally:
                    snapshot = json.loads(run([str(cli), "status"], capture_output=True, text=True, timeout=5).stdout)
                    private_file(trial / "check-metrics.json", json.dumps(snapshot["metrics"], indent=2))
                print("Private local model, Fcitx addon and native transport passed.", flush=True)
                return
            registry = os.environ.get("BADI_DESKTOP_REGISTRY")
            if registry:
                Path(registry).parent.mkdir(mode=0o700, exist_ok=True)
                registration = Path(registry) / f"{trial.name}.json"
                label = ("Xournal++" if args.app == "xournalpp" else "Omawrite") + " · " + time.strftime("%H:%M")
                private_file(registration, json.dumps({"id": trial.name, "app": args.app, "label": label, "socket": str(socket)}))
            print(f"\nBadi is ready in this {args.app} test window. English/US keyboard.", flush=True)
            print("Xournal++: choose the Text tool and click the page first.", flush=True)
            print("Type: Please find attached the", flush=True)
            print("Tab: request at the end of a phrase; Tab again: accept; Escape: dismiss.", flush=True)
            print("Suggestions appear only on request and remain for up to five seconds while the text is unchanged.", flush=True)
            print("Try your own sentences too. Native spelling replacement is not available yet.", flush=True)
            print(f"Test profile and any files saved there are retained in: {trial}", flush=True)
            print("Close the test app normally to save your work and end the trial.\n", flush=True)
            with (trial / "application.log").open("w") as app_log:
                app = subprocess.Popen([args.app], cwd=trial, stdout=app_log, stderr=app_log)
                while app.poll() is None:
                    if broker.poll() is not None or fcitx.poll() is not None:
                        print("Badi stopped unexpectedly. Save and close the test app; inspect the trial logs.", flush=True)
                        app.wait()
                        raise RuntimeError("A trial service exited unexpectedly")
                    time.sleep(0.2)
                if app.returncode != 0:
                    raise RuntimeError(f"Application exited with status {app.returncode}; inspect {trial / 'application.log'}")
        finally:
            if registration is not None:
                registration.unlink(missing_ok=True)
            # Attempt every cleanup even if one process fails to stop.
            errors = []
            for process in (app, fcitx, broker):
                try:
                    stop(process)
                except RuntimeError as error:
                    errors.append(str(error))
            if errors:
                raise RuntimeError("; ".join(errors))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("app", choices=APPS, nargs="?", default="xournalpp")
    parser.add_argument("--check", action="store_true", help="Check services/transport and exit without opening an editor")
    parser.add_argument("--models", help="Absolute Badi model/runtime data directory")
    parser.add_argument("--session-root", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.session_root:
        session(args)
        return
    for command in ("cargo", "cmake", "ninja", "fcitx5", "dbus-run-session", "gdbus", args.app):
        if shutil.which(command) is None:
            raise RuntimeError(f"Required command is missing: {command}")
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    wayland = os.environ.get("WAYLAND_DISPLAY")
    if not runtime or not wayland:
        raise RuntimeError("Run this command in a terminal in your Wayland desktop session")
    display = Path(wayland) if Path(wayland).is_absolute() else Path(runtime) / wayland
    if not display.is_socket():
        raise RuntimeError("The Wayland display socket is unavailable")
    models = Path(args.models) if args.models else Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share") / "badi"
    if not models.is_absolute():
        raise RuntimeError("The model directory must be absolute")
    print("Building the current broker and addon locally…", flush=True)
    run(["cargo", "build", "--release", "--locked", "--workspace", "--bins"], cwd=ROOT)
    build = ROOT / "adapters/fcitx5/build"
    run(["cmake", "-S", str(ROOT / "adapters/fcitx5"), "-B", str(build),
         "-G", "Ninja", "-DCMAKE_BUILD_TYPE=Release", "-DBUILD_TESTING=ON"])
    run(["cmake", "--build", str(build)])
    trials = ROOT / "output/native-trials"
    trials.mkdir(parents=True, exist_ok=True)
    trial = Path(tempfile.mkdtemp(prefix=f"{args.app}-", dir=trials))
    print(f"Trial files: {trial}", flush=True)
    for name in ("home", "config", "data", "cache"):
        (trial / name).mkdir(mode=0o700)
    prefix = trial / "addon"
    run(["cmake", "--install", str(build), "--prefix", str(prefix)])
    settings = {"schema": "badi.settings.v2", "revision": 1, "paused": False,
                "subjects": [{"identity": {"kind": "linux_app", "adapter": "fcitx", "app_id": APPS[args.app]},
                              "permissions": {"context_read": "allow", "display": "allow", "suggest": "allow",
                                              "learn": "block", "retention": {"mode": "none"}}}]}
    private_file(trial / "config/badi/settings.json", json.dumps(settings))
    private_file(trial / "config/fcitx5/profile", "[Groups/0]\nName=Default\nDefault Layout=us\nDefaultIM=keyboard-us\n\n[Groups/0/Items/0]\nName=keyboard-us\nLayout=\n\n[GroupOrder]\n0=Default\n")
    # Use the installed session-bus policy, but no service activation paths:
    # probing or opening an editor must not launch another Fcitx or portal stack.
    private_file(trial / "bus.conf", """<busconfig>
<type>session</type><keep_umask/><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth>
<policy context="default"><allow send_destination="*" eavesdrop="true"/>
<allow eavesdrop="true"/><allow own="*"/></policy>
</busconfig>""")
    with tempfile.TemporaryDirectory(prefix="badi-native-", dir="/tmp") as private_runtime:
        environment = {**os.environ, "HOME": str(trial / "home"), "XDG_CONFIG_HOME": str(trial / "config"),
                       "XDG_DATA_HOME": str(trial / "data"), "XDG_CACHE_HOME": str(trial / "cache"),
                       "XDG_RUNTIME_DIR": private_runtime, "WAYLAND_DISPLAY": str(display),
                       "BADI_DESKTOP_REGISTRY": str(Path(runtime) / "badi/native-trials"),
                       "GTK_USE_PORTAL": "0", "GTK_IM_MODULE": "fcitx", "QT_IM_MODULE": "fcitx", "GDK_BACKEND": "wayland",
                       "QT_IM_MODULES": "fcitx", "FCITX_CONFIG_HOME": str(trial / "config/fcitx5"),
                       "FCITX_DATA_HOME": str(trial / "data/fcitx5"), "FCITX_CONFIG_DIRS": "/etc/xdg/fcitx5",
                       "QT_QPA_PLATFORM": "wayland", "FCITX_ADDON_DIRS": f"{prefix}/lib/fcitx5:/usr/lib/fcitx5",
                       "FCITX_DATA_DIRS": f"{prefix}/share/fcitx5:/usr/share/fcitx5"}
        for name in ("DISPLAY", "SKIP_FCITX_PATH", "SKIP_FCITX_USER_PATH", "SKIP_FCITX_SYSTEM_PATH"):
            environment.pop(name, None)
        command = ["dbus-run-session", f"--config-file={trial / 'bus.conf'}", "--", sys.executable, str(Path(__file__).resolve()), args.app,
                   "--session-root", str(trial), "--models", str(models)]
        if args.check:
            command.append("--check")
        child = subprocess.Popen(command, env=environment, start_new_session=True)
        try:
            code = child.wait()
            if code:
                raise RuntimeError(f"Private trial exited with status {code}; logs are in {trial}")
        except BaseException:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGINT)
                child.wait(timeout=15)
            raise
    print("Trial stopped. Your normal Fcitx session was not replaced; trial files were kept.")


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(130))
    try:
        main()
    except (RuntimeError, subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
        print(f"Trial failed: {error}", file=sys.stderr)
        sys.exit(1)
    except KeyboardInterrupt:
        print("Trial interrupted. Files remain in the printed trial folder.", file=sys.stderr)
        sys.exit(130)
