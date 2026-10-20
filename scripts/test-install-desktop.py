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
                  'packaging/systemd/badi-accessibility.service', 'packaging/hypr/badi.lua',
                  'packaging/systemd/badi-session.service') + tuple(
                  'adapters/accessibility/' + name for name in installer.OBSERVER_MODULES) + tuple(
                  'adapters/session/' + name for name in installer.SESSION_MODULES)
STOCK = '{ path=/usr/bin/fcitx5 ; argv[]=%s ; ignore_errors=no ; start_time=[n/a] ; pid=0 }\n'
STOCK_COMMAND = '/usr/bin/fcitx5 --disable notificationitem'


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


def retired_frontend(home):
    """What an install from before Fcitx 5.1.23 left: the backport, its drop-in and receipt entries."""
    library = home / installer.RETIRED_COMPAT_LIBRARY
    files = {library / name: f'retired {name}'.encode() for name in installer.RETIRED_COMPAT_FILES}
    files[home / installer.RETIRED_COMPAT_DROPIN] = installer.RETIRED_COMPAT_DROPIN_TEXT.encode()
    for path, data in files.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    receipts.write_receipt(home, 'desktop', {'commit': '0' * 40, 'dirty': False}, list(files))
    return files


def broker_only_install(root):
    home, project = root / 'home', root / 'project'
    native_project(project)

    def execute(command, **kwargs):
        output = str(root / 'runtime') if command[0] == 'loginctl' else 'loaded' if '--property=LoadState' in command else '{}'
        return subprocess.CompletedProcess(command, 0, output, '')

    with patch.object(installer, 'ROOT', project), \
         patch.object(installer.Path, 'home', return_value=home), \
         patch.object(installer.subprocess, 'run', side_effect=execute), \
         patch.object(installer, 'require_writing_assets'), \
         patch.object(installer, 'require_writing_model'), \
         patch.object(installer, 'wait_session_helper', return_value=True), \
         patch.object(installer, 'probe_model', return_value={'provider': 'local_model', 'paused': False, 'control_plane_degraded': False}), \
         patch.dict(os.environ, {'WAYLAND_DISPLAY': 'wayland-test', 'XDG_RUNTIME_DIR': str(root / 'runtime'),
                                 'XDG_CONFIG_HOME': str(home / '.config')}), \
         patch('sys.argv', ['install-desktop.py', '--broker-only']), \
         contextlib.redirect_stdout(io.StringIO()) as output:
        installer.main()
    return output.getvalue()


class DesktopInstallTests(unittest.TestCase):
    def test_missing_or_wrong_runtime_stops_before_cargo(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            with self.assertRaisesRegex(RuntimeError, 'is missing'):
                installer.require_writing_assets(home)
            server = home / '.local/share/badi/runtimes/llama-b10726/llama-server'
            server.parent.mkdir(parents=True)
            server.write_bytes(b'not the runtime')
            with self.assertRaisesRegex(RuntimeError, 'does not match the pinned hash'):
                installer.require_writing_assets(home)

    def test_any_pinned_catalog_model_satisfies_the_install(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            data = b'model bytes'
            catalog = {'recommended': {'name': 'Gemma 3 1B'}, 'catalog': [
                {'filename': 'a.gguf', 'download_bytes': 1, 'sha256': '0' * 64},
                {'filename': 'b.gguf', 'download_bytes': len(data),
                 'sha256': hashlib.sha256(data).hexdigest()}]}
            reply = unittest.mock.Mock(stdout=json.dumps(catalog))
            with patch.object(installer.subprocess, 'run', return_value=reply):
                with self.assertRaisesRegex(RuntimeError, 'prints the pinned download of Gemma 3 1B'):
                    installer.require_writing_model(home, 'badictl')
                models = home / '.local/share/badi/models'
                models.mkdir(parents=True)
                (models / 'b.gguf').write_bytes(data)
                self.assertEqual(installer.require_writing_model(home, 'badictl')['filename'], 'b.gguf')

    def test_changed_flags_abort_before_any_file_is_copied(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            home, project = root / 'home', root / 'project'
            native_project(project)
            flags = home / '.config/chromium-flags.conf'
            flags.parent.mkdir(parents=True)
            flags.write_text('# mine\n')
            plan = installer.Plan(
                args=SimpleNamespace(broker_only=False, observed_app=['chromium']),
                home=home, config_home=home / '.config', runtime=root / 'runtime',
                flag_updates={flags: ('# mine\n', '# mine\n' + installer.OBSERVED_FLAG + '\n')},
                flags_present=[], broker_updating=True, accessibility_enabled=True, retired_compat=[])
            flags.write_text('# mine\n# edited while building\n')
            with patch.object(installer, 'ROOT', project), \
                 self.assertRaisesRegex(RuntimeError, 'changed during the build'):
                installer.install_files(plan)
            self.assertFalse((home / '.local/lib/badi/badi-broker').exists())

    def test_hyprland_layer_rule_is_added_once_and_changed_config_aborts(self):
        original = '-- mine\nrequire("hypr.monitors")\n'
        updated = installer.hyprland_with_badi(original)
        self.assertTrue(updated.endswith(installer.HYPR_REQUIRE + '\n'))
        self.assertIsNone(installer.hyprland_with_badi(updated), 'idempotent')
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            home, project = root / 'home', root / 'project'
            native_project(project)
            hyprland = home / '.config/hypr/hyprland.lua'
            hyprland.parent.mkdir(parents=True)
            hyprland.write_text(original)
            plan = installer.Plan(
                args=SimpleNamespace(broker_only=False, observed_app=[]),
                home=home, config_home=home / '.config', runtime=root / 'runtime',
                flag_updates={}, flags_present=[], broker_updating=True,
                accessibility_enabled=True, retired_compat=[],
                hypr_update=(hyprland, original, updated))
            hyprland.write_text(original + '-- edited while building\n')
            with patch.object(installer, 'ROOT', project), \
                 self.assertRaisesRegex(RuntimeError, 'Hyprland configuration changed'):
                installer.install_files(plan)
            self.assertFalse((home / '.local/lib/badi/badi-broker').exists())

    def test_hyprland_layer_rule_install_has_a_scoped_rollback(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            home, project = root / 'home', root / 'project'
            native_project(project)
            hyprland = home / '.config/hypr/hyprland.lua'
            hyprland.parent.mkdir(parents=True)
            original = '-- user configuration\nrequire("hypr.monitors")\n'
            hyprland.write_text(original)
            hyprland.chmod(0o640)
            updated = installer.hyprland_with_badi(original)
            plan = installer.Plan(
                args=SimpleNamespace(broker_only=False, observed_app=[]),
                home=home, config_home=home / '.config', runtime=root / 'runtime',
                flag_updates={}, flags_present=[], broker_updating=True,
                accessibility_enabled=True, retired_compat=[],
                checkout={'commit': '0' * 40, 'dirty': False},
                hypr_update=(hyprland, original, updated))
            with patch.object(installer, 'ROOT', project), \
                 patch.object(installer, 'xournal_launcher', return_value=None), \
                 contextlib.redirect_stdout(io.StringIO()):
                installation = installer.install_files(plan)
                self.assertEqual(hyprland.read_text(), updated)
                self.assertEqual(hyprland.stat().st_mode & 0o777, 0o640)
                self.assertEqual((hyprland.parent / 'badi.lua').read_bytes(),
                                 (project / 'packaging/hypr/badi.lua').read_bytes())
                self.assertEqual(receipts.restore(installation.directory, only=['.config/hypr']), [])
            self.assertEqual(hyprland.read_text(), original)
            self.assertEqual(hyprland.stat().st_mode & 0o777, 0o640)
            self.assertFalse((hyprland.parent / 'badi.lua').exists())
            self.assertTrue((home / '.local/lib/badi/badi-broker').exists(), 'rollback was scoped to Hyprland')

    def test_a_failed_install_names_the_backup_and_the_restore_command(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            home, project = root / 'home', root / 'project'
            native_project(project)

            def execute(command, **kwargs):
                output = str(root / 'runtime') if command[0] == 'loginctl' else 'loaded' if '--property=LoadState' in command else '{}'
                return subprocess.CompletedProcess(command, 0, output, '')

            with patch.object(installer, 'ROOT', project), \
                 patch.object(installer.Path, 'home', return_value=home), \
                 patch.object(installer.subprocess, 'run', side_effect=execute), \
                 patch.object(installer, 'require_writing_assets'), \
                 patch.object(installer, 'require_writing_model'), \
                 patch.object(installer, 'wait_model', side_effect=RuntimeError('broker did not become ready')), \
                 patch.dict(os.environ, {'WAYLAND_DISPLAY': 'wayland-test', 'XDG_RUNTIME_DIR': str(root / 'runtime'),
                                         'XDG_CONFIG_HOME': str(home / '.config')}), \
                 patch('sys.argv', ['install-desktop.py', '--broker-only']), \
                 contextlib.redirect_stdout(io.StringIO()), \
                 contextlib.redirect_stderr(io.StringIO()) as error:
                with self.assertRaisesRegex(RuntimeError, 'broker did not become ready'):
                    installer.main()
            text = error.getvalue()
            backup, = (home / '.local/state/badi/install-backups').iterdir()
            self.assertIn(str(backup), text)
            self.assertIn(f'python3 scripts/badi_install.py restore {backup}', text)
            self.assertEqual(text.count('Install stopped'), 1)

    def test_a_loaded_but_disabled_accessibility_unit_is_enabled_on_restart(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            runtime = home / 'runtime'
            profile = home / '.config/fcitx5/profile'
            profile.parent.mkdir(parents=True)
            profile.write_text('keep')
            units = home / '.config/systemd/user'
            units.mkdir(parents=True)
            (units / 'badi-broker.service').write_text('[Service]\n')
            (units / 'badi-accessibility.service').write_text('[Service]\n')
            calls = []

            def execute(command, **kwargs):
                calls.append(command)
                return subprocess.CompletedProcess(command, 0, '', '')

            plan = installer.Plan(
                args=SimpleNamespace(broker_only=False, observed_app=[], screen_reader_accessibility=False),
                home=home, config_home=home / '.config', runtime=runtime,
                flag_updates={}, flags_present=[], broker_updating=True,
                accessibility_enabled=False, retired_compat=[])
            installation = receipts.Installation(home, 'desktop')
            with patch.object(installer, 'run', side_effect=execute), \
                 patch.object(installer, 'require_unlocked'), \
                 patch.object(installer, 'wait_model', return_value={'paused': False}), \
                 patch.object(installer, 'wait_accessibility'), \
                 patch.object(installer, 'wait_native_addon'):
                installer.restart(plan, installation)
            self.assertIn(['systemctl', '--user', 'enable', 'badi-accessibility.service'], calls)

    def test_every_observer_module_is_installed(self):
        modules = {path.name for path in (CHECKOUT / 'adapters/accessibility').glob('*.py')}
        self.assertEqual(set(installer.OBSERVER_MODULES), modules)

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
                          'chatgpt': 'codex-flags.conf', 'code': 'code-flags.conf', 'cursor': 'cursor-flags.conf',
                          'grok-bot': 'grok-bot-flags.conf', 'com.anthropic.claude': 'claude-desktop-flags.conf'})
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
        for app in ('chatgpt', 'code', 'grok-bot', 'com.anthropic.claude', 'brave-origin', 'cursor'):
            with self.subTest(app=app):
                result = installer.observed_flags(quoted, app)
                self.assertIn('\n' + installer.OBSERVED_FLAG + '\n', result)
                self.assertIsNone(installer.observed_flags(result, app))
        # Word-splitting wrappers strip text after any # and split one line.
        for app in ('chatgpt', 'code', 'grok-bot', 'com.anthropic.claude'):
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
            with contextlib.redirect_stdout(io.StringIO()) as output:
                installer.enable_accessibility(installation)
            receipt = Path(temporary) / '.local/state/badi/install-backups/notes/accessibility-setting.json'
            self.assertIsNone(installation.directory)
            self.assertIn(str(receipt), output.getvalue(), 'The receipt location is reported')
            self.assertEqual(json.loads(receipt.read_text()), {'bus_enabled': False, 'toolkit_accessibility': False})
            self.assertEqual(receipt.stat().st_mode & 0o777, 0o600)
        self.assertIn('set-property', command.call_args_list[2].args[0])

    def test_screen_reader_opt_in_preserves_rollback_and_rejects_unknown_state(self):
        with tempfile.TemporaryDirectory() as temporary, patch.object(installer, 'require_unlocked'):
            installation = receipts.Installation(Path(temporary), 'desktop')
            receipt = Path(temporary) / '.local/state/badi/install-backups/notes/accessibility-screen-reader.json'

            def replies(values):
                return [subprocess.CompletedProcess([], 0, value + '\n', '') for value in values]

            with patch.object(installer, 'run', side_effect=replies(
                    ['b false', 'false', '', '', 'b true', 'true'])), contextlib.redirect_stdout(io.StringIO()):
                installer.enable_screen_reader_accessibility(installation)
            original = receipt.read_bytes()
            self.assertEqual(json.loads(original), {'screen_reader_enabled': False, 'screen_reader_setting': False})
            self.assertEqual(receipt.stat().st_mode & 0o777, 0o600)
            with patch.object(installer, 'run', side_effect=replies(['b true', 'true'])) as command:
                installer.enable_screen_reader_accessibility(installation)
                self.assertEqual(command.call_count, 2)
            self.assertEqual(receipt.read_bytes(), original)
            with patch.object(installer, 'run', side_effect=replies(['unknown', 'false'])) as command:
                with self.assertRaisesRegex(RuntimeError, 'prior screen-reader'):
                    installer.enable_screen_reader_accessibility(installation)
                self.assertEqual(command.call_count, 2)
            self.assertEqual(receipt.read_bytes(), original)

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
                       'broker/data/writing-lexicon/README.md', 'packaging/systemd/badi-session.service',
                       'adapters/session/daemon.py', 'adapters/session/link.py',
                       'adapters/session/style_key.py')
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
            retired = retired_frontend(home)
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
                 patch.object(installer, 'require_writing_assets'), \
                 patch.object(installer, 'require_writing_model'), \
                 patch.object(installer, 'wait_session_helper', return_value=True), \
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
            self.assertEqual({path: path.read_bytes() for path in retired}, retired,
                             'The retired frontend waits for a complete install, which restarts Fcitx')
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
            kept = {str(path.relative_to(home)) for path in retired}
            self.assertEqual(sorted(receipt['files']), sorted(set(changed) - set(links) | kept))
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
             patch.object(receipts.time, 'monotonic', return_value=0), \
             patch.object(receipts.time, 'sleep') as sleep:
            installer.wait_native_addon(Path('/installed/addon.so'))
            self.assertEqual(probe.call_count, 3)
            self.assertEqual(sleep.call_count, 2)
        with patch.object(installer, 'native_addon_loaded', return_value=False), \
             patch.object(receipts.time, 'monotonic', side_effect=[0, 11]), \
             patch.object(receipts.time, 'sleep') as sleep:
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
             patch.object(receipts.time, 'monotonic', return_value=0), \
             patch.object(receipts.time, 'sleep') as sleep:
            installer.wait_accessibility(Path('/helper'), Path('/endpoint'))
            self.assertEqual(probe.call_count, 2)
            sleep.assert_called_once_with(.2)
        with patch.object(installer, 'probe_accessibility', return_value=None), \
             patch.object(receipts.time, 'monotonic', side_effect=[0, 11]), \
             patch.object(receipts.time, 'sleep') as sleep:
            with self.assertRaisesRegex(RuntimeError, 'Fcitx was not restarted'):
                installer.wait_accessibility(Path('/helper'), Path('/endpoint'))
            sleep.assert_not_called()

    def test_session_helper_follows_the_broker_and_a_disabled_unit_stays_disabled(self):
        for state in ('enabled', 'disabled', ''):
            with self.subTest(state=state), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                home, project = root / 'home', root / 'project'
                native_project(project)
                calls = []

                def execute(command, **kwargs):
                    calls.append(command)
                    if command[0] == 'loginctl':
                        return subprocess.CompletedProcess(command, 0, str(root / 'runtime'), '')
                    if '--property=UnitFileState' in command and 'badi-session.service' in command:
                        return subprocess.CompletedProcess(command, 0, state, '')
                    output = 'loaded' if '--property=LoadState' in command else '{}'
                    return subprocess.CompletedProcess(command, 0, output, '')

                with patch.object(installer, 'ROOT', project), \
                     patch.object(installer.Path, 'home', return_value=home), \
                     patch.object(installer.subprocess, 'run', side_effect=execute), \
                     patch.object(installer, 'require_writing_assets'), \
                     patch.object(installer, 'require_writing_model'), \
                     patch.object(installer, 'wait_session_helper', return_value=True) as helper, \
                     patch.object(installer, 'probe_model', return_value={'provider': 'local_model', 'paused': False, 'control_plane_degraded': False}), \
                     patch.dict(os.environ, {'WAYLAND_DISPLAY': 'wayland-test', 'XDG_RUNTIME_DIR': str(root / 'runtime'),
                                             'XDG_CONFIG_HOME': str(home / '.config')}), \
                     patch('sys.argv', ['install-desktop.py', '--broker-only']), \
                     contextlib.redirect_stdout(io.StringIO()) as output:
                    installer.main()
                for name in installer.SESSION_MODULES:
                    self.assertEqual((home / '.local/lib/badi/session' / name).read_text(), 'new adapters/session/' + name)
                self.assertTrue((home / '.config/systemd/user/badi-session.service').is_file())
                restart = ['systemctl', '--user', 'restart', 'badi-session.service']
                if state == 'disabled':
                    self.assertNotIn(restart, calls)
                    self.assertNotIn(['systemctl', '--user', 'enable', 'badi-session.service'], calls)
                    helper.assert_not_called()
                    self.assertIn('The session helper is disabled', output.getvalue())
                else:
                    broker = calls.index(['systemctl', '--user', 'restart', 'badi-broker.service'])
                    self.assertLess(broker, calls.index(restart), 'the helper reconnects to the new broker')
                    self.assertIn(['systemctl', '--user', 'enable', 'badi-session.service'], calls)
                    helper.assert_called_once()
                    self.assertIn('Session helper reporting', output.getvalue())

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
                    output = str(runtime) if command[0] == 'loginctl' else 'loaded' if '--property=LoadState' in command else 'enabled' if '--property=UnitFileState' in command else '{}'
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
                     patch.object(installer, 'require_writing_assets'), \
                     patch.object(installer, 'require_writing_model'), \
                     patch.object(installer, 'require_unlocked'), \
                     patch.object(installer, 'require_accessibility_runtime'), \
                     patch.object(installer, 'wait_session_helper', return_value=True), \
                     patch.object(installer, 'probe_model', return_value={'provider': 'local_model', 'paused': False, 'control_plane_degraded': False}), \
                     patch.object(installer, 'wait_accessibility', side_effect=ready), \
                     patch.object(installer, 'wait_native_addon') as addon, \
                     patch.dict(os.environ, {'WAYLAND_DISPLAY': 'wayland-test', 'XDG_RUNTIME_DIR': str(runtime), 'XDG_CONFIG_HOME': str(home / '.config')}), \
                     patch('sys.argv', ['install-desktop.py']), \
                     contextlib.redirect_stdout(io.StringIO()) as output, \
                     contextlib.redirect_stderr(io.StringIO()):
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
                self.assertNotIn('retired', output.getvalue())


class FullInstall:
    def full_install(self, root, argv, *, retired=False, overridden=False):
        """A complete install; `retired` leaves an earlier backport in place first,
        and `overridden` adds another drop-in that still starts its launcher."""
        home, project, runtime = root / 'home', root / 'project', root / 'runtime'
        native_project(project)
        profile = home / '.config/fcitx5/profile'
        profile.parent.mkdir(parents=True, exist_ok=True)
        profile.write_text('preserve keyboard profile')
        self.retired = retired_frontend(home) if retired else {}
        retired_command = '/usr/bin/python3 -B %s/.local/lib/badi/compat/fcitx5-5.1.22/launch.py --disable notificationitem' % home
        calls = []

        def execute(command, **kwargs):
            calls.append(command)
            if command[0] == 'loginctl':
                return subprocess.CompletedProcess(command, 0, str(runtime), '')
            if '--property=ExecStart' in command:
                # Model systemd: a removed drop-in stops applying at daemon-reload.
                reloaded = ['systemctl', '--user', 'daemon-reload'] in calls
                dropin = (home / installer.RETIRED_COMPAT_DROPIN).exists() or not reloaded
                argv = retired_command if overridden or (retired and dropin) else STOCK_COMMAND
                return subprocess.CompletedProcess(command, 0, STOCK % argv, '')
            output = 'loaded' if '--property=LoadState' in command else 'enabled' if '--property=UnitFileState' in command else '{}'
            return subprocess.CompletedProcess(command, 0, output, '')

        with patch.object(installer, 'ROOT', project), \
             patch.object(installer, 'require_writing_assets'), \
             patch.object(installer, 'require_writing_model'), \
             patch.object(installer.Path, 'home', return_value=home), \
             patch.object(installer.subprocess, 'run', side_effect=execute), \
             patch.object(installer, 'require_unlocked'), \
             patch.object(installer, 'require_accessibility_runtime'), \
             patch.object(installer, 'wait_session_helper', return_value=True), \
             patch.object(installer, 'probe_model', return_value={'provider': 'local_model', 'paused': False, 'control_plane_degraded': False}), \
             patch.object(installer, 'wait_accessibility'), \
             patch.object(installer, 'wait_native_addon'), \
             patch.dict(os.environ, {'WAYLAND_DISPLAY': 'wayland-test', 'XDG_RUNTIME_DIR': str(runtime), 'XDG_CONFIG_HOME': str(home / '.config')}), \
             patch('sys.argv', ['install-desktop.py', *argv]), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            try:
                installer.main()
            finally:
                self.output = output.getvalue()
        return home, project, calls


class RetiredFrontendTests(FullInstall, unittest.TestCase):
    def test_full_install_removes_the_retired_frontend_with_a_restorable_backup(self):
        with tempfile.TemporaryDirectory() as temporary:
            home, _project, calls = self.full_install(Path(temporary), [], retired=True)
            for path in self.retired:
                self.assertFalse(os.path.lexists(path))
            self.assertFalse((home / installer.RETIRED_COMPAT_LIBRARY).exists(), 'Its emptied directories go too')
            self.assertTrue((home / '.config/systemd/user/omarchy-fcitx5.service.d/50-badi.conf').is_file(),
                            'The addon drop-in beside it stays')
            backup, document, _changed = only_backup(home)
            removed = {entry['path']: entry for entry in document['changes'] if entry['action'] == 'remove'}
            self.assertEqual(set(removed), {str(path.relative_to(home)) for path in self.retired})
            for path, data in self.retired.items():
                self.assertEqual((backup / removed[str(path.relative_to(home))]['saved']).read_bytes(), data)
            recorded = json.loads((home / '.local/state/badi/receipts/desktop.json').read_text())['files']
            self.assertFalse(any('compat' in key for key in recorded), 'The receipt no longer claims removed files')
            self.assertIn('.local/lib/badi/badi-broker', recorded)
            # The stock command must be effective before the one Fcitx restart.
            reload = calls.index(['systemctl', '--user', 'daemon-reload'])
            restart = calls.index(['systemctl', '--user', 'restart', 'omarchy-fcitx5.service'])
            checks = [index for index, call in enumerate(calls) if '--property=ExecStart' in call]
            self.assertTrue(checks and all(reload < index < restart for index in checks))
            self.assertIn('Removed the retired Fcitx 5.1.22 Wayland frontend backport', self.output)

            with patch.object(receipts.subprocess, 'run') as reload_units, contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(receipts.restore(backup, [str(installer.RETIRED_COMPAT_LIBRARY),
                                                           str(installer.RETIRED_COMPAT_DROPIN)]), [])
            self.assertEqual({path: path.read_bytes() for path in self.retired}, self.retired)
            reload_units.assert_called_once()

    def test_install_without_a_retired_frontend_leaves_fcitx_overrides_alone(self):
        with tempfile.TemporaryDirectory() as temporary:
            home, _project, calls = self.full_install(Path(temporary), [])
            _backup, _document, changed = only_backup(home)
            self.assertFalse(any('compat' in path for path in changed))
            self.assertFalse(any('--property=ExecStart' in call for call in calls))
            self.assertFalse(any(call[0] == '/usr/bin/fcitx5' for call in calls))
            self.assertIn(['systemctl', '--user', 'restart', 'omarchy-fcitx5.service'], calls)
            self.assertNotIn('retired', self.output)

    def test_unexpected_content_stops_before_any_change(self):
        def extra_file(home):
            (home / installer.RETIRED_COMPAT_LIBRARY / 'fcitx5-5.1.22/notes.txt').write_text('mine')

        def other_version(home):
            (home / installer.RETIRED_COMPAT_LIBRARY / 'fcitx5-5.1.24').mkdir()

        def edited_dropin(home):
            dropin = home / installer.RETIRED_COMPAT_DROPIN
            dropin.write_text(dropin.read_text() + 'Environment=MINE=1\n')

        def linked_module(home):
            module = home / installer.RETIRED_COMPAT_LIBRARY / 'fcitx5-5.1.22/addons/libwaylandim.so'
            module.unlink()
            module.symlink_to(home / 'elsewhere.so')

        def linked_library(home):
            library = home / installer.RETIRED_COMPAT_LIBRARY
            library.rename(home / 'moved')
            library.symlink_to(home / 'moved')

        for change, message in ((extra_file, 'unexpected files'), (other_version, 'unexpected files'),
                                (edited_dropin, 'not the drop-in'), (linked_module, 'unexpected files'),
                                (linked_library, 'symlink')):
            with self.subTest(change=change.__name__), tempfile.TemporaryDirectory() as temporary:
                home = Path(temporary) / 'home'
                retired_frontend(home)
                change(home)
                with self.assertRaisesRegex(RuntimeError, message):
                    installer.retired_compat_files(home)

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            retired_frontend(root / 'home')
            extra_file(root / 'home')
            with self.assertRaisesRegex(RuntimeError, 'unexpected files'):
                self.full_install(root, [], retired=False)
            self.assertFalse((root / 'home/.local/state/badi/install-backups').exists(), 'No file changed')
            self.assertTrue((root / 'home' / installer.RETIRED_COMPAT_DROPIN).exists())

    def test_another_override_starting_the_retired_launcher_stops_before_fcitx_restart(self):
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaisesRegex(RuntimeError, 'Fcitx was not restarted'), \
                 contextlib.redirect_stderr(io.StringIO()):
                self.full_install(Path(temporary), [], retired=True, overridden=True)

    def test_rejected_arguments_change_nothing(self):
        # The retired frontend's opt-out is gone with it.
        for argv in (['--broker-only', '--screen-reader-accessibility'], ['--no-wayland-compat'],
                     ['--wayland-compat-build', '/x']):
            with self.subTest(argv=argv), patch('sys.argv', ['install-desktop.py', *argv]), \
                 patch.object(installer.subprocess, 'run') as command, contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as stopped:
                    installer.main()
                self.assertEqual(stopped.exception.code, 2)
                command.assert_not_called()


class ObservedAppInstallTests(FullInstall, unittest.TestCase):
    def install_observed(self, root, apps):
        argv = []
        for app in apps:
            argv += ['--observed-app', app]
        with patch.object(installer, 'enable_accessibility') as enabled:
            home, _project, calls = self.full_install(root, argv)
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
