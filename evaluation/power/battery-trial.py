#!/usr/bin/env python3
"""Paced-typing comparison of battery modes against the real local model.

Starts a private broker (its own socket, settings and memory, with
``--model-directory`` reading the installed pinned model and runtime, which
it verifies and never writes), reports battery power as the session helper would,
and types fixed disposable English text one key at a time the way the Fcitx
addon's early path does: every key replaces the session and asks again.

For each arm it records provider calls, the runtime's CPU time (an energy
proxy: RAPL counters are root-only here), suggestions shown, how many began
with the next words of the reference text, and the visible latency after the
last key. Nothing typed by a person is used, and no output contains text
beyond these fixed fixtures.

    python3 evaluation/power/battery-trial.py --broker target/debug/badi-broker \\
        --output output/power/battery-trial.json
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]
# Disposable reference text: ordinary prose with predictable continuations.
TEXTS = (
    "Thank you for the update on the project. I will send the final report to the team by the end "
    "of the week. Please let me know if you have any questions about the schedule or the budget.",
    "The meeting has been moved to next Tuesday at ten in the morning. We will review the results "
    "of the first quarter and discuss the plan for the next release. I look forward to seeing you there.",
)
KEY_MS = 150
WORD_PAUSE_MS = 450
SENTENCE_PAUSE_MS = 1500
SETTLE_MS = 900


def frame(message):
    body = json.dumps(message, separators=(",", ":")).encode()
    return struct.pack("<I", len(body)) + body


class Connection:
    def __init__(self, path):
        self.socket = socket.socket(socket.AF_UNIX)
        self.socket.connect(str(path))
        self.buffer = b""

    def send(self, message_type, payload, message_id=None, coordinates=None):
        message = {"v": 2, "type": message_type, "mono_ms": int(time.monotonic() * 1000), "payload": payload}
        if message_id is not None:
            message["id"] = message_id
        if coordinates is not None:
            message.update(coordinates)
        self.socket.sendall(frame(message))

    def receive(self, timeout):
        self.socket.settimeout(timeout)
        while True:
            if len(self.buffer) >= 4:
                (length,) = struct.unpack_from("<I", self.buffer)
                if len(self.buffer) >= 4 + length:
                    body, self.buffer = self.buffer[4:4 + length], self.buffer[4 + length:]
                    return json.loads(body)
            chunk = self.socket.recv(65536)
            if not chunk:
                raise ConnectionError("broker closed")
            self.buffer += chunk

    def request(self, message_type, payload, expected, timeout=5):
        message_id = f"trial.{uuid.uuid4().hex}"
        self.send(message_type, payload, message_id)
        deadline = time.monotonic() + timeout
        while True:
            reply = self.receive(max(.01, deadline - time.monotonic()))
            if reply.get("id") == message_id:
                if reply["type"] != expected:
                    raise RuntimeError(f"{message_type}: {reply['type']} {reply['payload']}")
                return reply

    def hello(self, kind, capabilities):
        return self.request("hello", {"min_v": 2, "max_v": 2, "capabilities": capabilities,
                                      "adapter": {"kind": kind, "name": f"badi-trial-{kind}", "version": "0.1.0"}},
                            "hello.ack")


def settings(revision, mode):
    document = {"schema": "badi.settings.v2", "revision": revision, "paused": False, "subjects": [{
        "identity": {"kind": "linux_app", "adapter": "obsidian", "app_id": "obsidian"},
        "permissions": {"suggest": "allow", "display": "allow", "context_read": "allow", "learn": "block",
                        "retention": {"mode": "none"}}}]}
    # Explicit for every arm: an omitted mode means quiet, the default.
    document["battery_mode"] = mode
    return document


def runtime_cpu_seconds(broker_pid):
    """User plus system CPU time of every process below the broker (the
    llama-server runtime and its launch helper), excluding the broker."""
    total, pending = 0, [str(broker_pid)]
    while pending:
        parent = pending.pop()
        for children in Path(f"/proc/{parent}/task").glob("*/children"):
            for pid in children.read_text().split():
                pending.append(pid)
                try:
                    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
                except OSError:
                    continue
                total += int(fields[11]) + int(fields[12])
    return total / os.sysconf("SC_CLK_TCK")


def health(path):
    connection = Connection(path)
    connection.hello("cli", ["health"])
    payload = connection.request("health.request", {}, "health.status")["payload"]
    connection.socket.close()
    return payload


def type_text(path, text):
    """Types text key by key; returns per-suggestion observations."""
    connection = Connection(path)
    connection.hello("obsidian", ["context", "suggestion", "commit.applied", "control", "policy",
                                  "long_reading_window"])
    authority = connection.receive(5)
    assert authority["type"] == "authority.changed", authority
    connection.send("authority.ack", {"authority_epoch": authority["payload"]["authority_epoch"]})
    target = {"kind": "obsidian", "app_id": "obsidian", "target_id": "battery-trial"}
    policy = connection.request("policy.query", {"target": target}, "policy.status")["payload"]
    assert policy["suggestions_allowed"], policy
    shown = []
    covered = set()
    word_ends = {position for position in range(1, len(text) + 1)
                 if text[position - 1].isalnum() and (position == len(text) or not text[position].isalnum())}
    session = None
    last_key = time.monotonic()
    expected = {}

    def drain(until):
        while True:
            remaining = until - time.monotonic()
            if remaining <= 0:
                return
            try:
                message = connection.receive(remaining)
            except (TimeoutError, socket.timeout):
                return
            if message["type"] == "suggestion.show" and session and message.get("session_id") == session["session_id"]:
                key = session["session_id"]
                if key in expected:
                    offset = expected.pop(key)
                    suggestion = message["payload"]["text"]
                    useful = bool(suggestion.strip()) and text[offset:].startswith(suggestion.rstrip())
                    if useful:
                        # Reference words this suggestion would complete, by
                        # the offset where each word ends; type-through
                        # remainders of one suggestion cover the same words.
                        end = offset + len(suggestion.rstrip())
                        covered.update(position for position in word_ends if offset < position <= end)
                    shown.append({"latency_ms": round((time.monotonic() - last_key) * 1000),
                                  "useful": useful})

    for index in range(1, len(text) + 1):
        if session is not None:
            connection.send("session.close", {"reason": "session_closed"}, f"trial.{uuid.uuid4().hex}",
                            {**session, "revision": 1})
        before = text[:index]
        session = {"session_id": str(uuid.uuid4()), "focus_epoch": 1, "revision": 0}
        connection.send("session.open", {"target": {**target, "target_id": session["session_id"]},
                                         "activation": "always"}, f"trial.{uuid.uuid4().hex}", session)
        fingerprint = uuid.uuid4().hex
        coordinates = {**session, "revision": 1}
        connection.send("context.changed", {
            "fingerprint": fingerprint, "before": before[-512:], "after": "", "language": "en",
            "selection": {"anchor": len(before[-512:]), "head": len(before[-512:]), "unit": "unicode_scalar_values"},
            "field": {"purpose": "normal", "editable": True, "multiline": True, "composing": False,
                      "sensitive": False, "identity_known": True, "focused": True, "lock_screen": False},
            "activation": "always", "explicit": False}, f"trial.{uuid.uuid4().hex}", coordinates)
        connection.send("suggest.request", {"fingerprint": fingerprint, "explicit": False},
                        f"trial.{uuid.uuid4().hex}", coordinates)
        session["revision"] = 1
        expected[session["session_id"]] = index
        last_key = time.monotonic()
        pause = KEY_MS
        if text[index - 1] == " ":
            pause += SENTENCE_PAUSE_MS if before[-2:-1] in (".", "?", "!") else WORD_PAUSE_MS
        drain(time.monotonic() + pause / 1000)
    drain(time.monotonic() + SETTLE_MS / 1000)
    connection.socket.close()
    return shown, len(covered)


def run(arguments):
    broker = Path(arguments.broker).resolve()
    models = Path.home() / ".local/share/badi"
    # Private broker state lives in a short-lived directory under /tmp.
    work = Path(tempfile.mkdtemp(prefix="badi-trial-"))
    data = work / "data/badi"
    data.mkdir(parents=True)
    data.chmod(0o700)
    # Like /run/user/UID, the runtime directory sits in a parent that others
    # cannot write; the model runtime's socket is refused under /tmp itself.
    # The service gets badi/ from systemd (RuntimeDirectory=badi), and the
    # runtime socket lives there before the broker binds its own.
    runtime = work / "run"
    (runtime / "badi").mkdir(parents=True)
    for directory in (runtime, runtime / "badi"):
        directory.chmod(0o700)
    config = work / "config/badi"
    config.mkdir(parents=True)
    config.chmod(0o700)
    (config / "settings.json").write_text(json.dumps(settings(1, "normal")))
    (config / "settings.json").chmod(0o600)
    environment = {**os.environ, "XDG_RUNTIME_DIR": str(runtime), "XDG_CONFIG_HOME": str(work / "config"),
                   "XDG_DATA_HOME": str(work / "data"), "XDG_STATE_HOME": str(work / "state")}
    process = subprocess.Popen([str(broker), "--model-directory", str(models)], env=environment,
                               stdin=subprocess.DEVNULL,
                               stdout=subprocess.DEVNULL, stderr=(work / "broker.log").open("w"))
    path = runtime / "badi/broker.sock"
    results = []
    try:
        deadline = time.monotonic() + 120
        while not path.exists():
            if process.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError("broker did not start")
            time.sleep(.2)
        assert health(path)["provider"] == "local_model"
        helper = Connection(path)
        helper.hello("session", ["desktop_signals"])
        status = helper.request("desktop.state", {"session": "active", "on_battery": True, "power_saver": False},
                                "desktop.status")["payload"]
        assert status["low_power"], status
        control = Connection(path)
        control.hello("cli", ["settings"])
        revision = 1
        # Warm the prompt cache once so the first arm is not the only cold one.
        type_text(path, TEXTS[0][:60])
        for repeat in range(arguments.repeats):
            for mode in (("normal", "quiet") if repeat % 2 == 0 else ("quiet", "normal")):
                control.request("settings.replace", {"expected_revision": revision,
                                                     "document": settings(revision + 1, mode)},
                                "settings.status")
                revision += 1
                for text_index, text in enumerate(TEXTS):
                    before = health(path)["metrics"]
                    calls_before = before["provider_calls"]
                    cpu_before = runtime_cpu_seconds(process.pid)
                    started = time.monotonic()
                    shown, covered_words = type_text(path, text)
                    elapsed = time.monotonic() - started
                    after = health(path)
                    results.append({
                        "mode": mode, "repeat": repeat, "text": text_index, "keys": len(text),
                        "seconds": round(elapsed, 2),
                        "provider_calls": after["metrics"]["provider_calls"] - calls_before,
                        "runtime_cpu_seconds": round(runtime_cpu_seconds(process.pid) - cpu_before, 2),
                        "shown": len(shown), "useful": sum(item["useful"] for item in shown),
                        "covered_words": covered_words,
                        "latency_ms": sorted(item["latency_ms"] for item in shown),
                        # Content-free reasons the broker showed nothing.
                        "no_suggestion": {reason: count - before["no_suggestion"][reason]
                                          for reason, count in after["metrics"]["no_suggestion"].items()
                                          if reason != "last"},
                    })
                    print(json.dumps(results[-1]), flush=True)
    finally:
        process.send_signal(signal.SIGTERM)
        try:
            process.wait(10)
        except subprocess.TimeoutExpired:
            process.kill()
        log = (work / "broker.log").read_text()
        shutil.rmtree(work, ignore_errors=True)
        if process.returncode not in (0, -signal.SIGTERM):
            print(log, file=sys.stderr)
    summary = {}
    for mode in ("normal", "quiet"):
        rows = [row for row in results if row["mode"] == mode]
        latencies = sorted(value for row in rows for value in row["latency_ms"])
        calls = sum(row["provider_calls"] for row in rows)
        cpu = sum(row["runtime_cpu_seconds"] for row in rows)
        useful_words = sum(row["covered_words"] for row in rows)
        summary[mode] = {
            "keys": sum(row["keys"] for row in rows), "provider_calls": calls,
            "runtime_cpu_seconds": round(cpu, 1), "shown": sum(row["shown"] for row in rows),
            "useful": sum(row["useful"] for row in rows), "covered_words": useful_words,
            "cpu_seconds_per_covered_word": round(cpu / useful_words, 2) if useful_words else None,
            "latency_ms_p50": latencies[len(latencies) // 2] if latencies else None,
            "latency_ms_p95": latencies[min(len(latencies) - 1, int(len(latencies) * .95))] if latencies else None,
        }
    report = {"schema": "badi.battery-trial.v1", "key_ms": KEY_MS, "word_pause_ms": WORD_PAUSE_MS,
              "sentence_pause_ms": SENTENCE_PAUSE_MS,
              "repeats": arguments.repeats, "results": results, "summary": summary}
    output = Path(arguments.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--broker", default=str(ROOT / "target/debug/badi-broker"))
    parser.add_argument("--repeats", type=int, default=2)
    parser.add_argument("--output", default=str(ROOT / "output/power/battery-trial.json"))
    run(parser.parse_args())
    sys.exit(0)
