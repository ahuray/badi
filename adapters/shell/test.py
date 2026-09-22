"""Exercise real Bash Readline against an explicitly configured test broker."""
import os
import json
from pathlib import Path
import pty
import select
import signal
import subprocess
import tempfile
import time


ROOT = Path(__file__).resolve().parents[2]
PREFIX = os.environ.get("BADI_TEST_PREFIX", "thank you")
SUFFIX = os.environ.get("BADI_TEST_SUFFIX", " for your time")


def main():
    with tempfile.TemporaryDirectory(prefix="badi-readline-") as directory:
        root = Path(directory)
        (root / "badi-completion-proof").touch()
        rc = root / "bashrc"
        rc.write_text('PS1="BADI-TEST> "\nHISTFILE=/dev/null\nPROMPT_COMMAND=\n'
                      f'source "{ROOT}/adapters/shell/badi.bash"\n'
                      '_badi_probe() { printf "\\nBUFFER:%s\\n" "$READLINE_LINE"; }\n'
                      'bind -x \'"\\C-x\\C-b":_badi_probe\'\n')
        pid, descriptor = pty.fork()
        if pid == 0:
            os.chdir(root)
            os.execvpe("bash", ["bash", "--noprofile", "--rcfile", str(rc), "-i"],
                       {**os.environ, "BADI_EDITOR_DIR": str(ROOT / "adapters"),
                        "TERM": "xterm-256color", "INPUTRC": "/dev/null"})
        transcript = bytearray()

        def expect(text, timeout=4):
            start = len(transcript)
            deadline = time.monotonic() + timeout
            while text.encode() not in transcript[start:]:
                if time.monotonic() > deadline:
                    raise AssertionError(f"Readline did not report {text!r}: {transcript[start:].decode(errors='replace')}")
                if select.select([descriptor], [], [], .05)[0]:
                    chunk = os.read(descriptor, 65536)
                    if not chunk:
                        raise AssertionError("Test shell closed unexpectedly")
                    transcript.extend(chunk)

        def send(text):
            os.write(descriptor, text.encode())

        def control(*args):
            result = subprocess.run([str(ROOT / 'target/debug/badictl'), *args],
                                    check=True, capture_output=True, text=True, timeout=3)
            return json.loads(result.stdout)

        try:
            expect("BADI-TEST> ")
            send(PREFIX + "\x18\t")
            expect("\x1b[90m" + SUFFIX)
            send("\x18\t\x18\x02")
            expect("BUFFER:" + PREFIX + SUFFIX)
            send("\x1f\x18\x02")
            expect("BUFFER:" + PREFIX + "\r\n")
            send("\x15cat badi-completion-\t\x18\x02")
            expect("BUFFER:cat badi-completion-proof ")
            send("\x15" + PREFIX + "\x18\t")
            expect("\x1b[90m" + SUFFIX)
            send("\x18\x1b\x18\x02")
            expect("BUFFER:" + PREFIX + "\r\n")
            previous = control('status')['metrics']['context_updates']
            try:
                control('pause', 'on')
                send("\x15never transmit this disabled buffer\x18\t")
                expect('Badi: app_disabled_or_model_paused')
                assert control('status')['metrics']['context_updates'] == previous
            finally:
                control('pause', 'off')
            send("\x15" + PREFIX + "\x18\t")
            expect("\x1b[90m" + SUFFIX)
            assert b"command not found" not in transcript
            print("Bash: request, accept, undo, dismissal, permission preflight, resume and ordinary Tab completion passed; no line was submitted.")
        finally:
            # Never submit a pending test line to exit an interactive shell.
            os.kill(pid, signal.SIGHUP)
            deadline = time.monotonic() + 3
            while os.waitpid(pid, os.WNOHANG)[0] == 0:
                if time.monotonic() > deadline:
                    os.kill(pid, signal.SIGKILL)
                    os.waitpid(pid, 0)
                    break
                time.sleep(.02)
            os.close(descriptor)


if __name__ == "__main__":
    main()
