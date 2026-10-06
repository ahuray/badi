"""Shared installer and CLI helpers: backups, receipts, atomic writes, systemd queries."""
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

    def test_prior_setting_notes_create_the_backup_on_demand(self):
        installation = badi_install.Installation(self.home, 'desktop')
        note = installation.note('accessibility-setting.json', {'bus_enabled': False})
        self.assertEqual(note.parent, installation.directory)
        self.assertEqual(note.stat().st_mode & 0o777, 0o600)
        self.assertEqual(self.changes(installation)['changes'], [])


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


if __name__ == '__main__':
    unittest.main()
