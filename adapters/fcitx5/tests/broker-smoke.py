#!/usr/bin/env python3
"""Check the actual C++/Rust socket boundary in disposable XDG roots, without an IME install."""

import argparse
import json
import os
import select
from pathlib import Path
import subprocess
import tempfile
import time


ROOT = Path(__file__).resolve().parents[3]
APPS = ("com.github.xournalpp.xournalpp", "omawrite")


def run(args):
    with tempfile.TemporaryDirectory(prefix="badi-fcitx5-", dir="/tmp") as directory:
        root = Path(directory)
        runtime = root / "runtime"
        config = root / "config"
        data = root / "data"
        for path in (runtime, config, data, config / "badi"):
            path.mkdir(mode=0o700)
        settings = {
            "schema": "badi.settings.v2", "revision": 1, "paused": False,
            "subjects": [{
                "identity": {"kind": "linux_app", "adapter": "fcitx", "app_id": app},
                "permissions": {
                    "context_read": "allow", "display": "allow", "suggest": "allow",
                    "learn": "block", "retention": {"mode": "none"},
                },
            } for app in APPS],
        }
        settings_path = config / "badi/settings.json"
        with settings_path.open("x") as stream:
            os.chmod(settings_path, 0o600)
            json.dump(settings, stream)
        environment = {
            **os.environ, "XDG_RUNTIME_DIR": str(runtime),
            "XDG_CONFIG_HOME": str(config), "XDG_DATA_HOME": str(data),
        }
        socket = runtime / "badi/broker.sock"
        with (root / "broker.log").open("w+") as log:
            def start_broker():
                return subprocess.Popen(
                    [args.broker, "--provider", "local" if args.model_directory else "phrase",
                     *(["--model-directory", args.model_directory] if args.model_directory else [])],
                    env=environment, stdin=subprocess.DEVNULL, stdout=log, stderr=log)

            broker = start_broker()
            recovery = None
            try:
                deadline = time.monotonic() + 60
                while not socket.exists():
                    if broker.poll() is not None or time.monotonic() >= deadline:
                        raise RuntimeError("broker did not become ready within sixty seconds")
                    time.sleep(0.01)
                for app in APPS:
                    subprocess.run([args.client, str(socket), app,
                                    *(["Please find attached the", " latest version of the"]
                                      if args.model_directory else [])], env=environment,
                                   check=True, timeout=10)
                deadline = time.monotonic() + 5
                while True:
                    result = subprocess.run(
                        [args.cli, "status"], env=environment, check=True,
                        capture_output=True, text=True, timeout=5,
                    )
                    status = json.loads(result.stdout)
                    if status["sessions"] == 0:
                        break
                    if time.monotonic() >= deadline:
                        raise RuntimeError("disconnected native sessions were not retired")
                    time.sleep(0.01)
                expected = {
                    "context_updates": 4, "provider_calls": 4, "suggestions_shown": 4,
                    "commits_prepared": 2, "commits_applied": 0, "dismissals": 2,
                    "commit_failures": 0, "provider_errors": 0, "stale_results": 0,
                }
                for metric, value in expected.items():
                    if status["metrics"][metric] != value:
                        raise RuntimeError(f"unexpected broker counter: {metric}")
                if not args.model_directory:
                    recovery = subprocess.Popen(
                        [args.client, "--reconnect", str(socket), "omawrite"],
                        env=environment, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                        stderr=subprocess.PIPE, text=True)
                    if not select.select([recovery.stdout], [], [], 5)[0] or recovery.stdout.readline().strip() != "READY_FOR_RESTART":
                        raise RuntimeError("native recovery client did not prepare a candidate")
                    broker.terminate()
                    broker.wait(timeout=5)
                    if broker.returncode != 0 or socket.exists():
                        raise RuntimeError("first broker did not cleanly retire")
                    # The adapter must survive failed reconnect attempts while
                    # the service is absent, without another focus or key event.
                    time.sleep(0.8)
                    broker = start_broker()
                    output, error = recovery.communicate(timeout=12)
                    if recovery.returncode != 0:
                        raise RuntimeError(f"native recovery failed: {output} {error}")
            finally:
                if recovery is not None and recovery.poll() is None:
                    recovery.terminate()
                    try:
                        recovery.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        recovery.kill()
                        recovery.wait(timeout=3)
                if broker.poll() is None:
                    broker.terminate()
                try:
                    broker.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    broker.kill()
                    broker.wait(timeout=5)
                    raise RuntimeError("broker required forced cleanup") from None
            if broker.returncode != 0 or socket.exists():
                raise RuntimeError("broker shutdown/socket cleanup failed")
            log.seek(0)
            diagnostics = log.read()
            if args.model_directory:
                if not diagnostics.startswith("provider=local_model model="):
                    raise RuntimeError("local model was not activated")
            elif diagnostics:
                raise RuntimeError("broker emitted unexpected diagnostics")
    print("Both native identities passed real broker transport checks; isolated state removed.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model-directory", help="Absolute path to pinned local writing assets")
    parser.add_argument("--broker", default=str(ROOT / "target/debug/badi-broker"))
    parser.add_argument("--cli", default=str(ROOT / "target/debug/badictl"))
    parser.add_argument("--client", default=str(ROOT / "adapters/fcitx5/build/badi-fcitx5-broker-smoke"))
    run(parser.parse_args())
