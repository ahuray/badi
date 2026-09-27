"""Shared installer and CLI helpers: atomic writes, systemd queries, polling, grants."""
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


if __name__ == '__main__':
    unittest.main()
