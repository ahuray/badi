"""A missing or partial lock status must never authorize a shell reload."""

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('installer', Path(__file__).with_name('install-omarchy-ui.py'))
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


def setUpModule():
    # These tests describe Omarchy; the host running them may be another desktop.
    platform = patch.object(installer.badi_install, 'desktop_platform', return_value=installer.badi_install.OMARCHY)
    platform.start()
    unittest.addModuleCleanup(platform.stop)


class PlatformTests(unittest.TestCase):
    def test_another_desktop_is_refused_before_anything_runs(self):
        with patch.object(installer.badi_install, 'desktop_platform', return_value=installer.badi_install.GENERIC), \
             patch.object(installer.subprocess, 'run') as command, patch('sys.argv', ['install-omarchy-ui.py']):
            with self.assertRaisesRegex(RuntimeError, "Omarchy's bar.*nothing was changed"):
                installer.main()
        command.assert_not_called()


class LockBoundaryTests(unittest.TestCase):
    def test_only_explicitly_unlocked_state_allows_reload(self):
        state = dict.fromkeys(('locked', 'secure', 'requested', 'pending', 'sessionLocked'), False)
        with patch.object(installer, 'ipc', return_value=state):
            self.assertTrue(installer.unlocked())
        for key in state:
            with self.subTest(key=key), patch.object(installer, 'ipc', return_value={**state, key: True}):
                self.assertFalse(installer.unlocked())

    def test_partial_unknown_or_mistyped_state_does_not_allow_reload(self):
        for state in ({}, {'locked': False}, {'locked': 'false'}, {'secure': None}):
            with self.subTest(state=state), patch.object(installer, 'ipc', return_value=state):
                self.assertFalse(installer.unlocked())

    def test_unavailable_lock_status_aborts(self):
        with patch.object(installer, 'ipc', side_effect=RuntimeError('unavailable')):
            with self.assertRaisesRegex(RuntimeError, 'unavailable'):
                installer.unlocked()


class UpdateTests(unittest.TestCase):
    def test_update_replaces_the_plugin_files_with_a_backup(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            target = home / '.config/omarchy/plugins/io.github.ahuray.badi'
            target.mkdir(parents=True)
            (target / 'manifest.json').write_text(json.dumps({'id': 'io.github.ahuray.badi', 'kinds': ['panel']}))
            (target / 'Old.qml').write_text('stale')
            unlocked = dict.fromkeys(('locked', 'secure', 'requested', 'pending', 'sessionLocked'), False)
            answers = {('lock', 'status'): unlocked, ('badi-writing', 'state'): {'page': 0, 'service': {}}}
            with patch.object(installer.Path, 'home', return_value=home), \
                 patch.dict(os.environ, {'XDG_CONFIG_HOME': str(home / '.config'), 'XDG_STATE_HOME': str(home / '.local/state'),
                                          'XDG_DATA_HOME': str(home / '.local/share')}), \
                 patch.object(installer, 'ipc', side_effect=lambda *call: answers[call]), \
                 patch.object(installer.subprocess, 'run') as run, \
                 patch('sys.argv', ['install-omarchy-ui.py']), \
                 contextlib.redirect_stdout(io.StringIO()):
                installer.main()
            # Only the source gate and the supported shell restart run; no real shell is touched.
            self.assertEqual([call.args[0][:2] for call in run.call_args_list if call.args[0][0] != 'git'],
                             [['bash', 'ui/omarchy-plugin/tests/check-source.sh'], ['busctl', '--user'],
                              ['omarchy', 'restart']])
            self.assertCountEqual([path.name for path in target.iterdir()], installer.FILES)
            self.assertFalse((target / 'Old.qml').exists())
            for name in installer.FILES:
                self.assertEqual((target / name).read_bytes(),
                                 (installer.ROOT / 'ui/omarchy-plugin' / name).read_bytes())
            [backup] = (home / '.local/state/badi/ui-backups').iterdir()
            changes = {entry['path']: entry for entry in json.loads((backup / 'changes.json').read_text())['changes']}
            plugin = '.config/omarchy/plugins/io.github.ahuray.badi/'
            self.assertIn('"panel"', (backup / changes[plugin + 'manifest.json']['saved']).read_text())

    def test_first_install_creates_the_plugin_enables_it_and_drops_removed_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            config = home / 'xdg'
            target = config / 'omarchy/plugins/io.github.ahuray.badi'
            unlocked = dict.fromkeys(('locked', 'secure', 'requested', 'pending', 'sessionLocked'), False)
            answers = {('lock', 'status'): unlocked, ('badi-writing', 'state'): {'page': 0, 'service': {}}}
            with patch.object(installer.Path, 'home', return_value=home), \
                 patch.dict(os.environ, {'XDG_CONFIG_HOME': str(config), 'XDG_STATE_HOME': str(home / '.local/state'),
                                          'XDG_DATA_HOME': str(home / '.local/share')}), \
                 patch.object(installer, 'ipc', side_effect=lambda *call: answers[call]), \
                 patch.object(installer.subprocess, 'run') as run, \
                 patch('sys.argv', ['install-omarchy-ui.py']), \
                 contextlib.redirect_stdout(io.StringIO()):
                installer.main()
            self.assertEqual([call.args[0][:2] for call in run.call_args_list if call.args[0][0] != 'git'],
                             [['bash', 'ui/omarchy-plugin/tests/check-source.sh'],
                              ['busctl', '--user'],
                              ['omarchy', 'plugin'],
                              ['omarchy', 'restart']])
            self.assertEqual(run.call_args_list[2].args[0][2:], ['enable', 'io.github.ahuray.badi'])
            self.assertCountEqual([path.name for path in target.iterdir()], installer.FILES)
            self.assertTrue((home / '.local/state/badi/receipts/omarchy-ui.json').is_file())


class FcitxThemeTests(unittest.TestCase):
    COLORS = 'background = "#1a1b26"\nforeground = "#a9b1d6"\naccent = "#7aa2f7"\n'

    def run_installer(self, home, classicui=None):
        theme = home / '.local/state/omarchy/current/theme'
        theme.mkdir(parents=True)
        (theme / 'colors.toml').write_text(self.COLORS)
        conf = home / '.config/fcitx5/conf/classicui.conf'
        if classicui is not None:
            conf.parent.mkdir(parents=True)
            conf.write_text(classicui)
        unlocked = dict.fromkeys(('locked', 'secure', 'requested', 'pending', 'sessionLocked'), False)
        answers = {('lock', 'status'): unlocked, ('badi-writing', 'state'): {'page': 0, 'service': {}}}
        font = unittest.mock.Mock(stdout='JetBrainsMono Nerd Font\n')
        with patch.object(installer.Path, 'home', return_value=home), \
             patch.dict(os.environ, {'XDG_CONFIG_HOME': str(home / '.config'), 'XDG_STATE_HOME': str(home / '.local/state'),
                                     'XDG_DATA_HOME': str(home / '.local/share')}), \
             patch.object(installer, 'ipc', side_effect=lambda *call: answers[call]), \
             patch.object(installer.subprocess, 'run', return_value=font), \
             patch('sys.argv', ['install-omarchy-ui.py']), \
             contextlib.redirect_stdout(io.StringIO()):
            installer.main()
        return conf

    def test_the_panel_theme_follows_omarchy_and_keeps_a_users_fcitx_choice(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            conf = self.run_installer(home)
            theme = (home / '.local/share/fcitx5/themes/badi-omarchy/theme.conf').read_text()
            self.assertIn('Color=#1a1b26', theme)
            self.assertIn('HighlightColor=#7aa2f7', theme)
            self.assertIn('Theme=badi-omarchy', conf.read_text())
            self.assertIn('Font="JetBrainsMono Nerd Font 10"', conf.read_text())
            for event in installer.HOOK_EVENTS:
                hook = home / f'.config/omarchy/hooks/{event}.d/badi-fcitx-theme'
                self.assertTrue(os.access(hook, os.X_OK))
                self.assertIn('fcitx-theme.py', hook.read_text())
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            conf = self.run_installer(home, classicui='Theme=default-dark\n')
            self.assertEqual(conf.read_text(), 'Theme=default-dark\n')

    def test_the_softened_text_keeps_readable_contrast(self):
        theme = installer.fcitx_theme()
        for background, foreground in (('#1a1b26', '#a9b1d6'), ('#fdf6e3', '#657b83'), ('#000000', '#777777')):
            colors = theme.palette({'background': background, 'foreground': foreground, 'accent': foreground})
            self.assertGreaterEqual(theme.contrast(colors['text'], background), 4.5 if
                                    theme.contrast(foreground, background) >= 4.5 else 1)


if __name__ == '__main__':
    unittest.main()
