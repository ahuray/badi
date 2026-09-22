"""A missing or partial lock status must never authorize a shell reload."""

import importlib.util
from pathlib import Path
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


if __name__ == '__main__':
    unittest.main()
