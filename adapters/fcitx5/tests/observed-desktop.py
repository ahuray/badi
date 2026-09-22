#!/usr/bin/env python3
"""Exercise native quarantine and a synthetic desktop observer on private D-Bus.

No display, input device, normal service, editor or desktop accessibility tree is
used. CommitString is observed as a dispatch, never claimed as an applied edit.
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
FIXTURE_APP = 'badi-native-fixture'
QUARANTINED_APPS = ('chromium', 'chromium-browser', 'chrome', 'google-chrome',
                    'brave', 'brave-origin', 'brave-browser', 'firefox', 'chatgpt')
UNAVAILABLE_NOTICE = 'Badi cannot safely insert suggestions in this app yet'


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


def session(root, report):
    import gi
    gi.require_version('Gio', '2.0')
    from gi.repository import Gio, GLib
    spec = importlib.util.spec_from_file_location('desktop_tab', Path(__file__).with_name('desktop-tab.py'))
    desktop = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(desktop)
    sys.path.insert(0, str(ROOT / 'adapters/accessibility'))
    from contract import Denied, Observer
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

        def budget(self):
            if time.monotonic() >= self.deadline:
                raise Denied('operation_timeout')

        def metadata(self, app):
            self.budget()
            self.inspections += 1
            if app != FIXTURE_APP:
                raise Denied('unsupported_app')
            return {'bus': ':1.synthetic', 'path': f'/synthetic/field/{self.field}',
                    'process_id': os.getpid(), 'app_id': app, 'uri': self.uri,
                    'browser': self.browser, 'role': 'text', 'tag': 'textarea', 'input_type': 'textarea',
                    'focused': True, 'editable': True, 'showing': True, 'visible': True, 'enabled': True,
                    'selection_count': 0, 'caret': len(self.value) if self.caret is None else self.caret,
                    'total_chars': len(self.value)}

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
    daemon.observer.render = lambda focus, _text, _ttl: render_calls.append(focus) or False
    original_request = daemon.observer.request
    observer_calls = {'snapshot': 0, 'preview': 0}
    silent_change = {'op': None, 'call': 0, 'mode': 'repeat', 'triggered': False}

    def observer_request(request):
        op = request.get('op')
        if op in observer_calls:
            observer_calls[op] += 1
        trigger = op == silent_change['op'] and observer_calls[op] == silent_change['call']
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
                    'context', 'suggestion', 'commit.dispatched_unverified', 'control', 'policy']
                checks.append('Actual loaded native addon hello omits unsafe text replacement capability')
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
        for app in QUARANTINED_APPS:
            reads = backend.reads
            inspections = backend.inspections
            context = desktop.InputContext(app)
            assert not context.key(desktop.TAB), app + ' must retain Tab without surrounding text'
            assert context.key(ord(' '), 5), app + ' explicit notice must not require surrounding text'
            wait(lambda: auxiliary() == UNAVAILABLE_NOTICE, app + ' current auxiliary notice')
            for _ in range(3):
                context.text(PREFIX)
                quiet(.06)
                assert auxiliary() == UNAVAILABLE_NOTICE, app + ' republishing must preserve the current notice'
            assert not context.key(desktop.TAB), app + ' must retain ordinary Tab'
            wait(lambda: auxiliary() == '', app + ' original Tab clears the notice')
            assert not context.key(0xff51) and not context.key(0xff57), app + ' must retain navigation'
            assert context.key(ord(' '), 5), app + ' supports an explicit unavailable notice'
            wait(lambda: auxiliary() == UNAVAILABLE_NOTICE, app + ' refreshed auxiliary notice')
            assert 'Tab to accept' not in json.dumps(context.events)
            assert not context.key(ord('Y'), 5), app + ' cannot accept unsupported native editing'
            wait(lambda: auxiliary() == '', app + ' ordinary input clears the notice')
            assert backend.reads == reads and backend.inspections == inspections
            assert context.candidate() is None and context.commits() == []
            assert control('status')['metrics']['context_updates'] == 0
            context.close()
            context = None
        checks.append('Allowed browser/Codex aliases acquire no prose, publish no context and preserve Tab/navigation')
        checks.append('Current unavailable notice survives repeated surrounding publication and clears on original keys')

        context = desktop.InputContext('chromium')
        assert context.key(ord(' '), 5)
        wait(lambda: auxiliary() == UNAVAILABLE_NOTICE, 'notice before focus loss')
        context.ic('FocusOut')
        wait(lambda: auxiliary() == '', 'focus loss clears the unavailable notice')
        context.ic('FocusIn')
        quiet(.1)
        assert auxiliary() == '', 'Focus recovery must not restore an old notice'
        assert context.key(ord(' '), 5)
        wait(lambda: auxiliary() == UNAVAILABLE_NOTICE, 'notice before foreign composition')
        assert context.key(ord('U'), 5)
        assert context.key(ord('4'))
        wait(lambda: preedit() == 'U+4', 'quarantined field real Unicode preedit')
        assert auxiliary() != UNAVAILABLE_NOTICE, 'Foreign composition must own the panel'
        assert context.key(desktop.ESCAPE)
        wait(lambda: preedit() == '' and auxiliary() == '', 'foreign composition clears without restoring notice')
        assert context.key(ord(' '), 5)
        wait(lambda: auxiliary() == UNAVAILABLE_NOTICE, 'notice before five-second expiry')
        notice_started = time.monotonic()
        for _ in range(4):
            quiet(.9)
            context.text(PREFIX)
            quiet(.03)
            assert auxiliary() == UNAVAILABLE_NOTICE, 'Surrounding publication must not shorten the notice lease'
        wait(lambda: auxiliary() == '', 'five-second notice expiry despite surrounding publication', timeout=2)
        assert 4.8 <= time.monotonic() - notice_started < 5.8, 'Publication must not renew or defeat notice expiry'
        assert context.commits() == [] and control('status')['metrics']['context_updates'] == 0
        context.close()
        context = None
        checks.append('Unavailable notice clears on focus loss, foreign Unicode composition and its original five-second expiry')

        context = desktop.InputContext(FIXTURE_APP)
        context.text(PREFIX)
        wait(lambda: backend.inspections > 0, 'automatic field metadata observation')
        quiet(.4)
        assert backend.reads == 0 and control('status')['metrics']['context_updates'] == 0
        assert context.candidate() is None
        checks.append('Browser-origin target is unavailable even for an unrecognized program alias')

        assert context.key(ord(' '), 5)
        wait(lambda: auxiliary() == UNAVAILABLE_NOTICE, 'observed browser unavailable notice')
        inspections = backend.inspections
        context.text(PREFIX)
        wait(lambda: auxiliary() == '' and backend.inspections > inspections,
             'unknown-alias browser republishing invalidates and reinspects authority')
        assert backend.reads == 0 and control('status')['metrics']['context_updates'] == 0
        checks.append('Unknown-alias observed browser notices do not exempt surrounding publication from authority reinspection')

        backend.uri = 'https://allowed.example.test/document'
        daemon.observer.invalidate('fixture_field_changed')
        context.text(PREFIX)
        quiet(.3)
        assert not context.key(desktop.TAB)
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
        wait(lambda: context.candidate() == SUFFIX, 'automatic observed suggestion')
        metrics = control('status')['metrics']
        assert metrics['context_updates'] == 1 and metrics['provider_calls'] == 1
        assert backend.reads >= 2
        checks.append('Synthetic desktop inspect → exact app policy → corroborated snapshot → real broker suggestion')

        context.text(PREFIX)
        quiet(.25)
        assert context.candidate() == SUFFIX
        assert control('status')['metrics']['context_updates'] == metrics['context_updates']
        checks.append('Unchanged toolkit context preserves the visible candidate and session')

        assert context.key(desktop.TAB)
        wait(lambda: context.commits() == [SUFFIX], 'Tab dispatch after observed verification')
        checks.append('Explicit Tab verifies the current observed field and dispatches exact CommitString once')

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
            wait(lambda: context.candidate() == SUFFIX, 'fresh synthetic desktop binding after malformed metadata')
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
            wait(lambda: context.candidate() == SUFFIX, 'fresh repeated-text candidate before stale commit')
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
        wait(lambda: context.candidate() == SUFFIX, 'automatic new field recovery behind stale observer RPC')
        assert preview_fields[-1] == f'/synthetic/field/{backend.field}'
        assert context.commits() == previous_commits
        checks.append('A field switch during an observer RPC retries boundedly and automatically previews the new field')

        change(PREFIX + ' x')
        quiet(.15)
        change(PREFIX)
        wait(lambda: context.candidate() == SUFFIX, 'fresh suggestion before same-text field switch')
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
        wait(lambda: context.candidate() == SUFFIX, 'policy recovery before pause')
        control('pause', 'on')
        wait(lambda: context.candidate() is None, 'pause clears candidate')
        reads = backend.reads
        change('never acquire this paused synthetic text')
        quiet(.4)
        assert backend.reads == reads
        checks.append('Broker pause clears UI and blocks subsequent observer prose reads')
        control('pause', 'off')
        change(PREFIX)
        wait(lambda: context.candidate() == SUFFIX, 'resume with fresh context')

        # Fcitx 5.1.21's shipped Unicode module owns real foreign preedit. Its
        # default direct-mode shortcut is Control+Shift+U; only this private
        # D-Bus input context receives the key, with no graphical input device.
        assert context.key(ord('U'), 5)
        assert context.key(ord('4'))
        wait(lambda: preedit() == 'U+4', 'real Unicode formatted preedit')
        wait(lambda: context.candidate() != SUFFIX, 'Unicode composition takes over Badi UI')
        reads = backend.reads
        context.text(PREFIX)
        quiet(.3)
        assert backend.reads == reads and context.candidate() != SUFFIX
        assert context.key(desktop.ESCAPE)
        wait(lambda: context.candidate() == SUFFIX, 'automatic recovery after foreign preedit clears')
        assert preedit() == ''
        checks.append('Real Unicode addon preedit takes priority, prevents prose reads, then automatically resumes on clear')

        context.ic('FocusOut')
        wait(lambda: context.candidate() is None, 'focus loss clears candidate')
        context.ic('FocusIn')
        context.text(PREFIX)
        wait(lambda: context.candidate() == SUFFIX, 'fresh focus recovery')
        checks.append('Focus loss clears and fresh focus obtains new authority')

        stop(broker)
        broker = None
        wait(lambda: context.candidate() is None, 'broker exit clears candidate')
        broker = start_broker()
        quiet(.8)
        change(PREFIX)
        wait(lambda: context.candidate() == SUFFIX, 'same input context after broker restart', timeout=6)
        assert context.key(desktop.TAB)
        wait(lambda: len(context.commits()) == previous + 1, 'recovered Tab dispatch')
        assert context.commits()[-1] == SUFFIX
        checks.append('Same live input context recovers after broker restart with a fresh verified dispatch')

        context.close()
        context = desktop.InputContext('omawrite')
        context.text(PREFIX)
        updates = control('status')['metrics']['context_updates']
        reads = backend.reads
        quiet(.3)
        assert context.candidate() is None and control('status')['metrics']['context_updates'] == updates
        assert backend.reads == reads
        assert context.key(desktop.TAB), 'Unobserved native manual invocation remains available'
        wait(lambda: context.candidate() == SUFFIX, 'native manual phrase candidate')
        assert context.key(desktop.TAB)
        wait(lambda: context.commits() == [SUFFIX], 'native manual exact append dispatch')
        assert backend.reads == reads
        checks.append('Native manual unknown-widget append still invokes and dispatches once without observer prose')

        result = {'boundary': 'browser quarantine + synthetic desktop D-Bus fields + actual Fcitx addon/observer + phrase broker',
                  'physical_editor_or_overlay_proof': False, 'passed': True, 'checks': checks,
                  'broker_metrics': control('status')['metrics'], 'observer_reads': backend.reads}
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
    if not args.addon.is_file():
        raise RuntimeError('Build required fixture addon: ' + str(args.addon))
    for path in ['adapters/fcitx5/build/badi.conf',
                 'target/debug/badi-broker', 'target/debug/badictl']:
        if not (ROOT / path).is_file():
            raise RuntimeError('Build required fixture artifact: ' + path)
    reports = ROOT / 'output/native-trials'
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
                                   for app in sorted((*QUARANTINED_APPS, FIXTURE_APP, 'omawrite')))]}
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
        child = subprocess.Popen(command, env=environment, start_new_session=True)
        try:
            if child.wait(timeout=55):
                raise RuntimeError('Observed addon integration failed; diagnostics: ' + str(report))
        finally:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGTERM)
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait(timeout=5)
    print('Disposable observed addon integration passed; report: ' + str(report))


if __name__ == '__main__':
    main()
