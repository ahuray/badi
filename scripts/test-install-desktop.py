"""Exercise desktop installation boundaries without changing the running session."""
import contextlib
import datetime
import hashlib
import importlib.util
import io
import json
import mmap
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('install_desktop', Path(__file__).with_name('install-desktop.py'))
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)
import badi_install as receipts  # noqa: E402  (scripts/ is this test's own directory)
CHECKOUT = Path(__file__).resolve().parents[1]
NATIVE_SOURCES = ('target/release/badi-broker', 'target/release/badictl', 'scripts/badi-desktop.py', 'scripts/badi_install.py',
                  'packaging/io.github.ahuray.badi.desktop', 'packaging/io.github.ahuray.badi.svg',
                  'packaging/systemd/badi-broker.service', 'broker/data/writing-lexicon/LICENSE',
                  'broker/data/writing-lexicon/README.md', 'adapters/fcitx5/build/libbadi-fcitx5.so',
                  'adapters/fcitx5/build/badi.conf', 'packaging/systemd/fcitx-badi.conf',
                  'packaging/systemd/badi-accessibility.service') + tuple(
                  'adapters/accessibility/' + name for name in ('daemon.py', 'contract.py', 'preview.py', 'health.py'))
STOCK = '{ path=/usr/bin/fcitx5 ; argv[]=%s ; ignore_errors=no ; start_time=[n/a] ; pid=0 }\n'


def only_backup(home):
    """The one desktop backup directory, its change map and the changed paths."""
    backup, = (home / '.local/state/badi/install-backups').iterdir()
    document = json.loads((backup / 'changes.json').read_text())
    return backup, document, [entry['path'] for entry in document['changes']]


def saved(backup, document, relative):
    """Where the backup keeps the original of one changed path."""
    entry, = (entry for entry in document['changes'] if entry['path'] == relative)
    return backup / entry['saved']


def native_project(project):
    for name in NATIVE_SOURCES:
        source = project / name
        source.parent.mkdir(parents=True, exist_ok=True)
        source.write_text('new ' + name)
    # The pinned selector and manifest define the supported Fcitx cell.
    for name in ('launch.py', 'manifest.json'):
        target = project / 'packaging/fcitx5-wayland-compat' / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(CHECKOUT / 'packaging/fcitx5-wayland-compat' / name, target)


def compat_fixture(root):
    """A checked build receipt bound to task-owned stand-ins for system Fcitx files."""
    runtime = root / 'system'
    runtime.mkdir()
    files = []
    for index in range(5):
        path = runtime / f'runtime-{index}'
        path.write_bytes(f'runtime {index}'.encode())
        files.append(str(path))
    launcher = SimpleNamespace(VERSION='5.1.22', RUNTIME_FILES=tuple(files),
                               digest=lambda path: hashlib.sha256(Path(path).read_bytes()).hexdigest())
    build = root / 'build'
    build.mkdir()
    (build / 'libwaylandim.so').write_bytes(b'patched frontend')
    receipt = {'schema': 'badi.fcitx-wayland-compat-build.v1',
               'source': json.loads((CHECKOUT / 'packaging/fcitx5-wayland-compat/manifest.json').read_text()),
               'installed_build_versions': {name: '5.1.22' for name in ('Fcitx5Core', 'Fcitx5Config', 'Fcitx5Utils')},
               'runtime_files_sha256': {path: launcher.digest(path) for path in files},
               'artifact_sha256': hashlib.sha256(b'patched frontend').hexdigest(),
               'protocol_checks_passed': True, 'physical_editor_verified': False, 'installed': False}
    (build / 'build-receipt.json').write_text(json.dumps(receipt))
    return launcher, build, receipt


def old_compat(home):
    old = home / '.local/lib/badi/compat/fcitx5-5.1.21'
    (old / 'addons').mkdir(parents=True)
    for name in installer.COMPAT_FILES:
        (old / name).write_text('retired ' + name)
    return old


def retired_browser(home):
    """What the former install-editors.py --chromium left, beside foreign files."""
    host = home / '.local/lib/badi/badi-native-host'
    renderer = home / '.local/lib/badi/badi-native-manifest'
    extension = home / '.local/lib/badi/chromium'
    (extension / 'icons').mkdir(parents=True)
    for path in (host, renderer, extension / 'manifest.json', extension / 'icons/badi-16.png'):
        path.write_text('retired ' + path.name)
    hosts = lambda browser: home / '.config' / browser / 'NativeMessagingHosts'
    badi = [hosts(browser) / 'io.github.ahuray.badi.json' for browser in ('chromium', 'BraveSoftware/Brave-Browser')]
    foreign = hosts('google-chrome') / 'io.github.ahuray.badi.json'
    malformed = hosts('BraveSoftware/Brave-Browser-Beta') / 'io.github.ahuray.badi.json'
    linked = hosts('BraveSoftware/Brave-Origin') / 'io.github.ahuray.badi.json'
    for path, text in ((badi[0], json.dumps({'name': 'io.github.ahuray.badi', 'path': str(host)})),
                       (badi[1], json.dumps({'name': 'io.github.ahuray.badi', 'path': str(host)})),
                       (foreign, json.dumps({'name': 'io.github.ahuray.badi', 'path': '/opt/other/host'})),
                       (malformed, '{')):
        path.parent.mkdir(parents=True)
        path.write_text(text)
    linked.parent.mkdir(parents=True)
    linked.symlink_to(badi[0])
    return SimpleNamespace(retired=[host, renderer, extension / 'icons/badi-16.png', extension / 'manifest.json', *badi],
                           extension=extension, kept=[foreign, malformed, linked])


def broker_only_install(root):
    home, project = root / 'home', root / 'project'
    native_project(project)

    def execute(command, **kwargs):
        output = str(root / 'runtime') if command[0] == 'loginctl' else 'loaded' if '--property=LoadState' in command else '{}'
        return subprocess.CompletedProcess(command, 0, output, '')

    with patch.object(installer, 'ROOT', project), \
         patch.object(installer.Path, 'home', return_value=home), \
         patch.object(installer.subprocess, 'run', side_effect=execute), \
         patch.object(installer, 'probe_model', return_value={'provider': 'local_model', 'paused': False, 'control_plane_degraded': False}), \
         patch.dict(os.environ, {'WAYLAND_DISPLAY': 'wayland-test', 'XDG_RUNTIME_DIR': str(root / 'runtime'),
                                 'XDG_CONFIG_HOME': str(home / '.config')}), \
         patch('sys.argv', ['install-desktop.py', '--broker-only']), \
         contextlib.redirect_stdout(io.StringIO()) as output:
        installer.main()
    return output.getvalue()


class DesktopInstallTests(unittest.TestCase):
    def test_retired_browser_selection_spares_foreign_and_unexpected_entries(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary).resolve()
            browser = retired_browser(home)
            files, folders, kept = installer.retired_browser_components(home, home / '.config')
            self.assertEqual(sorted(files), sorted(browser.retired))
            self.assertEqual(folders, [browser.extension / 'icons', browser.extension])
            self.assertEqual(kept, [])
            (browser.extension / 'link').symlink_to(home)
            files, folders, kept = installer.retired_browser_components(home, home / '.config')
            self.assertFalse(any(path.is_relative_to(browser.extension) for path in files))
            self.assertEqual((folders, kept), ([], [browser.extension]))

    def test_install_retires_the_browser_extension_with_backup_once(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            home = root / 'home'
            browser = retired_browser(home)
            shell = home / '.local/lib/badi/editors/shell/badi.bash'
            shell.parent.mkdir(parents=True)
            shell.write_text('kept editor')
            editors = {'.local/lib/badi/editors/shell/badi.bash': {'sha256': 'b' * 64},
                       **{str(path.relative_to(home)): {'sha256': 'a' * 64} for path in browser.retired}}
            receipts.store(receipts.receipt_path(home, 'editors'), {
                'schema': receipts.SCHEMA, 'installer': 'editors', 'installed_at': '2026-09-01T00:00:00Z',
                'source': {'commit': 'c' * 40, 'dirty': False}, 'files': editors})
            original = {str(path.relative_to(home)): path.read_text() for path in browser.retired}
            output = broker_only_install(root)
            self.assertEqual(output.count('brave://extensions'), 1)
            self.assertFalse(any(path.exists() for path in browser.retired))
            self.assertFalse(browser.extension.exists())
            self.assertTrue(all(path.is_symlink() or path.exists() for path in browser.kept))
            backup, document, changed = only_backup(home)
            for relative, text in original.items():
                self.assertIn(relative, changed)
                self.assertEqual(saved(backup, document, relative).read_text(), text)
            receipt = json.loads(receipts.receipt_path(home, 'editors').read_text())
            self.assertEqual(receipt['files'], {'.local/lib/badi/editors/shell/badi.bash': {'sha256': 'b' * 64}})
            self.assertEqual((receipt['installed_at'], receipt['source']['commit']), ('2026-09-01T00:00:00Z', 'c' * 40))
            self.assertEqual(shell.read_text(), 'kept editor')
            second = broker_only_install(root)
            self.assertNotIn('brave://extensions', second)
            self.assertTrue(all(path.is_symlink() or path.exists() for path in browser.kept))
            self.assertIn('already current; no backup was needed', second)
            self.assertEqual(only_backup(home)[0], backup, 'An unchanged reinstall makes no backup')

    def test_observed_flags_add_only_accessibility_and_are_idempotent(self):
        original = '# user comment\n--ozone-platform=wayland\n--some-user-option'
        for app in installer.OBSERVED_APP_FLAGS:
            with self.subTest(app=app):
                updated = installer.observed_flags(original, app)
                self.assertTrue(updated.startswith(original + '\n'), 'user options stay byte-identical')
                added = updated[len(original) + 1:].splitlines()
                self.assertEqual([line for line in added if not line.startswith('#')], [installer.OBSERVED_FLAG])
                for absent in ('--enable-wayland-ime', '--wayland-text-input-version', '--ozone-platform=x11'):
                    self.assertNotIn(absent, updated[len(original):])
                self.assertIsNone(installer.observed_flags(updated, app), 'a second run changes nothing')
                self.assertEqual(installer.observed_flags('', app),
                                 '# Badi: renderer accessibility for the focused-field observer\n' + installer.OBSERVED_FLAG + '\n')
                # The bare switch already selects the complete mode.
                self.assertIsNone(installer.observed_flags('--force-renderer-accessibility\n', app))
                # Input and platform flags are the user's; only accessibility is ours.
                self.assertIsNotNone(installer.observed_flags('--ozone-platform=x11\n', app))

    def test_reduced_or_disabled_accessibility_is_reported_not_overridden(self):
        for app in installer.OBSERVED_APP_FLAGS:
            for text in ('--force-renderer-accessibility=basic\n', '--force-renderer-accessibility=form-controls\n',
                         '--force-renderer-accessibility=complete\n--force-renderer-accessibility=basic\n',
                         '--disable-renderer-accessibility\n'):
                with self.subTest(app=app, text=text), self.assertRaisesRegex(RuntimeError, 'conflicts'):
                    installer.observed_flags(text, app)

    def test_observed_setup_is_never_a_broker_only_side_effect(self):
        with patch('sys.argv', ['install-desktop.py', '--broker-only', '--observed-app', 'chatgpt']), \
             patch.object(installer.subprocess, 'run') as command, contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as stopped:
                installer.main()
        self.assertEqual(stopped.exception.code, 2)
        command.assert_not_called()

    def test_observed_apps_use_each_launchers_own_flags_file(self):
        self.assertEqual({app: name for app, (name, _parser) in installer.OBSERVED_APP_FLAGS.items()},
                         {'chromium': 'chromium-flags.conf', 'brave-origin': 'brave-origin-flags.conf',
                          'chatgpt': 'codex-flags.conf', 'code': 'code-flags.conf', 'cursor': 'cursor-flags.conf'})
        self.assertNotIn('discord', installer.OBSERVED_APP_FLAGS, 'Discord has no supported flags file')

    def test_observed_flags_follow_each_installed_wrappers_quoting(self):
        quoted = "'--force-renderer-accessibility=basic'\n"
        # Chromium's launcher removes GLib shell quotes, so this is a real choice.
        with self.assertRaisesRegex(RuntimeError, 'conflicts'):
            installer.observed_flags(quoted, 'chromium')
        original = '--force-renderer-accessibility="complete" # chosen mode\n'
        self.assertIsNone(installer.observed_flags(original, 'chromium'))
        with self.assertRaisesRegex(RuntimeError, 'unbalanced quoting'):
            installer.observed_flags("'--force-renderer-accessibility\n", 'chromium')
        # The other wrappers pass quote characters literally: not a recognized switch.
        for app in ('chatgpt', 'code', 'brave-origin', 'cursor'):
            with self.subTest(app=app):
                result = installer.observed_flags(quoted, app)
                self.assertIn('\n' + installer.OBSERVED_FLAG + '\n', result)
                self.assertIsNone(installer.observed_flags(result, app))
        # Word-splitting wrappers strip text after any # and split one line.
        for app in ('chatgpt', 'code'):
            with self.subTest(app=app):
                self.assertIsNone(installer.observed_flags('--a --force-renderer-accessibility=complete#note\n', app))
                self.assertIsNotNone(installer.observed_flags('--a #--force-renderer-accessibility=complete\n', app))
        # Line wrappers pass each non-comment line as one argument, verbatim.
        for app in ('brave-origin', 'cursor'):
            with self.subTest(app=app):
                self.assertIsNotNone(installer.observed_flags('--a --force-renderer-accessibility=complete\n', app))
                self.assertIsNotNone(installer.observed_flags('  # --force-renderer-accessibility=complete\n', app))
                with self.assertRaisesRegex(RuntimeError, 'conflicts'):
                    installer.observed_flags('--force-renderer-accessibility=complete # note\n', app)

    def test_chromium_comments_preserve_embedded_quoted_and_escaped_hashes(self):
        for line, tokens in (
            ('--some=value#suffix', ['--some=value#suffix']),
            ('--some="value#text" # comment', ['--some=value#text']),
            ('--some=value\\#text', ['--some=value#text']),
            ("'#literal' --ozone-platform=wayland", ['#literal', '--ozone-platform=wayland']),
            ('# whole line comment', []),
        ):
            self.assertEqual(installer.chromium_line_tokens(line), tokens)
        with self.assertRaisesRegex(RuntimeError, 'conflicts'):
            installer.observed_flags('--force-renderer-accessibility=complete#suffix\n', 'chromium')

    def test_observed_configuration_normalizes_paths_and_rejects_symlink_escape(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            home = root / 'home'
            home.mkdir()
            config = home / '.config'
            config.mkdir()
            expected = config / 'chromium-flags.conf'
            self.assertEqual(installer.observed_config_target(home, home / '.config/.', expected.name), expected)
            with self.assertRaisesRegex(RuntimeError, 'parent traversal'):
                installer.observed_config_target(home, home / '../outside', expected.name)
            with self.assertRaisesRegex(RuntimeError, 'inside the user home'):
                installer.observed_config_target(home, root / 'outside', expected.name)
            with self.assertRaisesRegex(RuntimeError, 'absolute'):
                installer.observed_config_target(home, Path('.config'), expected.name)
            (home / 'alias').symlink_to(config, target_is_directory=True)
            with self.assertRaisesRegex(RuntimeError, 'directory symlink'):
                installer.observed_config_target(home, home / 'alias', expected.name)
            expected.symlink_to(config / 'another-file')
            with self.assertRaisesRegex(RuntimeError, 'configuration symlink'):
                installer.observed_config_target(home, config, expected.name)

    def test_accessibility_enablement_records_both_prior_values_and_verifies_result(self):
        with tempfile.TemporaryDirectory() as temporary, \
             patch.object(installer, 'require_unlocked'), \
             patch.object(installer, 'run', side_effect=[
                 subprocess.CompletedProcess([], 0, 'b false\n', ''),
                 subprocess.CompletedProcess([], 0, 'false\n', ''),
                 subprocess.CompletedProcess([], 0, '', ''),
                 subprocess.CompletedProcess([], 0, 'b true\n', ''),
             ]) as command:
            installation = receipts.Installation(Path(temporary), 'desktop')
            installer.enable_accessibility(installation)
            receipt = installation.directory / 'accessibility-setting.json'
            self.assertEqual(json.loads(receipt.read_text()), {'bus_enabled': False, 'toolkit_accessibility': False})
            self.assertEqual(receipt.stat().st_mode & 0o777, 0o600)
        self.assertIn('set-property', command.call_args_list[2].args[0])

    def test_trial_runtime_is_rejected_before_installation(self):
        with patch.object(installer.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, '/run/user/1000\n', '')) as command, \
             patch.dict(os.environ, {'WAYLAND_DISPLAY': 'wayland-test', 'XDG_RUNTIME_DIR': '/tmp/badi-isolated-trial'}), \
             patch('sys.argv', ['install-desktop.py', '--broker-only']):
            with self.assertRaisesRegex(RuntimeError, 'isolated trial'):
                installer.main()
        self.assertEqual(command.call_count, 1)
        self.assertEqual(command.call_args.args[0][0], 'loginctl')

    def test_model_probe_checks_service_peer_and_explicit_socket(self):
        with tempfile.TemporaryDirectory() as temporary:
            endpoint = Path(temporary) / 'broker.sock'
            with socket.socket(socket.AF_UNIX) as listener:
                listener.bind(str(endpoint))
                listener.listen(2)
                def execute(command, **kwargs):
                    output = str(os.getpid()) if command[0] == 'systemctl' else '{"provider":"local_model"}'
                    return subprocess.CompletedProcess(command, 0, output, '')
                with patch.object(installer.subprocess, 'run', side_effect=execute) as command:
                    self.assertEqual(installer.probe_model(Path('/installed/badictl'), endpoint)['provider'], 'local_model')
                    self.assertIn(['/installed/badictl', '--socket', str(endpoint), 'status'],
                                  [call.args[0] for call in command.call_args_list])

    def test_model_probe_rejects_a_different_broker_on_the_socket(self):
        with tempfile.TemporaryDirectory() as temporary:
            endpoint = Path(temporary) / 'broker.sock'
            with socket.socket(socket.AF_UNIX) as listener:
                listener.bind(str(endpoint))
                listener.listen(1)
                with patch.object(installer.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, str(os.getpid() + 1), '')) as command:
                    with self.assertRaisesRegex(RuntimeError, 'different process'):
                        installer.probe_model(Path('/installed/badictl'), endpoint)
                    self.assertEqual(command.call_count, 1)

    def test_native_install_requires_explicit_unlocked_state_before_build_or_mutation(self):
        for state in ({'locked': True}, {}, {'locked': False}, None):
            with self.subTest(state=state), \
                 patch.object(installer.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, json.dumps(state), '')) as command, \
                 patch.dict(os.environ, {'WAYLAND_DISPLAY': 'wayland-test', 'XDG_RUNTIME_DIR': '/tmp/badi-test-runtime'}), \
                 patch('sys.argv', ['install-desktop.py']):
                with self.assertRaisesRegex(RuntimeError, 'Unlock'):
                    installer.main()
                self.assertEqual([call.args[0] for call in command.call_args_list], [['omarchy-shell', 'lock', 'status']])

    def test_broker_only_install_preserves_native_files_and_retains_recovery_map(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            home, project = root / 'home', root / 'project'
            sources = ('target/release/badi-broker', 'target/release/badictl', 'scripts/badi-desktop.py', 'scripts/badi_install.py',
                       'packaging/io.github.ahuray.badi.desktop', 'packaging/io.github.ahuray.badi.svg',
                       'packaging/systemd/badi-broker.service', 'broker/data/writing-lexicon/LICENSE',
                       'broker/data/writing-lexicon/README.md')
            for name in sources:
                source = project / name
                source.parent.mkdir(parents=True, exist_ok=True)
                source.write_text('new ' + name)
            # The real CLI and its helper module, to run them from the installed layout.
            for name in ('badi-desktop.py', 'badi_install.py'):
                shutil.copy2(CHECKOUT / 'scripts' / name, project / 'scripts' / name)
            native = home / '.local/lib/fcitx5/libbadi-fcitx5.so'
            native.parent.mkdir(parents=True)
            native.write_text('keep native addon')
            helper = home / '.local/lib/badi/accessibility/daemon.py'
            helper.parent.mkdir(parents=True)
            helper.write_text('keep running helper')
            previous = home / '.local/lib/badi/badi-broker'
            previous.parent.mkdir(parents=True, exist_ok=True)
            previous.write_text('previous broker')
            settings = home / '.config/badi/settings.json'
            settings.parent.mkdir(parents=True)
            settings.write_text('{"paused":true,"user_preference":"preserved"}')
            old_notice = home / '.local/share/badi/licenses/writing-lexicon/LICENSE'
            old_notice.parent.mkdir(parents=True)
            old_notice.write_text('previous complete notice')
            calls = []

            def execute(command, **kwargs):
                calls.append(command)
                output = str(root / 'runtime') if command[0] == 'loginctl' else 'loaded' if '--property=LoadState' in command else '{}'
                return subprocess.CompletedProcess(command, 0, output, '')

            with patch.object(installer, 'ROOT', project), \
                 patch.object(installer.Path, 'home', return_value=home), \
                 patch.object(installer.subprocess, 'run', side_effect=execute), \
                 patch.object(installer, 'probe_model', return_value={'provider': 'local_model', 'paused': True, 'control_plane_degraded': False}), \
                 patch.dict(os.environ, {'WAYLAND_DISPLAY': 'wayland-test', 'XDG_RUNTIME_DIR': str(root / 'runtime'),
                                         'XDG_CONFIG_HOME': str(home / '.config')}), \
                 patch('sys.argv', ['install-desktop.py', '--broker-only']), \
                 contextlib.redirect_stdout(io.StringIO()) as output:
                installer.main()
            self.assertIn('Predictions remain paused', output.getvalue())
            self.assertIn('native input addon was not restarted', output.getvalue())
            self.assertEqual(native.read_text(), 'keep native addon')
            self.assertEqual(helper.read_text(), 'keep running helper')
            self.assertEqual(json.loads(settings.read_text())['paused'], True)
            self.assertFalse(any('omarchy-fcitx5.service' in call or call[0] in ('omarchy-shell', 'npm', '/usr/bin/fcitx5') for call in calls))
            self.assertFalse((home / '.local/lib/badi/compat').exists())
            self.assertFalse((home / installer.COMPAT_DROPIN).exists())
            self.assertFalse(any('badi-accessibility.service' in call or call[0] == '/usr/bin/python3' for call in calls))
            self.assertNotIn(['systemctl', '--user', 'enable', 'badi-broker.service'], calls)
            backup, document, changed = only_backup(home)
            self.assertEqual(saved(backup, document, '.local/lib/badi/badi-broker').read_text(), 'previous broker')
            self.assertIn('.local/lib/badi/badi-broker', changed)
            self.assertFalse(any('fcitx' in path or 'accessibility' in path for path in changed))
            for name in ('LICENSE', 'README.md'):
                relative = '.local/share/badi/licenses/writing-lexicon/' + name
                self.assertEqual((home / relative).read_bytes(), (project / 'broker/data/writing-lexicon' / name).read_bytes())
                self.assertIn(relative, changed)
            self.assertEqual(saved(backup, document, '.local/share/badi/licenses/writing-lexicon/LICENSE').read_text(),
                             'previous complete notice')
            receipt_file = home / '.local/state/badi/receipts/desktop.json'
            self.assertEqual(receipt_file.stat().st_mode & 0o777, 0o600)
            receipt = json.loads(receipt_file.read_text())
            self.assertEqual((receipt['schema'], receipt['installer']), ('badi.install-receipt.v1', 'desktop'))
            # The mocked Git boundary yields no identity rather than a guess.
            self.assertEqual(receipt['source'], {'commit': 'unknown', 'dirty': None})
            links = {'.local/bin/badi': '../lib/badi/badi-desktop.py', '.local/bin/badictl': '../lib/badi/badictl',
                     '.local/bin/badi-desktop': '../lib/badi/badi-desktop.py'}
            self.assertEqual({relative: os.readlink(home / relative) for relative in links}, links,
                             'Each command is installed once; PATH names are links')
            self.assertEqual(sorted(receipt['files']), sorted(set(changed) - set(links)))
            # The installed CLI imports its helper module from beside its real path.
            for command in ('badi', 'badi-desktop'):
                result = subprocess.run([str(home / '.local/bin' / command), '--help'], capture_output=True,
                                        text=True, timeout=10, env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1'})
                self.assertEqual((result.returncode, result.stderr), (0, ''))
                self.assertIn('badi status [--json]', result.stdout)
            broker = receipt['files']['.local/lib/badi/badi-broker']
            self.assertEqual(broker['sha256'], hashlib.sha256(b'new target/release/badi-broker').hexdigest())
            self.assertIsNone(broker['version'])
            self.assertIn('Install receipt:', output.getvalue())

    def test_receipt_identity_comes_from_the_real_checkout_or_is_unknown(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'checkout'
            root.mkdir()
            self.assertEqual(receipts.source_identity(root), {'commit': 'unknown', 'dirty': None})
            git = ['git', '-C', str(root), '-c', 'user.name=Badi Test', '-c', 'user.email=test@invalid',
                   '-c', 'commit.gpgsign=false']
            subprocess.run([*git, 'init', '--quiet'], check=True)
            (root / 'tracked.txt').write_text('one\n')
            subprocess.run([*git, 'add', 'tracked.txt'], check=True)
            subprocess.run([*git, 'commit', '--quiet', '-m', 'fixture'], check=True)
            head = subprocess.run([*git, 'rev-parse', 'HEAD'], check=True, capture_output=True, text=True).stdout.strip()
            self.assertEqual(receipts.source_identity(root), {'commit': head, 'dirty': False})
            (root / 'untracked.txt').write_text('new\n')
            self.assertEqual(receipts.source_identity(root), {'commit': head, 'dirty': True})
            (root / 'untracked.txt').unlink()
            (root / 'tracked.txt').write_text('two\n')
            self.assertEqual(receipts.source_identity(root), {'commit': head, 'dirty': True})
            with patch.dict(os.environ, {'PATH': str(root / 'no-git')}):
                self.assertEqual(receipts.source_identity(root), {'commit': 'unknown', 'dirty': None})
            # An exported tree inside another repository must not inherit its
            # commit, matching the unknown identity build.rs embeds there.
            exported = root / 'exported-badi'
            exported.mkdir()
            (exported / 'README.md').write_text('export\n')
            self.assertEqual(receipts.source_identity(exported), {'commit': 'unknown', 'dirty': None})
            self.assertEqual(receipts.source_identity(root), {'commit': head, 'dirty': True})

    def test_receipt_records_digests_versions_and_keeps_untouched_entries(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            library = home / '.local/lib/badi'
            library.mkdir(parents=True)
            broker = library / 'badi-broker'
            line = 'badi-broker 0.1.0 commit=' + 'a' * 40 + ' dirty=false'
            broker.write_text(f'#!/bin/sh\necho "{line}"\n')
            broker.chmod(0o755)
            noisy = library / 'badictl'
            noisy.write_text('#!/bin/sh\necho "badictl typed prose"\n')
            noisy.chmod(0o755)
            notice = home / '.local/share/badi/notice'
            notice.parent.mkdir(parents=True)
            notice.write_text('notice')
            first = {'commit': 'b' * 40, 'dirty': True}
            when = datetime.datetime(2026, 9, 26, 12, 0, tzinfo=datetime.timezone.utc)
            path = receipts.write_receipt(home, 'desktop', first, [broker, noisy, notice, broker], now=when)
            self.assertEqual(path, home / '.local/state/badi/receipts/desktop.json')
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            receipt = json.loads(path.read_text())
            self.assertEqual(receipt['installed_at'], '2026-09-26T12:00:00Z')
            self.assertEqual(receipt['source'], first)
            entry = receipt['files']['.local/lib/badi/badi-broker']
            # A binary built from an earlier checkout keeps its own embedded
            # identity rather than the installing checkout's.
            self.assertEqual(entry, {'sha256': hashlib.sha256(broker.read_bytes()).hexdigest(),
                                     'installed_at': '2026-09-26T12:00:00Z', 'commit': 'a' * 40,
                                     'dirty': False, 'version': line})
            unrecognized = receipt['files']['.local/lib/badi/badictl']
            self.assertEqual((unrecognized['version'], unrecognized['commit'], unrecognized['dirty']),
                             (None, 'unknown', None),
                             'Unrecognized version output is neither copied nor replaced by the checkout identity')
            self.assertEqual(receipt['files']['.local/share/badi/notice']['commit'], 'b' * 40)
            self.assertNotIn('version', receipt['files']['.local/share/badi/notice'])

            rebuilt = 'badi-broker 0.1.0 commit=' + 'c' * 40 + ' dirty=true'
            broker.write_text(f'#!/bin/sh\necho "{rebuilt}"\n')
            second = {'commit': 'c' * 40, 'dirty': False}
            receipts.write_receipt(home, 'desktop', second, [broker],
                                   now=when + datetime.timedelta(hours=1))
            receipt = json.loads(path.read_text())
            self.assertEqual(receipt['source'], second)
            entry = receipt['files']['.local/lib/badi/badi-broker']
            self.assertEqual((entry['commit'], entry['dirty'], entry['version']), ('c' * 40, True, rebuilt))
            self.assertEqual(receipt['files']['.local/share/badi/notice']['commit'], 'b' * 40,
                             'A partial update keeps the earlier identity of untouched files')
            self.assertEqual([item.name for item in path.parent.iterdir()], ['desktop.json'])

    def test_model_readiness_rejects_degraded_or_missing_settings_health(self):
        for degraded in (True, None, 'false'):
            with self.subTest(degraded=degraded):
                with self.assertRaisesRegex(RuntimeError, 'persistent settings'):
                    installer.require_model_health({'provider': 'local_model', 'paused': False, 'control_plane_degraded': degraded})
        with self.assertRaisesRegex(RuntimeError, 'pause state'):
            installer.require_model_health({'provider': 'local_model', 'control_plane_degraded': False})
        probe = {'provider': 'local_model', 'paused': True, 'control_plane_degraded': False}
        installer.require_model_health(probe)
        self.assertTrue(probe['paused'], 'A successful update must preserve persisted pause')

    def test_native_readiness_requires_exact_mapped_file_and_stable_active_process(self):
        with tempfile.TemporaryDirectory() as temporary:
            addon = Path(temporary) / 'libbadi-fcitx5.so'
            addon.write_bytes(b'new addon')
            info = addon.stat()
            device = f'{os.major(info.st_dev):02x}:{os.minor(info.st_dev):02x}'
            active = subprocess.CompletedProcess([], 0, 'ActiveState=active\nMainPID=123\n', '')
            mapping = f'1000-2000 r-xp 00000000 {device} {info.st_ino} {addon}\n'
            for maps, final, expected in (
                (mapping, active, True),
                (mapping.replace(str(addon), str(addon) + ' (deleted)'), active, False),
                (mapping.replace(str(info.st_ino), str(info.st_ino + 1)), active, False),
                (mapping.replace(str(addon), str(addon.parent / 'other.so')), active, False),
                (mapping, subprocess.CompletedProcess([], 0, 'ActiveState=failed\nMainPID=0\n', ''), False),
                (mapping, subprocess.CompletedProcess([], 0, 'ActiveState=active\nMainPID=456\n', ''), False),
            ):
                with self.subTest(maps=maps, final=final.stdout), \
                     patch.object(installer.subprocess, 'run', side_effect=[active, final]) as command, \
                     patch.object(installer.Path, 'read_text', return_value=maps):
                    self.assertEqual(installer.native_addon_loaded(addon), expected)
                    self.assertTrue(all(call.args[0][0] == 'systemctl' for call in command.call_args_list))

    def test_native_wait_allows_delayed_loading_and_has_a_deadline(self):
        with patch.object(installer, 'native_addon_loaded', side_effect=[False, False, True]) as probe, \
             patch.object(installer.time, 'monotonic', return_value=0), \
             patch.object(installer.time, 'sleep') as sleep:
            installer.wait_native_addon(Path('/installed/addon.so'))
            self.assertEqual(probe.call_count, 3)
            self.assertEqual(sleep.call_count, 2)
        with patch.object(installer, 'native_addon_loaded', return_value=False), \
             patch.object(installer.time, 'monotonic', side_effect=[0, 11]), \
             patch.object(installer.time, 'sleep') as sleep:
            with self.assertRaisesRegex(RuntimeError, 'running Fcitx service'):
                installer.wait_native_addon(Path('/installed/addon.so'))
            sleep.assert_not_called()

    def test_native_readiness_matches_real_mapping_on_the_host_filesystem(self):
        # The project lives on Btrfs on the development workstation. Using the
        # real kernel boundary also exercises its distinct stat/maps devices.
        with tempfile.TemporaryDirectory(dir=Path(__file__).parent) as temporary:
            addon = Path(temporary).resolve() / 'addon.so'
            addon.write_bytes(b'installed addon')
            active = subprocess.CompletedProcess([], 0,
                f'ActiveState=active\nMainPID={os.getpid()}\n', '')
            with addon.open('rb') as stream, mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ), \
                 patch.object(installer.subprocess, 'run', return_value=active):
                self.assertTrue(installer.native_addon_loaded(addon))
                replacement = addon.with_suffix('.new')
                replacement.write_bytes(b'replaced addon')
                replacement.replace(addon)
                self.assertFalse(installer.native_addon_loaded(addon),
                                 'An old deleted mapping cannot verify a new installed file')

    def test_accessibility_preflight_uses_service_interpreter_and_loads_libraries(self):
        with patch.dict(os.environ, {'HYPRLAND_INSTANCE_SIGNATURE': 'fixture'}), \
             patch.object(installer.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, '', '')) as command:
            installer.require_accessibility_runtime()
        argv = command.call_args.args[0]
        self.assertEqual(argv[:3], ['/usr/bin/python3', '-B', '-c'])
        self.assertLess(argv[3].index('ctypes.CDLL'), argv[3].index('from gi.repository'))
        self.assertIn('from gi.repository import Atspi, Gtk, Gtk4LayerShell, GLibUnix', argv[3])
        with patch.dict(os.environ, {'HYPRLAND_INSTANCE_SIGNATURE': ''}), \
             patch.object(installer.subprocess, 'run') as command:
            with self.assertRaisesRegex(RuntimeError, 'current Hyprland'):
                installer.require_accessibility_runtime()
            command.assert_not_called()
        with patch.dict(os.environ, {'HYPRLAND_INSTANCE_SIGNATURE': 'fixture'}), \
             patch.object(installer.subprocess, 'run', side_effect=subprocess.CalledProcessError(1, [])):
            with self.assertRaisesRegex(RuntimeError, 'gtk4-layer-shell'):
                installer.require_accessibility_runtime()

    def test_accessibility_probe_requires_exact_command_and_stable_service(self):
        helper = Path('/installed/accessibility/daemon.py')
        endpoint = Path('/runtime/badi/accessibility.sock')
        active = subprocess.CompletedProcess([], 0, f'ActiveState=active\nMainPID={os.getpid()}\n', '')
        health = installer.accessibility_health_module()
        ready = {'ready': True, 'process_id': os.getpid(), 'protocol_version': 1}
        argv = b'/usr/bin/python3\0-B\0/installed/accessibility/daemon.py\0'
        for final, expected in ((active, ready),
                                (subprocess.CompletedProcess([], 0, 'ActiveState=failed\nMainPID=0\n', ''), None),
                                (subprocess.CompletedProcess([], 0, f'ActiveState=active\nMainPID={os.getpid()+1}\n', ''), None)):
            with self.subTest(final=final.stdout), \
                 patch.object(installer.subprocess, 'run', side_effect=[active, final]), \
                 patch.object(installer.Path, 'open', return_value=io.BytesIO(argv)), \
                 patch.object(installer, 'accessibility_health_module', return_value=health), \
                 patch.object(health, 'probe', return_value=ready) as probe:
                self.assertEqual(installer.probe_accessibility(helper, endpoint), expected)
                probe.assert_called_once_with(endpoint, os.getpid())
        for wrong in (argv.replace(b'/installed/', b'/other/'), argv.replace(b'-B\0', b'')):
            with self.subTest(argv=wrong), \
                 patch.object(installer.subprocess, 'run', return_value=active), \
                 patch.object(installer.Path, 'open', return_value=io.BytesIO(wrong)), \
                 patch.object(installer, 'accessibility_health_module') as load:
                with self.assertRaisesRegex(RuntimeError, 'expected interpreter'):
                    installer.probe_accessibility(helper, endpoint)
                load.assert_not_called()
        for code, retry in (('service_unavailable', True), ('timeout', True), ('unexpected_peer', False), ('unsafe_socket', False)):
            with self.subTest(code=code), \
                 patch.object(installer.subprocess, 'run', return_value=active), \
                 patch.object(installer.Path, 'open', return_value=io.BytesIO(argv)), \
                 patch.object(installer, 'accessibility_health_module', return_value=health), \
                 patch.object(health, 'probe', side_effect=health.ProbeError(code)):
                if retry:
                    self.assertIsNone(installer.probe_accessibility(helper, endpoint))
                else:
                    with self.assertRaisesRegex(RuntimeError, code):
                        installer.probe_accessibility(helper, endpoint)

    def test_accessibility_wait_is_bounded(self):
        with patch.object(installer, 'probe_accessibility', side_effect=[None, {'ready': True}]) as probe, \
             patch.object(installer.time, 'monotonic', return_value=0), \
             patch.object(installer.time, 'sleep') as sleep:
            installer.wait_accessibility(Path('/helper'), Path('/endpoint'))
            self.assertEqual(probe.call_count, 2)
            sleep.assert_called_once_with(.2)
        with patch.object(installer, 'probe_accessibility', return_value=None), \
             patch.object(installer.time, 'monotonic', side_effect=[0, 11]), \
             patch.object(installer.time, 'sleep') as sleep:
            with self.assertRaisesRegex(RuntimeError, 'Fcitx was not restarted'):
                installer.wait_accessibility(Path('/helper'), Path('/endpoint'))
            sleep.assert_not_called()

    def test_full_install_verifies_helper_before_fcitx_and_preserves_backup_and_autostart(self):
        for helper_ready in (True, False):
            with self.subTest(helper_ready=helper_ready), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                home, project, runtime = root / 'home', root / 'project', root / 'runtime'
                native_project(project)
                helper = home / '.local/lib/badi/accessibility/daemon.py'
                helper.parent.mkdir(parents=True)
                helper.write_text('previous helper')
                profile = home / '.config/fcitx5/profile'
                profile.parent.mkdir(parents=True)
                profile.write_text('preserve keyboard profile')
                calls = []
                def execute(command, **kwargs):
                    calls.append(command)
                    output = str(runtime) if command[0] == 'loginctl' else 'loaded' if '--property=LoadState' in command else '{}'
                    return subprocess.CompletedProcess(command, 0, output, '')
                def ready(script, socket_path):
                    self.assertEqual(script, helper)
                    self.assertEqual(socket_path, runtime / 'badi/accessibility.sock')
                    calls.append(['helper-readiness'])
                    if not helper_ready:
                        raise RuntimeError('helper readiness failed')
                with patch.object(installer, 'ROOT', project), \
                     patch.object(installer.Path, 'home', return_value=home), \
                     patch.object(installer.subprocess, 'run', side_effect=execute), \
                     patch.object(installer, 'require_unlocked'), \
                     patch.object(installer, 'require_accessibility_runtime'), \
                     patch.object(installer, 'probe_model', return_value={'provider': 'local_model', 'paused': False, 'control_plane_degraded': False}), \
                     patch.object(installer, 'wait_accessibility', side_effect=ready), \
                     patch.object(installer, 'wait_native_addon') as addon, \
                     patch.dict(os.environ, {'WAYLAND_DISPLAY': 'wayland-test', 'XDG_RUNTIME_DIR': str(runtime), 'XDG_CONFIG_HOME': str(home / '.config')}), \
                     patch('sys.argv', ['install-desktop.py']), \
                     contextlib.redirect_stdout(io.StringIO()) as output:
                    if helper_ready:
                        installer.main()
                    else:
                        with self.assertRaisesRegex(RuntimeError, 'helper readiness failed'):
                            installer.main()
                self.assertIn(['systemctl', '--user', 'reset-failed', 'badi-accessibility.service'], calls)
                self.assertNotIn(['systemctl', '--user', 'enable', 'badi-accessibility.service'], calls)
                restart = ['systemctl', '--user', 'restart', 'omarchy-fcitx5.service']
                if helper_ready:
                    self.assertLess(calls.index(['helper-readiness']), calls.index(restart))
                    self.assertIn('accessibility helper ready', output.getvalue())
                    addon.assert_called_once()
                else:
                    self.assertNotIn(restart, calls)
                    addon.assert_not_called()
                backup, document, changed = only_backup(home)
                self.assertEqual(saved(backup, document, '.local/lib/badi/accessibility/daemon.py').read_text(), 'previous helper')
                self.assertIn('.local/lib/badi/accessibility/health.py', changed)
                self.assertIn('.config/systemd/user/badi-accessibility.service', changed)
                self.assertEqual(profile.read_text(), 'preserve keyboard profile')
                self.assertFalse((home / installer.COMPAT_DROPIN).exists())
                if helper_ready:
                    self.assertIn('is not the pinned 5.1.22', output.getvalue())


class FullInstall:
    def full_install(self, root, argv, *, version='5.1.22', overridden=False, loaded=True, prebuilt=False):
        home, project, runtime = root / 'home', root / 'project', root / 'runtime'
        native_project(project)
        profile = home / '.config/fcitx5/profile'
        profile.parent.mkdir(parents=True, exist_ok=True)
        profile.write_text('preserve keyboard profile')
        launcher, build, _receipt = compat_fixture(root)
        if prebuilt:
            argv = [*argv, '--wayland-compat-build', str(build)]
        calls = []

        def execute(command, **kwargs):
            calls.append(command)
            if command[0] == 'loginctl':
                return subprocess.CompletedProcess(command, 0, str(runtime), '')
            if command == ['/usr/bin/fcitx5', '--version']:
                return subprocess.CompletedProcess(command, 0, version + '\n', '')
            if '--property=ExecStart' in command:
                # Model systemd: the managed drop-in applies after daemon-reload.
                reloaded = ['systemctl', '--user', 'daemon-reload'] in calls
                managed = reloaded and not overridden and (home / installer.COMPAT_DROPIN).exists()
                argv = installer.compat_command(home, '5.1.22') if managed else installer.STOCK_FCITX_COMMAND
                return subprocess.CompletedProcess(command, 0, STOCK % argv, '')
            if any(str(part).endswith('fcitx5-wayland-compat/build.py') for part in command):
                shutil.copytree(build, command[command.index('--work-dir') + 1])
            output = 'loaded' if '--property=LoadState' in command else '{}'
            return subprocess.CompletedProcess(command, 0, output, '')

        with patch.object(installer, 'ROOT', project), \
             patch.object(installer.Path, 'home', return_value=home), \
             patch.object(installer.subprocess, 'run', side_effect=execute), \
             patch.object(installer, 'compat_launcher', return_value=launcher), \
             patch.object(installer, 'require_unlocked'), \
             patch.object(installer, 'require_accessibility_runtime'), \
             patch.object(installer, 'probe_model', return_value={'provider': 'local_model', 'paused': False, 'control_plane_degraded': False}), \
             patch.object(installer, 'wait_accessibility'), \
             patch.object(installer, 'wait_native_addon'), \
             patch.object(installer, 'wait_compat_frontend', return_value=loaded) as frontend, \
             patch.dict(os.environ, {'WAYLAND_DISPLAY': 'wayland-test', 'XDG_RUNTIME_DIR': str(runtime), 'XDG_CONFIG_HOME': str(home / '.config')}), \
             patch('sys.argv', ['install-desktop.py', *argv]), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            try:
                installer.main()
            finally:
                self.output = output.getvalue()
        return home, project, calls, frontend


class WaylandCompatTests(FullInstall, unittest.TestCase):
    def test_full_install_builds_activates_and_retires_the_obsolete_frontend(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            home = root / 'home'
            old = old_compat(home)
            home, project, calls, frontend = self.full_install(root, [])
            build = [call for call in calls if any(str(part).endswith('build.py') for part in call)]
            self.assertEqual(len(build), 1)
            self.assertEqual(build[0][-5:], ['--work-dir', build[0][-4], '--jobs', '2', '--check'])
            self.assertTrue(build[0][-4].startswith(str(project / 'output/extensionless/fcitx-wayland-compat-5.1.22-')))
            self.assertLess(calls.index(['npm', 'run', 'fcitx5:check']), calls.index(build[0]))
            compat = home / '.local/lib/badi/compat/fcitx5-5.1.22'
            self.assertEqual((compat / 'addons/libwaylandim.so').read_bytes(), b'patched frontend')
            self.assertEqual((compat / 'launch.py').read_bytes(),
                             (project / 'packaging/fcitx5-wayland-compat/launch.py').read_bytes())
            self.assertTrue(json.loads((compat / 'build-receipt.json').read_text())['protocol_checks_passed'])
            dropin = (home / installer.COMPAT_DROPIN).read_text()
            self.assertEqual(dropin.splitlines()[-2:], ['ExecStart=',
                'ExecStart=/usr/bin/python3 -B %h/.local/lib/badi/compat/fcitx5-5.1.22/launch.py --disable notificationitem'])
            self.assertFalse(old.exists())
            backup, document, changed = only_backup(home)
            for name in installer.COMPAT_FILES:
                relative = '.local/lib/badi/compat/fcitx5-5.1.21/' + name
                self.assertEqual(saved(backup, document, relative).read_text(), 'retired ' + name)
                self.assertIn(relative, changed)
                self.assertIn('.local/lib/badi/compat/fcitx5-5.1.22/' + name, changed)
            self.assertIn(str(installer.COMPAT_DROPIN), changed)
            recorded = json.loads((home / '.local/state/badi/receipts/desktop.json').read_text())['files']
            self.assertIn('.local/lib/badi/compat/fcitx5-5.1.22/addons/libwaylandim.so', recorded)
            self.assertIn(str(installer.COMPAT_DROPIN), recorded)
            self.assertFalse(any('5.1.21' in path for path in recorded))
            # The managed command must be effective before the one Fcitx restart.
            restart = calls.index(['systemctl', '--user', 'restart', 'omarchy-fcitx5.service'])
            reload = calls.index(['systemctl', '--user', 'daemon-reload'])
            self.assertLess(calls.index(['systemd-analyze', '--user', 'verify', 'omarchy-fcitx5.service']), reload)
            checks = [index for index, call in enumerate(calls) if '--property=ExecStart' in call]
            self.assertTrue(any(reload < index < restart for index in checks))
            frontend.assert_called_once_with(compat / 'addons/libwaylandim.so')
            self.assertIn('Wayland compatibility frontend verified in the running service', self.output)

    def test_overriding_drop_in_stops_before_fcitx_restart(self):
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaisesRegex(RuntimeError, 'Fcitx was not restarted'):
                self.full_install(Path(temporary), [], overridden=True)

    def test_unselected_frontend_is_reported_without_claiming_activation(self):
        with tempfile.TemporaryDirectory() as temporary:
            self.full_install(Path(temporary), [], loaded=False)
            self.assertIn('selected the system frontend', self.output)
            self.assertNotIn('verified in the running service', self.output)

    def test_opt_out_and_other_fcitx_versions_leave_the_frontend_untouched(self):
        for argv, version in ((['--no-wayland-compat'], '5.1.22'), ([], '5.1.23')):
            with self.subTest(argv=argv, version=version), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                old = old_compat(root / 'home')
                home, _project, calls, frontend = self.full_install(root, argv, version=version)
                self.assertFalse(any(any(str(part).endswith('build.py') for part in call) for call in calls))
                self.assertFalse(any('--property=ExecStart' in call for call in calls))
                self.assertNotIn(['systemd-analyze', '--user', 'verify', 'omarchy-fcitx5.service'], calls)
                self.assertEqual(['/usr/bin/fcitx5', '--version'] in calls, not argv)
                self.assertFalse((home / installer.COMPAT_DROPIN).exists())
                self.assertFalse((home / '.local/lib/badi/compat/fcitx5-5.1.22').exists())
                self.assertEqual((old / 'launch.py').read_text(), 'retired launch.py')
                frontend.assert_not_called()
                self.assertIn('left unchanged' if argv else 'not the pinned 5.1.22', self.output)

    def test_prebuilt_directory_is_reused_after_verification(self):
        with tempfile.TemporaryDirectory() as temporary:
            home, _project, calls, _frontend = self.full_install(Path(temporary), [], prebuilt=True)
            self.assertFalse(any(any(str(part).endswith('build.py') for part in call) for call in calls))
            self.assertTrue((home / '.local/lib/badi/compat/fcitx5-5.1.22/addons/libwaylandim.so').exists())

    def test_build_verification_binds_source_runtime_artifact_and_checks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            launcher, build, receipt = compat_fixture(root)
            self.assertEqual(installer.verify_compat_build(build, launcher), build.resolve())
            changes = [
                lambda r: r.update(protocol_checks_passed=False),
                lambda r: r.update(schema='badi.other'),
                lambda r: r['source'].update(patch_sha256='0' * 64),
                lambda r: r['installed_build_versions'].update(Fcitx5Core='5.1.23'),
                lambda r: r['runtime_files_sha256'].popitem(),
                lambda r: r.update(artifact_sha256='0' * 64),
            ]
            for change in changes:
                altered = json.loads(json.dumps(receipt))
                change(altered)
                (build / 'build-receipt.json').write_text(json.dumps(altered))
                with self.assertRaisesRegex(RuntimeError, 'pinned source'):
                    installer.verify_compat_build(build, launcher)
            (build / 'build-receipt.json').write_text(json.dumps(receipt))
            Path(launcher.RUNTIME_FILES[0]).write_bytes(b'upgraded system file')
            with self.assertRaisesRegex(RuntimeError, 'pinned source'):
                installer.verify_compat_build(build, launcher)
            Path(launcher.RUNTIME_FILES[0]).write_bytes(b'runtime 0')
            module = build / 'libwaylandim.so'
            module.rename(build / 'real.so')
            module.symlink_to(build / 'real.so')
            with self.assertRaisesRegex(RuntimeError, 'pinned source'):
                installer.verify_compat_build(build, launcher)
            with self.assertRaisesRegex(RuntimeError, 'pinned source'):
                installer.verify_compat_build(root / 'missing', launcher)

    def test_plan_accepts_only_stock_or_managed_commands_and_keeps_referenced_directories(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary) / 'home'
            old = old_compat(home)
            launcher = SimpleNamespace(VERSION='5.1.22', RUNTIME_FILES=(), digest=None)

            def plan(command, version='5.1.22', build=None):
                replies = {'/usr/bin/fcitx5': subprocess.CompletedProcess([], 0, version + '\n', ''),
                           'systemctl': subprocess.CompletedProcess([], 0, STOCK % command, '')}
                with patch.object(installer, 'compat_launcher', return_value=launcher), \
                     patch.object(installer.Path, 'home', return_value=home), \
                     patch.object(installer.subprocess, 'run', side_effect=lambda argv, **_: replies[argv[0]]):
                    return installer.plan_wayland_compat(home, build)

            result = plan(installer.STOCK_FCITX_COMMAND)
            self.assertTrue(result['install'])
            self.assertEqual(result['obsolete'], [(old, sorted(old / name for name in installer.COMPAT_FILES))])
            self.assertEqual(result['root'], home / '.local/lib/badi/compat/fcitx5-5.1.22')
            for command in ('/usr/bin/fcitx5 --replace', '/usr/bin/python3 -B /elsewhere/launch.py --disable notificationitem',
                            installer.compat_command(home, '5.1.21')):
                with self.assertRaisesRegex(RuntimeError, 'unrecognized command'):
                    plan(command)  # The last one lacks Badi's drop-in file.
            dropin = home / installer.COMPAT_DROPIN
            dropin.parent.mkdir(parents=True)
            dropin.write_text(installer.compat_dropin('5.1.21'))
            result = plan(installer.compat_command(home, '5.1.21'))
            self.assertEqual((result['obsolete'], result['kept']), ([], [old]))
            (old / 'user-note').write_text('unexpected')
            result = plan(installer.STOCK_FCITX_COMMAND)
            self.assertEqual((result['obsolete'], result['kept']), ([], [old]))
            self.assertEqual(plan('ignored', version='5.1.23'), {'install': False, 'version': '5.1.23', 'pinned': '5.1.22'})
            with self.assertRaisesRegex(RuntimeError, 'not the pinned 5.1.22'):
                plan('ignored', version='5.1.23', build=Path(temporary))
            (old / 'user-note').unlink()
            shutil.rmtree(old)
            old.symlink_to(Path(temporary))
            with self.assertRaisesRegex(RuntimeError, 'symlink'):
                plan(installer.STOCK_FCITX_COMMAND)

    def test_compat_flags_are_not_broker_only_side_effects(self):
        for argv in (['--broker-only', '--wayland-compat-build', '/x'],
                     ['--no-wayland-compat', '--wayland-compat-build', '/x']):
            with self.subTest(argv=argv), patch('sys.argv', ['install-desktop.py', *argv]), \
                 patch.object(installer.subprocess, 'run') as command, contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as stopped:
                    installer.main()
                self.assertEqual(stopped.exception.code, 2)
                command.assert_not_called()


class ObservedAppInstallTests(FullInstall, unittest.TestCase):
    def install_observed(self, root, apps):
        argv = ['--no-wayland-compat']
        for app in apps:
            argv += ['--observed-app', app]
        with patch.object(installer, 'enable_accessibility') as enabled:
            home, _project, calls, _frontend = self.full_install(root, argv)
        return home, calls, enabled

    def test_flags_are_backed_up_mode_preserving_and_recorded_in_the_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            config = root / 'home/.config'
            config.mkdir(parents=True)
            chromium = config / 'chromium-flags.conf'
            original = '# mine\n--ozone-platform=wayland\n'
            chromium.write_text(original)
            chromium.chmod(0o644)
            home, _calls, enabled = self.install_observed(root, ['chromium', 'code', 'chromium'])
            flag_lines = '# Badi: renderer accessibility for the focused-field observer\n' + installer.OBSERVED_FLAG + '\n'
            self.assertEqual(chromium.read_text(), original + flag_lines)
            self.assertEqual(chromium.stat().st_mode & 0o777, 0o644)
            code = config / 'code-flags.conf'
            self.assertEqual(code.read_text(), flag_lines)
            self.assertEqual(code.stat().st_mode & 0o777, 0o600)
            backup, document, changed = only_backup(home)
            self.assertEqual(saved(backup, document, '.config/chromium-flags.conf').read_text(), original)
            created, = (entry for entry in document['changes'] if entry['path'] == '.config/code-flags.conf')
            self.assertEqual((created['action'], created.get('saved')), ('create', None), 'a new file has no predecessor')
            self.assertEqual(changed.count('.config/chromium-flags.conf'), 1)
            receipt = json.loads((home / '.local/state/badi/receipts/desktop.json').read_text())
            for path in (chromium, code):
                entry = receipt['files'][str(path.relative_to(home))]
                self.assertEqual(entry['sha256'], hashlib.sha256(path.read_bytes()).hexdigest())
            self.assertEqual(enabled.call_args.args[0].directory, backup)
            enabled.assert_called_once()
            self.assertIn(installer.OBSERVED_FLAG + ' added for chromium, code.', self.output)
            self.assertIn('CPU and memory', self.output)

    def test_already_enabled_flags_are_left_untouched(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            config = root / 'home/.config'
            config.mkdir(parents=True)
            cursor = config / 'cursor-flags.conf'
            cursor.write_text('--force-renderer-accessibility\n')
            cursor.chmod(0o640)
            before = cursor.stat()
            home, _calls, enabled = self.install_observed(root, ['cursor'])
            self.assertEqual(cursor.read_text(), '--force-renderer-accessibility\n')
            self.assertEqual((cursor.stat().st_ino, cursor.stat().st_mode), (before.st_ino, before.st_mode))
            _backup, _document, changed = only_backup(home)
            self.assertNotIn('.config/cursor-flags.conf', changed)
            receipt = json.loads((home / '.local/state/badi/receipts/desktop.json').read_text())
            self.assertNotIn('.config/cursor-flags.conf', receipt['files'])
            enabled.assert_called_once()
            self.assertIn('already enabled for cursor', self.output)
            self.assertNotIn(' added for ', self.output)

    def test_conflicting_accessibility_choice_stops_before_building_or_changing_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            config = root / 'home/.config'
            config.mkdir(parents=True)
            brave = config / 'brave-origin-flags.conf'
            brave.write_text('--force-renderer-accessibility=basic\n')
            with self.assertRaisesRegex(RuntimeError, 'conflicts'):
                self.install_observed(root, ['brave-origin'])
            self.assertEqual(brave.read_text(), '--force-renderer-accessibility=basic\n')
            self.assertFalse((root / 'home/.local/state/badi/install-backups').exists())

    def test_discord_is_not_an_observed_app_choice(self):
        with patch('sys.argv', ['install-desktop.py', '--observed-app', 'discord']), \
             patch.object(installer.subprocess, 'run') as command, contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as stopped:
                installer.main()
        self.assertEqual(stopped.exception.code, 2)
        command.assert_not_called()


if __name__ == '__main__':
    unittest.main()
