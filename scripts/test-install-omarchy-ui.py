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


def fake_run(enabled=True, font=None):
    """A subprocess.run stand-in answering Omarchy's plugin listing."""
    def run(command, *args, **kwargs):
        if list(command[:3]) == ['omarchy', 'plugin', 'list']:
            return unittest.mock.Mock(stdout=json.dumps([{'id': 'io.github.ahuray.badi', 'enabled': enabled}]))
        return font if font is not None else unittest.mock.Mock(stdout='')
    return run


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
                 patch.object(installer.subprocess, 'run', side_effect=fake_run(enabled=True)) as run, \
                 patch('sys.argv', ['install-omarchy-ui.py']), \
                 contextlib.redirect_stdout(io.StringIO()):
                installer.main()
            # Only the source gate, the plugin listing and the supported shell restart run; no real shell is touched.
            self.assertEqual([call.args[0][:2] for call in run.call_args_list if call.args[0][0] != 'git'],
                             [['bash', 'ui/omarchy-plugin/tests/check-source.sh'], ['busctl', '--user'],
                              ['omarchy', 'plugin'], ['omarchy', 'restart']])
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
                 patch.object(installer.subprocess, 'run', side_effect=fake_run(enabled=False)) as run, \
                 patch('sys.argv', ['install-omarchy-ui.py']), \
                 contextlib.redirect_stdout(io.StringIO()):
                installer.main()
            self.assertEqual([call.args[0][:2] for call in run.call_args_list if call.args[0][0] != 'git'],
                             [['bash', 'ui/omarchy-plugin/tests/check-source.sh'],
                              ['busctl', '--user'],
                              ['omarchy', 'plugin'],
                              ['omarchy', 'plugin'],
                              ['omarchy', 'restart']])
            plugin_calls = [call.args[0] for call in run.call_args_list if call.args[0][:2] == ['omarchy', 'plugin']]
            self.assertEqual(plugin_calls, [['omarchy', 'plugin', 'list', '--json'],
                                            ['omarchy', 'plugin', 'enable', 'io.github.ahuray.badi']])
            self.assertCountEqual([path.name for path in target.iterdir()], installer.FILES)
            self.assertTrue((home / '.local/state/badi/receipts/omarchy-ui.json').is_file())

    def run_update(self, home, *, enabled, panel_answers=True):
        target = home / '.config/omarchy/plugins/io.github.ahuray.badi'
        target.mkdir(parents=True)
        (target / 'manifest.json').write_text(json.dumps({'id': 'io.github.ahuray.badi', 'kinds': ['panel']}))
        unlocked = dict.fromkeys(('locked', 'secure', 'requested', 'pending', 'sessionLocked'), False)
        answers = {('lock', 'status'): unlocked,
                   ('badi-writing', 'state'): {'page': 0, 'service': {}} if panel_answers else {}}
        with patch.object(installer.Path, 'home', return_value=home), \
             patch.dict(os.environ, {'XDG_CONFIG_HOME': str(home / '.config'), 'XDG_STATE_HOME': str(home / '.local/state'),
                                      'XDG_DATA_HOME': str(home / '.local/share')}), \
             patch.object(installer, 'ipc', side_effect=lambda *call: answers[call]), \
             patch.object(installer.subprocess, 'run', side_effect=fake_run(enabled=enabled)) as run, \
             patch.object(installer.badi_install, 'poll', lambda check, timeout, interval: check()), \
             patch('sys.argv', ['install-omarchy-ui.py']), \
             contextlib.redirect_stdout(io.StringIO()), \
             contextlib.redirect_stderr(io.StringIO()) as errors:
            installer.main()
        return run, errors.getvalue()

    def test_an_update_enables_a_plugin_an_earlier_run_left_disabled(self):
        with tempfile.TemporaryDirectory() as temporary:
            run, _errors = self.run_update(Path(temporary), enabled=False)
            plugin_calls = [call.args[0] for call in run.call_args_list if call.args[0][:2] == ['omarchy', 'plugin']]
            self.assertIn(['omarchy', 'plugin', 'enable', 'io.github.ahuray.badi'], plugin_calls,
                          'an update retries the enable an earlier failed run never reached')

    def test_a_panel_that_never_answers_gets_no_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            with self.assertRaisesRegex(RuntimeError, 'did not answer IPC'):
                self.run_update(home, enabled=True, panel_answers=False)
            self.assertFalse((home / '.local/state/badi/receipts/omarchy-ui.json').exists(),
                             'the receipt records an answering panel, not files merely written')


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
                                     'XDG_DATA_HOME': str(home / '.local/share'),
                                     # The terminal lists and configs stay inside the test home too.
                                     'XDG_CONFIG_DIRS': str(home / 'etc/xdg'), 'XDG_DATA_DIRS': str(home / 'usr/share'),
                                     'XDG_CURRENT_DESKTOP': 'Hyprland'}), \
             patch.object(installer, 'ipc', side_effect=lambda *call: answers[call]), \
             patch.object(installer.subprocess, 'run', side_effect=fake_run(enabled=True, font=font)), \
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

    INSTALLED_TERMINALS = ('com.mitchellh.ghostty.desktop', 'Alacritty.desktop', 'alacritty.desktop',
                           'kitty.desktop', 'foot.desktop', 'footclient.desktop')

    TERMINAL_ENTRY = '[Desktop Entry]\nType=Application\nExec=sh\nCategories=System;TerminalEmulator;\n'

    @staticmethod
    def isolated_environment(home, desktop='Hyprland'):
        """XDG paths inside the temporary home, so no test reads this machine's lists."""
        return {'XDG_CONFIG_DIRS': str(home / 'etc/xdg'), 'XDG_DATA_HOME': str(home / '.local/share'),
                'XDG_DATA_DIRS': str(home / 'usr/share'), 'XDG_CURRENT_DESKTOP': desktop}

    def read_font(self, configs, family='JetBrainsMono Nerd Font', installed=INSTALLED_TERMINALS):
        theme = installer.fcitx_theme()
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            applications = home / 'usr/share/applications'
            applications.mkdir(parents=True)
            for entry in installed:
                (applications / entry).write_text(self.TERMINAL_ENTRY)
            for relative, content in configs.items():
                path = home / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                if isinstance(content, bytes):
                    path.write_bytes(content)
                else:
                    path.write_text(content)
            with patch.object(theme.subprocess, 'run', return_value=unittest.mock.Mock(stdout=f'{family}\n')):
                return theme.font(home, self.isolated_environment(home))

    def test_font_reads_the_preferred_terminal_across_all_four_terminals(self):
        cases = (
            ('com.mitchellh.ghostty.desktop\n', {'.config/ghostty/config': 'font-size = 11\n'}, 11),
            ('Alacritty.desktop\n', {
                '.config/ghostty/config': 'font-size = 9\n',
                '.config/alacritty/alacritty.toml': '[font]\nnormal = { family = "Mono" }\nsize = 12\n',
            }, 12),
            ('alacritty.desktop\n', {
                '.config/ghostty/config': 'font-size = 9\n',
                '.config/alacritty/alacritty.toml': '[font]\nsize = 13\n',
            }, 13),
            ('kitty.desktop\n', {
                '.config/ghostty/config': 'font-size = 9\n',
                '.config/kitty/kitty.conf': '# comment\nfont_size 14.0\n',
            }, 14),
            ('foot.desktop\n', {
                '.config/ghostty/config': 'font-size = 9\n',
                '.config/foot/foot.ini': 'font=JetBrainsMono Nerd Font:size=15\n\n[csd]\nfont=Sans:size=8\n',
            }, 15),
            ('footclient.desktop\n', {
                '.config/ghostty/config': 'font-size = 9\n',
                '.config/foot/foot.ini': 'font=JetBrainsMono Nerd Font:style=Regular:size=16\n',
            }, 16),
        )
        for terminals, configs, expected in cases:
            with self.subTest(terminals=terminals.strip(), expected=expected):
                self.assertEqual(self.read_font({'.config/xdg-terminals.list': terminals, **configs}),
                                 f'JetBrainsMono Nerd Font {expected}')

    def test_font_skips_comments_and_unknown_entries_and_falls_back_to_ghostty_or_default(self):
        self.assertEqual(self.read_font({
            '.config/xdg-terminals.list': '\n  # preferred later\norg.wezfurlong.wezterm.desktop\nkitty.desktop\n',
            '.config/ghostty/config': 'font-size = 9\n',
            '.config/kitty/kitty.conf': 'font_size 13\n',
        }), 'JetBrainsMono Nerd Font 13')
        self.assertEqual(self.read_font({'.config/ghostty/config': 'font-size = 9\n'}),
                         'JetBrainsMono Nerd Font 9')
        self.assertEqual(self.read_font({}), 'JetBrainsMono Nerd Font 10')
        self.assertEqual(self.read_font({
            '.config/xdg-terminals.list': '# only unknown\norg.wezfurlong.wezterm.desktop\n',
            '.config/ghostty/config': 'font-size = 9\n',
        }), 'JetBrainsMono Nerd Font 9')
        self.assertEqual(self.read_font({
            '.config/xdg-terminals.list': 'org.wezfurlong.wezterm.desktop\n',
        }), 'JetBrainsMono Nerd Font 10')

    def test_font_follows_xdg_terminal_execs_own_list_order(self):
        # Stock Omarchy: no user list, the packaged Hyprland list names foot.
        self.assertEqual(self.read_font({
            'usr/share/xdg-terminal-exec/hyprland-xdg-terminals.list': 'foot.desktop\n',
            '.config/foot/foot.ini': 'font=JetBrainsMono Nerd Font:size=14\n',
            '.config/ghostty/config': 'font-size = 9\n',
        }), 'JetBrainsMono Nerd Font 14')
        # The user's list comes first, and the desktop's own list before the shared one.
        self.assertEqual(self.read_font({
            '.config/xdg-terminals.list': 'foot.desktop\n',
            '.config/hyprland-xdg-terminals.list': 'kitty.desktop\n',
            'usr/share/xdg-terminal-exec/hyprland-xdg-terminals.list': 'foot.desktop\n',
            '.config/kitty/kitty.conf': 'font_size 12\n',
            '.config/foot/foot.ini': 'font=Mono:size=14\n',
        }), 'JetBrainsMono Nerd Font 12')
        # A terminal that is not installed, and an exclusion line, are passed over.
        self.assertEqual(self.read_font({
            '.config/xdg-terminals.list': '-com.mitchellh.ghostty.desktop\nkitty.desktop\nfoot.desktop\n',
            '.config/kitty/kitty.conf': 'font_size 12\n',
            '.config/foot/foot.ini': 'font=Mono:size=14\n',
        }, installed=('foot.desktop',)), 'JetBrainsMono Nerd Font 14')
        # An installed terminal Badi cannot read is still the one that opens.
        self.assertEqual(self.read_font({
            '.config/xdg-terminals.list': 'org.wezfurlong.wezterm.desktop\nkitty.desktop\n',
            '.config/kitty/kitty.conf': 'font_size 12\n',
            '.config/ghostty/config': 'font-size = 9\n',
        }, installed=('org.wezfurlong.wezterm.desktop', 'kitty.desktop')), 'JetBrainsMono Nerd Font 9')

    def test_an_entry_xdg_terminal_exec_would_not_run_is_passed_over(self):
        configs = {
            '.config/xdg-terminals.list': 'foot.desktop\nkitty.desktop\n',
            '.config/kitty/kitty.conf': 'font_size 14\n',
            '.config/foot/foot.ini': 'font=Mono:size=15\n',
        }
        for leftover in ('TryExec=badi-no-such-terminal\nExec=sh\n', 'Hidden=true\nExec=sh\n',
                         'Categories=Utility;\nExec=sh\n', 'Exec=badi-no-such-terminal --server\n'):
            with self.subTest(leftover=leftover.splitlines()[0]):
                foot = '[Desktop Entry]\n' + leftover + ('' if leftover.startswith('Categories')
                                                         else 'Categories=TerminalEmulator;\n')
                self.assertEqual(self.read_font({**configs, '.local/share/applications/foot.desktop': foot}),
                                 'JetBrainsMono Nerd Font 14')

    def test_foot_reads_the_system_config_without_a_user_one(self):
        self.assertEqual(self.read_font({
            '.config/xdg-terminals.list': 'foot.desktop\n',
            'etc/xdg/foot/foot.ini': 'font=JetBrainsMono Nerd Font:size=14\n',
            '.config/ghostty/config': 'font-size = 9\n',
        }), 'JetBrainsMono Nerd Font 14')

    def test_the_last_size_setting_wins(self):
        self.assertEqual(self.read_font({
            '.config/xdg-terminals.list': 'foot.desktop\n',
            '.config/foot/foot.ini': 'font=JetBrainsMono Nerd Font:size=16\nfont=Iosevka\n',
            '.config/ghostty/config': 'font-size = 9\n',
        }), 'JetBrainsMono Nerd Font 9')
        self.assertEqual(self.read_font({
            '.config/xdg-terminals.list': 'kitty.desktop\n',
            '.config/kitty/kitty.conf': 'font_size 12\nfont_size 48\n',
            '.config/ghostty/config': 'font-size = 9\n',
        }), 'JetBrainsMono Nerd Font 9')

    def test_alacritty_reads_its_config_where_alacritty_finds_it(self):
        self.assertEqual(self.read_font({
            '.config/xdg-terminals.list': 'Alacritty.desktop\n',
            '.config/alacritty.toml': '[font]\nsize = 16\n',
            '.config/ghostty/config': 'font-size = 9\n',
        }), 'JetBrainsMono Nerd Font 16')

    def test_kitty_layers_the_users_config_over_the_systems(self):
        # Omarchy's template leaves the size to /etc/xdg/kitty/kitty.conf.
        self.assertEqual(self.read_font({
            '.config/xdg-terminals.list': 'kitty.desktop\n',
            '.config/kitty/kitty.conf': 'include theme.conf\n# font_size 12\n',
            'etc/xdg/kitty/kitty.conf': 'font_size        9.0\n',
            '.config/ghostty/config': 'font-size = 13\n',
        }), 'JetBrainsMono Nerd Font 9')
        # A size of the user's own overrides it.
        self.assertEqual(self.read_font({
            '.config/xdg-terminals.list': 'kitty.desktop\n',
            '.config/kitty/kitty.conf': 'font_size 11\n',
            'etc/xdg/kitty/kitty.conf': 'font_size 9.0\n',
        }), 'JetBrainsMono Nerd Font 11')

    def test_font_rounds_decimal_sizes_and_ignores_out_of_range_values(self):
        self.assertEqual(self.read_font({
            '.config/xdg-terminals.list': 'kitty.desktop\n',
            '.config/kitty/kitty.conf': 'font_size 10.5\n',
        }), 'JetBrainsMono Nerd Font 11')
        self.assertEqual(self.read_font({
            '.config/xdg-terminals.list': 'kitty.desktop\n',
            '.config/kitty/kitty.conf': 'font_size 9.6\n',
        }), 'JetBrainsMono Nerd Font 10')
        self.assertEqual(self.read_font({
            '.config/xdg-terminals.list': 'Alacritty.desktop\n',
            '.config/alacritty/alacritty.toml': '[font]\nsize = 11.4\n',
        }), 'JetBrainsMono Nerd Font 11')
        self.assertEqual(self.read_font({
            '.config/xdg-terminals.list': 'foot.desktop\n',
            '.config/foot/foot.ini': 'font=JetBrainsMono Nerd Font:size=5.0\n',
        }), 'JetBrainsMono Nerd Font 5')
        self.assertEqual(self.read_font({
            '.config/xdg-terminals.list': 'com.mitchellh.ghostty.desktop\n',
            '.config/ghostty/config': 'font-size = 40.0\n',
        }), 'JetBrainsMono Nerd Font 40')
        for invalid in ('4', '41', '0', '-9', '120'):
            with self.subTest(invalid=invalid):
                self.assertEqual(self.read_font({
                    '.config/xdg-terminals.list': 'kitty.desktop\n',
                    '.config/kitty/kitty.conf': f'font_size {invalid}\n',
                    '.config/ghostty/config': 'font-size = 9\n',
                }), 'JetBrainsMono Nerd Font 9')
        self.assertEqual(self.read_font({'.config/ghostty/config': 'font-size = 99\n'}),
                         'JetBrainsMono Nerd Font 10')

    def test_font_falls_back_on_malformed_or_unreadable_configs_without_raising(self):
        for broken in ('[font\nsize = ???\n', 'font = "not a table"\n', '[font]\nsize = true\n',
                       '[font]\nsize = nan\n', b'\xff\xfe\x00\x80'):
            with self.subTest(broken=broken):
                self.assertEqual(self.read_font({
                    '.config/xdg-terminals.list': 'Alacritty.desktop\n',
                    '.config/alacritty/alacritty.toml': broken,
                    '.config/ghostty/config': 'font-size = 9\n',
                }), 'JetBrainsMono Nerd Font 9')
        if os.geteuid() != 0:
            theme = installer.fcitx_theme()
            with tempfile.TemporaryDirectory() as temporary:
                home = Path(temporary)
                (home / '.config').mkdir()
                (home / '.config/xdg-terminals.list').write_text('kitty.desktop\n')
                unreadable = home / '.config/kitty/kitty.conf'
                unreadable.parent.mkdir()
                unreadable.write_text('font_size 14\n')
                unreadable.chmod(0)
                try:
                    environment = self.isolated_environment(home)
                    applications = home / 'usr/share/applications'
                    applications.mkdir(parents=True)
                    (applications / 'kitty.desktop').write_text(self.TERMINAL_ENTRY)
                    with patch.object(theme.subprocess, 'run', return_value=unittest.mock.Mock(stdout='Mono\n')):
                        self.assertEqual(theme.font(home, environment), 'Mono 10')
                finally:
                    unreadable.chmod(0o600)


if __name__ == '__main__':
    unittest.main()
