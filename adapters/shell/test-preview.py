"""Real Bash/Readline PTY checks for the display builtin and shell-owned edits.

The controlled bridge supplies synthetic grants; the separate editor integration
suite exercises the real broker. No test line is submitted to the shell.
"""
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import shutil
import signal
import struct
import subprocess
import tempfile
import termios
import time
import unittest

SHELL = Path(__file__).resolve().parent
GREY = b'\x1b[38;2;128;128;128m'
INDEXED_GREY = b'\x1b[38;5;244m'
BASIC_ITALIC = b'\x1b[39;3m'
REQUEST = '\x18\t'
PROBE = '\x18\x02'

BRIDGE = r'''
import { createInterface } from 'node:readline';
import { appendFileSync } from 'node:fs';
let text = '', replace = '';
const send = (status, value = '') => process.stdout.write(`${status} ${value}\n`);
const encode = value => Buffer.from(value).toString('base64');
send('READY', '1');
for await (const line of createInterface({input: process.stdin, terminal: false})) {
  const command = JSON.parse(line);
  appendFileSync(process.env.BADI_TEST_EVENTS, JSON.stringify({operation:command.operation, applied:command.applied})+'\n');
  if (command.operation === 'status') send('READY', '1');
  else if (command.operation === 'suggest') {
    const before = Buffer.from(command.before, 'base64').toString('utf8');
    replace = before.endsWith('teh ') ? 'teh ' : before.endsWith('teh') ? 'teh' : '';
    text = replace ? replace.replace('teh', 'the') : before === 'از همکاری' ? ' شما' : ' ghost words';
    if (before === 'literal') text = ' $(touch should-not-exist)';
    if (before === 'controls') text = String.fromCharCode(27) + '[31mred';
    send('SUGGEST', encode(text) + (replace ? ' '+encode(replace) : ''));
  } else if (command.operation === 'accept') {
    send('INSERT', encode(text) + (replace ? ' '+encode(process.env.BADI_TEST_BAD_GRANT ? 'other' : replace) : ''));
  } else send('DONE');
}
'''


class ShellSession:
    def __init__(self, *, bad_grant=False, multiline=False, module=None, terminal='xterm-256color', colorterm='truecolor'):
        self.temporary = tempfile.TemporaryDirectory(prefix='badi-preview-test-')
        self.root = Path(self.temporary.name)
        self.events = self.root / 'events.jsonl'
        self.events.touch()
        editor = self.root / 'editors'
        (editor / 'shell').mkdir(parents=True)
        (editor / 'shell/bridge.mjs').write_text(BRIDGE)
        shutil.copy2(module or SHELL / 'build/badi-preview.so', editor / 'shell/badi-preview.so')
        (self.root / 'badi-completion-proof').touch()
        prompt = r'HEADER\n\[\e[32m\]TEST>\[\e[0m\] ' if multiline else 'TEST> '
        rc = self.root / 'bashrc'
        rc.write_text(f'PS1="{prompt}"\nHISTFILE=/dev/null\nPROMPT_COMMAND=\n'
                      f'source "{SHELL / "badi.bash"}"\n'
                      '_probe() { printf "\\nBUFFER:%s\\nPOINT:%s\\n" "$READLINE_LINE" "$READLINE_POINT"; }\n'
                      'bind -x \'"\\C-x\\C-b":_probe\'\n'
                      '_unload() { enable -d badi_preview; }\n'
                      'bind -x \'"\\C-x\\C-u":_unload\'\n')
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.chdir(self.root)
            os.execvpe('bash', ['bash', '--noprofile', '--rcfile', str(rc), '-i'], {
                **os.environ, 'BADI_EDITOR_DIR': str(editor), 'INPUTRC': '/dev/null',
                'TERM': terminal, 'COLORTERM': colorterm, 'LC_ALL': 'C.UTF-8',
                'BADI_TEST_EVENTS': str(self.events), 'BADI_TEST_BAD_GRANT': '1' if bad_grant else '',
            })
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 100, 0, 0))
        try:
            self.read_until(b'TEST>')
            self.read_for(.1)
        except BaseException:
            self.close()
            raise

    def send(self, text):
        os.write(self.fd, text.encode())

    def read_for(self, seconds=.15):
        result = bytearray()
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if select.select([self.fd], [], [], min(.025, max(0, deadline-time.monotonic())))[0]:
                result.extend(os.read(self.fd, 65536))
        return bytes(result)

    def read_until(self, needle, timeout=3):
        result = bytearray()
        deadline = time.monotonic() + timeout
        while needle not in result:
            if time.monotonic() > deadline:
                raise AssertionError(f'PTY did not emit {needle!r}: {bytes(result)!r}')
            result.extend(self.read_for(.025))
        return bytes(result)

    def request(self, prefix, display=' ghost words', *, style=GREY):
        self.send('\x15' + prefix + REQUEST)
        result = self.read_until(style + display.encode())
        return result + self.read_for()

    def buffer(self, expected):
        self.send(PROBE)
        return self.read_until(('BUFFER:' + expected + '\r\n').encode()) + self.read_for()

    def operations(self):
        return [json.loads(line) for line in self.events.read_text().splitlines()]

    def close(self):
        # No Enter, shell command, or global process matching is used for cleanup.
        try:
            if self.pid is not None:
                os.kill(self.pid, signal.SIGKILL)
                os.waitpid(self.pid, 0)
        finally:
            os.close(self.fd)
            self.temporary.cleanup()


class PreviewTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        subprocess.run(['python3', str(SHELL / 'build-preview.py')], check=True)

    def shell(self, **options):
        shell = ShellSession(**options)
        self.addCleanup(shell.close)
        return shell

    def test_inline_grey_at_caret_preserves_colored_multiline_prompt(self):
        shell = self.shell(multiline=True)
        output = shell.request('hello')
        self.assertIn(b'hello\x1b7' + GREY + b' ghost words\x1b[39m\x1b8', output)
        self.assertNotIn(b'\n', output, 'Request must not print a separate preview line or duplicate the prompt header')
        self.assertNotIn(b'HEADER', output)
        shell.buffer('hello')
        shell.send('\x02')  # Move left: clear and invalidate the old suggestion.
        cleared = shell.read_for()
        self.assertIn(b'\x1b[K', cleared)
        self.assertNotIn(GREY, cleared)
        shell.send('\x05' + REQUEST)
        shell.read_until(GREY)
        self.assertEqual([x['operation'] for x in shell.operations()].count('suggest'), 2)
        self.assertNotIn('accept', [x['operation'] for x in shell.operations()])

    def test_readable_styles_follow_advertised_terminal_capabilities(self):
        for terminal, colorterm, style, reset in (
            ('xterm-256color', '24bit', GREY, b'\x1b[39m'),
            ('xterm-256color', '', INDEXED_GREY, b'\x1b[39m'),
            ('xterm', '', BASIC_ITALIC, b'\x1b[23;39m'),
        ):
            with self.subTest(terminal=terminal, colorterm=colorterm):
                shell = self.shell(terminal=terminal, colorterm=colorterm)
                output = shell.request('hello', style=style)
                self.assertIn(b'\x1b7' + style + b' ghost words' + reset + b'\x1b8', output)
                self.assertNotIn(b'\x1b[90m', output)
                shell.buffer('hello')

    def test_accept_correction_and_native_undo(self):
        for trailing in ['', ' ']:
            with self.subTest(trailing=bool(trailing)):
                shell = self.shell()
                prefix = 'write teh' + trailing
                shell.request(prefix, '  teh → the')
                shell.send(REQUEST)
                shell.buffer('write the' + trailing)
                shell.send('\x1f')
                shell.buffer(prefix)
                self.assertIn({'operation': 'result', 'applied': True}, shell.operations())

    def test_unicode_caret_and_acceptance(self):
        shell = self.shell()
        shell.request('از همکاری', ' شما')
        shell.send(REQUEST)
        output = shell.buffer('از همکاری شما')
        version = tuple(int(part) for part in subprocess.check_output(
            ['bash', '-c', 'printf "%s %s" "${BASH_VERSINFO[0]}" "${BASH_VERSINFO[1]}"'], text=True).split())
        point = len('از همکاری شما') if version >= (5, 3) else len('از همکاری شما'.encode())
        self.assertIn(f'POINT:{point}\r\n'.encode(), output)
        shell.send('\x1f')
        shell.buffer('از همکاری')

    def test_expiry_clears_preview_and_requires_a_new_request(self):
        shell = self.shell()
        shell.request('hello')
        output = shell.read_for(4.2)
        self.assertIn(b'\x1b[K', output)
        self.assertNotIn(GREY, output)
        shell.send(REQUEST)
        shell.read_until(GREY)
        self.assertEqual([x['operation'] for x in shell.operations()].count('suggest'), 2)
        self.assertNotIn('accept', [x['operation'] for x in shell.operations()])

    def test_rejects_mismatched_replacement_without_mutation(self):
        shell = self.shell(bad_grant=True)
        shell.request('write teh', '  teh → the')
        shell.send(REQUEST)
        shell.buffer('write teh')
        self.assertIn({'operation': 'result', 'applied': False}, shell.operations())

    def test_dismissal_edit_and_ordinary_tab(self):
        shell = self.shell()
        shell.request('hello')
        shell.send('\x18\x1b')
        self.assertNotIn(GREY, shell.read_for())
        shell.buffer('hello')
        shell.request('hello')
        shell.send('!')
        output = shell.read_for()
        self.assertIn(b'\x1b[K', output)
        self.assertNotIn(GREY, output)
        shell.buffer('hello!')
        shell.send('\x15cat badi-completion-\t')
        shell.buffer('cat badi-completion-proof ')
        shell.send('\x18\x15')  # Unloading removes the owned Readline hooks.
        shell.read_for()
        shell.send('\x15cat badi-completion-\t')
        shell.buffer('cat badi-completion-proof ')

    def test_never_evaluates_accepted_shell_syntax(self):
        shell = self.shell()
        shell.request('literal', ' $(touch should-not-exist)')
        shell.send(REQUEST)
        shell.buffer('literal $(touch should-not-exist)')
        self.assertFalse((shell.root / 'should-not-exist').exists())

    def test_interrupt_recovers_prompt_and_hangup_exits_normally(self):
        shell = self.shell()
        shell.request('hello')
        shell.send('\x03')
        shell.read_until(b'TEST> ')
        output = shell.buffer('')
        self.assertNotIn(GREY, output)
        shell.request('hello')
        os.kill(shell.pid, signal.SIGHUP)
        deadline = time.monotonic() + 1
        while time.monotonic() < deadline:
            if os.waitpid(shell.pid, os.WNOHANG)[0]:
                shell.pid = None
                break
            time.sleep(.025)
        self.assertIsNone(shell.pid, 'The timed display hook must preserve Bash signal handling')

    def test_no_wrapping_or_control_sequence_preview(self):
        shell = self.shell()
        shell.send('controls' + REQUEST)
        output = shell.read_until(b'inline preview needs')
        self.assertNotIn(b'\x1b[31m', output)
        shell.buffer('controls')
        shell.request('hello')
        fcntl.ioctl(shell.fd, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 18, 0, 0))
        output = shell.read_for()
        self.assertNotIn(GREY, output)
        shell.send(REQUEST)
        shell.read_until(b'inline preview needs')
        self.assertNotIn('accept', [x['operation'] for x in shell.operations()])


if __name__ == '__main__':
    unittest.main()
