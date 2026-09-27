"""Keep editor installation local, idempotent and recoverable."""
import contextlib
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('install_editors', Path(__file__).with_name('install-editors.py'))
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class InstallEditorsTests(unittest.TestCase):
    def test_bash_install_preserves_customizations_and_is_idempotent(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            rc = home / '.bashrc'
            original = '# My shell\n[[ $- == *i* ]] || return\nalias mine="echo local"\n'
            rc.write_text(original)
            backups = home / '.local/state/badi/editor-backups'
            with patch.object(installer.Path, 'home', return_value=home), \
                 patch.object(installer, 'build_shell_preview', return_value=b'test display builtin') as build, \
                 patch('sys.argv', ['install-editors.py', '--bash']), \
                 contextlib.redirect_stdout(io.StringIO()) as output:
                installer.main()
                first = rc.read_text()
                [backup] = backups.iterdir()
                installed = {path: path.stat().st_mtime_ns for path in (home / '.local/lib/badi/editors').rglob('*')}
                installer.main()
            self.assertEqual(build.call_count, 2)
            module = home / '.local/lib/badi/editors/shell/badi-preview.so'
            self.assertEqual(module.read_bytes(), b'test display builtin')
            self.assertEqual(module.stat().st_mode & 0o777, 0o755)
            self.assertTrue(first.startswith(original))
            self.assertEqual(first, rc.read_text())
            self.assertEqual(first.count('source "$HOME/.local/lib/badi/editors/shell/badi.bash"'), 1)
            self.assertEqual(list(backups.iterdir()), [backup], 'An unchanged reinstall makes no backup')
            self.assertEqual({path: path.stat().st_mtime_ns for path in installed}, installed,
                             'Identical files are not rewritten')
            self.assertIn('already current; no backup was needed', output.getvalue())
            changes = json.loads((backup / 'changes.json').read_text())['changes']
            rc_change, = (entry for entry in changes if entry['path'] == '.bashrc')
            self.assertEqual((backup / rc_change['saved']).read_text(), original)
            self.assertTrue(all(entry['action'] == 'create' for entry in changes if entry['path'] != '.bashrc'))
            receipt = json.loads((home / '.local/state/badi/receipts/editors.json').read_text())
            self.assertEqual(receipt['installer'], 'editors')
            preview = receipt['files']['.local/lib/badi/editors/shell/badi-preview.so']
            self.assertEqual(preview['sha256'], hashlib.sha256(b'test display builtin').hexdigest())
            self.assertIn('.bashrc', receipt['files'])
            # Both runs use this checkout's actual Git identity (or explicit unknown).
            self.assertRegex(receipt['source']['commit'], r'^([0-9a-f]{40}|unknown)$')
            self.assertEqual(preview['commit'], receipt['source']['commit'])

            # Execute the installed entrypoint outside the checkout: a missing
            # imported helper must fail this test even though Bash syntax passes.
            bridge = home / '.local/lib/badi/editors/shell/bridge.mjs'
            result = subprocess.run(['node', str(bridge)], input='', capture_output=True, text=True,
                                    env={**os.environ, 'XDG_RUNTIME_DIR': str(home / 'no-broker')}, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), 'ERROR model_offline_run_badi_doctor')
            self.assertEqual(result.stderr, '')

    def test_failed_native_build_preserves_shell_and_installation(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            rc = home / '.bashrc'
            rc.write_text('# keep\n')
            with patch.object(installer.Path, 'home', return_value=home), \
                 patch.object(installer, 'build_shell_preview', side_effect=RuntimeError('compiler failed')), \
                 patch('sys.argv', ['install-editors.py', '--bash']):
                with self.assertRaisesRegex(RuntimeError, 'compiler failed'):
                    installer.main()
            self.assertEqual(rc.read_text(), '# keep\n')
            self.assertFalse((home / '.local').exists())

    def test_existing_bashrc_symlink_is_not_replaced(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            original = home / 'managed-rc'
            original.write_text('echo keep\n')
            (home / '.bashrc').symlink_to(original)
            with patch.object(installer.Path, 'home', return_value=home), \
                 patch.object(installer, 'build_shell_preview', return_value=b'test display builtin'), \
                 patch('sys.argv', ['install-editors.py', '--bash']):
                with self.assertRaisesRegex(RuntimeError, 'symlink'):
                    installer.main()
            self.assertTrue((home / '.bashrc').is_symlink())
            self.assertEqual(original.read_text(), 'echo keep\n')


if __name__ == '__main__':
    unittest.main()
