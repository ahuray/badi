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
import unittest.mock
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
            rc.chmod(0o600)
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
            self.assertEqual(rc.stat().st_mode & 0o777, 0o600)
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

    def test_neovim_installs_a_start_package_and_a_runnable_bridge(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            environment = {key: value for key, value in os.environ.items()
                           if key not in ('XDG_DATA_HOME', 'XDG_CONFIG_HOME', 'NVIM_APPNAME')}
            with patch.object(installer.Path, 'home', return_value=home), \
                 patch.dict(os.environ, environment, clear=True), \
                 patch('sys.argv', ['install-editors.py', '--neovim']), \
                 contextlib.redirect_stdout(io.StringIO()) as output:
                installer.main()
                installed = {path: path.stat().st_mtime_ns for root in ('.local/lib', '.local/share')
                             for path in (home / root).rglob('*') if path.is_file()}
                installer.main()
            package = home / '.local/share/nvim/site/pack/badi/start/badi'
            for path in installer.NEOVIM_PLUGIN:
                self.assertEqual((package / path).read_bytes(),
                                 (installer.ROOT / 'adapters/neovim' / path).read_bytes())
                self.assertEqual((package / path).stat().st_mode & 0o777, 0o644)
            self.assertIn('badi app nvim on', output.getvalue())
            self.assertIn('already current; no backup was needed', output.getvalue())
            self.assertEqual({path: path.stat().st_mtime_ns for path in installed}, installed,
                             'Identical files are not rewritten')
            receipt = json.loads((home / '.local/state/badi/receipts/editors.json').read_text())
            self.assertIn('.local/share/nvim/site/pack/badi/start/badi/lua/badi/init.lua', receipt['files'])
            self.assertIn('.local/lib/badi/editors/neovim/bridge.mjs', receipt['files'])
            # The installed bridge resolves its imports outside the checkout.
            bridge = home / '.local/lib/badi/editors/neovim/bridge.mjs'
            result = subprocess.run(['node', str(bridge)], input='{"id":1,"op":"status"}\n', capture_output=True,
                                    text=True, env={**os.environ, 'XDG_RUNTIME_DIR': str(home / 'no-broker')},
                                    timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stderr, '')
            replies = [json.loads(line) for line in result.stdout.splitlines()]
            self.assertIn({'id': 1, 'ok': True, 'connected': False, 'allowed': False, 'automatic': False}, replies)

    def test_neovim_follows_xdg_data_home_and_refuses_linked_or_foreign_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary) / 'home'
            data = Path(temporary) / 'data'
            home.mkdir()
            with patch.object(installer.Path, 'home', return_value=home), \
                 patch.dict(os.environ, {'XDG_DATA_HOME': str(data), 'NVIM_APPNAME': 'writer'}), \
                 patch('sys.argv', ['install-editors.py', '--neovim']), \
                 contextlib.redirect_stdout(io.StringIO()):
                installer.main()
            self.assertTrue((data / 'writer/site/pack/badi/start/badi/plugin/badi.lua').is_file())
            for appname in ('../escape', 'a/b', '..'):
                with patch.dict(os.environ, {'NVIM_APPNAME': appname}), \
                     self.assertRaisesRegex(RuntimeError, 'NVIM_APPNAME'):
                    installer.neovim_package(home)

        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            checkout = home / 'checkout'
            checkout.mkdir()
            start = home / '.local/share/nvim/site/pack/badi/start'
            start.mkdir(parents=True)
            (start / 'badi').symlink_to(checkout)
            with patch.object(installer.Path, 'home', return_value=home), \
                 patch.dict(os.environ, {'NVIM_APPNAME': '', 'XDG_DATA_HOME': ''}), \
                 patch('sys.argv', ['install-editors.py', '--neovim']):
                with self.assertRaisesRegex(RuntimeError, 'symlink'):
                    installer.main()
            self.assertEqual(list(checkout.iterdir()), [], 'Nothing is written through the link')
            (start / 'badi').unlink()
            (start / 'badi').mkdir()
            (start / 'badi/README').write_text('someone else\n')
            with patch.object(installer.Path, 'home', return_value=home), \
                 patch.dict(os.environ, {'NVIM_APPNAME': '', 'XDG_DATA_HOME': ''}), \
                 patch('sys.argv', ['install-editors.py', '--neovim']):
                with self.assertRaisesRegex(RuntimeError, 'unexpected package'):
                    installer.main()
            self.assertEqual(sorted(path.name for path in (start / 'badi').iterdir()), ['README'])

    def test_neovim_lazy_gets_a_spec_file_and_plain_configs_do_not(self):
        clean = {key: value for key, value in os.environ.items()
                 if key not in ('XDG_DATA_HOME', 'XDG_CONFIG_HOME', 'NVIM_APPNAME')}
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            (home / '.local/share/nvim/lazy/lazy.nvim').mkdir(parents=True)
            with patch.object(installer.Path, 'home', return_value=home), \
                 patch.dict(os.environ, clean, clear=True), \
                 patch('sys.argv', ['install-editors.py', '--neovim']), \
                 contextlib.redirect_stdout(io.StringIO()) as output:
                installer.main()
                installer.main()
            spec = home / '.config/nvim/lua/plugins/badi.lua'
            package = home / '.local/share/nvim/site/pack/badi/start/badi'
            self.assertEqual(spec.read_bytes(), installer.neovim_lazy_spec(package))
            self.assertIn(f'dir = "{package}"', spec.read_text())
            self.assertIn('lazy = false', spec.read_text())
            self.assertIn('lazy.nvim detected', output.getvalue())
            self.assertIn('already current; no backup was needed', output.getvalue())
            receipt = json.loads((home / '.local/state/badi/receipts/editors.json').read_text())
            self.assertIn('.config/nvim/lua/plugins/badi.lua', receipt['files'])
            # Our own earlier spec (marker, stale path) is replaced, backed up.
            spec.write_bytes(b'-- badi:lazy-spec v1\nreturn { dir = "/old", lazy = false }\n')
            with patch.object(installer.Path, 'home', return_value=home), \
                 patch.dict(os.environ, clean, clear=True), \
                 patch('sys.argv', ['install-editors.py', '--neovim']), \
                 contextlib.redirect_stdout(io.StringIO()):
                installer.main()
            self.assertEqual(spec.read_bytes(), installer.neovim_lazy_spec(package))
            backups = home / '.local/state/badi/editor-backups'
            saved = [path for path in backups.rglob('badi.lua') if b'/old' in path.read_bytes()]
            self.assertEqual(len(saved), 1, 'The replaced spec is backed up for restore')
            # A spec the user wrote is never overwritten.
            spec.write_text('return { "mine" }\n')
            with patch.object(installer.Path, 'home', return_value=home), \
                 patch.dict(os.environ, clean, clear=True), \
                 patch('sys.argv', ['install-editors.py', '--neovim']):
                with self.assertRaisesRegex(RuntimeError, 'will not overwrite'):
                    installer.main()
            self.assertEqual(spec.read_text(), 'return { "mine" }\n')

        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            with patch.object(installer.Path, 'home', return_value=home), \
                 patch.dict(os.environ, clean, clear=True), \
                 patch('sys.argv', ['install-editors.py', '--neovim']), \
                 contextlib.redirect_stdout(io.StringIO()) as output:
                installer.main()
            self.assertFalse((home / '.config/nvim/lua/plugins/badi.lua').exists())
            self.assertNotIn('lazy.nvim detected', output.getvalue())

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

    def test_libreoffice_install_builds_records_and_registers_once_closed(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            package = home / installer.LIBREOFFICE_PACKAGE
            with patch.object(installer.Path, 'home', return_value=home), \
                 patch.object(installer.shutil, 'which', return_value='/usr/bin/unopkg'), \
                 patch.object(installer, 'libreoffice_running', return_value=False), \
                 patch.object(installer, 'register_libreoffice_extension') as register, \
                 patch('sys.argv', ['install-editors.py', '--libreoffice']), \
                 contextlib.redirect_stdout(io.StringIO()) as output:
                installer.main()
                built = package.read_bytes()
                installer.main()
            # The real build: a reproducible archive of the extension source.
            self.assertEqual(built[:2], b'PK')
            self.assertEqual(package.stat().st_mode & 0o777, 0o644)
            self.assertEqual(register.call_args_list, [unittest.mock.call('/usr/bin/unopkg', package)] * 2)
            self.assertIn('badi app libreoffice-writer on', output.getvalue())
            self.assertIn('already current; no backup was needed', output.getvalue())
            receipt = json.loads((home / '.local/state/badi/receipts/editors.json').read_text())
            entry = receipt['files'][installer.LIBREOFFICE_PACKAGE]
            self.assertEqual(entry['sha256'], hashlib.sha256(built).hexdigest())
            # The first run's backup keeps the removal note; an unchanged rerun files it under notes/.
            [backup] = (entry for entry in (home / '.local/state/badi/editor-backups').iterdir()
                        if entry.name != 'notes')
            note = json.loads((backup / 'libreoffice-extension.json').read_text())
            self.assertEqual(note['remove'], 'unopkg remove io.github.ahuray.badi.libreoffice')

    def test_libreoffice_install_refuses_while_libreoffice_runs_or_is_missing(self):
        for running, which, message in ((True, '/usr/bin/unopkg', 'Close LibreOffice'),
                                        (False, None, 'unopkg was not found')):
            with tempfile.TemporaryDirectory() as temporary:
                home = Path(temporary)
                with patch.object(installer.Path, 'home', return_value=home), \
                     patch.object(installer.shutil, 'which', return_value=which), \
                     patch.object(installer, 'libreoffice_running', return_value=running), \
                     patch.object(installer, 'build_libreoffice_extension') as build, \
                     patch.object(installer, 'register_libreoffice_extension') as register, \
                     patch('sys.argv', ['install-editors.py', '--libreoffice']):
                    with self.assertRaisesRegex(RuntimeError, message):
                        installer.main()
                build.assert_not_called()
                register.assert_not_called()
                self.assertFalse((home / '.local').exists())


if __name__ == '__main__':
    unittest.main()
