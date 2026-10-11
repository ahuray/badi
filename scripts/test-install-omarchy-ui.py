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
                 patch.dict(os.environ, {'XDG_CONFIG_HOME': str(home / '.config')}), \
                 patch.object(installer, 'ipc', side_effect=lambda *call: answers[call]), \
                 patch.object(installer.subprocess, 'run') as run, \
                 patch('sys.argv', ['install-omarchy-ui.py']), \
                 contextlib.redirect_stdout(io.StringIO()):
                installer.main()
            # Only the source gate and the supported shell restart run; no real shell is touched.
            self.assertEqual([call.args[0][:2] for call in run.call_args_list if call.args[0][0] != 'git'],
                             [['bash', 'ui/omarchy-plugin/tests/check-source.sh'], ['omarchy', 'restart']])
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
                 patch.dict(os.environ, {'XDG_CONFIG_HOME': str(config)}), \
                 patch.object(installer, 'ipc', side_effect=lambda *call: answers[call]), \
                 patch.object(installer.subprocess, 'run') as run, \
                 patch('sys.argv', ['install-omarchy-ui.py']), \
                 contextlib.redirect_stdout(io.StringIO()):
                installer.main()
            self.assertEqual([call.args[0][:2] for call in run.call_args_list if call.args[0][0] != 'git'],
                             [['bash', 'ui/omarchy-plugin/tests/check-source.sh'],
                              ['omarchy', 'plugin'],
                              ['omarchy', 'restart']])
            self.assertEqual(run.call_args_list[1].args[0][2:], ['enable', 'io.github.ahuray.badi'])
            self.assertCountEqual([path.name for path in target.iterdir()], installer.FILES)
            self.assertTrue((home / '.local/state/badi/receipts/omarchy-ui.json').is_file())


if __name__ == '__main__':
    unittest.main()
