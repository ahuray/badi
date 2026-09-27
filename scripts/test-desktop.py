"""Behavior checks for the desktop controls, settings updates and diagnostics."""
import contextlib
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('desktop', Path(__file__).with_name('badi-desktop.py'))
desktop = importlib.util.module_from_spec(spec)
spec.loader.exec_module(desktop)
REAL_INSTALL_STATE = desktop.install_state
ORIGINAL_VSCODE_NOTE = desktop.vscode_edit_context_note
BREAKDOWN = {'request_abstained': 0, 'budget_prefill': 2, 'budget_stream': 1, 'model_abstained': 0,
             'output_rejected': 0, 'stale': 0, 'timeout': 0, 'provider_error': 0, 'last': 'budget_prefill'}


class DesktopTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name) / 'badi'
        self.directory.mkdir(mode=0o700)
        # Doctor tests must not depend on this machine's installed receipts or service.
        installed = patch.object(desktop, 'install_state', return_value={
            'desktop': {'installed_at': '2026-09-26T12:00:00Z', 'commit': 'unknown', 'dirty': None, 'broker_version': None},
            'editors': None, 'running_broker': None})
        installed.start()
        self.addCleanup(installed.stop)
        # Nor on this machine's editor installations and settings.
        editors = patch.object(desktop, 'vscode_edit_context_note', return_value=None)
        editors.start()
        self.addCleanup(editors.stop)

    def desktop_socket(self, mode=0o600):
        endpoint = self.directory / 'broker.sock'
        listener = socket.socket(socket.AF_UNIX)
        listener.bind(str(endpoint))
        endpoint.chmod(mode)
        self.addCleanup(listener.close)
        return endpoint

    def test_ctl_reaches_only_the_private_desktop_socket(self):
        with patch.object(desktop, 'runtime_directory', return_value=self.directory), \
             patch.object(desktop.subprocess, 'run') as command:
            with self.assertRaisesRegex(RuntimeError, 'offline'):
                desktop.main(['ctl', 'pause', 'on'])
            endpoint = self.desktop_socket(0o666)
            with self.assertRaisesRegex(RuntimeError, 'offline'):
                desktop.main(['ctl', 'pause', 'on'])
            self.assertEqual(command.call_count, 0)
            endpoint.chmod(0o600)
            command.return_value.returncode = 0
            command.return_value.stdout = '{}'
            with contextlib.redirect_stdout(io.StringIO()):
                desktop.main(['ctl', 'pause', 'on'])
            self.assertEqual(command.call_args.args[0][1:], ['--socket', str(endpoint), 'pause', 'on'])

    def test_overview_adds_desktop_counters_and_activity(self):
        self.desktop_socket()
        metrics = {'provider_calls': 4, 'suggestions_shown': 3, 'provider_errors': 1}
        with patch.object(desktop, 'runtime_directory', return_value=self.directory), \
             patch.object(desktop, 'control', side_effect=[json.dumps({'schema': 'badi.overview.v2'}),
                                                           json.dumps({'metrics': metrics})]) as command, \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['ctl', 'overview', '--json'])
        self.assertEqual([call.args[1] for call in command.call_args_list], [self.directory / 'broker.sock'] * 2)
        overview = json.loads(output.getvalue())
        self.assertEqual(overview['schema'], 'badi.overview.v2')
        self.assertEqual({key: overview['desktop'][key] for key in ('requests', 'suggestions', 'errors')},
                         {'requests': 4, 'suggestions': 3, 'errors': 1})
        self.assertFalse(overview['desktop']['activity']['enabled'])

    def test_launcher_uses_normal_desktop_service_and_toolkit_modules(self):
        with patch.object(desktop.subprocess, 'run') as command:
            desktop.main(['launch', 'xournalpp'])
            self.assertEqual(command.call_args_list[0].args[0],
                             ['systemctl', '--user', 'start', 'badi-broker.service'])
            launch = command.call_args_list[1].args[0]
            self.assertIn('--setenv=GTK_IM_MODULE=fcitx', launch)
            self.assertEqual(launch[-1], 'xournalpp')

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
        browser = {'identity': {'kind': 'browser_origin', 'adapter': 'chromium', 'scheme': 'https', 'host': 'example.com', 'port': 443},
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

    def test_all_sites_is_a_canonical_opt_in_that_keeps_exact_rules(self):
        document = {'schema': 'badi.settings.v2', 'revision': 3, 'paused': False, 'subjects': []}
        desktop.set_site(document, 'https://bank.example', False)
        exact = [dict(item) for item in document['subjects']]
        desktop.set_all_sites(document, True)
        self.assertIs(document['all_web_origins'], True)
        self.assertEqual(document['subjects'], exact)
        desktop.set_all_sites(document, False)
        self.assertNotIn('all_web_origins', document)
        self.assertEqual(document['subjects'], exact)

        settings = {'schema': 'badi.settings.v2', 'revision': 7, 'paused': False, 'subjects': exact}
        for mode, expected in (('on', True), ('off', None)):
            with patch.object(desktop, 'control', side_effect=[json.dumps(settings), '{}']) as command, \
                 contextlib.redirect_stdout(io.StringIO()) as output:
                desktop.main(['site', 'all', mode])
            request = command.call_args.args[0]
            self.assertEqual(request[:5], ['settings', 'replace', '--if-revision', '7', '--json'])
            sent = json.loads(request[5])
            self.assertEqual(sent['revision'], 8)
            self.assertEqual(sent.get('all_web_origins'), expected)
            self.assertEqual(sent['subjects'], exact)
            if expected:
                self.assertIn('unless its exact site rule blocks it', output.getvalue())
        for invalid in (['site', 'all', 'maybe'], ['site', 'all']):
            with patch.object(desktop, 'control') as command, self.assertRaises(RuntimeError):
                desktop.main(invalid)
            command.assert_not_called()

    def test_status_and_doctor_state_the_all_sites_default(self):
        health = {'paused': False, 'provider': 'local_model',
                  'metrics': {'provider_calls': 1, 'suggestions_shown': 1, 'provider_errors': 0, 'no_suggestion': BREAKDOWN}}
        settings = {'schema': 'badi.settings.v2', 'revision': 2, 'paused': False,
                    'all_web_origins': True, 'subjects': []}
        self.assertIn('Web sites: every http(s) site unless blocked', desktop.status_text(health, settings))
        self.assertNotIn('Web sites:', desktop.status_text(health, {**settings, 'all_web_origins': False}))
        with patch.object(desktop, 'control', side_effect=[json.dumps(health), json.dumps(settings)]), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['status'])
        self.assertIn('Web sites: every http(s) site', output.getvalue())
        service = {'loaded': True, 'active': 'active', 'state': 'running', 'autostart': True}
        for document, noted in ((settings, True), ({**settings, 'all_web_origins': False}, False)):
            def control(arguments, endpoint=None):
                return json.dumps(document if arguments[0] == 'settings' else health)
            with patch.object(desktop, 'service_state', return_value=service), \
                 patch.object(desktop, 'accessibility_state', return_value={'loaded': False, 'ready': False}), \
                 patch.object(desktop, 'native_state', return_value={'active': 'active', 'addon_loaded': True}), \
                 patch.object(desktop, 'control', side_effect=control):
                report = desktop.health_report()
            self.assertEqual(desktop.ALL_SITES_NOTE in report['notes'], noted)
            self.assertEqual(report['problems'], [])
            self.assertTrue(any('use IME-parity' in note for note in report['notes']))
            self.assertFalse(any('writing is disabled' in note for note in report['notes']))
        # Site-all also opens the observed browser path, which has no second
        # host gate; both the note and help must say so.
        # Zen shares the Chromium browser-origin grants, so the disclosure names it.
        self.assertIn('Chromium/Brave/Zen fields, which have no second site gate', desktop.ALL_SITES_NOTE)
        self.assertIn('private windows', desktop.ALL_SITES_NOTE)
        self.assertIn('Covers\n', desktop.HELP)
        self.assertIn('Chromium/Brave/Zen fields', desktop.HELP)
        self.assertIn('one site grant covers all three\nbrowsers', desktop.HELP)

    def test_vscode_edit_context_check_is_read_only_jsonc_and_content_free(self):
        home = Path(self.temporary.name) / 'home'
        settings = home / '.config/Code/User/settings.json'
        secret = 'private-draft-text'
        with patch.object(desktop.Path, 'home', return_value=home), \
             patch.object(desktop.shutil, 'which', return_value=None):
            self.assertIsNone(ORIGINAL_VSCODE_NOTE(), 'VS Code is not installed')
        with patch.object(desktop.Path, 'home', return_value=home), \
             patch.object(desktop.shutil, 'which', return_value='/usr/bin/code'):
            missing = ORIGINAL_VSCODE_NOTE()
            self.assertIn('EditContext on', missing)
            settings.parent.mkdir(parents=True)
            for text, noted in (
                (f'// {secret}\n{{\n  "editor.editContext": false, /* {secret} */\n  "x": "{secret}//",\n}}\n', False),
                (f'{{"editor.editContext": true, "note": "{secret}"}}', True),
                (f'{{"editor.fontSize": 14, "note": "{secret}",}}', True),
                ('', True),
                (' \n\t\r\n', True),
                ('\ufeff', True),
                ('\ufeff{"editor.editContext": false}', False),
                (f'{{"[markdown]": {{"editor.editContext": false}}, "note": "{secret}"}}', True),
            ):
                settings.write_text(text)
                note = ORIGINAL_VSCODE_NOTE()
                self.assertEqual(note is not None, noted, text)
                if note:
                    self.assertIn('"editor.editContext": false', note)
                    self.assertNotIn(secret, note)
            for text in (f'{{"editor.editContext": /* {secret}', f'[{secret}', '{"a": 1}' + ' ' * (1 << 20)):
                settings.write_text(text)
                note = ORIGINAL_VSCODE_NOTE()
                self.assertIn('could not read its user settings', note)
                self.assertNotIn(secret, note)
            settings.write_bytes(b'\xff\xfe')
            self.assertIn('could not read', ORIGINAL_VSCODE_NOTE())
        # An existing configuration directory also counts as installed.
        with patch.object(desktop.Path, 'home', return_value=home), \
             patch.object(desktop.shutil, 'which', return_value=None):
            settings.write_text('{"editor.editContext": false}')
            self.assertIsNone(ORIGINAL_VSCODE_NOTE())
            settings.write_text('{}')
            self.assertIn('EditContext on', ORIGINAL_VSCODE_NOTE())

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
        with patch.object(desktop, 'runtime_directory', return_value=self.directory), \
             patch.object(desktop.time, 'time', return_value=1000), \
             contextlib.redirect_stdout(io.StringIO()):
            desktop.debug_mode('on')
            first = desktop.private_json(self.directory / 'debug-control.json')
            self.assertEqual(first['expires_at'], 1900)
            self.assertEqual(desktop.debug_activity()['reason'], 'no_input_events')
            native = self.directory / 'debug-native.json'
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
        flag = self.directory / 'debug-control.json'
        with patch.object(desktop, 'runtime_directory', return_value=self.directory), \
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


    def test_status_text_states_real_coverage_and_names_no_show_reasons(self):
        health = {'paused': False, 'provider': 'local_model',
                  'metrics': {'provider_calls': 5, 'suggestions_shown': 2, 'provider_errors': 0, 'no_suggestion': BREAKDOWN}}
        text = desktop.status_text(health)
        self.assertIn('No suggestion: 3 (last: budget_prefill)', text)
        self.assertIn('Observed fields: automatic for Omawrite, Telegram and IME-parity apps', text)
        self.assertIn('Tab accepts a visible suggestion, otherwise stays Tab', text)
        self.assertIn('IME-parity accepts once, append-only, like\ntyping', desktop.HELP)
        for stale in ('native browser/Codex writing is disabled', 'automatic only for Omawrite'):
            self.assertNotIn(stale, text)
            self.assertNotIn(stale, desktop.HELP)
        self.assertNotIn('Observed fields: automatic ·', text)
        self.assertNotIn('Observed fields: automatic suggestions', desktop.HELP)
        older = {**health, 'metrics': {'provider_calls': 1, 'suggestions_shown': 1, 'provider_errors': 0}}
        self.assertIn('not reported by this broker', desktop.status_text(older))
        with patch.object(desktop, 'control', return_value=json.dumps(health)), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['status'])
        self.assertEqual(output.getvalue().strip(), text)

    def test_doctor_and_debug_name_no_suggestion_classes_without_text(self):
        service = {'loaded': True, 'active': 'active', 'state': 'running', 'autostart': True}
        broker = {'provider': 'local_model', 'paused': False,
                  'metrics': {'provider_calls': 5, 'suggestions_shown': 2, 'provider_errors': 0, 'no_suggestion': BREAKDOWN}}
        with patch.object(desktop, 'service_state', return_value=service), \
             patch.object(desktop, 'accessibility_state', return_value={'loaded': False, 'ready': False}), \
             patch.object(desktop, 'native_state', return_value={'active': 'active', 'addon_loaded': True}), \
             patch.object(desktop, 'control', return_value=json.dumps(broker)):
            report = desktop.health_report()
        note = next(item for item in report['notes'] if 'showed no suggestion' in item)
        self.assertIn('budget_prefill=2: the writing budget (550 ms while typing, 1.2 s after Tab) ended before the model started answering', note)
        self.assertLess(note.index('budget_prefill=2'), note.index('budget_stream=1'))
        self.assertIn('Last: budget_prefill.', note)
        self.assertEqual(report['problems'], [])
        older = {**broker, 'metrics': {'provider_calls': 5}}
        with patch.object(desktop, 'service_state', return_value=service), \
             patch.object(desktop, 'accessibility_state', return_value={'loaded': False, 'ready': False}), \
             patch.object(desktop, 'native_state', return_value={'active': 'active', 'addon_loaded': True}), \
             patch.object(desktop, 'control', return_value=json.dumps(older)):
            self.assertTrue(any('predates no-suggestion' in item for item in desktop.health_report()['notes']))

        line = desktop.debug_line({'enabled': True, 'native': {'reason': 'context_received', 'counts': {}},
                                   'model': {'provider': 'local_model', 'metrics': broker['metrics']}})
        self.assertIn('no_show=3(last=budget_prefill)', line)
        with patch.object(desktop, 'runtime_directory', return_value=self.directory), \
             patch.object(desktop, 'control', return_value=json.dumps({**broker, 'sessions': 0})), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.debug_mode('status')
        summary = json.loads(output.getvalue())['model']['no_suggestion']
        self.assertEqual(summary['counts'], {'budget_prefill': 2, 'budget_stream': 1})
        self.assertEqual(summary['last'], 'budget_prefill')
        self.assertIn('prompt prefill', summary['last_meaning'])

    def write_receipt(self, home, broker_entry, mode=0o600):
        path = home / '.local/state/badi/receipts/desktop.json'
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({'schema': 'badi.install-receipt.v1', 'installer': 'desktop',
                                    'installed_at': '2026-09-26T12:00:00Z',
                                    'source': {'commit': 'a' * 40, 'dirty': False},
                                    'files': {'.local/lib/badi/badi-broker': broker_entry}}))
        path.chmod(mode)

    def test_doctor_compares_the_running_broker_with_the_install_receipt(self):
        version = 'badi-broker 0.1.0 commit=' + 'a' * 40 + ' dirty=false'
        entry = {'sha256': 'f' * 64, 'version': version, 'commit': 'a' * 40, 'dirty': False}
        home = Path(self.temporary.name) / 'home'
        self.write_receipt(home, entry)
        service = {'loaded': True, 'active': 'active', 'state': 'running', 'autostart': True}
        for running, mismatch in (({'version': version, 'sha256': 'f' * 64}, False),
                                  ({'version': version.replace('a' * 40, 'b' * 40), 'sha256': 'f' * 64}, True),
                                  ({'version': version, 'sha256': 'e' * 64}, True),
                                  (None, False)):
            with self.subTest(running=running), \
                 patch.object(desktop.Path, 'home', return_value=home), \
                 patch.object(desktop, 'running_broker_identity', return_value=None if running is None else dict(running)), \
                 patch.object(desktop, 'install_state', REAL_INSTALL_STATE), \
                 patch.object(desktop, 'service_state', return_value=service), \
                 patch.object(desktop, 'accessibility_state', return_value={'loaded': False, 'ready': False}), \
                 patch.object(desktop, 'native_state', return_value={'active': 'active', 'addon_loaded': True}), \
                 patch.object(desktop, 'control', return_value='{"metrics": {"provider_calls": 1}}'):
                report = desktop.health_report()
            self.assertEqual(report['install']['desktop'], {'installed_at': '2026-09-26T12:00:00Z', 'commit': 'a' * 40,
                                                            'dirty': False, 'broker_version': version})
            self.assertEqual('broker_build_mismatch' in [item['code'] for item in report['problems']], mismatch)
            if running is not None:
                self.assertEqual(report['install']['running_broker']['version_matches_receipt'],
                                 running['version'] == version)
        self.write_receipt(home, entry, mode=0o644)
        with patch.object(desktop.Path, 'home', return_value=home), \
             patch.object(desktop, 'running_broker_identity', return_value=None):
            self.assertEqual(REAL_INSTALL_STATE()['desktop'], {'error': 'receipt_unreadable'})

    def test_running_broker_identity_hashes_and_executes_the_live_process_image(self):
        real_run = subprocess.run
        def execute(command, **kwargs):
            if command[0] == 'systemctl':
                return subprocess.CompletedProcess(command, 0, f'{os.getpid()}\n', '')
            return real_run(command, **kwargs)
        with patch.object(desktop.subprocess, 'run', side_effect=execute) as command:
            identity = desktop.running_broker_identity()
        executed = [call.args[0] for call in command.call_args_list if call.args[0][0] != 'systemctl']
        self.assertEqual(executed, [[f'/proc/{os.getpid()}/exe', '--version']])
        with open(sys.executable, 'rb') as stream:
            self.assertEqual(identity['sha256'], hashlib.sha256(stream.read()).hexdigest())
        # This test process is Python, whose version line is not a Badi identity.
        self.assertIsNone(identity['version'])
        with patch.object(desktop.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, '0\n', '')):
            self.assertIsNone(desktop.running_broker_identity())


if __name__ == '__main__':
    unittest.main()
