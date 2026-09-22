"""Behavior checks for session discovery and exact-session desktop controls."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('desktop', Path(__file__).with_name('badi-desktop.py'))
desktop = importlib.util.module_from_spec(spec)
spec.loader.exec_module(desktop)


class DesktopTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name) / 'native-trials'
        self.directory.mkdir()

    def register(self, name):
        endpoint = self.directory / (name + '.sock')
        listener = socket.socket(socket.AF_UNIX)
        listener.bind(str(endpoint))
        endpoint.chmod(0o600)
        self.addCleanup(listener.close)
        entry = {'id': name, 'app': 'xournalpp', 'socket': str(endpoint)}
        receipt = self.directory / (name + '.json')
        receipt.write_text(json.dumps(entry))
        receipt.chmod(0o600)
        return entry, receipt

    def test_discovery_excludes_stale_public_and_symlink_receipts(self):
        live, _ = self.register('live')
        _, public = self.register('public')
        public.chmod(0o644)
        stale, _ = self.register('stale')
        Path(stale['socket']).unlink()
        _, linked = self.register('linked')
        saved = linked.with_suffix('.saved')
        linked.rename(saved)
        linked.symlink_to(saved)
        (self.directory / 'broken.json').write_text('{')
        self.assertEqual(desktop.sessions(self.directory), [live])

    def test_mutations_need_an_explicit_live_session(self):
        self.register('live')
        with patch.object(desktop, 'registry_directory', return_value=self.directory):
            with self.assertRaisesRegex(RuntimeError, 'Refresh'):
                desktop.main(['ctl', 'pause', 'on'])
            with self.assertRaisesRegex(RuntimeError, 'offline'):
                desktop.main(['ctl', '--trial', 'closed', 'pause', 'on'])

    def test_private_desktop_socket_is_preferred_and_needs_no_receipt(self):
        trial, _ = self.register('trial')
        endpoint = self.directory.parent / 'broker.sock'
        with socket.socket(socket.AF_UNIX) as listener:
            listener.bind(str(endpoint))
            endpoint.chmod(0o600)
            entries = desktop.sessions(self.directory)
            self.assertEqual([entry['id'] for entry in entries], ['desktop', 'trial'])
            self.assertEqual(entries[0]['socket'], str(endpoint))
            endpoint.chmod(0o666)
            self.assertEqual(desktop.sessions(self.directory), [trial])

    def test_launcher_uses_normal_desktop_service_and_toolkit_modules(self):
        with patch.object(desktop.subprocess, 'run') as command:
            desktop.main(['launch', 'xournalpp'])
            self.assertEqual(command.call_args_list[0].args[0],
                             ['systemctl', '--user', 'start', 'badi-broker.service'])
            launch = command.call_args_list[1].args[0]
            self.assertIn('--setenv=GTK_IM_MODULE=fcitx', launch)
            self.assertEqual(launch[-1], 'xournalpp')

    def test_mutation_keeps_selected_session_when_a_newer_one_opens(self):
        first, receipt = self.register('first')
        os.utime(receipt, ns=(1, 1))
        self.register('newer')
        with patch.object(desktop, 'registry_directory', return_value=self.directory), \
             patch.object(desktop.subprocess, 'run') as command, contextlib.redirect_stdout(io.StringIO()):
            command.return_value.returncode = 0
            command.return_value.stdout = '{}'
            desktop.main(['ctl', '--trial', 'first', 'pause', 'on'])
            self.assertEqual(command.call_args.args[0][1:], ['--socket', first['socket'], 'pause', 'on'])

    def test_pause_persists_with_compare_and_swap(self):
        settings = {'schema': 'badi.settings.v2', 'revision': 7, 'paused': False, 'subjects': []}
        with patch.object(desktop, 'control', side_effect=[json.dumps(settings), '{}']) as command, \
             contextlib.redirect_stdout(io.StringIO()):
            desktop.main(['pause'])
        request = command.call_args.args[0]
        self.assertEqual(request[:5], ['settings', 'replace', '--if-revision', '7', '--json'])
        document = json.loads(request[5])
        self.assertEqual(document['revision'], 8)
        self.assertTrue(document['paused'])

    def test_conflicts_are_not_retried_or_overwritten(self):
        settings = {'schema': 'badi.settings.v2', 'revision': 7, 'paused': False, 'subjects': []}
        with patch.object(desktop, 'control', side_effect=[json.dumps(settings), RuntimeError('settings_conflict')]) as command:
            with self.assertRaisesRegex(RuntimeError, 'settings_conflict'):
                desktop.main(['pause'])
        self.assertEqual(command.call_count, 2)

    def test_app_toggle_preserves_other_subjects_and_canonical_order(self):
        browser = {'identity': {'kind': 'browser_origin', 'adapter': 'chromium', 'scheme': 'https', 'host': 'dillinger.io', 'port': 443},
                   'permissions': {'suggest': 'block'}}
        document = {'subjects': [browser]}
        desktop.set_app(document, 'omawrite', True)
        desktop.set_app(document, 'xournalpp', True)
        desktop.set_app(document, 'omawrite', False)
        self.assertEqual(document['subjects'][0], browser)
        self.assertEqual([entry['identity'].get('app_id') for entry in document['subjects'][1:]],
                         ['com.github.xournalpp.xournalpp', 'omawrite'])
        permissions = document['subjects'][2]['permissions']
        self.assertEqual(permissions, {'context_read': 'block', 'display': 'block', 'suggest': 'block',
                                      'learn': 'block', 'retention': {'mode': 'none'}})

    def test_disabling_startup_does_not_stop_running_model(self):
        with patch.object(desktop.subprocess, 'run') as command, \
             patch.object(desktop, 'service_state', return_value={'autostart': False}), \
             contextlib.redirect_stdout(io.StringIO()):
            desktop.main(['autostart', 'off'])
            self.assertEqual(command.call_args.args[0], ['systemctl', '--user', 'disable', 'badi-broker.service'])

    def test_native_grants_accept_canonical_ids_without_a_compiled_list(self):
        document = {'subjects': []}
        desktop.set_app(document, 'org.gnome.texteditor', True)
        self.assertEqual(document['subjects'][0]['identity']['app_id'], 'org.gnome.texteditor')
        for app in ('A window title', '', 'x' * 129, '../omawrite'):
            with self.assertRaisesRegex(RuntimeError, 'canonical'):
                desktop.set_app(document, app, True)

    def test_editor_grants_use_the_buffer_owner_adapter(self):
        document = {'subjects': []}
        desktop.set_app(document, 'obsidian', True)
        desktop.set_app(document, 'bash', True)
        self.assertEqual([item['identity']['adapter'] for item in document['subjects']], ['obsidian', 'shell'])

    def test_site_grants_preserve_native_settings_and_exact_ports(self):
        document = {'subjects': []}
        desktop.set_app(document, 'omawrite', True)
        native = document['subjects'][0].copy()
        desktop.set_site(document, 'https://example.com:8443', True)
        desktop.set_site(document, 'https://example.com', False)
        self.assertEqual(document['subjects'][-1], native)
        self.assertEqual([item['identity']['port'] for item in document['subjects'][:-1]], [443, 8443])
        for value in ['https://user:secret@example.com', 'file:///tmp/note', 'https://example.com/private', 'https://example.com?q=1']:
            with self.assertRaises(RuntimeError):
                desktop.set_site(document, value, True)

    def test_service_state_handles_offline_broker(self):
        with patch.object(desktop.subprocess, 'run') as command:
            command.return_value.returncode = 0
            command.return_value.stdout = 'LoadState=loaded\nActiveState=inactive\nSubState=dead\nUnitFileState=disabled\n'
            self.assertEqual(desktop.service_state(), {'loaded': True, 'active': 'inactive', 'state': 'dead', 'autostart': False})

    def test_atomically_replaced_native_addon_is_reported_as_loaded_but_pending(self):
        with patch.object(desktop.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, 'ActiveState=active\nMainPID=123\n', '')), \
             patch.object(desktop.Path, 'read_text', return_value='metadata /private/libbadi-fcitx5.so (deleted)\n'):
            self.assertEqual(desktop.native_state(), {'active': 'active', 'addon_loaded': True, 'addon_update_pending': True})

    def test_startup_diagnosis_reports_missing_model_without_exposing_logs(self):
        with patch.object(desktop.subprocess, 'run') as command:
            command.side_effect = [
                subprocess.CompletedProcess([], 0, 'a' * 32 + '\n', ''),
                subprocess.CompletedProcess([], 0,
                    'unrelated confidential text\nerror_code=local_model: writing model/runtime not installed (Qwen3-0.6B-Q8_0.gguf); run badictl models writing for the pinned download plan\n', ''),
            ]
            problem = desktop.startup_problem()
            self.assertEqual(problem['code'], 'model_not_installed')
            self.assertIn('Qwen3-0.6B-Q8_0.gguf', problem['message'])
            self.assertNotIn('confidential', json.dumps(problem))
            self.assertIn('_SYSTEMD_INVOCATION_ID=' + 'a' * 32, command.call_args.args[0])

    def test_doctor_retains_broker_health_when_native_inspection_fails(self):
        service = {'loaded': True, 'active': 'active', 'state': 'running', 'autostart': True}
        broker = {'provider': 'local_model', 'paused': False, 'metrics': {'provider_calls': 3}}
        for failure in (RuntimeError('private subsystem error'), subprocess.TimeoutExpired('systemctl', 4)):
            with self.subTest(failure=type(failure).__name__), \
                 patch.object(desktop, 'service_state', return_value=service), \
                 patch.object(desktop, 'accessibility_state', return_value={'loaded': False, 'ready': False}), \
                 patch.object(desktop, 'native_state', side_effect=failure), \
                 patch.object(desktop, 'control', return_value=json.dumps(broker)):
                report = desktop.health_report()
                self.assertEqual(report['broker'], broker)
                self.assertIsNone(report['native']['addon_loaded'])
                self.assertEqual([item['code'] for item in report['problems']], ['native_status_unknown'])
                self.assertNotIn('private subsystem', json.dumps(report))

    def test_runtime_death_has_a_distinct_content_free_recovery_diagnosis(self):
        with patch.object(desktop.subprocess, 'run', side_effect=[
            subprocess.CompletedProcess([], 0, 'b' * 32 + '\n', ''),
            subprocess.CompletedProcess([], 0, 'private runtime detail\nerror_code=local_model: runtime_process_exited\n', ''),
        ]):
            problem = desktop.startup_problem()
            self.assertEqual(problem['code'], 'runtime_process_exited')
            self.assertIn('automatically', problem['action'])
            self.assertNotIn('private runtime', json.dumps(problem))

    def test_unreadable_native_maps_are_unknown_not_proof_of_a_missing_addon(self):
        with patch.object(desktop.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, 'ActiveState=active\nMainPID=123\n', '')), \
             patch.object(desktop.Path, 'read_text', side_effect=PermissionError('private maps detail')):
            native = desktop.native_state()
            self.assertIsNone(native['addon_loaded'])
            self.assertEqual(native['inspection_error'], 'native_process_uninspectable')
            self.assertNotIn('private maps detail', json.dumps(native))

    def test_startup_diagnosis_does_not_reuse_logs_without_an_invocation(self):
        with patch.object(desktop.subprocess, 'run') as command:
            command.return_value = subprocess.CompletedProcess([], 0, '\n', '')
            self.assertIsNone(desktop.startup_problem())
            self.assertEqual(command.call_count, 1)

    def test_startup_diagnosis_survives_unavailable_journal(self):
        with patch.object(desktop.subprocess, 'run', side_effect=subprocess.TimeoutExpired('journalctl', 3)):
            self.assertIsNone(desktop.startup_problem())

    def test_doctor_explains_failed_service_even_when_socket_error_is_opaque(self):
        output = io.StringIO()
        service = {'loaded': True, 'active': 'failed', 'state': 'failed', 'autostart': True}
        with patch.object(desktop, 'service_state', return_value=service), \
             patch.object(desktop, 'accessibility_state', return_value={'loaded': False, 'ready': False}), \
             patch.object(desktop, 'native_state', return_value={'active': 'active', 'addon_loaded': True}), \
             patch.object(desktop, 'control', side_effect=RuntimeError('error_code=frame')), \
             patch.object(desktop, 'startup_problem', return_value={'code': 'model_not_installed', 'message': 'Missing model', 'action': 'Provision the pinned model'}), \
             contextlib.redirect_stdout(output):
            with self.assertRaises(SystemExit) as stopped:
                desktop.main(['doctor'])
        self.assertEqual(stopped.exception.code, 1)
        report = json.loads(output.getvalue())
        self.assertEqual(report['problems'][0]['code'], 'model_not_installed')
        self.assertIn('service_failed', [problem['code'] for problem in report['problems']])

    def test_observer_readiness_is_bound_to_a_stable_service_process(self):
        running = {'loaded': True, 'active': 'active', 'process_id': 123}
        replacement = {**running, 'process_id': 456}
        with patch.object(desktop, 'observer_service_state', side_effect=[running, replacement]), \
             patch.object(desktop, 'observer_probe', return_value={'ready': True}) as probe, \
             patch.object(desktop.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, 'b true\n', '')):
            state = desktop.accessibility_state()
        self.assertFalse(state['ready'])
        self.assertEqual(state['error'], 'service_changed')
        self.assertTrue(state['bus_enabled'])
        self.assertEqual(probe.call_args.args[1], 123)

    def test_observer_health_reports_disabled_bridge_separately_from_ready_process(self):
        running = {'loaded': True, 'active': 'active', 'process_id': 123}
        with patch.object(desktop, 'observer_service_state', return_value=running), \
             patch.object(desktop, 'observer_probe', return_value={'ready': True}), \
             patch.object(desktop.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, 'b false\n', '')):
            state = desktop.accessibility_state()
        self.assertTrue(state['ready'])
        self.assertFalse(state['bus_enabled'])
        with patch.object(desktop, 'service_state', return_value={'active': 'active'}), \
             patch.object(desktop, 'native_state', return_value={'addon_loaded': True}), \
             patch.object(desktop, 'accessibility_state', return_value=state), \
             patch.object(desktop, 'control', return_value='{"metrics":{"provider_calls":1}}'):
            report = desktop.health_report()
        self.assertEqual([p['code'] for p in report['problems']], ['accessibility_disabled'])

    def test_observer_failure_does_not_expose_subprocess_details_or_hide_broker_health(self):
        with patch.object(desktop, 'observer_service_state', side_effect=RuntimeError('private process detail')):
            state = desktop.accessibility_state()
        self.assertEqual(state['error'], 'observer_uninspectable')
        self.assertNotIn('private', json.dumps(state))
        with patch.object(desktop, 'observer_service_state', return_value={'loaded': False, 'active': 'inactive', 'process_id': 0}), \
             patch.object(desktop, 'observer_probe') as probe, \
             patch.object(desktop.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1, '', '')):
            state = desktop.accessibility_state()
        probe.assert_not_called()
        self.assertIsNone(state['bus_enabled'])

    def test_service_start_clears_start_limit_before_starting(self):
        with patch.object(desktop, 'service_state', return_value={'active': 'failed'}), \
             patch.object(desktop.subprocess, 'run') as command, \
             contextlib.redirect_stdout(io.StringIO()):
            desktop.main(['service', 'start'])
        self.assertEqual([call.args[0] for call in command.call_args_list], [
            ['systemctl', '--user', 'reset-failed', desktop.SERVICE],
            ['systemctl', '--user', 'start', desktop.SERVICE],
        ])

    def test_debug_watch_is_readable_without_typed_prose(self):
        line = desktop.debug_line({'enabled': True, 'native': {'reason': 'context_received',
            'counts': {'input': 8}}, 'editors': {'obsidian': {'reason': 'suggestion',
            'reason_counts': {'sent context.changed': 3}}}, 'model': {'provider': 'local_model',
            'metrics': {'provider_calls': 3, 'suggestions_shown': 2, 'provider_errors': 0}}})
        for expected in ('input=8', 'requests=3', 'displayed=2', 'obsidian=suggestion(3 contexts)'):
            self.assertIn(expected, line)

    def test_debug_is_expiring_private_and_does_not_reuse_a_previous_run(self):
        with patch.object(desktop, 'registry_directory', return_value=self.directory), \
             patch.object(desktop.time, 'time', return_value=1000), \
             contextlib.redirect_stdout(io.StringIO()):
            desktop.debug_mode('on')
            first = desktop.private_json(self.directory.parent / 'debug-control.json')
            self.assertEqual(first['expires_at'], 1900)
            self.assertEqual(desktop.debug_activity()['reason'], 'no_input_events')
            native = self.directory.parent / 'debug-native.json'
            native.write_text(json.dumps({'id': first['id'], 'schema': 'badi.native-activity.v1',
                                         'at': 999, 'reason': 'unsupported_app', 'counts': {'tab': 1}}))
            native.chmod(0o600)
            self.assertEqual(desktop.debug_activity()['counts'], {'tab': 1})
            desktop.debug_mode('on')
            self.assertEqual(desktop.debug_activity()['counts'], {})
            desktop.debug_mode('off')
            self.assertFalse(native.exists())
            self.assertFalse(desktop.debug_activity()['enabled'])

    def test_debug_refuses_public_symlink_expired_and_malformed_data(self):
        flag = self.directory.parent / 'debug-control.json'
        with patch.object(desktop, 'registry_directory', return_value=self.directory), \
             patch.object(desktop.time, 'time', return_value=1000):
            flag.write_text('{')
            flag.chmod(0o600)
            self.assertEqual(desktop.debug_activity()['reason'], 'invalid_debug_data')
            flag.write_text(json.dumps({'expires_at': 999, 'id': 'old'}))
            self.assertFalse(desktop.debug_activity()['enabled'])
            flag.chmod(0o644)
            self.assertEqual(desktop.debug_activity()['reason'], 'invalid_debug_data')
            saved = flag.with_suffix('.saved')
            flag.rename(saved)
            flag.symlink_to(saved)
            self.assertEqual(desktop.debug_activity()['reason'], 'invalid_debug_data')


if __name__ == '__main__':
    unittest.main()
