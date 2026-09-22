"""Exercise desktop installation boundaries without changing the running session."""
import contextlib
import importlib.util
import io
import json
import mmap
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('install_desktop', Path(__file__).with_name('install-desktop.py'))
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class DesktopInstallTests(unittest.TestCase):
    def test_observed_flags_preserve_user_options_and_are_idempotent(self):
        original = '# user comment\n--ozone-platform=wayland\n--some-user-option\n'
        updated = installer.observed_flags(original)
        self.assertTrue(updated.startswith(original))
        self.assertEqual(updated.count('--ozone-platform=wayland'), 1)
        for flag in installer.OBSERVED_FLAGS:
            self.assertIn(flag, updated)
        self.assertEqual(installer.observed_flags(updated), updated)
        with self.assertRaisesRegex(RuntimeError, 'conflicts'):
            installer.observed_flags('--ozone-platform=x11\n')

    def test_observed_setup_is_never_a_broker_only_side_effect(self):
        with patch('sys.argv', ['install-desktop.py', '--broker-only', '--observed-app', 'chatgpt']), \
             patch.object(installer.subprocess, 'run') as command, contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as stopped:
                installer.main()
        self.assertEqual(stopped.exception.code, 2)
        command.assert_not_called()

    def test_observed_flags_follow_each_installed_wrappers_quoting(self):
        with self.assertRaisesRegex(RuntimeError, 'conflicts'):
            installer.observed_flags("'--ozone-platform=x11'\n", 'chromium')
        original = '--ozone-platform="wayland" # chosen platform\n'
        result = installer.observed_flags(original, 'chromium')
        self.assertTrue(result.startswith(original))
        self.assertNotIn('\n--ozone-platform=wayland\n', result)
        self.assertEqual(installer.observed_flags(result, 'chromium'), result)
        with self.assertRaisesRegex(RuntimeError, 'unbalanced quoting'):
            installer.observed_flags("'--ozone-platform=wayland\n", 'chromium')
        # ChatGPT passes these quote characters literally, so this is not an
        # existing recognized platform option in that specific wrapper.
        result = installer.observed_flags("'--ozone-platform=x11'\n", 'chatgpt')
        self.assertIn('\n--ozone-platform=wayland\n', result)
        self.assertEqual(installer.observed_flags(result, 'chatgpt'), result)

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
            installer.enable_accessibility(Path(temporary))
            receipt = Path(temporary) / 'accessibility-setting.json'
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
            sources = ('target/release/badi-broker', 'target/release/badictl', 'scripts/badi-desktop.py',
                       'packaging/io.github.ahuray.badi.desktop', 'packaging/io.github.ahuray.badi.svg',
                       'packaging/systemd/badi-broker.service', 'broker/data/writing-lexicon/LICENSE',
                       'broker/data/writing-lexicon/README.md')
            for name in sources:
                source = project / name
                source.parent.mkdir(parents=True, exist_ok=True)
                source.write_text('new ' + name)
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
            self.assertFalse(any('omarchy-fcitx5.service' in call or call[0] in ('omarchy-shell', 'npm') for call in calls))
            self.assertFalse(any('badi-accessibility.service' in call or call[0] == '/usr/bin/python3' for call in calls))
            self.assertNotIn(['systemctl', '--user', 'enable', 'badi-broker.service'], calls)
            backup, = (home / '.local/state/badi/install-backups').iterdir()
            self.assertEqual((backup / '.local/lib/badi/badi-broker').read_text(), 'previous broker')
            changed = json.loads((backup / 'changed-files.json').read_text())
            self.assertIn('.local/lib/badi/badi-broker', changed)
            self.assertFalse(any('fcitx' in path or 'accessibility' in path for path in changed))
            for name in ('LICENSE', 'README.md'):
                relative = '.local/share/badi/licenses/writing-lexicon/' + name
                self.assertEqual((home / relative).read_bytes(), (project / 'broker/data/writing-lexicon' / name).read_bytes())
                self.assertIn(relative, changed)
            self.assertEqual((backup / '.local/share/badi/licenses/writing-lexicon/LICENSE').read_text(), 'previous complete notice')

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
                sources = ('target/release/badi-broker', 'target/release/badictl', 'scripts/badi-desktop.py',
                           'packaging/io.github.ahuray.badi.desktop', 'packaging/io.github.ahuray.badi.svg',
                           'packaging/systemd/badi-broker.service', 'broker/data/writing-lexicon/LICENSE',
                           'broker/data/writing-lexicon/README.md', 'adapters/fcitx5/build/libbadi-fcitx5.so',
                           'adapters/fcitx5/build/badi.conf', 'packaging/systemd/fcitx-badi.conf',
                           'packaging/systemd/badi-accessibility.service')
                sources += tuple('adapters/accessibility/' + name for name in ('daemon.py', 'contract.py', 'preview.py', 'health.py'))
                for name in sources:
                    source = project / name
                    source.parent.mkdir(parents=True, exist_ok=True)
                    source.write_text('new ' + name)
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
                backup, = (home / '.local/state/badi/install-backups').iterdir()
                self.assertEqual((backup / '.local/lib/badi/accessibility/daemon.py').read_text(), 'previous helper')
                changed = json.loads((backup / 'changed-files.json').read_text())
                self.assertIn('.local/lib/badi/accessibility/health.py', changed)
                self.assertIn('.config/systemd/user/badi-accessibility.service', changed)
                self.assertEqual(profile.read_text(), 'preserve keyboard profile')


if __name__ == '__main__':
    unittest.main()
