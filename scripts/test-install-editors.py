"""Keep editor installation local, idempotent and recoverable."""
import importlib.util
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
    def test_native_host_reaches_installed_browser_variants(self):
        with tempfile.TemporaryDirectory() as temporary:
            config = Path(temporary)
            for browser in ('BraveSoftware/Brave-Origin', 'BraveSoftware/Brave-Browser', 'google-chrome'):
                (config / browser).mkdir(parents=True)
            targets = [str(path.relative_to(config)) for path in installer.native_host_directories(config)]
            self.assertCountEqual(targets, [f'{browser}/NativeMessagingHosts' for browser in
                ('chromium', 'google-chrome', 'BraveSoftware/Brave-Origin', 'BraveSoftware/Brave-Browser')])

    def test_bash_install_preserves_customizations_and_is_idempotent(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            rc = home / '.bashrc'
            original = '# My shell\n[[ $- == *i* ]] || return\nalias mine="echo local"\n'
            rc.write_text(original)
            with patch.object(installer.Path, 'home', return_value=home), \
                 patch.object(installer, 'build_shell_preview', return_value=b'test display builtin') as build, \
                 patch('sys.argv', ['install-editors.py', '--bash']):
                installer.main()
                first = rc.read_text()
                installer.main()
            self.assertEqual(build.call_count, 2)
            module = home / '.local/lib/badi/editors/shell/badi-preview.so'
            self.assertEqual(module.read_bytes(), b'test display builtin')
            self.assertEqual(module.stat().st_mode & 0o777, 0o755)
            self.assertTrue(first.startswith(original))
            self.assertEqual(first, rc.read_text())
            self.assertEqual(first.count('source "$HOME/.local/lib/badi/editors/shell/badi.bash"'), 1)
            backups = list((home / '.local/state/badi/editor-backups').iterdir())
            originals = []
            for backup in backups:
                for entry in json.loads((backup / 'changes.json').read_text()):
                    if entry['path'] == str(rc):
                        originals.append((backup / entry['backup']).read_text())
            self.assertEqual(originals, [original])

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
