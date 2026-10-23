#!/usr/bin/env python3
"""Exercise native app classes and a synthetic field observer on private D-Bus.

Covers unavailable apps, IME-parity browser/desktop targets, synthetic desktop
fields and native manual apps. No display, input device, normal service, editor
or desktop accessibility tree is used. CommitString is observed as a dispatch,
never claimed as an applied edit.
"""
import argparse
import importlib.util
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

ROOT = Path(__file__).resolve().parents[3]
PREFIX, SUFFIX = 'thank you', ' for your time'
# The panel shows a suggestion without its separating space.
SHOWN = SUFFIX.lstrip()
FIXTURE_APP = 'badi-native-fixture'
UNAVAILABLE_APPS = ('firefox', 'zen-browser', 'app.zen_browser.zen', 'librewolf', 'obsidian')
# Mixed-case Chromium- and Gecko-family programs fold to granted ids without an observer rule.
UNOBSERVED_FAMILY_PROGRAMS = {'chrome': 'chrome', 'Google-chrome': 'google-chrome',
                              'brave': 'brave', 'Brave-browser': 'brave-browser',
                              'chrome-app.hey.com__-Default': 'chrome-app.hey.com__-default',
                              'com.google.Chrome': 'com.google.chrome',
                              'Vivaldi-stable': 'vivaldi-stable', 'code-oss': 'code-oss',
                              # Gecko: Zen Twilight and a PWAsForFirefox window.
                              'Zen-Twilight': 'zen-twilight',
                              'FFPWA-01HVY3F0GDT6DCG5V6TTRHK4BT': 'ffpwa-01hvy3f0gdt6dcg5v6ttrhk4bt'}
IME_PARITY_APPS = ('chromium', 'chromium-browser', 'brave-origin', 'zen', 'chatgpt', 'code', 'cursor', 'discord')
RIGHT, CONTROL = 0xff53, 1 << 2
# Chromium's text-input-v3 path adds UppercaseWords (0x80072 in total).
CHROMIUM_HINT = 1 << 19
TERMINAL_HINT = 1 << 32
ALLOWED_URI = 'https://allowed.example.test/document'


def private_file(path, content):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    path.write_text(content)
    path.chmod(0o600)


def stop(process):
    if process is None or process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=4)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=4)
        raise AssertionError('A private fixture process required forced cleanup') from None


class FakeBroker:
    """Speaks just enough broker v2 to inject frames the real broker never sends."""

    def __init__(self, path):
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.listener.bind(str(path))
        path.chmod(0o600)
        self.listener.listen(2)
        self.listener.setblocking(False)
        self.peer = None
        self.buffer = bytearray()
        self.received = []
        self.connections = 0
        self.client_closes = 0
        self.epoch = 100

    def close(self):
        if self.peer is not None:
            self.peer.close()
        self.listener.close()

    def send(self, value):
        body = json.dumps(value).encode()
        self.peer.sendall(len(body).to_bytes(4, 'little') + body)

    def poll(self):
        if self.peer is None:
            try:
                self.peer, _ = self.listener.accept()
            except BlockingIOError:
                return
            self.peer.setblocking(False)
            self.buffer.clear()
            self.connections += 1
        while True:
            try:
                chunk = self.peer.recv(65_536)
            except BlockingIOError:
                break
            if not chunk:
                self.peer.close()
                self.peer = None
                self.client_closes += 1
                return
            self.buffer.extend(chunk)
        while len(self.buffer) >= 4 and len(self.buffer) >= 4 + int.from_bytes(self.buffer[:4], 'little'):
            size = int.from_bytes(self.buffer[:4], 'little')
            frame = json.loads(self.buffer[4:4 + size])
            del self.buffer[:4 + size]
            self.received.append(frame)
            if frame['type'] == 'hello':
                self.epoch += 1
                self.send({'v': 2, 'type': 'hello.ack', 'id': 'fcitx.hello', 'mono_ms': 0, 'payload': {
                    'selected_v': 2, 'connection_id': f'fake-{self.connections}',
                    'enabled_capabilities': frame['payload']['capabilities'], 'max_frame_bytes': 65536,
                    'max_before_chars': 512, 'max_after_chars': 128, 'max_suggestion_chars': 64,
                    'max_suggestion_words': 8, 'paused': False}})
                self.send({'v': 2, 'type': 'authority.changed', 'mono_ms': 0, 'payload': {
                    'authority_epoch': self.epoch, 'settings_revision': 1, 'paused': False}})
            elif frame['type'] == 'policy.query':
                self.send({'v': 2, 'type': 'policy.status', 'id': frame['id'], 'mono_ms': 0, 'payload': {
                    'authority_epoch': self.epoch, 'settings_revision': 1, 'paused': False,
                    'activation': 'always', 'context_allowed': True, 'display_allowed': True,
                    'suggestions_allowed': True, 'learning_allowed': False, 'reason': 'matched_rule'}})

    def count(self, kind):
        return sum(frame['type'] == kind for frame in self.received)

    def last(self, kind):
        return next(frame for frame in reversed(self.received) if frame['type'] == kind)

    def respond(self, request, kind, payload):
        self.send({'v': 2, 'type': kind, 'id': request['id'], 'session_id': request['session_id'],
                   'focus_epoch': request['focus_epoch'], 'revision': request['revision'], 'mono_ms': 0,
                   'payload': {'fingerprint': request['payload']['fingerprint'], **payload}})


def session(root, report):
    import gi
    gi.require_version('Gio', '2.0')
    from gi.repository import Gio, GLib
    spec = importlib.util.spec_from_file_location('desktop_tab', Path(__file__).with_name('desktop-tab.py'))
    desktop = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(desktop)
    sys.path.insert(0, str(ROOT / 'adapters/accessibility'))
    from contract import SCHEMA, Denied, Observer
    from daemon import Daemon

    class Backend:
        def __init__(self):
            self.deadline = 0
            self.value = PREFIX
            self.caret = None
            self.uri = 'https://blocked.example.test/document'
            self.field = 1
            self.reads = 0
            self.inspections = 0
            self.browser = True
            self.invalid_target = None
            self.apps = {FIXTURE_APP}
            self.requested = []
            self.role = 'text'

        def budget(self):
            if time.monotonic() >= self.deadline:
                raise Denied('operation_timeout')

        def metadata(self, app, _calibrate=False, _check_lock=False, _caret_rect=None):
            self.budget()
            self.inspections += 1
            self.requested.append(app)
            if app not in self.apps:
                raise Denied('unsupported_app')
            return {'bus': ':1.synthetic', 'path': f'/synthetic/field/{self.field}',
                    'process_id': os.getpid(), 'app_id': app, 'uri': self.uri,
                    'browser': self.browser, 'web': self.browser, 'role': self.role, 'tag': 'textarea',
                    'input_type': 'textarea', 'focused': True, 'editable': True, 'showing': True, 'visible': True,
                    'enabled': True, **self.position({})}

        def position(self, metadata):
            self.budget()
            return {**metadata, 'selection_count': 0, 'total_chars': len(self.value),
                    'caret': len(self.value) if self.caret is None else self.caret}

        def text(self, metadata, start, end):
            self.budget()
            self.reads += 1
            return self.value[start:end]

    backend = Backend()
    daemon = Daemon(root / 'runtime/badi/accessibility.sock', None, GLib)
    daemon.backend = backend
    daemon.observer = Observer(backend)
    daemon.observer.notify = daemon.broadcast
    # This fixture deliberately declines UI rendering; Fcitx's client-side
    # candidate signal remains observable without a window or synthetic keys.
    render_calls = []
    render_result = {'value': False}
    daemon.observer.render = lambda focus, _text, _ttl: render_calls.append(focus) or render_result['value']
    original_request = daemon.observer.request
    observer_calls = {'snapshot': 0, 'preview': 0, 'hide': 0}
    silent_change = {'op': None, 'call': 0, 'mode': 'repeat', 'triggered': False}

    def observer_request(request):
        op = request.get('op')
        if op in observer_calls:
            observer_calls[op] += 1
        trigger = op == silent_change['op'] and observer_calls[op] == silent_change['call']
        if trigger and silent_change['mode'] == 'deny':
            silent_change['triggered'] = True
            return {'schema': SCHEMA, 'id': request.get('id'), 'ok': False,
                    'error': 'operation_timeout', 'epoch': daemon.observer.epoch}
        if trigger and silent_change['mode'] == 'repeat':
            # No observer invalidation or Fcitx surrounding update: the app's
            # publication gap leaves an identical cropped toolkit buffer.
            backend.value *= 2
        if trigger and silent_change['mode'] == 'length':
            backend.caret = len(backend.value)
            backend.value += 'x'
        response = original_request(request)
        if op == 'inspect' and response.get('ok') and backend.invalid_target is not None:
            response['focus']['target'] = backend.invalid_target
        if trigger:
            silent_change['triggered'] = True
            if silent_change['mode'] == 'total' and response.get('ok'):
                # Independently corrupt only the counter in a later snapshot;
                # identical caret/text must not hide changed document length.
                response['focus']['total_chars'] += 1
            if silent_change['mode'] == 'before' and response.get('ok'):
                # Same field, caret and length; the observed prefix alone
                # disagrees with Fcitx's live surrounding text.
                response['focus']['before'] = 'observer disagrees'
        return response

    daemon.observer.request = observer_request
    original_send = daemon.send
    delayed_preview = {'enabled': False, 'started': 0, 'sent': 0}
    preview_fields = []

    def send(fd, document):
        preview = 'rendered' in document.get('focus', {})
        if preview:
            preview_fields.append(document['focus']['binding']['path'])
        if delayed_preview['enabled'] and preview:
            delayed_preview['started'] += 1

            def finish():
                delayed_preview['sent'] += 1
                original_send(fd, document)
                return False

            # Below the real client's 500 ms deadline, long enough to issue an
            # ordinary key before the preview acknowledgement reaches the addon.
            GLib.timeout_add(250, finish)
        else:
            original_send(fd, document)

    daemon.send = send
    daemon.bind()
    checks = []
    context = None
    broker = None
    fcitx = None
    fake = None
    broker_log = (report / 'broker.log').open('w')
    fcitx_log = (report / 'fcitx.log').open('w')

    def control(*args):
        return json.loads(subprocess.check_output([str(ROOT / 'target/debug/badictl'), *args], text=True, timeout=4))

    def wait(predicate, label, timeout=4):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            desktop.pump()
            if broker is not None and broker.poll() is not None:
                raise AssertionError('Private broker exited during ' + label)
            if fcitx is not None and fcitx.poll() is not None:
                raise AssertionError('Private Fcitx exited during ' + label)
            if predicate():
                return
            time.sleep(.01)
        raise AssertionError('Timed out: ' + label)

    def quiet(seconds=.35):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            desktop.pump()
            time.sleep(.01)

    def start_broker():
        process = subprocess.Popen([str(ROOT / 'target/debug/badi-broker'), '--provider', 'phrase'],
                                   stdin=subprocess.DEVNULL, stdout=broker_log, stderr=broker_log)
        deadline = time.monotonic() + 4
        while not (root / 'runtime/badi/broker.sock').exists():
            if process.poll() is not None or time.monotonic() > deadline:
                raise AssertionError('Private broker startup failed')
            time.sleep(.01)
        return process

    def change(value, *, uri=None, new_field=False):
        backend.value = value
        backend.caret = None
        if uri is not None:
            backend.uri = uri
        if new_field:
            backend.field += 1
        daemon.observer.invalidate('fixture_field_changed')
        context.text(value)

    def preedit():
        for name, values in reversed(context.events):
            if name == 'UpdateFormattedPreedit':
                return ''.join(part[0] for part in values[0])
        return ''

    def auxiliary():
        for name, values in reversed(context.events):
            if name == 'UpdateClientSideUI':
                return ''.join(part[0] for part in values[2])
        return ''

    def reason_count(reason):
        # The addon's content-free activity snapshot; the private debug
        # control file below enables it for this run only.
        try:
            native = json.loads((root / 'runtime/badi/debug-native.json').read_text())
        except FileNotFoundError:
            return 0
        return native['reason_counts'].get(reason, 0)

    try:
        # Inspect the loaded addon at its actual transport boundary, then let
        # that same process reconnect to the real broker for the remaining
        # checks. Native Fcitx has no field-owned replacement transaction.
        endpoint = root / 'runtime/badi/broker.sock'
        hello_peer = None
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
            listener.bind(str(endpoint))
            endpoint.chmod(0o600)
            listener.listen(1)
            listener.setblocking(False)
            try:
                fcitx = subprocess.Popen(['fcitx5', '-D', '-k', '--disable=all',
                                          '--enable=dbus,dbusfrontend,keyboard,unicode,badi'],
                                         stdin=subprocess.DEVNULL, stdout=fcitx_log, stderr=fcitx_log)
                hello_frame = bytearray()

                def received_hello():
                    nonlocal hello_peer
                    if hello_peer is None:
                        try:
                            hello_peer, _ = listener.accept()
                        except BlockingIOError:
                            return False
                        credentials = hello_peer.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)
                        peer_pid, peer_uid, _ = struct.unpack('3i', credentials)
                        assert peer_pid == fcitx.pid and peer_uid == os.getuid()
                        hello_peer.setblocking(False)
                    try:
                        chunk = hello_peer.recv(16_389 - len(hello_frame))
                    except BlockingIOError:
                        return False
                    assert chunk, 'Loaded addon closed before its hello was captured'
                    hello_frame.extend(chunk)
                    if len(hello_frame) < 4:
                        return False
                    size = int.from_bytes(hello_frame[:4], 'little')
                    assert 0 < size <= 16_384, 'Loaded addon hello exceeded its bounded capture'
                    assert len(hello_frame) <= size + 4, 'Unexpected traffic before hello acknowledgement'
                    return len(hello_frame) == size + 4

                wait(received_hello, 'actual loaded addon hello')
                hello = json.loads(hello_frame[4:])
                assert hello['type'] == 'hello'
                assert hello['payload']['adapter'] == {
                    'kind': 'fcitx', 'name': 'badi-fcitx5', 'version': '0.1.0'}
                assert hello['payload']['capabilities'] == [
                    'context', 'suggestion', 'commit.dispatched_unverified', 'text_replacement',
                    'control', 'policy', 'long_reading_window', 'selection_revision']
                checks.append('Actual loaded native addon hello negotiates exact text replacement')
            finally:
                if hello_peer is not None:
                    hello_peer.close()
                endpoint.unlink()
        broker = start_broker()
        bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)

        def registered():
            return bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
                'org.freedesktop.DBus', 'NameHasOwner', GLib.Variant('(s)', ('org.fcitx.Fcitx5',)),
                None, Gio.DBusCallFlags.NONE, 1000, None).unpack()[0]

        wait(registered, 'private Fcitx registration')
        for app in (*UNAVAILABLE_APPS, *UNOBSERVED_FAMILY_PROGRAMS):
            reads = backend.reads
            inspections = backend.inspections
            context = desktop.InputContext(app)
            assert not context.key(desktop.TAB), app + ' must retain Tab without surrounding text'
            assert not context.key(ord(' '), 5), app + ' Ctrl+Shift+Space reaches an app Badi cannot edit'
            quiet(.05)
            assert auxiliary() == '', app + ' shows no unavailable notice'
            for _ in range(3):
                context.text(PREFIX)
                quiet(.06)
                assert auxiliary() == '', app + ' republishing stays quiet'
            assert not context.key(desktop.TAB), app + ' must retain ordinary Tab'
            assert not context.key(0xff51) and not context.key(0xff57), app + ' must retain navigation'
            assert not context.key(ord('Y'), 5), app + ' cannot accept unsupported native editing'
            assert auxiliary() == '', app + ' still shows no notice'
            assert 'Tab to accept' not in json.dumps(context.events)
            assert backend.reads == reads and backend.inspections == inspections
            assert context.candidate() is None and context.commits() == []
            assert control('status')['metrics']['context_updates'] == 0
            context.close()
            context = None
        assert not {*UNAVAILABLE_APPS, *UNOBSERVED_FAMILY_PROGRAMS, *UNOBSERVED_FAMILY_PROGRAMS.values()} & \
            set(backend.requested), 'Unavailable apps are never inspected'
        checks.append('Allowed non-Zen Gecko/Obsidian identities acquire no prose, publish no context and preserve Tab/navigation')
        checks.append('Granted Chrome and Brave aliases, Chromium web-app, Flatpak, Vivaldi and Code-OSS identities, '
                      'Zen Twilight and a PWAsForFirefox window without an observer rule never take the manual path')
        checks.append('Ctrl+Shift+Space in an app Badi cannot edit reaches the app and publishes nothing')

        context = desktop.InputContext('firefox')
        assert not context.key(ord(' '), 5)
        quiet(.05)
        assert auxiliary() == '', 'ungranted Firefox keeps Ctrl+Shift+Space'
        context.ic('FocusOut')
        context.ic('FocusIn')
        quiet(.1)
        assert auxiliary() == '', 'Focus recovery must not invent a notice'
        assert context.key(ord('U'), 5)
        assert context.key(ord('4'))
        wait(lambda: preedit() == 'U+4', 'unavailable app real Unicode preedit')
        assert auxiliary() == '', 'Foreign composition owns the panel'
        assert context.key(desktop.ESCAPE)
        wait(lambda: preedit() == '' and auxiliary() == '', 'foreign composition clears')
        assert context.commits() == [] and control('status')['metrics']['context_updates'] == 0
        context.close()
        context = None
        checks.append('Ungranted Firefox leaves Ctrl+Shift+Space to the app and still yields to Unicode composition')

        context = desktop.InputContext(FIXTURE_APP)
        context.text(PREFIX)
        wait(lambda: backend.inspections > 0, 'automatic field metadata observation')
        quiet(.4)
        assert backend.reads == 0 and control('status')['metrics']['context_updates'] == 0
        assert context.candidate() is None
        checks.append('Browser-origin target is unavailable even for an unrecognized program alias')

        assert not context.key(ord(' '), 5)
        quiet(.05)
        assert auxiliary() == '', 'ungranted browser keeps Ctrl+Shift+Space'
        inspections = backend.inspections
        context.text(PREFIX)
        wait(lambda: backend.inspections > inspections,
             'unknown-alias browser republishing reinspects authority')
        assert backend.reads == 0 and control('status')['metrics']['context_updates'] == 0
        checks.append('Ctrl+Shift+Space in an ungranted browser reaches the app; surrounding publication still reinspects')

        backend.uri = 'https://allowed.example.test/document'
        daemon.observer.invalidate('fixture_field_changed')
        context.text(PREFIX)
        quiet(.3)
        hides = observer_calls['hide']
        assert not context.key(desktop.TAB)
        quiet(.1)
        assert observer_calls['hide'] == hides, 'A key without a requested preview sends the observer nothing'
        assert backend.reads == 0 and context.candidate() is None and context.commits() == []
        assert control('status')['metrics']['context_updates'] == 0
        assert control('status')['metrics']['provider_calls'] == 0
        checks.append('Configured app and exact origin allow rules cannot bypass unavailable browser editing')

        # The following automatic tests represent an explicitly synthetic
        # cooperative desktop editor, never a Chromium/Electron proof cell.
        backend.browser = False
        backend.value = PREFIX + ' mismatched'
        daemon.observer.invalidate('fixture_field_changed')
        context.text(PREFIX)
        wait(lambda: backend.reads > 0, 'authorized observer snapshot')
        quiet(.25)
        assert control('status')['metrics']['context_updates'] == 0 and context.candidate() is None
        checks.append('Observer and native surrounding-text disagreement blocks model context publication')

        change(PREFIX, uri='https://allowed.example.test/document')
        wait(lambda: context.candidate() == SHOWN, 'automatic observed suggestion')
        metrics = control('status')['metrics']
        assert metrics['context_updates'] == 1 and metrics['provider_calls'] == 1
        assert backend.reads >= 2
        checks.append('Synthetic desktop inspect → exact app policy → corroborated snapshot → real broker suggestion')

        context.text(PREFIX)
        quiet(.25)
        assert context.candidate() == SHOWN
        assert control('status')['metrics']['context_updates'] == metrics['context_updates']
        checks.append('Unchanged toolkit context preserves the visible candidate and session')

        inspections, awaiting = backend.inspections, reason_count('observer_awaiting_input')
        daemon.observer.invalidate('field_changed')
        wait(lambda: context.candidate() is None, 'a field event retires the observed candidate')
        quiet(.5)
        assert backend.inspections == inspections and reason_count('observer_awaiting_input') > awaiting, \
            'A field event without Fcitx input waits for input instead of inspecting again'
        context.text(PREFIX)
        wait(lambda: context.candidate() == SHOWN, 'the next surrounding-text publication inspects again')
        checks.append('A field event without Fcitx input waits for the next publication before inspecting again')

        assert context.key(RIGHT, CONTROL)
        wait(lambda: context.commits() == [SUFFIX], 'Ctrl+Right dispatch after observed verification')
        checks.append('Explicit Ctrl+Right verifies the current observed field and dispatches the whole '
                      'suggestion as one exact CommitString')

        for invalid_target in ({}, {'kind': 'unknown', 'target_id': 'invalid-kind'},
                               {'kind': 'desktop_application', 'app_id': 'another-app', 'target_id': 'wrong-app'}):
            reads = backend.reads
            updates = control('status')['metrics']['context_updates']
            inspections = backend.inspections
            backend.invalid_target = invalid_target
            change(PREFIX, new_field=True)
            wait(lambda: backend.inspections > inspections, 'invalid target metadata observation')
            quiet(.15)
            assert not context.key(desktop.TAB)
            assert backend.reads == reads and context.candidate() is None
            assert context.commits() == [SUFFIX]
            assert control('status')['metrics']['context_updates'] == updates
            backend.invalid_target = None
            change(PREFIX, new_field=True)
            wait(lambda: context.candidate() == SHOWN, 'fresh synthetic desktop binding after malformed metadata')
        checks.append('Missing, unknown and mismatched target metadata retire prior authority without prose acquisition')

        repeated_block = 'x' * (512 - len(PREFIX) - 1) + '\n' + PREFIX
        assert len(repeated_block) == 512 and (repeated_block * 2)[-512:] == repeated_block

        def arm_change(op, call=1, mode='repeat'):
            silent_change.update(op=op, call=observer_calls[op] + call, mode=mode, triggered=False)

        for phase, op, call, expected_contexts, mode in (
                ('initial acquisition', 'snapshot', 1, 0, 'repeat'),
                ('suggestion verification', 'snapshot', 2, 1, 'repeat'),
                ('preview rendering', 'preview', 1, 1, 'repeat'),
                ('preview rendering with unchanged caret', 'preview', 1, 1, 'length')):
            metrics_before = control('status')['metrics']['context_updates']
            commits_before = context.commits()
            renders_before = len(render_calls)
            arm_change(op, call, mode)
            change(repeated_block, new_field=True)
            wait(lambda: silent_change['triggered'], 'silent absolute caret move during ' + phase)
            quiet(.4)
            assert backend.value == (repeated_block * 2 if mode == 'repeat' else repeated_block + 'x')
            assert control('status')['metrics']['context_updates'] == metrics_before + expected_contexts
            assert context.candidate() is None and context.commits() == commits_before, phase
            assert len(render_calls) == renders_before, 'Stale field reached renderer during ' + phase
            silent_change['op'] = None
            detail = 'Same bounded text with absolute AX caret 512 → 1024' if mode == 'repeat' else 'Changed document length'
            checks.append(detail + ' is rejected before render/dispatch during ' + phase)

        for mode, detail in (('repeat', 'absolute caret changes beyond the identical cropped text'),
                             ('total', 'document length changes with identical caret and text')):
            change(repeated_block, new_field=True)
            wait(lambda: context.candidate() == SHOWN, 'fresh repeated-text candidate before stale commit')
            commits_before = context.commits()
            arm_change('snapshot', mode=mode)
            assert context.key(desktop.TAB)
            wait(lambda: silent_change['triggered'], 'stale commit observer snapshot')
            wait(lambda: context.candidate() is None, 'stale observed commit clears candidate')
            quiet(.15)
            assert context.commits() == commits_before
            silent_change['op'] = None
            checks.append('Commit dispatch is rejected when ' + detail)

        delayed_preview['enabled'] = True
        change(PREFIX + ' x')
        quiet(.15)
        change(PREFIX)
        wait(lambda: delayed_preview['started'] > 0, 'delayed preview acknowledgement')
        assert context.candidate() is None
        previous_commits = context.commits()
        # Deliberately do not publish the app's next surrounding text yet. The
        # pre-IME key event must fence the old reply during that toolkit gap.
        assert not context.key(ord('x'))
        wait(lambda: delayed_preview['sent'] == delayed_preview['started'], 'late preview reply after ordinary key')
        quiet(.15)
        assert context.candidate() is None and context.commits() == previous_commits
        delayed_preview['enabled'] = False
        checks.append('Ordinary key fences an in-flight preview before surrounding text changes; late reply stays hidden')

        delayed_preview['enabled'] = True
        started = delayed_preview['started']
        change(PREFIX + ' x')
        quiet(.15)
        change(PREFIX)
        wait(lambda: delayed_preview['started'] > started, 'pending preview before field switch')
        change(PREFIX, new_field=True)
        delayed_preview['enabled'] = False
        wait(lambda: context.candidate() == SHOWN, 'automatic new field recovery behind stale observer RPC')
        assert preview_fields[-1] == f'/synthetic/field/{backend.field}'
        assert context.commits() == previous_commits
        checks.append('A field switch during an observer RPC retries boundedly and automatically previews the new field')

        change(PREFIX + ' x')
        quiet(.15)
        change(PREFIX)
        wait(lambda: context.candidate() == SHOWN, 'fresh suggestion before same-text field switch')
        backend.browser = True
        change(PREFIX, uri='https://allowed.example.test/document', new_field=True)
        wait(lambda: context.candidate() is None, 'same-text field invalidation')
        previous = len(context.commits())
        context.key(desktop.TAB)
        quiet(.3)
        assert len(context.commits()) == previous and context.candidate() is None
        checks.append('Same-text switch to an allowed browser origin retires desktop authority and cannot commit')

        backend.browser = False
        change(PREFIX, uri='https://allowed.example.test/document')
        wait(lambda: context.candidate() == SHOWN, 'policy recovery before pause')
        control('pause', 'on')
        wait(lambda: context.candidate() is None, 'pause clears candidate')
        reads = backend.reads
        change('never acquire this paused synthetic text')
        quiet(.4)
        assert backend.reads == reads
        checks.append('Broker pause clears UI and blocks subsequent observer prose reads')
        control('pause', 'off')
        change(PREFIX)
        wait(lambda: context.candidate() == SHOWN, 'resume with fresh context')

        # Fcitx's shipped Unicode module owns real foreign preedit. Its
        # default direct-mode shortcut is Control+Shift+U; only this private
        # D-Bus input context receives the key, with no graphical input device.
        assert context.key(ord('U'), 5)
        assert context.key(ord('4'))
        wait(lambda: preedit() == 'U+4', 'real Unicode formatted preedit')
        wait(lambda: context.candidate() != SHOWN, 'Unicode composition takes over Badi UI')
        reads = backend.reads
        context.text(PREFIX)
        quiet(.3)
        assert backend.reads == reads and context.candidate() != SHOWN
        assert context.key(desktop.ESCAPE)
        wait(lambda: context.candidate() == SHOWN, 'automatic recovery after foreign preedit clears')
        assert preedit() == ''
        checks.append('Real Unicode addon preedit takes priority, prevents prose reads, then automatically resumes on clear')

        context.ic('FocusOut')
        wait(lambda: context.candidate() is None, 'focus loss clears candidate')
        context.ic('FocusIn')
        context.text(PREFIX)
        wait(lambda: context.candidate() == SHOWN, 'fresh focus recovery')
        checks.append('Focus loss clears and fresh focus obtains new authority')

        stop(broker)
        broker = None
        wait(lambda: context.candidate() is None, 'broker exit clears candidate')
        broker = start_broker()
        quiet(.8)
        change(PREFIX)
        wait(lambda: context.candidate() == SHOWN, 'same input context after broker restart', timeout=6)
        assert context.key(RIGHT, CONTROL)
        wait(lambda: len(context.commits()) == previous + 1, 'recovered Ctrl+Right dispatch')
        assert context.commits()[-1] == SUFFIX
        checks.append('Same live input context recovers after broker restart with a fresh verified dispatch')

        # The replacement broker presents the same initial authority as this
        # one, like a restart with unchanged settings: the transport alone reconnects.
        stop(broker)
        broker = None
        broker = start_broker()
        wait(lambda: control('status')['sessions'] == 1, 'observed session reopened without Fcitx input', timeout=8)
        quiet(.4)
        assert control('status')['metrics']['context_updates'] == 0 and context.candidate() is None, \
            'A reconnect alone must not republish unchanged observed prose'
        assert not context.key(desktop.TAB), 'Tab without a suggestion stays the application Tab after a reconnect'
        assert context.key(ord(' '), 5), 'Explicit chord after an unchanged reconnect must read current Fcitx text'
        wait(lambda: context.candidate() == SHOWN, 'explicit observed request after unchanged reconnect', timeout=6)
        checks.append('Unchanged-authority reconnect republishes no observed prose automatically; the explicit chord '
                      'requests afresh while plain Tab stays the application Tab')

        # fcitx5-qt 5.1.16 (fcitx/fcitx5-qt@bac684ce07d9) withdraws
        # SurroundingText right after every text update. An observed field
        # keeps that text; its observer must still agree with it.
        surrounding, password = 1 << 6, 1 << 3

        def context_updates():
            return control('status')['metrics']['context_updates']

        def qt_text(value):
            context.ic('SetCapability', '(t)', (desktop.CAPABILITIES,))
            context.text(value)
            context.ic('SetCapability', '(t)', (desktop.CAPABILITIES & ~surrounding,))

        context.close()
        context = desktop.InputContext(FIXTURE_APP)
        backend.value, backend.caret = PREFIX, None
        backend.field += 1
        daemon.observer.invalidate('fixture_field_changed')
        echoes = reason_count('surrounding_echo')
        qt_text(PREFIX)
        wait(lambda: context.candidate() == SHOWN, 'observed suggestion under the fcitx5-qt echo', timeout=6)
        assert reason_count('surrounding_echo') > echoes
        assert context.key(RIGHT, CONTROL)
        wait(lambda: context.commits() == [SUFFIX], 'dispatch under the fcitx5-qt echo')
        checks.append('fcitx5-qt withdrawing SurroundingText after each text update keeps an observed field '
                      'suggesting and dispatching after observer corroboration')

        mismatch, before = reason_count('observer_context_mismatch'), context_updates()
        backend.value = PREFIX + ' mismatched'
        daemon.observer.invalidate('fixture_field_changed')
        qt_text(PREFIX)
        wait(lambda: reason_count('observer_context_mismatch') > mismatch, 'observer disagreement under the echo')
        quiet(.2)
        assert context_updates() == before and context.candidate() is None
        backend.value = PREFIX
        daemon.observer.invalidate('fixture_field_changed')
        qt_text(PREFIX)
        wait(lambda: context.candidate() == SHOWN, 'agreement restores the suggestion under the echo')
        context.ic('SetCapability', '(t)', ((desktop.CAPABILITIES & ~surrounding) | password,))
        wait(lambda: context.candidate() is None, 'a password capability ends the echo')
        context.text(PREFIX)
        quiet(.3)
        assert not context.key(desktop.TAB) and context.candidate() is None and context.commits() == [SUFFIX]
        checks.append('Under the echo, observer disagreement still blocks context, and any other capability '
                      'change, such as a password field, ends it')

        context.close()
        context = desktop.InputContext('omawrite')
        before, reads = context_updates(), backend.reads
        qt_text(PREFIX)
        quiet(.3)
        context.key(desktop.TAB)
        quiet(.3)
        assert context.candidate() is None and context.commits() == []
        assert context_updates() == before and backend.reads == reads
        checks.append('Without an observed field, the echo leaves the unknown-identity manual path closed')

        context.close()
        context = desktop.InputContext('omawrite')
        context.text(PREFIX)
        updates = control('status')['metrics']['context_updates']
        reads = backend.reads
        quiet(.3)
        assert context.candidate() is None and control('status')['metrics']['context_updates'] == updates
        assert backend.reads == reads
        assert context.key(desktop.TAB), 'Unobserved native manual invocation remains available'
        wait(lambda: context.candidate() == SHOWN, 'native manual phrase candidate')
        assert context.key(RIGHT, CONTROL)
        wait(lambda: context.commits() == [SUFFIX], 'native manual exact append dispatch')
        assert backend.reads == reads
        checks.append('Native manual unknown-widget append still invokes and dispatches once without observer prose')

        assert context.key(desktop.TAB)
        wait(lambda: context.candidate() == SHOWN, 'native manual candidate before transport loss')
        stop(broker)
        broker = None
        wait(lambda: context.candidate() is None, 'transport loss retires the native candidate')
        broker = start_broker()
        wait(lambda: control('status')['sessions'] == 1, 'native session reopened without Fcitx input', timeout=8)
        assert not context.key(ord('Y'), 5), 'A candidate from the closed connection cannot be accepted'
        assert control('status')['metrics']['context_updates'] == 0
        assert context.key(desktop.TAB), 'Tab after an unchanged reconnect must not require new input'
        wait(lambda: context.candidate() == SHOWN, 'native manual request after unchanged reconnect')
        assert control('status')['metrics']['context_updates'] == 1
        assert context.key(RIGHT, CONTROL)
        wait(lambda: context.commits() == [SUFFIX, SUFFIX], 'native exact append after unchanged reconnect')
        assert backend.reads == reads
        checks.append('Native manual Tab survives an unchanged-authority reconnect with republished context and no retired grant')

        def open_context(app, capabilities=0):
            nonlocal context
            if context is not None:
                context.close()
            context = None
            context = desktop.InputContext(app, capabilities)

        def updates():
            return control('status')['metrics']['context_updates']

        def ime_parity(app, browser, capabilities, label):
            # The observer first declines this app's field: no manual fallback.
            backend.browser, backend.uri = browser, ALLOWED_URI
            backend.apps.discard(app)
            backend.value, backend.caret = PREFIX, None
            backend.field += 1
            daemon.observer.invalidate('fixture_field_changed')
            open_context(app, capabilities)
            start_updates, reads, requested = updates(), backend.reads, len(backend.requested)
            unavailable = reason_count('ime_parity_observer_unavailable')
            context.text(PREFIX)
            wait(lambda: app in backend.requested[requested:], label + ' automatic inspection')
            quiet(.3)
            assert not context.key(desktop.TAB), label + ' Tab must pass through without an observed field'
            quiet(.3)
            assert context.candidate() is None and context.commits() == []
            assert not context.key(ord(' '), 5), label + ' Ctrl+Shift+Space reaches the app without an edit path'
            quiet(.05)
            assert auxiliary() == '', label + ' explains the declined field in badi debug, not a notice'
            assert reason_count('ime_parity_observer_unavailable') > unavailable
            assert updates() == start_updates and backend.reads == reads, \
                label + ' must never send unknown-identity manual context'
            checks.append(label + ': a declined field fails closed; Tab passes through and a linux_app grant '
                          'cannot authorize the unknown-identity manual path')

            # The observer service itself is gone: still no manual fallback.
            path = root / 'runtime/badi/accessibility.sock'
            away = path.with_name('accessibility.sock.away')
            path.rename(away)
            try:
                for descriptor in list(daemon.clients):
                    daemon.close_client(descriptor)
                assert not context.key(desktop.ESCAPE)
                assert auxiliary() == '', label + ' no notice to clear'
                unavailable = reason_count('ime_parity_observer_unavailable')
                context.text(PREFIX)
                quiet(.3)
                assert not context.key(desktop.TAB)
                assert not context.key(ord(' '), 5)
                quiet(.05)
                assert auxiliary() == '', label + ' absent observer leaves the chord to the app'
                assert reason_count('ime_parity_observer_unavailable') > unavailable
                assert updates() == start_updates and backend.reads == reads and context.commits() == []
            finally:
                away.rename(path)
            checks.append(label + ': an absent observer service fails closed with a content-free reason')

            # An observed field of the other target kind is not this app's field.
            backend.apps.add(app)
            backend.browser = not browser
            mismatches = reason_count('ime_parity_target_mismatch')
            change(PREFIX, uri=ALLOWED_URI, new_field=True)
            wait(lambda: reason_count('ime_parity_target_mismatch') > mismatches, label + ' target kind mismatch')
            quiet(.3)
            assert not context.key(desktop.TAB)
            assert updates() == start_updates and backend.reads == reads and context.candidate() is None
            backend.browser = browser
            checks.append(label + ': an observed target of the other kind retires authority without reading prose')

            backend.role = 'password text'
            denied = reason_count('ime_parity_field_denied')
            change(PREFIX, new_field=True)
            quiet(.3)
            assert not context.key(desktop.TAB)
            assert not context.key(ord(' '), 5)
            quiet(.05)
            assert auxiliary() == '', label + ' a denied field leaves the chord to the app'
            assert reason_count('ime_parity_field_denied') > denied
            assert updates() == start_updates and backend.reads == reads and context.candidate() is None
            backend.role = 'text'
            checks.append(label + ': an observer-denied password field reads no prose and sends no context')

            change(PREFIX, new_field=True)
            wait(lambda: context.candidate() == SHOWN, label + ' automatic observed suggestion')
            assert auxiliary() == '', label + ' fallback panel omits the keyboard-help banner'
            panel_event = next(values for name, values in reversed(context.events) if name == 'UpdateClientSideUI')
            assert panel_event[4][0][0] == '', label + ' fallback candidate has no numeric label'
            assert panel_event[5] == -1, label + ' fallback candidate has no selection highlight'
            assert updates() == start_updates + 1 and backend.reads > reads
            if app == 'brave-origin':
                quiet(5.5)
                assert context.candidate() == SHOWN, 'the negotiated native reading window survives five seconds'
            assert context.key(RIGHT, CONTROL)
            wait(lambda: context.commits() == [SUFFIX], label + ' Ctrl+Right dispatch after observed verification')
            quiet(.3)
            assert context.commits() == [SUFFIX] and context.candidate() is None, label + ' exactly one CommitString'
            checks.append(label + ': automatic observed suggestion in the single-line Fcitx panel; '
                          'Ctrl+Right dispatches the whole suggestion as one exact append')

            change(PREFIX)
            wait(lambda: context.candidate() == SHOWN, label + ' candidate before dismissal')
            assert context.key(desktop.ESCAPE)
            wait(lambda: context.candidate() is None, label + ' Escape dismissal')
            context.text(PREFIX)
            quiet(.3)
            assert context.candidate() is None, 'A dismissed automatic context stays dismissed'
            # Do not pump the observer between deleting and retyping. Its
            # snapshot may skip the empty field even though Fcitx publishes it.
            change('')
            assert context.candidate() is None, label + ' clearing the field does not revive the dismissed suggestion'
            change(PREFIX)
            wait(lambda: context.candidate() == SHOWN,
                 label + ' the same words suggest again after the dismissed text is deleted')
            assert context.key(desktop.ESCAPE)
            wait(lambda: context.candidate() is None, label + ' the repeated suggestion dismisses again')
            inspections = backend.inspections
            assert not context.key(desktop.TAB), label + ' Tab without a suggestion stays the application Tab'
            assert context.candidate() is None and context.commits() == [SUFFIX]
            assert context.key(ord(' '), 5), label + ' explicit chord is claimed on an observed field'
            wait(lambda: context.candidate() == SHOWN, label + ' explicit observed request')
            assert backend.inspections > inspections
            assert context.key(RIGHT, CONTROL)
            wait(lambda: context.commits() == [SUFFIX, SUFFIX], label + ' explicit acceptance')
            checks.append(label + ': after Escape, Tab stays the application Tab; the explicit chord re-inspects '
                          'the observed field and Ctrl+Right accepts it all once')

            mismatch = reason_count('observer_context_mismatch')
            before = updates()
            backend.value, backend.caret = PREFIX + ' mismatched', None
            daemon.observer.invalidate('fixture_field_changed')
            context.text(PREFIX)
            wait(lambda: reason_count('observer_context_mismatch') > mismatch, label + ' request disagreement')
            quiet(.2)
            assert updates() == before and context.candidate() is None
            mismatch = reason_count('observer_display_mismatch')
            arm_change('snapshot', 2, 'before')
            change(PREFIX)
            wait(lambda: reason_count('observer_display_mismatch') > mismatch, label + ' display disagreement')
            quiet(.3)
            assert silent_change['triggered'] and updates() == before + 1 and context.candidate() is None
            silent_change['op'] = None
            for mode, reason in (('before', 'observer_dispatch_mismatch'), ('deny', 'observer_dispatch_unavailable')):
                change(PREFIX)
                wait(lambda: context.candidate() == SHOWN, label + ' candidate before stale dispatch')
                commits, count = context.commits(), reason_count(reason)
                arm_change('snapshot', 1, mode)
                assert context.key(desktop.TAB)
                wait(lambda: reason_count(reason) > count, label + ' ' + reason)
                wait(lambda: context.candidate() is None, label + ' stale dispatch clears the candidate')
                quiet(.2)
                assert context.commits() == commits
                silent_change['op'] = None
            checks.append(label + ': observer disagreement at request, display and dispatch, and an unanswered '
                          'dispatch snapshot, fail closed with distinct content-free reasons')

            # ADR 0011: a typo is corrected in place once the observer has
            # confirmed the field: Fcitx asks the app to delete exactly the
            # typed typo, commits the fix, then reads the app's text back.
            typo, fixed = 'Please send it to the adress ', 'Please send it to the address '
            confirmed, unconfirmed = reason_count('correction_confirmed'), reason_count('correction_unconfirmed')
            change(typo)
            wait(lambda: context.candidate() == 'adress \u2192 address', label + ' a correction names its typo')
            first = len(context.events)
            assert context.key(desktop.TAB)
            wait(lambda: context.commits()[-1:] == ['address '], label + ' Tab commits the fix')
            edits = [(name, values) for name, values in context.events[first:]
                     if name in ('DeleteSurroundingText', 'CommitString')]
            assert edits == [('DeleteSurroundingText', (-7, 7)), ('CommitString', ('address ',))], \
                label + f' deletes exactly the typed typo, then commits the fix: {edits}'
            quiet(.2)
            assert reason_count('correction_confirmed') == confirmed, label + ' no verdict before the app reports'
            change(fixed)
            wait(lambda: reason_count('correction_confirmed') > confirmed,
                 label + " the app's own text confirms the correction")
            # A page that loses the edit: the read-back gives up and says so.
            change(typo)
            wait(lambda: context.candidate() == 'adress \u2192 address', label + ' a second correction')
            assert context.key(desktop.TAB)
            wait(lambda: context.commits()[-1:] == ['address '], label + ' the second fix is committed')
            wait(lambda: reason_count('correction_unconfirmed') > unconfirmed,
                 label + ' an unconfirmed correction is recorded', timeout=3)
            wait(lambda: 'could not confirm that correction' in auxiliary(), label + ' and named in a notice')
            assert reason_count('correction_confirmed') == confirmed + 1
            checks.append(label + ': a typo shows as typo → fix; Tab deletes exactly the typed typo, commits the fix '
                          'and confirms it from the app\'s own text, or names an unconfirmed correction')

        ime_parity('brave-origin', True, CHROMIUM_HINT, 'IME-parity browser (brave-origin, exact origin)')

        # Type-through (ADR 0004). Each edit ends the observer's binding, so the
        # broker answers the next field session with the remainder instead of
        # generating; Tab dispatches only the next word, and a second Tab in
        # the gap is held for the remainder instead of reaching the app.
        def provider_calls():
            return control('status')['metrics']['provider_calls']

        change(PREFIX)
        wait(lambda: context.candidate() == SHOWN, 'candidate before type-through')
        calls, typed = provider_calls(), reason_count('typed_through')
        assert not context.key(ord(' ')), 'a typed-through key still reaches the application'
        # The app's edit retires the observed field; the next key arrives
        # before it is bound again and the remainder returns, and still types through.
        daemon.observer.invalidate('fixture_field_changed')
        quiet(.05)
        assert not context.key(ord('f'))
        assert reason_count('typed_through') == typed + 2, 'a key typed before the remainder returns'
        commits = context.commits()
        change(PREFIX + ' f')
        wait(lambda: context.candidate() == SUFFIX[2:], 'type-through remainder')
        assert provider_calls() == calls, 'the remainder is carried, never generated'
        assert context.key(desktop.TAB), 'Tab is claimed over a live candidate'
        wait(lambda: context.commits() == [*commits, 'or'], 'Tab dispatches one word')
        held = reason_count('tab_held')
        assert context.key(desktop.TAB), 'a quick second Tab is held, not sent to the application'
        wait(lambda: reason_count('tab_held') == held + 1, 'the second Tab is recorded held')
        assert context.commits() == [*commits, 'or'], 'the held Tab commits nothing before the rest returns'
        change(PREFIX + ' for')
        wait(lambda: context.commits() == [*commits, 'or', ' your'],
             'the held Tab accepts the next word when the carried rest is displayed')
        assert reason_count('tab_held_accepted') >= 1
        assert provider_calls() == calls
        change(PREFIX + ' for your')
        wait(lambda: context.candidate() == 'time', 'remainder after the held Tab')
        assert provider_calls() == calls
        assert context.key(RIGHT, CONTROL), 'Ctrl+Right is claimed over a live candidate'
        wait(lambda: context.commits() == [*commits, 'or', ' your', ' time'],
             'Ctrl+Right dispatches the whole remainder')
        change(PREFIX)
        wait(lambda: context.candidate() == SHOWN, 'candidate before Escape ends the carry')
        calls = provider_calls()
        assert context.key(desktop.ESCAPE)
        wait(lambda: context.candidate() is None, 'Escape ends type-through')
        assert not context.key(RIGHT, CONTROL), 'without a candidate Ctrl+Right stays the application key'
        assert not context.key(desktop.TAB), 'without a candidate or a pending rest Tab stays the application Tab'
        change(PREFIX + ' fo')
        wait(lambda: provider_calls() > calls, 'after Escape a typed prefix is generated again')
        checks.append('IME-parity type-through: a typed prefix of the suggestion shows its remainder in the next '
                      'field session without a model call; Tab dispatches one word, a quick second Tab is held '
                      'and takes the next word when the rest returns, and Ctrl+Right takes the rest; '
                      'Escape ends the carry')

        # At most one hold per word acceptance: when the rest never returns
        # (the field is left unchanged), the held Tab times out and the next
        # Tab reaches the application instead of being held again.
        change(PREFIX)
        wait(lambda: context.candidate() == SHOWN, 'candidate before the held-Tab timeout')
        commits = context.commits()
        assert context.key(desktop.TAB)
        wait(lambda: context.commits() == [*commits, ' for'], 'Tab dispatches one word before the timeout step')
        held = reason_count('tab_held')
        dropped = reason_count('tab_held_dropped')
        assert context.key(desktop.TAB), 'the first Tab in the gap is held'
        wait(lambda: reason_count('tab_held') == held + 1, 'the timeout step hold is recorded')
        wait(lambda: reason_count('tab_held_dropped') == dropped + 1, 'the held Tab times out')
        assert context.commits() == [*commits, ' for'], 'the timed-out Tab committed and inserted nothing'
        assert not context.key(desktop.TAB), 'after the timeout the next Tab reaches the application'
        assert reason_count('tab_held') == held + 1, 'no second hold for the same word acceptance'
        change(PREFIX + ' for')
        wait(lambda: context.candidate() == 'your time', 'the rest still returns when the field catches up')
        checks.append('IME-parity held Tab: at most one hold per word acceptance; after the 700 ms timeout '
                      'the next Tab is the application Tab again')

        # Foreign composition and a rendered overlay on the same browser field.
        change(PREFIX)
        wait(lambda: context.candidate() == SHOWN, 'browser candidate before foreign preedit')
        assert context.key(ord('U'), 5) and context.key(ord('4'))
        wait(lambda: preedit() == 'U+4', 'IME-parity field real Unicode preedit')
        wait(lambda: context.candidate() != SHOWN, 'foreign preedit owns the panel')
        reads = backend.reads
        context.text(PREFIX)
        quiet(.3)
        assert backend.reads == reads and context.candidate() != SHOWN, 'No prose is read during foreign composition'
        assert context.key(desktop.ESCAPE)
        wait(lambda: context.candidate() == SHOWN, 'IME-parity recovery after foreign preedit clears')
        assert preedit() == ''
        checks.append('IME-parity Fcitx panel fallback yields to foreign preedit and recovers afterwards')
        render_result['value'] = True
        renders, commits = len(render_calls), context.commits()
        change(PREFIX)
        wait(lambda: len(render_calls) > renders, 'rendered IME-parity preview')
        quiet(.3)
        assert context.candidate() is None and auxiliary() == '', 'A rendered preview replaces the Fcitx panel'
        assert context.key(RIGHT, CONTROL)
        wait(lambda: context.commits() == [*commits, SUFFIX], 'Ctrl+Right accepts the rendered preview once')
        render_result['value'] = False
        checks.append('A rendered observer preview is accepted once without a Fcitx candidate panel')

        # Zen's wayland_v2 context carries no purpose hints (0x72, live
        # 2026-09-26), and its urlbar shares that same input context without
        # the Url purpose. Only the observer's denial keeps the urlbar out.
        ime_parity('zen', True, 0, 'IME-parity browser (zen, Gecko)')
        backend.role = 'combo box'
        denied, start_updates, reads = reason_count('ime_parity_field_denied'), updates(), backend.reads
        commits = context.commits()
        change(PREFIX, new_field=True)
        quiet(.3)
        assert not context.key(desktop.TAB), 'Tab passes through in a denied Zen urlbar'
        assert not context.key(ord(' '), 5), 'Ctrl+Shift+Space passes through in a denied Zen urlbar'
        quiet(.05)
        assert auxiliary() == '', 'a denied urlbar leaves the chord to the app'
        assert reason_count('ime_parity_field_denied') > denied
        assert updates() == start_updates and backend.reads == reads
        assert context.candidate() is None and context.commits() == commits
        backend.role = 'text'
        checks.append('IME-parity browser (zen, Gecko): an observer-denied urlbar on the same unhinted input '
                      'context reads no prose, sends no context and keeps Tab')

        ime_parity('code', False, 0, 'IME-parity desktop (code)')

        # A ProseMirror composer (Codex desktop): Chromium ends each <p> with
        # "\n\n" (Chromium 152, WAYLAND_DEBUG, 2026-09-27), so the caret at the
        # end of the last paragraph reports it after the caret, and the
        # observer's flattened paragraphs report the same suffix.
        def paragraph_end(observed_after, fcitx_after, new_field=False):
            backend.value, backend.caret = PREFIX + observed_after, len(PREFIX)
            if new_field:
                backend.field += 1
            daemon.observer.invalidate('fixture_field_changed')
            context.text(PREFIX + fcitx_after, len(PREFIX))

        backend.browser, backend.uri, backend.role = False, ALLOWED_URI, 'text'
        backend.apps.add('chatgpt')
        open_context('chatgpt')
        start_updates, normalized = updates(), reason_count('observed_field_request_paragraph_end')
        paragraph_end('\n\n', '\n\n', new_field=True)
        wait(lambda: context.candidate() == SHOWN, 'chatgpt composer suggestion before the paragraph end')
        assert updates() == start_updates + 1
        assert reason_count('observed_field_request_paragraph_end') == normalized + 1
        assert context.key(RIGHT, CONTROL)
        wait(lambda: context.commits() == [SUFFIX], 'chatgpt composer Ctrl+Right dispatch before the paragraph end')
        quiet(.3)
        assert context.commits() == [SUFFIX] and context.candidate() is None
        checks.append('IME-parity desktop (chatgpt): a paragraph end "\\n\\n" agreed by the observer is end of '
                      'field; the suggestion displays and Ctrl+Right dispatches one exact append')
        for observed_after, fcitx_after in (('', '\n\n'), ('\n', '\n\n')):
            unchanged, count = updates(), reason_count('observer_context_mismatch')
            paragraph_end(observed_after, fcitx_after)
            wait(lambda: reason_count('observer_context_mismatch') > count,
                 'chatgpt observer_context_mismatch for ' + repr(fcitx_after))
            quiet(.2)
            assert updates() == unchanged and context.candidate() is None, repr((observed_after, fcitx_after))
            assert not context.key(desktop.TAB), repr((observed_after, fcitx_after))
            quiet(.3)
            assert updates() == unchanged and context.candidate() is None and context.commits() == [SUFFIX]
        checks.append('IME-parity desktop (chatgpt): observer disagreement about the paragraph end fails closed '
                      'and keeps the application Tab')
        # Agreed text after the caret is a request; the phrase fixture abstains
        # there, so nothing shows and Tab stays the application's.
        for after in ('\n\n\n', '\n\nmore'):
            unchanged = updates()
            paragraph_end(after, after)
            wait(lambda: updates() > unchanged, 'chatgpt request with text after the caret ' + repr(after))
            quiet(.3)
            assert context.candidate() is None and not context.key(desktop.TAB), repr(after)
            quiet(.3)
            assert context.candidate() is None and context.commits() == [SUFFIX]
        checks.append('IME-parity desktop (chatgpt): agreed text after the caret requests; the fixture abstains '
                      'and Tab stays the application\'s')
        # A <div> line ends with one break; an EditContext editor sends none while the DOM shows one.
        for observed_after, fcitx_after in (('\n', '\n'), ('\n', '')):
            paragraph_end(observed_after, fcitx_after, new_field=True)
            wait(lambda: context.candidate() == SHOWN, 'chatgpt suggestion for ' + repr((observed_after, fcitx_after)))
            assert context.key(desktop.ESCAPE)
            quiet(.3)
            assert context.candidate() is None and context.commits() == [SUFFIX]
        checks.append('IME-parity desktop (chatgpt): a <div> line end "\\n", and an empty Fcitx end against an '
                      'observed block end, are end of field')

        open_context('Telegram')
        requested = len(backend.requested)
        context.text(PREFIX)
        wait(lambda: 'telegram' in backend.requested[requested:], 'observer receives the canonical identity')
        assert 'Telegram' not in backend.requested
        assert context.key(desktop.TAB), 'A canonicalized native app keeps the manual path'
        wait(lambda: context.candidate() == SHOWN, 'Telegram native manual candidate')
        assert context.key(RIGHT, CONTROL)
        wait(lambda: context.commits() == [SUFFIX], 'Telegram native manual dispatch')
        checks.append("Program 'Telegram' becomes 'telegram' for observer, policy and session; native manual "
                      "Tab requests and Ctrl+Right dispatches once")
        open_context('A window title')
        context.text(PREFIX)
        assert not context.key(desktop.TAB) and not context.key(ord(' '), 5)
        quiet(.2)
        assert context.candidate() is None and auxiliary() == ''
        assert 'A window title' not in backend.requested
        checks.append('A non-identifier program never becomes an app identity')

        # App blocklist mode: an app without a rule is allowed by default, which
        # opens only a field the observer corroborates; the manual path still
        # needs an exact rule, so Tab stays the application's.
        listed = control('settings', 'show', '--json')
        default_mode = {**listed, 'revision': listed['revision'] + 1, 'all_linux_apps': True,
                        'subjects': [rule for rule in listed['subjects']
                                     if rule['identity'].get('app_id') not in ('cursor', FIXTURE_APP)]}
        control('settings', 'replace', '--if-revision', str(listed['revision']), '--json', json.dumps(default_mode))
        required = reason_count('app_rule_required')
        open_context(FIXTURE_APP)
        context.text(PREFIX)
        wait(lambda: reason_count('app_rule_required') > required, 'default-allowed manual app needs a rule')
        assert not context.key(desktop.TAB), 'the manual path keeps Tab without an exact app rule'
        quiet(.3)
        assert context.candidate() is None and context.commits() == []
        backend.browser, backend.uri, backend.role = False, ALLOWED_URI, 'text'
        backend.apps.add('cursor')
        backend.value, backend.caret = PREFIX, len(PREFIX)
        backend.field += 1
        daemon.observer.invalidate('fixture_field_changed')
        open_context('cursor')
        context.text(PREFIX, len(PREFIX))
        # A settings change starts a new authority epoch first: the bound of
        # the other waits that follow one (#81 timed out here at 4 s).
        wait(lambda: context.candidate() == SHOWN, 'default-allowed observed app suggestion', timeout=8)
        assert context.key(desktop.ESCAPE)
        quiet(.2)
        restored = control('settings', 'show', '--json')
        control('settings', 'replace', '--if-revision', str(restored['revision']), '--json',
                json.dumps({**listed, 'revision': restored['revision'] + 1}))
        checks.append('App blocklist mode opens an unlisted IME-parity app through its observed field and leaves an '
                      'unlisted manual-path app with its own Tab')

        # Timed quiet survives fresh observer epochs of one field visit,
        # without retaining its retired session or grant.
        listed = control('settings', 'show', '--json')
        control('settings', 'replace', '--if-revision', str(listed['revision']), '--json',
                json.dumps({**listed, 'revision': listed['revision'] + 1, 'escape_quiet_seconds': 60}))
        open_context('chatgpt')
        backend.apps.add('chatgpt')
        change(PREFIX, new_field=True)
        wait(lambda: context.candidate() == SHOWN, 'candidate before timed quiet', timeout=8)
        assert context.key(desktop.ESCAPE)
        quiet(.1)
        calls = control('status')['metrics']['provider_calls']
        for value in (PREFIX + ' f', PREFIX + ' fo'):
            change(value)
            quiet(.4)
            assert context.candidate() is None
            assert control('status')['metrics']['provider_calls'] == calls
        change(PREFIX)
        quiet(.3)
        assert control('status')['metrics']['provider_calls'] == calls
        assert context.key(ord(' '), 5)
        wait(lambda: context.candidate() is not None, 'explicit request ends quiet across the rebind')
        assert context.key(desktop.ESCAPE)
        change(PREFIX, new_field=True)
        wait(lambda: context.candidate() == SHOWN, 'a different field is not quieted')
        restored = control('settings', 'show', '--json')
        control('settings', 'replace', '--if-revision', str(restored['revision']), '--json',
                json.dumps({**listed, 'revision': restored['revision'] + 1}))
        checks.append('Timed Escape quiet survives same-field observer rebinds without a model call; '
                      'an explicit request resumes and another field is not quieted')

        # A terminal's typed line (ADR 0009): automatic requests follow the
        # keys typed into the context, and an automatic miss stays quiet.
        # Only an explicit request may show the no-continuation notice.
        allowed = reason_count('app_allowed')
        open_context('com.mitchellh.ghostty', TERMINAL_HINT)
        wait(lambda: reason_count('app_allowed') > allowed, 'the exact terminal rule answers policy')

        def typed(text):
            for char in text:
                assert not context.key(ord(char)), 'typing on a typed line reaches the terminal'

        typed('hello brave world')
        quiet(.6)
        assert context.candidate() is None and auxiliary() == '', \
            'an automatic terminal miss shows no notice'
        assert context.key(ord(' '), 5), 'the explicit chord is claimed on a typed line'
        wait(lambda: 'no continuation' in auxiliary(), 'an explicit terminal miss may show its notice')
        assert not context.key(ord('u'), CONTROL), 'Ctrl+U reaches the terminal and clears the tracked line'
        typed(PREFIX)
        wait(lambda: context.candidate() == SHOWN, 'a typed line suggests automatically after the pause')
        assert context.key(desktop.TAB)
        wait(lambda: context.commits() == [' for'], 'Tab types one word into the terminal')
        wait(lambda: context.candidate() == 'your time', 'the rest follows the accepted word on the line')
        assert context.key(RIGHT, CONTROL)
        wait(lambda: context.commits() == [' for', ' your time'], 'Ctrl+Right types the rest')
        # A terminal ignores the input method's delete request: a correction
        # leads its one commit with a DEL per scalar of the tracked typo.
        assert not context.key(ord('u'), CONTROL)
        typed('send it to the adress ')
        wait(lambda: context.candidate() == 'adress \u2192 address', 'a typed-line correction shows the typo')
        assert context.key(desktop.TAB)
        wait(lambda: context.commits()[-1:] == ['\x7f' * 7 + 'address '],
             'Tab erases exactly the typo with DELs, then types the correction')
        checks.append('Terminal typed line: automatic misses stay quiet, an explicit miss may show its '
                      'notice, Tab types one word and Ctrl+Right the rest, and a correction erases '
                      'exactly the typo before typing its fix')

        # A correction is negotiated on IME-parity observed sessions too
        # (ADR 0011): one bound to the typed text shows as typo → fix.
        broker_metrics = control('status')['metrics']
        stop(broker)
        broker = None
        endpoint.unlink(missing_ok=True)
        fake = FakeBroker(endpoint)

        def fake_wait(predicate, label):
            wait(lambda: fake.poll() or predicate(), label, timeout=8)

        def fake_quiet(seconds):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                fake.poll()
                desktop.pump()
                time.sleep(.01)

        def fake_request():
            requests = fake.count('suggest.request')
            fake_wait(lambda: fake.count('authority.ack') >= fake.connections > 0 and fake.peer is not None,
                      'fake broker handshake')
            change(PREFIX, new_field=True)
            fake_wait(lambda: fake.count('suggest.request') > requests, 'observed request reaches fake broker')
            return fake.last('suggest.request')

        for app, browser, capabilities in (('brave-origin', True, CHROMIUM_HINT), ('code', False, 0)):
            backend.browser, backend.uri = browser, ALLOWED_URI
            open_context(app, capabilities)
            closes = fake.client_closes
            fake.respond(fake_request(), 'suggestion.show', {
                'suggestion_id': 'injected-show', 'text': SUFFIX, 'accept_word': ' for', 'ttl_ms': 3000,
                'provider': 'phrase_v1', 'replace_before': 'you'})
            fake_wait(lambda: context.candidate() == 'you \u2192 for your time', app + ' a correction from the broker')
            assert fake.client_closes == closes and context.commits() == []
            fake.respond(fake_request(), 'suggestion.show', {
                'suggestion_id': 'append-show', 'text': SUFFIX, 'accept_word': ' for', 'ttl_ms': 3000,
                'provider': 'phrase_v1'})
            fake_wait(lambda: context.candidate() == SHOWN, app + ' append suggestion from the fake broker')
            controls = fake.count('control.request')
            assert context.key(RIGHT, CONTROL)
            fake_wait(lambda: fake.count('control.request') > controls, app + ' acceptance request')
            results = fake.count('commit.result')
            fake.respond(fake.last('control.request'), 'commit.prepare', {
                'suggestion_id': 'append-show', 'text': SUFFIX, 'acceptance': 'all', 'replace_before': 'you'})
            fake_wait(lambda: fake.count('commit.result') > results, app + ' reports the mismatched replacement stale')
            fake_quiet(.3)
            assert fake.client_closes == closes and context.commits() == []
            assert fake.last('commit.result')['payload']['status'] == 'stale'
            checks.append(app + ': an injected replacement is not shown or applied, and the transport stays open')

        # Under fcitx5-qt's echo the client has dropped its copy of the text
        # and applies Fcitx's delete request to that copy, so a native
        # replacement would only append: it is neither shown nor dispatched.
        backend.browser, backend.uri = False, ALLOWED_URI
        open_context(FIXTURE_APP)
        fake_wait(lambda: fake.count('authority.ack') >= fake.connections > 0 and fake.peer is not None,
                  'fake broker handshake')
        replacement = {'text': SUFFIX, 'accept_word': ' for', 'ttl_ms': 3000, 'provider': 'phrase_v1',
                       'replace_before': 'you'}

        def native_request(send_text):
            requests = fake.count('suggest.request')
            backend.value, backend.caret = PREFIX, None
            backend.field += 1
            daemon.observer.invalidate('fixture_field_changed')
            send_text(PREFIX)
            fake_wait(lambda: fake.count('suggest.request') > requests, 'native observed request')
            return fake.last('suggest.request')

        blocked = reason_count('replacement_unavailable')
        fake.respond(native_request(qt_text), 'suggestion.show', {'suggestion_id': 'echo-replace', **replacement})
        fake_wait(lambda: reason_count('replacement_unavailable') > blocked, 'replacement withheld under the echo')
        assert context.candidate() is None and context.commits() == []

        def granted_text(value):
            context.ic('SetCapability', '(t)', (desktop.CAPABILITIES,))
            context.text(value)

        fake.respond(native_request(granted_text), 'suggestion.show', {'suggestion_id': 'shown-replace', **replacement})
        fake_wait(lambda: context.candidate() is not None, 'replacement shown while SurroundingText is set')
        context.ic('SetCapability', '(t)', (desktop.CAPABILITIES & ~surrounding,))
        controls, blocked = fake.count('control.request'), reason_count('replacement_unavailable')
        assert context.key(RIGHT, CONTROL)
        fake_wait(lambda: fake.count('control.request') > controls, 'replacement acceptance request')
        results = fake.count('commit.result')
        fake.respond(fake.last('control.request'), 'commit.prepare', {
            'suggestion_id': 'shown-replace', 'text': SUFFIX, 'acceptance': 'all', 'replace_before': 'you'})
        fake_wait(lambda: fake.count('commit.result') > results, 'replacement refused once the echo starts')
        fake_quiet(.3)
        assert fake.last('commit.result')['payload']['status'] == 'stale' and context.commits() == []
        assert reason_count('replacement_unavailable') > blocked
        checks.append('Under the fcitx5-qt echo a native replacement is neither shown nor dispatched, since the '
                      'client would drop the delete and only append')

        backend.browser, backend.uri = False, ALLOWED_URI
        open_context('chatgpt')
        requests = fake.count('suggest.request')
        fake_wait(lambda: fake.count('authority.ack') >= fake.connections > 0 and fake.peer is not None,
                  'fake broker handshake')
        paragraph_end('\n\n', '\n\n', new_field=True)
        fake_wait(lambda: fake.count('suggest.request') > requests, 'chatgpt paragraph-end request reaches fake broker')
        payload = fake.last('context.changed')['payload']
        assert (payload['before'], payload['after'], payload['selection']['head']) == (PREFIX, '', len(PREFIX)), payload
        checks.append('chatgpt: the broker receives after = "" at the unchanged caret for the normalized paragraph end')

        for changed_field in (False, True):
            request = fake_request()
            fake.respond(request, 'suggestion.show', {
                'suggestion_id': 'first-word', 'text': ' for', 'accept_word': ' for',
                'ttl_ms': 3000, 'provider': 'phrase_v1'})
            fake_wait(lambda: context.candidate() == 'for', 'first immutable preview')
            delayed_preview['enabled'] = True
            started = delayed_preview['started']
            fake.respond(request, 'suggestion.show', {
                'suggestion_id': 'extended-preview', 'text': SUFFIX, 'accept_word': ' for',
                'ttl_ms': 3000, 'provider': 'phrase_v1'})
            fake_wait(lambda: delayed_preview['started'] > started, 'extension owns the observer RPC')
            controls, results, commits = fake.count('control.request'), fake.count('commit.result'), context.commits()
            assert context.key(desktop.TAB)
            fake_wait(lambda: fake.count('control.request') > controls, 'accept while extension is pending')
            acceptance = fake.last('control.request')
            assert acceptance['payload']['suggestion_id'] == 'first-word'
            fake.respond(acceptance, 'commit.prepare', {
                'suggestion_id': 'first-word', 'text': ' for', 'acceptance': 'word'})
            if changed_field:
                backend.field += 1  # Silent identity change must be caught by the fresh snapshot.
            fake_wait(lambda: fake.count('commit.result') > results, 'pending preview finishes before commit snapshot')
            delayed_preview['enabled'] = False
            expected = 'stale' if changed_field else 'dispatched-unverified'
            assert fake.last('commit.result')['payload']['status'] == expected
            assert context.commits() == (commits if changed_field else [*commits, ' for'])
            fake_quiet(.1)
            assert context.candidate() is None, 'the late extension never replaces the accepted version'
        checks.append('Tab during a delayed growing preview waits for its own snapshot and dispatches the exact '
                      'first word once; a silent field change instead reports stale without dispatch')

        result = {'boundary': 'unavailable apps + IME-parity observed targets + synthetic desktop D-Bus fields '
                              '+ actual Fcitx addon/observer + phrase broker + injected-frame peer',
                  'physical_editor_or_overlay_proof': False, 'passed': True, 'checks': checks,
                  'broker_metrics': broker_metrics, 'observer_reads': backend.reads}
        (report / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps(result, indent=2), flush=True)
    except BaseException:
        debug_path = root / 'runtime/badi/debug-native.json'
        if debug_path.exists():
            shutil.copy2(debug_path, report / 'native-debug.json')
        (report / 'result.json').write_text(json.dumps({'passed': False, 'completed': checks,
                                                       'observer_inspections': backend.inspections,
                                                       'observer_reads': backend.reads,
                                                       'events': context.events if context else []}, indent=2) + '\n')
        raise
    finally:
        try:
            if context is not None and fcitx is not None and fcitx.poll() is None:
                context.close()
        finally:
            try:
                stop(fcitx)
            finally:
                try:
                    stop(broker)
                finally:
                    if fake is not None:
                        fake.close()
                    for descriptor in list(daemon.clients):
                        daemon.close_client(descriptor)
                    daemon.server.close()
                    if daemon.lock_fd is not None:
                        os.close(daemon.lock_fd)
                    broker_log.close()
                    fcitx_log.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--session-root', type=Path)
    parser.add_argument('--report', type=Path)
    parser.add_argument('--addon', type=Path, default=ROOT / 'adapters/fcitx5/build/libbadi-fcitx5.so')
    args = parser.parse_args()
    if args.session_root:
        session(args.session_root, args.report)
        return
    for command in ['fcitx5', 'dbus-run-session']:
        if not shutil.which(command):
            raise RuntimeError('Missing fixture prerequisite: ' + command)
    # The session runs under this interpreter and needs PyGObject; a virtual
    # environment's python3 first on PATH has none.
    if importlib.util.find_spec('gi') is None:
        raise RuntimeError(f'{sys.executable} cannot import gi (PyGObject); run this with the system python3')
    if not args.addon.is_file():
        raise RuntimeError('Build required fixture addon: ' + str(args.addon))
    for path in ['adapters/fcitx5/build/badi.conf',
                 'target/debug/badi-broker', 'target/debug/badictl']:
        if not (ROOT / path).is_file():
            raise RuntimeError('Build required fixture artifact: ' + path)
    reports = ROOT / 'output/observed-desktop'
    reports.mkdir(parents=True, exist_ok=True)
    report = Path(tempfile.mkdtemp(prefix='observed-dbus-', dir=reports))
    with tempfile.TemporaryDirectory(prefix='badi-observed-dbus-') as temporary:
        root = Path(temporary)
        for name in ['home', 'runtime', 'config', 'data', 'cache', 'addon/lib/fcitx5', 'addon/share/fcitx5/addon']:
            (root / name).mkdir(parents=True, mode=0o700)
        shutil.copy2(args.addon, root / 'addon/lib/fcitx5/libbadi-fcitx5.so')
        shutil.copy2(ROOT / 'adapters/fcitx5/build/badi.conf', root / 'addon/share/fcitx5/addon')
        permissions = {'context_read': 'allow', 'display': 'allow', 'suggest': 'allow',
                       'learn': 'block', 'retention': {'mode': 'none'}}
        settings = {'schema': 'badi.settings.v2', 'revision': 1, 'paused': False,
                    'subjects': [{'identity': {'kind': 'browser_origin', 'adapter': 'chromium', 'scheme': 'https',
                                               'host': 'allowed.example.test', 'port': 443}, 'permissions': permissions},
                                 *({'identity': {'kind': 'linux_app', 'adapter': 'fcitx', 'app_id': app},
                                    'permissions': permissions}
                                   # IME-parity apps also hold app rules: a manual
                                   # request would be authorized if ever sent.
                                   for app in sorted((*UNAVAILABLE_APPS, *IME_PARITY_APPS, *UNOBSERVED_FAMILY_PROGRAMS.values(),
                                                      FIXTURE_APP, 'omawrite', 'telegram', 'com.mitchellh.ghostty')))]}
        private_file(root / 'config/badi/settings.json', json.dumps(settings))
        private_file(root / 'runtime/badi/debug-control.json', json.dumps({
            'id': '550e8400-e29b-41d4-a716-446655440000', 'expires_at': int(time.time()) + 180}))
        private_file(root / 'config/fcitx5/profile', '[Groups/0]\nName=Default\nDefault Layout=us\nDefaultIM=keyboard-us\n\n'
                     '[Groups/0/Items/0]\nName=keyboard-us\nLayout=\n\n[GroupOrder]\n0=Default\n')
        # Keep the system's delayed focus-language indicator from replacing
        # the panel under test. This config belongs only to the private process.
        private_file(root / 'config/fcitx5/config', '[Behavior]\nShowInputMethodInformation=False\n'
                     'showInputMethodInformationWhenFocusIn=False\n')
        private_file(root / 'bus.conf', '<busconfig><type>session</type><keep_umask/><listen>unix:tmpdir=/tmp</listen>'
                     '<auth>EXTERNAL</auth><policy context="default"><allow send_destination="*" eavesdrop="true"/>'
                     '<allow eavesdrop="true"/><allow own="*"/></policy></busconfig>')
        environment = {**os.environ, 'HOME': str(root / 'home'), 'XDG_RUNTIME_DIR': str(root / 'runtime'),
                       'XDG_CONFIG_HOME': str(root / 'config'), 'XDG_DATA_HOME': str(root / 'data'),
                       'XDG_CACHE_HOME': str(root / 'cache'), 'FCITX_CONFIG_HOME': str(root / 'config/fcitx5'),
                       'FCITX_DATA_HOME': str(root / 'data/fcitx5'), 'FCITX_CONFIG_DIRS': '/etc/xdg/fcitx5',
                       'FCITX_ADDON_DIRS': f'{root}/addon/lib/fcitx5:/usr/lib/fcitx5',
                       'FCITX_DATA_DIRS': f'{root}/addon/share/fcitx5:/usr/share/fcitx5'}
        for name in ['DISPLAY', 'WAYLAND_DISPLAY', 'AT_SPI_BUS_ADDRESS', 'DBUS_SESSION_BUS_ADDRESS',
                     'HYPRLAND_INSTANCE_SIGNATURE', 'SKIP_FCITX_PATH', 'SKIP_FCITX_USER_PATH', 'SKIP_FCITX_SYSTEM_PATH']:
            environment.pop(name, None)
        command = ['dbus-run-session', f'--config-file={root / "bus.conf"}', '--', sys.executable,
                   str(Path(__file__).resolve()), '--session-root', str(root), '--report', str(report)]
        # The session's own output stays with its report, so a failure before
        # its logs open (an import, the bus) still explains itself.
        session_log = report / 'session.log'
        ended = 'timeout'
        with session_log.open('w') as output:
            child = subprocess.Popen(command, env=environment, start_new_session=True,
                                     stdout=output, stderr=subprocess.STDOUT)
            try:
                code = child.wait(timeout=150)
                ended = 'passed' if code == 0 else f'exit {code}'
            except subprocess.TimeoutExpired:
                pass
            finally:
                if child.poll() is None:
                    os.killpg(child.pid, signal.SIGTERM)
                    try:
                        child.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(child.pid, signal.SIGKILL)
                        child.wait(timeout=5)
    log = session_log.read_text(errors='replace')
    if ended != 'passed':
        if not (report / 'result.json').exists():
            (report / 'result.json').write_text(json.dumps(
                {'passed': False, 'completed': [], 'ended': ended, 'log': 'session.log'}, indent=2) + '\n')
        sys.stderr.write(''.join(log.splitlines(keepends=True)[-40:]))
        raise RuntimeError(f'Observed addon integration failed ({ended}); diagnostics: {report}')
    sys.stdout.write(log)
    print('Disposable observed addon integration passed; report: ' + str(report))


if __name__ == '__main__':
    main()
