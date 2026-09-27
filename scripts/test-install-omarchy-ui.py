"""A missing or partial lock status must never authorize a shell reload."""

import importlib.util
import json
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
    def test_update_replaces_current_files_and_backs_up_then_removes_obsolete_ones(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            target = home / '.config/omarchy/plugins/io.github.ahuray.badi'
            target.mkdir(parents=True)
            (target / 'manifest.json').write_text(json.dumps({'id': 'io.github.ahuray.badi', 'kinds': ['panel']}))
            (target / 'Panel.qml').write_text('legacy panel')
            unlocked = dict.fromkeys(('locked', 'secure', 'requested', 'pending', 'sessionLocked'), False)
            answers = {('lock', 'status'): unlocked, ('badi-writing', 'state'): {'page': 0, 'service': {}}}
            with patch.object(installer.Path, 'home', return_value=home), \
                 patch.object(installer, 'ipc', side_effect=lambda *call: answers[call]), \
                 patch.object(installer.subprocess, 'run') as run, \
                 patch('sys.argv', ['install-omarchy-ui.py']):
                installer.main()
            # Only the source gate and the supported shell restart run; no real shell is touched.
            self.assertEqual([call.args[0][:2] for call in run.call_args_list],
                             [['bash', 'ui/omarchy-plugin/tests/check-source.sh'], ['omarchy', 'restart']])
            self.assertCountEqual([path.name for path in target.iterdir()], installer.FILES)
            for name in installer.FILES:
                self.assertEqual((target / name).read_bytes(),
                                 (installer.ROOT / 'ui/omarchy-plugin' / name).read_bytes())
            [backup] = (home / '.local/state/badi/ui-backups').iterdir()
            changes = {entry['path']: entry for entry in json.loads((backup / 'changes.json').read_text())['changes']}
            plugin = '.config/omarchy/plugins/io.github.ahuray.badi/'
            self.assertEqual(changes[plugin + 'Panel.qml']['action'], 'remove')
            self.assertEqual((backup / changes[plugin + 'Panel.qml']['saved']).read_text(), 'legacy panel')
            self.assertIn('"panel"', (backup / changes[plugin + 'manifest.json']['saved']).read_text())


if __name__ == '__main__':
    unittest.main()
