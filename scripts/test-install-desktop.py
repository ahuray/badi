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
import struct
import subprocess
import tempfile
import threading
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
                  'packaging/systemd/badi-session.service', 'packaging/omarchy/badi-repair.hook',
                  'THIRD-PARTY-NOTICES.txt') + tuple(
                  'adapters/accessibility/' + name for name in installer.OBSERVER_MODULES) + tuple(
                  'adapters/session/' + name for name in installer.SESSION_MODULES)


def setUpModule():
    # These tests describe Omarchy unless a test names another desktop; the
    # host running them (a CI runner, a Debian container) is neither.
    platform = patch.object(receipts, 'desktop_platform', return_value=receipts.OMARCHY)
    platform.start()
    unittest.addModuleCleanup(platform.stop)


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


def broker_only_install(root):
    home, project = root / 'home', root / 'project'
    native_project(project)

    def execute(command, **kwargs):
        output = (str(root / 'runtime') if command[0] == 'loginctl'
                  else 'loaded' if '--property=LoadState' in command
                  else 'active' if '--property=ActiveState' in command
                  else 'enabled' if '--property=UnitFileState' in command else '{}')
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
            server = home / '.local/share/badi/runtimes/llama-b11840/llama-server'
            server.parent.mkdir(parents=True)
            server.write_bytes(b'not the runtime')
            with self.assertRaisesRegex(RuntimeError, 'does not match the pinned hash'):
                installer.require_writing_assets(home)

    def test_any_pinned_catalog_model_satisfies_the_install(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            data = b'model bytes'
            catalog = {'recommended': {'id': 'gemma-3-1b', 'name': 'Gemma 3 1B'}, 'catalog': [
                {'id': 'gemma-3-1b', 'name': 'Gemma 3 1B', 'license': 'Gemma', 'fits': True,
                 'filename': 'a.gguf', 'download_bytes': 1, 'sha256': '0' * 64},
                {'id': 'qwen3-1.7b', 'name': 'Qwen3 1.7B', 'license': 'Apache-2.0', 'fits': True,
                 'filename': 'b.gguf', 'download_bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}]}
            reply = unittest.mock.Mock(stdout=json.dumps(catalog))
            with patch.object(installer.subprocess, 'run', return_value=reply), \
                 patch.object(installer.sys.stdin, 'isatty', return_value=False), \
                 patch.object(installer.badi_install, 'download_model') as download:
                with self.assertRaisesRegex(RuntimeError, 'Gemma Terms of Use.*--model gemma-3-1b --accept-model-license'):
                    installer.require_writing_model(home, 'badictl')
                with self.assertRaisesRegex(RuntimeError, '--accept-model-license'):
                    installer.require_writing_model(home, 'badictl', 'gemma-3-1b')
                download.assert_not_called()
                models = home / '.local/share/badi/models'
                models.mkdir(parents=True)
                (models / 'b.gguf').write_bytes(data)
                self.assertEqual(installer.require_writing_model(home, 'badictl')['filename'], 'b.gguf')
                download.assert_not_called()

    def test_the_first_model_downloads_only_after_its_terms_are_accepted(self):
        catalog = {'recommended': {'id': 'gemma-3-1b'}, 'catalog': [
            {'id': 'gemma-3-1b', 'name': 'Gemma 3 1B', 'license': 'Gemma', 'fits': True,
             'filename': 'a.gguf', 'download_bytes': 7, 'sha256': '0' * 64},
            {'id': 'qwen3-1.7b', 'name': 'Qwen3 1.7B', 'license': 'Apache-2.0', 'fits': True,
             'filename': 'b.gguf', 'download_bytes': 7, 'sha256': '1' * 64},
            {'id': 'too-big', 'name': 'Too big', 'license': 'Apache-2.0', 'fits': False,
             'filename': 'c.gguf', 'download_bytes': 7, 'sha256': '2' * 64}]}
        reply = unittest.mock.Mock(stdout=json.dumps(catalog))
        record = Path('.local/state/badi/model-terms.json')
        # On a terminal: Enter picks the recommended model, then its terms
        # are asked; a number picks another; n or q stops.
        for model_id, accept, tty, answers, downloaded in (
                ('gemma-3-1b', True, False, [], 'gemma-3-1b'),
                ('qwen3-1.7b', False, False, [], 'qwen3-1.7b'),
                (None, False, True, ['', 'y'], 'gemma-3-1b'),
                (None, False, True, ['2'], 'qwen3-1.7b'),
                (None, False, True, ['1', 'n', 'q'], None),
                (None, False, True, ['n'], None),
                ('gemma-3-1b', False, True, ['y'], 'gemma-3-1b'),
                ('too-big', True, False, [], None)):
            with self.subTest(model=model_id, tty=tty, answers=answers), tempfile.TemporaryDirectory() as temporary, \
                 patch.object(installer.subprocess, 'run', return_value=reply), \
                 patch.object(installer.sys.stdin, 'isatty', return_value=tty), \
                 patch('builtins.input', side_effect=answers) as asked, \
                 patch('builtins.print'), \
                 patch.object(installer.badi_install, 'download_model') as download:
                home = Path(temporary)
                if downloaded is None:
                    with self.assertRaises(RuntimeError):
                        installer.require_writing_model(home, 'badictl', model_id, accept)
                    download.assert_not_called()
                    self.assertFalse((home / record).exists())
                    continue
                self.assertEqual(installer.require_writing_model(home, 'badictl', model_id, accept)['id'], downloaded)
                self.assertEqual(download.call_args.args[0]['id'], downloaded)
                self.assertEqual(download.call_args.args[1], home / '.local/share/badi/models')
                self.assertEqual(asked.call_count, len(answers))
                if downloaded == 'gemma-3-1b':
                    accepted, = json.loads((home / record).read_text())['accepted']
                    self.assertEqual((accepted['model'], accepted['terms']), ('gemma-3-1b', 'https://ai.google.dev/gemma/terms'))
                else:
                    self.assertFalse((home / record).exists(), 'an Apache-2.0 model records no terms')

    def test_a_new_user_picks_among_the_models_that_fit(self):
        catalog = {'recommended': {'id': 'gemma-3-1b'}, 'catalog': [
            {'id': 'qwen3-0.6b', 'name': 'Qwen3 0.6B', 'license': 'Apache-2.0', 'fits': True,
             'filename': 'q.gguf', 'download_bytes': 639_446_688, 'sha256': '0' * 64, 'screen_useful': None},
            {'id': 'qwen3-1.7b', 'name': 'Qwen3 1.7B', 'license': 'Apache-2.0', 'fits': True,
             'filename': 'b.gguf', 'download_bytes': 1_282_439_264, 'sha256': '1' * 64, 'screen_useful': 33},
            {'id': 'too-big', 'name': 'Too big', 'license': 'Apache-2.0', 'fits': False,
             'filename': 'c.gguf', 'download_bytes': 7, 'sha256': '2' * 64, 'screen_useful': 60},
            {'id': 'llama-3.2-1b', 'name': 'Llama 3.2 1B', 'license': 'Llama-3.2-Community', 'fits': True,
             'filename': 'l.gguf', 'download_bytes': 807_690_208, 'sha256': '3' * 64, 'screen_useful': 41},
            {'id': 'gemma-3-1b', 'name': 'Gemma 3 1B', 'license': 'Gemma', 'fits': True,
             'filename': 'a.gguf', 'download_bytes': 806_056_736, 'sha256': '4' * 64, 'screen_useful': 46}]}
        self.assertEqual([model['id'] for model in installer.writing_model_choices(catalog)],
                         ['gemma-3-1b', 'llama-3.2-1b', 'qwen3-1.7b', 'qwen3-0.6b'],
                         'recommended first, then by screen result, unscreened last; too large left out')
        reply = unittest.mock.Mock(stdout=json.dumps(catalog))
        output = io.StringIO()
        with tempfile.TemporaryDirectory() as temporary, \
             patch.object(installer.subprocess, 'run', return_value=reply), \
             patch.object(installer.sys.stdin, 'isatty', return_value=True), \
             patch('builtins.input', side_effect=['7', '3']) as asked, \
             contextlib.redirect_stdout(output), \
             patch.object(installer.badi_install, 'download_model') as download:
            self.assertEqual(installer.require_writing_model(Path(temporary), 'badictl')['id'], 'qwen3-1.7b')
        listing = output.getvalue()
        self.assertIn('1  Gemma 3 1B    0.81 GB  46 of 135 useful   Gemma Terms of Use  recommended', listing)
        self.assertIn('3  Qwen3 1.7B    1.28 GB  33 of 135 useful   Apache-2.0', listing)
        self.assertIn('4  Qwen3 0.6B    0.64 GB  not screened       Apache-2.0', listing)
        self.assertNotIn('Too big', listing)
        self.assertEqual(asked.call_count, 2, 'a number out of range asks again; an Apache-2.0 model asks no terms')
        self.assertEqual(download.call_args.args[0]['id'], 'qwen3-1.7b')
        # Without a terminal the error names every model that fits.
        with tempfile.TemporaryDirectory() as temporary, \
             patch.object(installer.subprocess, 'run', return_value=reply), \
             patch.object(installer.sys.stdin, 'isatty', return_value=False), \
             self.assertRaisesRegex(RuntimeError, r'--model gemma-3-1b --accept-model-license to download it, or with '
                                    r'another that fits: llama-3.2-1b \(Llama 3.2 Community License\), '
                                    r'qwen3-1.7b \(Apache-2.0\), qwen3-0.6b \(Apache-2.0\)\.'):
            installer.require_writing_model(Path(temporary), 'badictl')

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
                flags_present=[], broker_updating=True, accessibility_enabled=True)
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
                accessibility_enabled=True,
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
                accessibility_enabled=True,
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
                output = (str(root / 'runtime') if command[0] == 'loginctl'
                          else 'loaded' if '--property=LoadState' in command
                          else 'active' if '--property=ActiveState' in command
                          else 'enabled' if '--property=UnitFileState' in command else '{}')
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
                args=SimpleNamespace(broker_only=False, observed_app=[]),
                home=home, config_home=home / '.config', runtime=runtime,
                flag_updates={}, flags_present=[], broker_updating=True,
                accessibility_enabled=False)
            installation = receipts.Installation(home, 'desktop')
            with patch.object(installer, 'run', side_effect=execute), \
                 patch.object(installer, 'require_unlocked'), \
                 patch.object(installer, 'wait_model', return_value={'paused': False}), \
                 patch.object(installer, 'wait_accessibility'), \
                 patch.object(installer, 'wait_native_addon'):
                installer.restart(plan, installation)
            self.assertIn(['systemctl', '--user', 'enable', 'badi-accessibility.service'], calls)

    def test_a_fresh_fcitx_without_a_profile_does_not_stop_the_restart(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            units = home / '.config/systemd/user'
            units.mkdir(parents=True)
            (units / 'badi-broker.service').write_text('[Service]\n')
            (units / 'badi-accessibility.service').write_text('[Service]\n')
            profile = home / '.config/fcitx5/profile'

            def execute(command, **kwargs):
                if command[-1] == 'omarchy-fcitx5.service' and 'restart' in command:
                    profile.parent.mkdir(parents=True, exist_ok=True)
                    profile.write_text('written by Fcitx on first start')
                return subprocess.CompletedProcess(command, 0, '', '')

            plan = installer.Plan(
                args=SimpleNamespace(broker_only=False, observed_app=[]),
                home=home, config_home=home / '.config', runtime=home / 'runtime',
                flag_updates={}, flags_present=[], broker_updating=True,
                accessibility_enabled=True)
            with patch.object(installer, 'run', side_effect=execute), \
                 patch.object(installer, 'require_unlocked'), \
                 patch.object(installer, 'wait_model', return_value={'paused': False}), \
                 patch.object(installer, 'wait_accessibility'), \
                 patch.object(installer, 'wait_native_addon'):
                installer.restart(plan, receipts.Installation(home, 'desktop'))
            self.assertEqual(profile.read_text(), 'written by Fcitx on first start')

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
        self.assertEqual(installer.OBSERVED_LAUNCHERS, {'discord': 'discord.desktop'})

    def test_observed_launcher_puts_the_switch_after_each_program(self):
        packaged = ('[Desktop Entry]\nName=Discord\nExec=/usr/bin/discord --url -- %u\nPath=/usr/bin\n'
                    '[Desktop Action New]\nExec=env A=1 B=2 /usr/bin/discord --new\n')
        updated = installer.observed_launcher(packaged)
        self.assertEqual(updated, packaged.replace(
            'Exec=/usr/bin/discord --url', f'Exec=/usr/bin/discord {installer.OBSERVED_FLAG} --url').replace(
            'B=2 /usr/bin/discord --new', f'B=2 /usr/bin/discord {installer.OBSERVED_FLAG} --new'))
        self.assertIsNone(installer.observed_launcher(updated), 'a second run changes nothing')
        self.assertIsNone(installer.observed_launcher('Exec=discord --force-renderer-accessibility\n'))
        self.assertEqual(installer.observed_launcher('Exec=discord'), f'Exec=discord {installer.OBSERVED_FLAG}')
        for line in ('Exec=discord --force-renderer-accessibility=basic', 'Exec=discord --disable-renderer-accessibility'):
            with self.subTest(line=line), self.assertRaisesRegex(RuntimeError, 'conflicts'):
                installer.observed_launcher(line)
        for line in ('Exec="/opt/Discord App/Discord" --url', 'Exec=env A=1', 'Exec='):
            with self.subTest(line=line), self.assertRaisesRegex(RuntimeError, 'missing or quoted'):
                installer.observed_launcher(line)

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
                       'adapters/session/style_key.py', 'THIRD-PARTY-NOTICES.txt')
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
                output = (str(root / 'runtime') if command[0] == 'loginctl'
                          else 'loaded' if '--property=LoadState' in command
                          else 'active' if '--property=ActiveState' in command
                          else 'enabled' if '--property=UnitFileState' in command else '{}')
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

    def test_fcitx_older_than_5_1_23_or_unreadable_stops_the_install(self):
        self.assertEqual(installer.fcitx_version('5.1.23\n'), (5, 1, 23))
        self.assertEqual(installer.fcitx_version('5.2.0-git\n'), (5, 2, 0))
        self.assertIsNone(installer.fcitx_version('fcitx5 version unknown'))
        for output, error in (('5.1.23\n', None), ('6.0.1\n', None),
                              ('5.1.22\n', 'needs Fcitx 5.1.23 or later; this desktop has 5.1.22'),
                              ('garbled', 'Could not read the Fcitx version')):
            with self.subTest(output=output), \
                 patch.object(installer.subprocess, 'run',
                              return_value=subprocess.CompletedProcess(['fcitx5'], 0, output, '')):
                if error is None:
                    installer.require_fcitx_version()
                else:
                    with self.assertRaisesRegex(RuntimeError, error):
                        installer.require_fcitx_version()
        with patch.object(installer.subprocess, 'run', side_effect=FileNotFoundError('fcitx5')), \
             self.assertRaisesRegex(RuntimeError, 'Could not read the Fcitx version'):
            installer.require_fcitx_version()

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
                    output = ('loaded' if '--property=LoadState' in command
                              else 'active' if '--property=ActiveState' in command
                              else 'enabled' if '--property=UnitFileState' in command else '{}')
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
                    output = (str(runtime) if command[0] == 'loginctl'
                              else 'loaded' if '--property=LoadState' in command
                              else 'active' if '--property=ActiveState' in command
                              else 'enabled' if '--property=UnitFileState' in command else '{}')
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
                     patch.object(installer, 'require_fcitx_version'), \
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


class FullInstall:
    def full_install(self, root, argv):
        """A complete install in a disposable home."""
        home, project, runtime = root / 'home', root / 'project', root / 'runtime'
        native_project(project)
        profile = home / '.config/fcitx5/profile'
        profile.parent.mkdir(parents=True, exist_ok=True)
        profile.write_text('preserve keyboard profile')
        calls = []

        def execute(command, **kwargs):
            calls.append(command)
            if command[0] == 'loginctl':
                return subprocess.CompletedProcess(command, 0, str(runtime), '')
            output = ('loaded' if '--property=LoadState' in command
                      else 'active' if '--property=ActiveState' in command
                      else 'enabled' if '--property=UnitFileState' in command else '{}')
            return subprocess.CompletedProcess(command, 0, output, '')

        with patch.object(installer, 'ROOT', project), \
             patch.object(installer, 'require_writing_assets'), \
             patch.object(installer, 'require_writing_model'), \
             patch.object(installer.Path, 'home', return_value=home), \
             patch.object(installer.subprocess, 'run', side_effect=execute), \
             patch.object(installer, 'require_unlocked'), \
             patch.object(installer, 'require_accessibility_runtime'), \
             patch.object(installer, 'require_fcitx_version'), \
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


class FullInstallTests(FullInstall, unittest.TestCase):
    def test_a_full_install_restarts_fcitx_once(self):
        with tempfile.TemporaryDirectory() as temporary:
            home, _project, calls = self.full_install(Path(temporary), [])
            self.assertEqual(calls.count(['systemctl', '--user', 'restart', 'omarchy-fcitx5.service']), 1)
            self.assertTrue((home / '.config/systemd/user/omarchy-fcitx5.service.d/50-badi.conf').is_file())

    def test_omarchy_hooks_get_the_repair_only_where_omarchy_keeps_hooks(self):
        for omarchy in (True, False):
            with self.subTest(omarchy=omarchy), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                hooks = root / 'home/.config/omarchy/hooks'
                if omarchy:
                    for name in installer.OMARCHY_REPAIR_HOOKS:
                        (hooks / name).mkdir(parents=True)
                home, _project, _calls = self.full_install(root, [])
                for name in installer.OMARCHY_REPAIR_HOOKS:
                    hook = hooks / name / 'badi-repair'
                    self.assertEqual(hook.exists(), omarchy)
                    if omarchy:
                        self.assertEqual(hook.read_text(), 'new packaging/omarchy/badi-repair.hook')
                if not omarchy:
                    self.assertFalse((home / '.config/omarchy').exists(), 'no Omarchy, no hook folder')

    def test_rejected_arguments_change_nothing(self):
        for argv in (['--broker-only', '--observed-app', 'discord'], ['--no-such-option']):
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

    def test_discord_gets_a_backed_up_user_launcher_with_the_switch(self):
        packaged_text = '[Desktop Entry]\nName=Discord\nExec=/usr/bin/discord --url -- %u\n'
        for mine in (None, '[Desktop Entry]\nName=My Discord\nExec=/usr/bin/discord --url -- %u\n'):
            with self.subTest(mine=mine is not None), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                system = root / 'system'
                system.mkdir()
                (system / 'discord.desktop').write_text(packaged_text)
                launcher = root / 'home/.local/share/applications/discord.desktop'
                if mine is not None:
                    launcher.parent.mkdir(parents=True)
                    launcher.write_text(mine)
                with patch.object(installer, 'SYSTEM_APPLICATIONS', system):
                    home, _calls, enabled = self.install_observed(root, ['discord'])
                base = mine if mine is not None else packaged_text
                self.assertEqual(launcher.read_text(), base.replace('--url', installer.OBSERVED_FLAG + ' --url'))
                self.assertEqual((system / 'discord.desktop').read_text(), packaged_text, 'the package is unchanged')
                backup, document, _changed = only_backup(home)
                entry, = (e for e in document['changes'] if e['path'] == '.local/share/applications/discord.desktop')
                if mine is None:
                    self.assertEqual(entry['action'], 'create')
                else:
                    self.assertEqual(saved(backup, document, '.local/share/applications/discord.desktop').read_text(),
                                     mine)
                enabled.assert_called_once()
                self.assertIn(installer.OBSERVED_FLAG + ' added for discord.', self.output)
                with patch.object(installer, 'SYSTEM_APPLICATIONS', system):
                    self.install_observed(root, ['discord'])
                self.assertIn('already enabled for discord', self.output)

    def test_discord_without_a_launcher_stops_before_changing_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with patch.object(installer, 'SYSTEM_APPLICATIONS', root / 'system'), \
                    self.assertRaisesRegex(RuntimeError, 'No discord.desktop launcher'):
                self.install_observed(root, ['discord'])
            self.assertFalse((root / 'home/.local/share/applications').exists())
            self.assertFalse((root / 'home/.local/state/badi/install-backups').exists())


SYSTEM_ADDONS = Path('/usr/lib/x86_64-linux-gnu/fcitx5')


class GenericInstallTests(unittest.TestCase):
    """A desktop without Omarchy: GNOME or KDE Plasma on Debian or Ubuntu."""

    def generic_install(self, root, argv=(), environment=None, fcitx_process=0, mapped=False, wayland_v2=False):
        home, project, runtime = root / 'home', root / 'project', root / 'runtime'
        native_project(project)
        profile = home / '.config/fcitx5/profile'
        profile.parent.mkdir(parents=True, exist_ok=True)
        profile.write_text('preserve keyboard profile')
        calls = []

        def execute(command, **kwargs):
            calls.append(command)
            if command[0] == 'loginctl':
                return subprocess.CompletedProcess(command, 0, str(runtime), '')
            if command[:2] == ['fcitx5', '--version']:
                return subprocess.CompletedProcess(command, 0, '5.1.12\n', '')
            output = ('loaded' if '--property=LoadState' in command
                      else 'active' if '--property=ActiveState' in command
                      else 'disabled' if '--property=UnitFileState' in command and 'badi-accessibility.service' in command
                      else 'enabled' if '--property=UnitFileState' in command else '{}')
            return subprocess.CompletedProcess(command, 0, output, '')

        with patch.object(receipts, 'desktop_platform', return_value=receipts.GENERIC), \
             patch.object(receipts, 'fcitx_addon_directory', return_value=SYSTEM_ADDONS), \
             patch.object(receipts, 'fcitx_bus_process', return_value=fcitx_process), \
             patch.object(installer, 'process_maps_file', return_value=mapped), \
             patch.object(installer, 'offers_input_method_v2', return_value=wayland_v2), \
             patch.object(installer, 'ROOT', project), \
             patch.object(installer, 'require_writing_assets'), \
             patch.object(installer, 'require_writing_model'), \
             patch.object(installer, 'require_accessibility_runtime') as runtime_check, \
             patch.object(installer.Path, 'home', return_value=home), \
             patch.object(installer.subprocess, 'run', side_effect=execute), \
             patch.object(installer, 'wait_session_helper', return_value=True), \
             patch.object(installer, 'probe_model', return_value={'provider': 'local_model', 'paused': False, 'control_plane_degraded': False}), \
             patch.object(installer, 'wait_accessibility') as observer, \
             patch.object(installer, 'wait_native_addon') as omarchy_addon, \
             patch.dict(os.environ, {'WAYLAND_DISPLAY': 'wayland-test', 'XDG_RUNTIME_DIR': str(runtime),
                                     'XDG_CONFIG_HOME': str(home / '.config'), 'HYPRLAND_INSTANCE_SIGNATURE': '',
                                     **(environment or {})}), \
             patch('sys.argv', ['install-desktop.py', *argv]), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            try:
                installer.main()
            finally:
                self.output = output.getvalue()
        omarchy_addon.assert_not_called()
        self.observer_waited = observer.called
        self.runtime_checked = runtime_check.called
        return home, calls

    def test_a_generic_install_leaves_omarchy_out_and_never_restarts_fcitx(self):
        with tempfile.TemporaryDirectory() as temporary:
            home, calls = self.generic_install(Path(temporary), fcitx_process=4242)
            flat = [' '.join(map(str, call)) for call in calls]
            self.assertFalse([call for call in flat if 'omarchy' in call or call.startswith('npm')], flat)
            self.assertFalse([call for call in flat if call.startswith('systemctl') and 'fcitx' in call])
            self.assertIn(['ctest', '--test-dir', str(Path(temporary) / 'project/adapters/fcitx5/build'),
                           '--output-on-failure'], calls)
            environment = home / '.config/environment.d/50-badi-fcitx5.conf'
            self.assertEqual(environment.read_text().splitlines()[1:], [
                f'FCITX_ADDON_DIRS={home}/.local/lib/fcitx5:{SYSTEM_ADDONS}',
                f'FCITX_DATA_DIRS={home}/.local/share/fcitx5:/usr/local/share/fcitx5:/usr/share/fcitx5'])
            self.assertEqual(environment.stat().st_mode & 0o777, 0o644)
            for path in ('.local/lib/fcitx5/libbadi-fcitx5.so', '.local/share/fcitx5/addon/badi.conf',
                         '.local/lib/badi/accessibility/daemon.py', '.config/systemd/user/badi-accessibility.service'):
                self.assertTrue((home / path).is_file(), path)
            for path in ('.config/hypr', '.config/omarchy', '.config/systemd/user/omarchy-fcitx5.service.d',
                         '.local/share/applications/io.github.ahuray.badi.desktop'):
                self.assertFalse((home / path).exists(), path)
            self.assertEqual((home / '.config/fcitx5/profile').read_text(), 'preserve keyboard profile')
            # The observer is enabled for a later Hyprland session but not started here.
            self.assertIn(['systemctl', '--user', 'enable', 'badi-accessibility.service'], calls)
            self.assertNotIn(['systemctl', '--user', 'restart', 'badi-accessibility.service'], calls)
            self.assertFalse(self.observer_waited or self.runtime_checked)
            self.assertIn('next login', self.output)
            self.assertIn('Fcitx was not restarted', self.output)
            self.assertIn('runs only in a Hyprland session', self.output)
            _backup, _document, changed = only_backup(home)
            self.assertIn('.config/environment.d/50-badi-fcitx5.conf', changed, 'the file has a scoped rollback')

    def test_without_a_running_fcitx_the_report_names_the_distribution_way_to_start_it(self):
        with tempfile.TemporaryDirectory() as temporary:
            self.generic_install(Path(temporary), fcitx_process=0)
            self.assertIn('im-config -n fcitx5', self.output)
            self.assertNotIn('Fcitx was not restarted', self.output)

    def test_a_running_fcitx_that_maps_this_build_needs_no_login(self):
        with tempfile.TemporaryDirectory() as temporary:
            self.generic_install(Path(temporary), fcitx_process=4242, mapped=True)
            self.assertIn('already uses this addon build', self.output)
            self.assertNotIn('next login', self.output)

    def test_a_generic_hyprland_session_starts_and_verifies_the_observer(self):
        with tempfile.TemporaryDirectory() as temporary:
            _home, calls = self.generic_install(Path(temporary), environment={'HYPRLAND_INSTANCE_SIGNATURE': 'fixture'})
            self.assertIn(['systemctl', '--user', 'restart', 'badi-accessibility.service'], calls)
            self.assertTrue(self.observer_waited and self.runtime_checked)
            self.assertNotIn('runs only in a Hyprland session', self.output)
            self.assertIn('hl.layer_rule({ name = "badi-preview-noanim"', self.output)
            self.assertIn('layerrule = noanim, badi-writing-preview', self.output)

    def test_broker_enable_recovers_an_interrupted_install_but_preserves_autostart_off(self):
        cases = (
            (False, False, True, 'an interrupted first install is enabled'),
            (False, True, False, 'a completed install preserves badi autostart off'),
            (True, False, False, 'an enabled unit stays enabled'),
            (True, True, False, 'an enabled completed install is untouched'))
        for enabled, installed_before, expected, message in cases:
            with self.subTest(enabled=enabled, installed_before=installed_before):
                plan = SimpleNamespace(broker_enabled=enabled, broker_installed_before=installed_before)
                self.assertIs(installer.broker_needs_enable(plan), expected, message)

    def test_an_inactive_graphical_session_target_is_reported_with_recovery_steps(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            plan = installer.Plan(
                args=SimpleNamespace(broker_only=False, observed_app=[]),
                home=home, config_home=home / '.config', runtime=home / 'runtime',
                flag_updates={}, flags_present=[], broker_updating=True,
                accessibility_enabled=True, graphical_session=False)
            with contextlib.redirect_stdout(io.StringIO()) as output:
                installer.report(plan, {'paused': False})
        self.assertIn('graphical-session.target is not active', output.getvalue())
        self.assertIn('uwsm start hyprland.desktop', output.getvalue())
        self.assertIn('after the next login it starts only once', output.getvalue())

    def test_desktop_wide_accessibility_options_are_refused_before_any_change(self):
        for argv in (['--observed-app', 'chromium'], ['--observed-app', 'discord']):
            with self.subTest(argv=argv), tempfile.TemporaryDirectory() as temporary:
                with self.assertRaisesRegex(RuntimeError, 'not available'):
                    self.generic_install(Path(temporary), argv)
                self.assertFalse((Path(temporary) / 'home/.local').exists())

    def test_the_5_1_23_floor_applies_only_where_the_wayland_v2_frontend_serves_apps(self):
        with tempfile.TemporaryDirectory() as temporary:
            self.generic_install(Path(temporary), wayland_v2=False)
            self.assertIn('requirement of the wayland_v2 frontend does not apply', self.output)
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaisesRegex(RuntimeError, 'needs Fcitx 5.1.23 or later; this desktop has 5.1.12.*'
                                                      'input-method-v2.*does not apply'):
                self.generic_install(Path(temporary), wayland_v2=True)
            self.assertFalse((Path(temporary) / 'home/.local').exists())
        with patch.object(installer.subprocess, 'run',
                          return_value=subprocess.CompletedProcess(['fcitx5'], 0, '5.1.7\n', '')):
            self.assertEqual(installer.require_fcitx_version(False, receipts.GENERIC), (5, 1, 7))
            with self.assertRaisesRegex(RuntimeError, r'omarchy-update'):
                installer.require_fcitx_version()

    def test_omarchy_always_has_the_wayland_v2_floor_and_others_ask_the_compositor(self):
        with patch.object(installer, 'wayland_globals') as globals_:
            self.assertTrue(installer.offers_input_method_v2(receipts.OMARCHY))
            globals_.assert_not_called()
            for names, expected in (({'wl_compositor', installer.INPUT_METHOD_V2}, True),
                                    ({'wl_compositor', 'zwp_input_method_v1'}, False), (None, True)):
                globals_.return_value = names
                self.assertEqual(installer.offers_input_method_v2(receipts.GENERIC), expected, names)


class WaylandRegistryTests(unittest.TestCase):
    @staticmethod
    def event(sender, opcode, body=b''):
        return struct.pack('=II', sender, (8 + len(body)) << 16 | opcode) + body

    @classmethod
    def global_(cls, name, interface, version):
        text = interface.encode() + b'\0'
        text += b'\0' * (-len(text) % 4)
        return cls.event(2, 0, struct.pack('=II', name, len(interface) + 1) + text + struct.pack('=I', version))

    def compositor(self, *events):
        """A one-shot compositor socket; returns its environment and the request it read."""
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        path = Path(directory.name) / 'wayland-test'
        server = socket.socket(socket.AF_UNIX)
        server.bind(str(path))
        server.listen(1)
        received = []

        def serve():
            with server, server.accept()[0] as connection:
                received.append(connection.recv(24))
                for event in events:
                    connection.sendall(event)

        thread = threading.Thread(target=serve, daemon=True)
        thread.start()
        self.addCleanup(thread.join, 5)
        return {'WAYLAND_DISPLAY': 'wayland-test', 'XDG_RUNTIME_DIR': directory.name}, received

    def test_globals_are_read_until_the_sync_is_done(self):
        events = [self.global_(1, 'wl_compositor', 6), self.event(1, 1, struct.pack('=I', 9)),
                  self.global_(7, installer.INPUT_METHOD_V2, 1), self.event(3, 0, struct.pack('=I', 0)),
                  self.global_(8, 'after_the_sync', 1)]
        # A message split across reads is reassembled.
        stream = b''.join(events)
        environment, received = self.compositor(stream[:13], stream[13:])
        self.assertEqual(installer.wayland_globals(environment), {'wl_compositor', installer.INPUT_METHOD_V2})
        self.assertEqual(received, [struct.pack('=IIIIII', 1, 12 << 16 | 1, 2, 1, 12 << 16, 3)])

    def test_an_error_a_hangup_or_no_display_is_unknown(self):
        environment, _received = self.compositor(self.event(1, 0, struct.pack('=III', 1, 0, 0)))
        self.assertIsNone(installer.wayland_globals(environment))
        environment, _received = self.compositor(self.global_(1, 'wl_compositor', 6))
        self.assertIsNone(installer.wayland_globals(environment))
        self.assertIsNone(installer.wayland_globals({'XDG_RUNTIME_DIR': '/nonexistent'}))
        self.assertIsNone(installer.wayland_globals({'WAYLAND_DISPLAY': '/nonexistent/wayland-0'}))


class PayloadListTests(unittest.TestCase):
    """The installers and the Arch package list the same files, and the lists
    name every module the tree has; a new module fails here, not on a user's
    desktop."""
    ROOT = Path(__file__).resolve().parents[1]

    def pkgbuild_loop(self, folder):
        import re
        text = (self.ROOT / 'packaging/aur/PKGBUILD').read_text().replace('\\\n', ' ')
        for match in re.finditer(r'for file in ([^;]+); do\s+install -Dm644 "([^"$]+)\$file', text):
            if match.group(2).rstrip('/') == folder:
                return sorted(match.group(1).split())
        self.fail(f'no PKGBUILD loop installs {folder}')

    def modules(self, folder):
        return sorted(path.name for path in (self.ROOT / folder).glob('*.py'))

    def test_observer_and_session_modules(self):
        self.assertEqual(sorted(installer.OBSERVER_MODULES), self.modules('adapters/accessibility'))
        self.assertEqual(sorted(installer.SESSION_MODULES), self.modules('adapters/session'))
        self.assertEqual([f'{name}.py' for name in self.pkgbuild_loop('adapters/accessibility')],
                         self.modules('adapters/accessibility'))
        self.assertEqual([f'{name}.py' for name in self.pkgbuild_loop('adapters/session')],
                         self.modules('adapters/session'))

    def test_panel_files(self):
        spec = importlib.util.spec_from_file_location('install_omarchy_ui', self.ROOT / 'scripts/install-omarchy-ui.py')
        panel = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(panel)
        shipped = sorted(path.name for path in (self.ROOT / 'ui/omarchy-plugin').iterdir()
                         if path.suffix in ('.qml', '.js') or path.name == 'manifest.json')
        self.assertEqual(sorted(panel.FILES), shipped)
        self.assertEqual(self.pkgbuild_loop('ui/omarchy-plugin'), shipped)


if __name__ == '__main__':
    unittest.main()
