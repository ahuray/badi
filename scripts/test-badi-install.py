"""Shared installer and CLI helpers: backups, receipts, atomic writes, systemd queries."""
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import badi_install


class AtomicWriteTests(unittest.TestCase):
    def test_replaces_content_mode_and_inode_without_leaving_a_staged_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / 'library.so'
            target.write_bytes(b'old')
            mapped = os.stat(target).st_ino
            badi_install.atomic_write(target, b'new', 0o755)
            self.assertEqual(target.read_bytes(), b'new')
            self.assertEqual(target.stat().st_mode & 0o777, 0o755)
            self.assertNotEqual(target.stat().st_ino, mapped, 'A running reader keeps the old inode')
            self.assertEqual(os.listdir(temporary), ['library.so'])
            badi_install.atomic_write(target, b'private')
            self.assertEqual(target.stat().st_mode & 0o777, 0o600, 'Private by default')

    def test_replaces_a_symlink_itself_and_never_its_destination(self):
        with tempfile.TemporaryDirectory() as temporary:
            destination = Path(temporary) / 'elsewhere'
            destination.write_text('keep')
            link = Path(temporary) / 'link'
            link.symlink_to(destination)
            badi_install.atomic_write(link, b'file', 0o644)
            self.assertFalse(link.is_symlink())
            self.assertEqual(destination.read_text(), 'keep')

    def test_failed_write_leaves_the_target_and_no_staged_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / 'settings.json'
            target.write_text('old')
            with patch.object(badi_install.os, 'fsync', side_effect=OSError('disk full')):
                with self.assertRaisesRegex(OSError, 'disk full'):
                    badi_install.atomic_write(target, b'new')
            self.assertEqual(target.read_text(), 'old')
            self.assertEqual(os.listdir(temporary), ['settings.json'])


class SystemdTests(unittest.TestCase):
    def test_properties_parse_key_values_and_value_keeps_equals_signs(self):
        reply = subprocess.CompletedProcess([], 0, 'ActiveState=active\nMainPID=42\nnoise\n', '')
        with patch.object(badi_install.subprocess, 'run', return_value=reply) as command:
            self.assertEqual(badi_install.unit_properties('x.service', 'ActiveState', 'MainPID'),
                             {'ActiveState': 'active', 'MainPID': '42'})
        self.assertEqual(command.call_args.args[0],
                         ['systemctl', '--user', 'show', 'x.service', '--property=ActiveState,MainPID'])
        reply = subprocess.CompletedProcess([], 0, '{ path=/usr/bin/fcitx5 ; argv[]=/usr/bin/fcitx5 }\n', '')
        with patch.object(badi_install.subprocess, 'run', return_value=reply) as command:
            self.assertEqual(badi_install.unit_value('x.service', 'ExecStart'),
                             '{ path=/usr/bin/fcitx5 ; argv[]=/usr/bin/fcitx5 }')
        self.assertEqual(command.call_args.args[0][-2:], ['--property=ExecStart', '--value'])

    def test_an_unreachable_service_manager_is_an_error_not_an_empty_state(self):
        failed = subprocess.CompletedProcess([], 1, '', 'Failed to connect to bus\n')
        for query in (lambda: badi_install.unit_properties('x.service', 'MainPID'),
                      lambda: badi_install.unit_value('x.service', 'MainPID')):
            with patch.object(badi_install.subprocess, 'run', return_value=failed), \
                 self.assertRaisesRegex(RuntimeError, 'Failed to connect to bus'):
                query()


class PollTests(unittest.TestCase):
    def test_returns_the_first_truthy_result_and_sleeps_between_checks(self):
        with patch.object(badi_install.time, 'monotonic', return_value=0), \
             patch.object(badi_install.time, 'sleep') as sleep:
            self.assertEqual(badi_install.poll(iter([None, False, {'ready': True}]).__next__, 5, .5), {'ready': True})
        self.assertEqual([call.args for call in sleep.call_args_list], [(.5,), (.5,)])

    def test_gives_up_after_the_check_that_reaches_the_deadline(self):
        checks = []
        with patch.object(badi_install.time, 'monotonic', side_effect=[0, 4, 10]), \
             patch.object(badi_install.time, 'sleep'):
            self.assertIsNone(badi_install.poll(lambda: checks.append(1), 10))
        self.assertEqual(len(checks), 2)


class ContractTests(unittest.TestCase):
    def test_only_an_explicit_all_false_lock_status_is_unlocked(self):
        unlocked = dict.fromkeys(badi_install.LOCK_FLAGS, False)
        self.assertTrue(badi_install.explicitly_unlocked(unlocked))
        for state in (None, [], {}, {**unlocked, 'pending': None}, {**unlocked, 'secure': 'false'},
                      {key: False for key in badi_install.LOCK_FLAGS[1:]}):
            with self.subTest(state=state):
                self.assertFalse(badi_install.explicitly_unlocked(state))

    def test_grants_never_allow_learning_or_retention(self):
        for decision in ('allow', 'block'):
            permissions = badi_install.grant(decision)
            self.assertEqual({key: permissions[key] for key in ('context_read', 'display', 'suggest')},
                             dict.fromkeys(('context_read', 'display', 'suggest'), decision))
            self.assertEqual((permissions['learn'], permissions['retention']), ('block', {'mode': 'none'}))
        self.assertIsNot(badi_install.grant('allow'), badi_install.grant('allow'), 'Callers may edit their copy')

    def test_configuration_directory_follows_xdg(self):
        with patch.dict(os.environ, {'XDG_CONFIG_HOME': '/custom/config'}):
            self.assertEqual(badi_install.config_home('/home/user'), Path('/custom/config'))
        with patch.dict(os.environ, {'XDG_CONFIG_HOME': ''}):
            self.assertEqual(badi_install.config_home('/home/user'), Path('/home/user/.config'))

    def test_keys_are_home_relative_only_inside_home(self):
        home = Path('/home/user')
        self.assertEqual(badi_install.home_relative(home, home / '.local/bin/badi'), '.local/bin/badi')
        self.assertEqual(badi_install.home_relative(home, Path('/srv/vault/main.js')), '/srv/vault/main.js')


class PlatformTests(unittest.TestCase):
    def test_omarchy_is_where_its_shell_command_is_installed(self):
        with patch.object(badi_install.shutil, 'which', return_value='/usr/bin/omarchy-shell'):
            self.assertEqual(badi_install.desktop_platform(), badi_install.OMARCHY)
        with patch.object(badi_install.shutil, 'which', return_value=None):
            self.assertEqual(badi_install.desktop_platform(), badi_install.GENERIC)

    def test_distribution_family_reads_id_and_id_like(self):
        with tempfile.TemporaryDirectory() as temporary:
            release = Path(temporary) / 'os-release'
            for text, family in (('ID=debian\nVERSION_ID="13"\n', 'debian'),
                                 ('ID=ubuntu\nID_LIKE=debian\nVERSION_ID="24.04"\n', 'debian'),
                                 ("ID=linuxmint\nID_LIKE='ubuntu debian'\n", 'debian'),
                                 ('ID=arch\n', 'arch'), ('ID=fedora\n', None), ('garbage\n', None)):
                with self.subTest(text=text):
                    release.write_text(text)
                    self.assertEqual(badi_install.distribution_family(badi_install.os_release(release)), family)
            self.assertEqual(badi_install.os_release(Path(temporary) / 'missing'), {})
            release.write_text('ID="debian"\nPRETTY_NAME="Debian GNU/Linux 13 (trixie)"\n')
            self.assertEqual(badi_install.os_release(release)['PRETTY_NAME'], 'Debian GNU/Linux 13 (trixie)')

    def test_the_fcitx_addon_directory_sits_beside_the_fcitx_core_library(self):
        layouts = {'debian': ('usr/lib/x86_64-linux-gnu', 'x86_64-linux-gnu'), 'arch': ('usr/lib', ''),
                   'fedora': ('usr/lib64', '')}
        for name, (library, multiarch) in layouts.items():
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                (root / library / 'fcitx5').mkdir(parents=True)
                (root / library / 'libFcitx5Core.so.7').write_bytes(b'')
                # A directory without the core library is not the one Fcitx reads.
                (root / 'usr/lib/fcitx5').mkdir(parents=True, exist_ok=True)
                self.assertEqual(badi_install.fcitx_addon_directory(root, multiarch), Path('/') / library / 'fcitx5')
        with tempfile.TemporaryDirectory() as temporary:
            self.assertIsNone(badi_install.fcitx_addon_directory(Path(temporary), 'x86_64-linux-gnu'))

    def test_the_environment_file_names_badi_before_the_system_addons(self):
        text = badi_install.fcitx_environment(Path('/home/someone'), Path('/usr/lib/x86_64-linux-gnu/fcitx5'))
        self.assertTrue(text.startswith('# Badi: '))
        self.assertIn('FCITX_ADDON_DIRS=/home/someone/.local/lib/fcitx5:/usr/lib/x86_64-linux-gnu/fcitx5\n', text)
        self.assertIn('FCITX_DATA_DIRS=/home/someone/.local/share/fcitx5:/usr/local/share/fcitx5:/usr/share/fcitx5\n', text)
        # The same directories as Omarchy's service drop-in.
        drop_in = (Path(__file__).resolve().parents[1] / 'packaging/systemd/fcitx-badi.conf').read_text()
        self.assertIn('FCITX_DATA_DIRS=%h/.local/share/fcitx5:/usr/local/share/fcitx5:/usr/share/fcitx5', drop_in)
        self.assertIn('FCITX_ADDON_DIRS=%h/.local/lib/fcitx5:', drop_in)

    def test_the_running_fcitx_is_its_session_bus_name_owner(self):
        def bus(owner, pid='u 4242', failure=None):
            def reply(command, **kwargs):
                if failure:
                    raise failure
                if 'NameHasOwner' in command:
                    return subprocess.CompletedProcess(command, 0, f'b {owner}\n', '')
                return subprocess.CompletedProcess(command, 0 if pid else 1, f'{pid}\n' if pid else '', '')
            return reply
        for reply, expected in ((bus('true'), 4242), (bus('false'), 0), (bus('true', pid=None), None),
                                (bus('true', failure=FileNotFoundError('busctl')), None),
                                (bus('true', failure=subprocess.TimeoutExpired('busctl', 4)), None)):
            with patch.object(badi_install.subprocess, 'run', side_effect=reply) as command:
                self.assertEqual(badi_install.fcitx_bus_process(), expected)
                if expected:
                    self.assertEqual(command.call_args.args[0][-3:], ['GetConnectionUnixProcessID', 's',
                                                                      'org.fcitx.Fcitx5'])


class InstallationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.home = self.root / 'home'
        self.home.mkdir()
        self.backups = self.home / '.local/state/badi/install-backups'
        self.quiet = patch('builtins.print')
        self.quiet.start()
        self.addCleanup(self.quiet.stop)

    def changes(self, installation):
        return json.loads((installation.directory / 'changes.json').read_text())

    def test_identical_files_are_neither_backed_up_nor_rewritten(self):
        target = self.home / '.local/lib/badi/badi-broker'
        target.parent.mkdir(parents=True)
        target.write_bytes(b'same build')
        target.chmod(0o755)
        before = target.stat()
        link = self.home / '.local/bin/badi'
        link.parent.mkdir(parents=True)
        link.symlink_to('../lib/badi/badi-desktop.py')
        installation = badi_install.Installation(self.home, 'desktop')
        self.assertFalse(installation.write(target, b'same build', 0o755))
        self.assertFalse(installation.link(link, '../lib/badi/badi-desktop.py'))
        self.assertEqual((target.stat().st_ino, target.stat().st_mtime_ns), (before.st_ino, before.st_mtime_ns))
        self.assertIsNone(installation.directory)
        self.assertFalse(self.backups.exists(), 'No backup directory without a change')
        self.assertEqual(installation.current, [target], 'Identical files still count as installed')
        self.assertTrue(installation.write(target, b'same build', 0o700), 'A mode change is a change')

    def test_every_change_is_mapped_before_it_happens_and_restores_exactly(self):
        replaced = self.home / '.local/lib/badi/badictl'
        replaced.parent.mkdir(parents=True)
        replaced.write_text('old ctl')
        replaced.chmod(0o755)
        copied = self.home / '.local/bin/badi'
        copied.parent.mkdir(parents=True)
        copied.write_text('old cli copy')
        retired = self.home / '.local/lib/badi/badi-native-host'
        retired.write_text('retired host')
        vault = self.root / 'vault/.obsidian/plugins/badi/main.js'
        created = self.home / '.config/code-flags.conf'
        installation = badi_install.Installation(self.home, 'desktop')
        self.assertTrue(installation.write(replaced, b'new ctl', 0o755))
        self.assertTrue(installation.link(copied, '../lib/badi/badi-desktop.py'))
        installation.remove(retired)
        self.assertTrue(installation.write(created, b'--flag\n', 0o600))
        vault.parent.mkdir(parents=True)
        vault.write_text('old plugin')
        installation.write(vault, b'plugin', 0o644)
        document = self.changes(installation)
        self.assertEqual((document['schema'], document['installer'], document['home']),
                         ('badi.install-backup.v1', 'desktop', str(self.home)))
        self.assertEqual([(entry['path'], entry['action']) for entry in document['changes']], [
            ('.local/lib/badi/badictl', 'replace'), ('.local/bin/badi', 'replace'),
            ('.local/lib/badi/badi-native-host', 'remove'), ('.config/code-flags.conf', 'create'),
            (str(vault), 'replace')])
        self.assertEqual(document['changes'][0]['saved'], 'home/.local/lib/badi/badictl')
        self.assertEqual(document['changes'][4]['saved'], 'root' + str(vault), 'Paths outside home keep their own tree')
        self.assertEqual(document['changes'][1]['symlink'], '../lib/badi/badi-desktop.py')
        self.assertEqual(installation.directory.parent, self.backups)
        self.assertEqual(installation.directory.stat().st_mode & 0o777, 0o700)

        created.write_text('--flag\n--edited-by-the-user\n')
        with self.assertRaises(SystemExit) as stopped:
            badi_install.main(['restore', str(installation.directory)])
        self.assertEqual(stopped.exception.code, 1, 'A path changed since installation is reported')
        self.assertEqual(replaced.read_text(), 'old ctl')
        self.assertEqual(replaced.stat().st_mode & 0o777, 0o755)
        self.assertFalse(copied.is_symlink())
        self.assertEqual(copied.read_text(), 'old cli copy')
        self.assertEqual(retired.read_text(), 'retired host')
        self.assertEqual(vault.read_text(), 'old plugin', 'A path outside home is restored too')
        self.assertEqual(created.read_text(), '--flag\n--edited-by-the-user\n', 'Later edits are kept')
        created.write_text('--flag\n')
        self.assertEqual(badi_install.restore(installation.directory), [])
        self.assertFalse(created.exists(), 'A created file is removed once it is what the installation left')

    def test_restore_can_be_limited_to_paths_below_a_prefix(self):
        compat = self.home / '.local/lib/badi/compat/fcitx5-5.1.22/launch.py'
        broker = self.home / '.local/lib/badi/badi-broker'
        installation = badi_install.Installation(self.home, 'desktop')
        installation.write(compat, b'launcher', 0o644)
        installation.write(broker, b'broker', 0o755)
        self.assertEqual(badi_install.restore(installation.directory, ['.local/lib/badi/compat/']), [])
        self.assertFalse(compat.exists())
        self.assertEqual(broker.read_bytes(), b'broker')

    def test_symlinks_and_foreign_entries_are_not_replaced_without_consent(self):
        installation = badi_install.Installation(self.home, 'omarchy-ui')
        link = self.home / 'Panel.qml'
        link.symlink_to(self.home / 'checkout.qml')
        with self.assertRaisesRegex(RuntimeError, 'symlink'):
            installation.write(link, b'qml')
        (self.home / 'folder').mkdir()
        with self.assertRaisesRegex(RuntimeError, 'ownership/type'):
            installation.write(self.home / 'folder', b'qml')
        self.assertIsNone(installation.directory)
        self.assertTrue(installation.write(link, b'qml', replace_link=True))
        saved = installation.directory / self.changes(installation)['changes'][0]['saved']
        self.assertTrue(saved.is_symlink(), 'The link itself is saved, not what it names')
        self.assertFalse(link.is_symlink())

    def test_older_loose_backup_directories_become_private(self):
        state = self.home / '.local/state/badi'
        self.backups.mkdir(parents=True)
        for directory in (state, self.backups):
            directory.chmod(0o755)
        installation = badi_install.Installation(self.home, 'desktop')
        installation.write(self.home / '.bashrc', b'changed\n', 0o644)
        for directory in (state, self.backups, installation.directory):
            self.assertEqual(directory.stat().st_mode & 0o777, 0o700, directory)
        self.assertNotEqual((self.home / '.local').stat().st_mode & 0o777, 0o700,
                            'nothing above the state directory is touched')

    def test_a_note_alone_does_not_create_a_backup_directory(self):
        installation = badi_install.Installation(self.home, 'desktop')
        note = installation.note('accessibility-setting.json', {'bus_enabled': False})
        self.assertIsNone(installation.directory)
        self.assertEqual(note.parent, self.backups / 'notes')
        self.assertEqual(note.stat().st_mode & 0o777, 0o600)
        self.assertEqual(json.loads(note.read_text()), {'bus_enabled': False})
        installation.write(self.home / '.bashrc', b'changed\n', 0o644)
        recorded = installation.note('accessibility-setting.json', {'bus_enabled': True})
        self.assertEqual(recorded.parent, installation.directory)


class PruneTests(unittest.TestCase):
    def test_keeps_the_newest_three_of_every_naming_format_and_nothing_foreign(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            root = home / '.local/state/badi/editor-backups'
            names = ['20260905-174546', '1788635497971086549', '1790447702770311229',
                     '20260926T120000.000000001Z', '20260927T170722.123456789Z']
            for name in names:
                (root / name).mkdir(parents=True)
                (root / name / 'changes.json').write_text('{}')
            (root / 'notes').mkdir()
            (root / '20200101-000000.txt').write_text('user file')
            (root / '1000000000000000000').symlink_to(home)
            other = home / '.local/state/badi/install-backups/20200101-000000'
            other.mkdir(parents=True)
            (root / names[0] / 'accessibility-setting.json').write_text('{"bus_enabled": false}')
            pruned = badi_install.prune_backups(home, 'editors')
            self.assertEqual(sorted(path.name for path in pruned), sorted(names[:2]))
            self.assertEqual(sorted(path.name for path in root.iterdir()),
                             sorted([*names[2:], 'notes', '20200101-000000.txt', '1000000000000000000']))
            self.assertEqual((root / 'notes' / names[0] / 'accessibility-setting.json').read_text(),
                             '{"bus_enabled": false}', 'A prior desktop setting outlives its pruned backup')
            self.assertTrue(other.is_dir(), 'Another installer keeps its own backups')
            self.assertTrue(home.is_dir(), 'A symlinked entry is never followed')
            self.assertEqual(badi_install.prune_backups(home, 'omarchy-ui'), [], 'A missing directory is fine')

    def test_the_original_backup_survives_pruning(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            root = home / '.local/state/badi/editor-backups'
            names = [f'2026092{index}T120000.00000000{index}Z' for index in range(5)]
            for name in names:
                (root / name).mkdir(parents=True)
                (root / name / 'changes.json').write_text('{"pinned": false}')
            (root / names[0] / 'pinned').write_text('')
            pruned = badi_install.prune_backups(home, 'editors')
            self.assertEqual(sorted(path.name for path in pruned), sorted(names[1:2]))
            self.assertTrue((root / names[0]).is_dir(), 'The pre-Badi backup stays')
            self.assertTrue(all((root / name).is_dir() for name in names[2:]))

    def test_backup_names_order_by_creation_time(self):
        self.assertLess(badi_install.backup_time('1790447702770311229'),
                        badi_install.backup_time('20260927T170722.123456789Z'))
        for name in ('notes', '2026-09-27', '20261399-999999', '1234'):
            self.assertIsNone(badi_install.backup_time(name))


class ReceiptTests(unittest.TestCase):
    def test_unchanged_bytes_keep_their_recorded_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            notice = home / '.local/share/badi/notice'
            notice.parent.mkdir(parents=True)
            notice.write_text('notice')
            first = {'commit': 'a' * 40, 'dirty': False}
            badi_install.write_receipt(home, 'desktop', first, [notice])
            entry = json.loads(badi_install.receipt_path(home, 'desktop').read_text())['files']['.local/share/badi/notice']
            badi_install.write_receipt(home, 'desktop', {'commit': 'b' * 40, 'dirty': False}, [notice])
            receipt = json.loads(badi_install.receipt_path(home, 'desktop').read_text())
            self.assertEqual(receipt['files']['.local/share/badi/notice'], entry)
            self.assertEqual(receipt['source']['commit'], 'b' * 40)

    def test_restore_reloads_units_and_points_the_receipt_at_restored_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            unit = home / '.config/systemd/user/badi-broker.service'
            unit.parent.mkdir(parents=True)
            unit.write_text('old unit\n')
            broker = home / '.local/lib/badi/badi-broker'
            broker.parent.mkdir(parents=True)
            broker.write_bytes(b'old broker')
            installation = badi_install.Installation(home, 'desktop')
            installation.write(unit, b'new unit\n', 0o644)
            installation.write(broker, b'new broker', 0o755)
            badi_install.write_receipt(home, 'desktop', {'commit': 'a' * 40, 'dirty': False}, [unit, broker])
            calls = []
            real_run = subprocess.run

            def execute(command, **kwargs):
                if command[:3] == ['systemctl', '--user', 'daemon-reload']:
                    calls.append(command)
                    return subprocess.CompletedProcess(command, 0, '', '')
                return real_run(command, **kwargs)

            with patch.object(badi_install.subprocess, 'run', side_effect=execute), \
                 contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(badi_install.restore(installation.directory), [])
            self.assertEqual(unit.read_text(), 'old unit\n')
            self.assertEqual(broker.read_bytes(), b'old broker')
            self.assertEqual(calls, [['systemctl', '--user', 'daemon-reload']])
            receipt = json.loads(badi_install.receipt_path(home, 'desktop').read_text())
            self.assertEqual(receipt['files']['.local/lib/badi/badi-broker']['sha256'],
                             hashlib.sha256(b'old broker').hexdigest())
            self.assertEqual(receipt['files']['.config/systemd/user/badi-broker.service']['sha256'],
                             hashlib.sha256(b'old unit\n').hexdigest())


class RepairTests(unittest.TestCase):
    """Omarchy's refresh commands replace hyprland.lua and the browser flag files."""

    def install(self, home):
        config = home / '.config'
        (config / 'hypr').mkdir(parents=True)
        (config / 'hypr/badi.lua').write_text('-- Badi layer rule\n')
        files = {
            'hypr/hyprland.lua': 'require("default")\n' + badi_install.HYPR_REQUIRE + '\n',
            'chromium-flags.conf': '--ozone-platform=wayland\n' + badi_install.OBSERVED_FLAG + '\n',
            'brave-origin-flags.conf': badi_install.OBSERVED_FLAG + '\n',
            'code-flags.conf': badi_install.OBSERVED_FLAG + '\n',
        }
        for name, text in files.items():
            (config / name).write_text(text)
        badi_install.write_receipt(home, 'desktop', {'commit': 'a' * 40, 'dirty': False},
                                   [config / name for name in files])
        return config

    def test_only_badi_lines_that_a_refresh_dropped_come_back(self):
        with tempfile.TemporaryDirectory() as temporary, \
             patch.dict(os.environ, {'XDG_CONFIG_HOME': str(Path(temporary) / '.config')}):
            home = Path(temporary)
            config = self.install(home)
            self.assertEqual(badi_install.dropped_lines(home), [])
            # What omarchy-refresh-hyprland and omarchy-refresh-chromium leave.
            (config / 'hypr/hyprland.lua').write_text('require("default")\n')
            (config / 'chromium-flags.conf').write_text('--ozone-platform=wayland\n')
            (config / 'code-flags.conf').write_text('# the user removed it\n')
            (config / 'brave-origin-flags.conf').write_text('--disable-renderer-accessibility\n')
            (config / 'cursor-flags.conf').write_text('--never-installed\n')
            names = [name for name, _, _ in badi_install.dropped_lines(home)]
            self.assertEqual(sorted(names), ['chromium-flags.conf', 'code-flags.conf', 'hypr/hyprland.lua'],
                             'a conflicting flag and a file the install never changed are left alone')
            self.assertEqual(sorted(badi_install.repair(home, omarchy_only=True)),
                             ['chromium-flags.conf', 'hypr/hyprland.lua'])
            self.assertTrue((config / 'hypr/hyprland.lua').read_text().startswith('require("default")\n'))
            self.assertIn(badi_install.HYPR_REQUIRE, (config / 'hypr/hyprland.lua').read_text())
            self.assertTrue((config / 'chromium-flags.conf').read_text().startswith('--ozone-platform=wayland\n'))
            self.assertEqual((config / 'code-flags.conf').read_text(), '# the user removed it\n',
                             "Omarchy's hooks never touch a file Omarchy does not replace")
            self.assertEqual(badi_install.repair(home), ['code-flags.conf'])
            self.assertEqual(badi_install.repair(home), [], 'a second repair changes nothing')
            self.assertEqual((config / 'brave-origin-flags.conf').read_text(), '--disable-renderer-accessibility\n')

    def test_no_receipt_or_no_layer_rule_repairs_nothing(self):
        with tempfile.TemporaryDirectory() as temporary, \
             patch.dict(os.environ, {'XDG_CONFIG_HOME': str(Path(temporary) / '.config')}):
            home = Path(temporary)
            self.assertEqual(badi_install.dropped_lines(home), [])
            config = self.install(home)
            (config / 'hypr/badi.lua').unlink()
            (config / 'hypr/hyprland.lua').write_text('require("default")\n')
            self.assertEqual(badi_install.dropped_lines(home), [], 'without Badi\'s layer rule there is nothing to load')


class ModelTermsTests(unittest.TestCase):
    def test_terms_are_named_and_each_acceptance_is_recorded_privately(self):
        import datetime
        self.assertIsNone(badi_install.model_terms({'license': 'Apache-2.0'}))
        self.assertIn('https://ai.google.dev/gemma/terms', badi_install.model_terms({'license': 'Gemma'}))
        self.assertIn('Llama 3.2 Community License', badi_install.model_terms({'license': 'Llama-3.2-Community'}))
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            self.assertIsNone(badi_install.record_model_terms(home, {'id': 'qwen3-1.7b', 'license': 'Apache-2.0'}))
            moment = datetime.datetime(2026, 10, 20, 12, 0, tzinfo=datetime.timezone.utc)
            path = badi_install.record_model_terms(home, {'id': 'gemma-3-1b', 'license': 'Gemma'}, now=moment)
            badi_install.record_model_terms(home, {'id': 'llama-3.2-1b', 'license': 'Llama-3.2-Community'}, now=moment)
            document = json.loads(path.read_text())
            self.assertEqual(document['schema'], 'badi.model-terms.v1')
            self.assertEqual([entry['model'] for entry in document['accepted']], ['gemma-3-1b', 'llama-3.2-1b'])
            self.assertEqual(document['accepted'][0], {'model': 'gemma-3-1b', 'license': 'Gemma',
                                                       'terms': 'https://ai.google.dev/gemma/terms',
                                                       'accepted_at': '2026-10-20T12:00:00Z'})
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            path.write_text('not json')
            badi_install.record_model_terms(home, {'id': 'gemma-3-1b', 'license': 'Gemma'}, now=moment)
            self.assertEqual(len(json.loads(path.read_text())['accepted']), 1, 'an unreadable record starts again')


class DownloadModelTests(unittest.TestCase):
    MODEL = {'name': 'Test', 'repository': 'example/test', 'revision': 'r', 'filename': 'test.gguf'}

    class Response(io.BytesIO):
        def __init__(self, data, fail_after=None):
            super().__init__(data)
            self.fail_after = fail_after

        def read(self, size=-1):
            if self.fail_after is not None and self.tell() >= self.fail_after:
                raise OSError('connection reset')
            return super().read(min(size, 4))

    def download(self, directory, data, response):
        model = {**self.MODEL, 'download_bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
        with patch('urllib.request.urlopen', return_value=response):
            return badi_install.download_model(model, directory)

    def test_a_checked_download_becomes_visible_whole(self):
        with tempfile.TemporaryDirectory() as temporary:
            data = b'weights-0123456789'
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                target = self.download(temporary, data, self.Response(data))
            self.assertEqual(output.getvalue(), 'Downloading Test (0.00 GB) from huggingface.co/example/test\n'
                             'test.gguf matches its pinned size and SHA-256.\n',
                             'without a terminal a log still shows the download')
            self.assertEqual(target.read_bytes(), data)
            self.assertEqual(target.stat().st_mode & 0o777, 0o600)
            self.assertEqual(os.listdir(temporary), ['test.gguf'])

    def test_a_wrong_hash_or_a_broken_connection_leaves_nothing(self):
        with tempfile.TemporaryDirectory() as temporary:
            data = b'weights-0123456789'
            with self.assertRaisesRegex(RuntimeError, 'pinned size and hash'):
                self.download(temporary, data, self.Response(b'tampered-012345678'))
            self.assertEqual(os.listdir(temporary), [])
            with self.assertRaises(OSError):
                self.download(temporary, data, self.Response(data, fail_after=8))
            self.assertEqual(os.listdir(temporary), [], 'no .part file after an interrupted download')


class RunInstallerTests(unittest.TestCase):
    def run_failing(self, error):
        def main():
            raise error
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr), self.assertRaises(SystemExit) as stopped:
            badi_install.run_installer(main)
        return stopped.exception.code, stderr.getvalue()

    def test_known_failures_end_with_one_line_and_others_propagate(self):
        self.assertEqual(self.run_failing(RuntimeError('No writing model was downloaded.')),
                         (1, 'No writing model was downloaded.\n'))
        code, message = self.run_failing(subprocess.CalledProcessError(101, ['cargo', 'build']))
        self.assertEqual((code, message), (1, 'cargo build failed with exit status 101; see its output above.\n'))
        self.assertEqual(self.run_failing(KeyboardInterrupt())[0], 130)
        with self.assertRaises(ValueError):
            badi_install.run_installer(lambda: (_ for _ in ()).throw(ValueError('a bug')))


if __name__ == '__main__':
    unittest.main()
