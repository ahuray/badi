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
from types import SimpleNamespace
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('desktop', Path(__file__).with_name('badi-desktop.py'))
desktop = importlib.util.module_from_spec(spec)
spec.loader.exec_module(desktop)
REAL_INSTALL_STATE = desktop.install_state
ORIGINAL_VSCODE_NOTE = desktop.vscode_edit_context_note
STATS = {'available': True, 'summary': {
    'today': '2026-10-20', 'retention_days': 90,
    'totals': {'today': {'shown': 3, 'accepted': 2, 'words': 5},
               'last_7_days': {'shown': 12, 'accepted': 6, 'words': 12},
               'retained': {'shown': 12, 'accepted': 6, 'words': 12}},
    'days': [{'day': '2026-10-19', 'shown': 9, 'accepted': 4, 'words': 7},
             {'day': '2026-10-20', 'shown': 3, 'accepted': 2, 'words': 5}],
    'subjects': [
        {'identity': {'kind': 'browser_origin', 'adapter': 'chromium', 'scheme': 'https', 'host': 'example.com',
                      'port': 443},
         'today': {'shown': 0, 'accepted': 0, 'words': 0},
         'last_7_days': {'shown': 9, 'accepted': 4, 'words': 7},
         'retained': {'shown': 9, 'accepted': 4, 'words': 7}},
        {'identity': {'kind': 'linux_app', 'adapter': 'fcitx', 'app_id': 'com.mitchellh.ghostty'},
         'today': {'shown': 3, 'accepted': 2, 'words': 5},
         'last_7_days': {'shown': 3, 'accepted': 2, 'words': 5},
         'retained': {'shown': 3, 'accepted': 2, 'words': 5}}]}}
BREAKDOWN = {'request_abstained': 0, 'budget_prefill': 2, 'budget_stream': 1, 'model_abstained': 0,
             'output_rejected': 0, 'stale': 0, 'timeout': 0, 'provider_error': 0, 'last': 'budget_prefill'}


class DesktopTests(unittest.TestCase):
    def test_repair_reports_what_it_restored_and_the_hook_stays_quiet(self):
        for arguments, omarchy, restored, expected in (
                (['repair'], False, ['hypr/hyprland.lua'], 'Restored Badi\'s lines in hypr/hyprland.lua'),
                (['repair'], False, [], 'Nothing to repair'),
                (['repair', '--omarchy'], True, ['chromium-flags.conf'], 'Restored Badi\'s lines in chromium-flags.conf'),
                (['repair', '--omarchy'], True, [], '')):
            with self.subTest(arguments=arguments, restored=restored), \
                 patch.object(desktop.badi_install, 'repair', return_value=restored) as repair, \
                 contextlib.redirect_stdout(io.StringIO()) as output:
                desktop.main(arguments)
            repair.assert_called_once_with(Path.home(), omarchy_only=omarchy)
            self.assertEqual(output.getvalue().strip()[:len(expected)], expected)
            if not expected:
                self.assertEqual(output.getvalue(), '', 'the login hook prints nothing when nothing was dropped')

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
        # Nor on whether this machine runs the session helper.
        helper = patch.object(desktop, 'session_helper_state', return_value={'loaded': True, 'active': 'active'})
        helper.start()
        self.addCleanup(helper.stop)
        # Nor ask this machine's Omarchy bar to refresh.
        self.refresh_bar = desktop.refresh_bar
        bar = patch.object(desktop, 'refresh_bar')
        self.bar = bar.start()
        self.addCleanup(bar.stop)
        # Nor set timers in this machine's user manager.
        self.schedule_debug_expiry = desktop.schedule_debug_expiry
        expiry = patch.object(desktop, 'schedule_debug_expiry', return_value=True)
        self.expiry = expiry.start()
        self.addCleanup(expiry.stop)

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

    def test_installed_companions_do_not_mix_package_and_user_local_code(self):
        library = self.directory / 'lib'
        library.mkdir()
        (library / 'badictl').write_text('packaged CLI')
        observer = library / 'accessibility'
        observer.mkdir()
        (observer / 'health.py').write_text(
            'def probe(endpoint, pid): return {"ready": True, "pid": pid}\n')
        with patch.object(desktop, 'LIBRARY', library):
            self.assertEqual(desktop.cli_path(), library / 'badictl')
            self.assertEqual(desktop.observer_probe(self.directory / 'accessibility.sock', 42),
                             {'ready': True, 'pid': 42})

    def test_overview_adds_desktop_counters_and_activity(self):
        self.desktop_socket()
        metrics = {'provider_calls': 4, 'suggestions_shown': 3, 'provider_errors': 1}
        with patch.object(desktop, 'runtime_directory', return_value=self.directory), \
             patch.object(desktop, 'control', side_effect=[json.dumps({'schema': 'badi.overview.v2'}),
                                                           json.dumps({'metrics': metrics}),
                                                           json.dumps(STATS)]) as command, \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['ctl', 'overview', '--json'])
        self.assertEqual([call.args[1] for call in command.call_args_list], [self.directory / 'broker.sock'] * 3)
        self.assertEqual(command.call_args_list[2].args[0], ['stats', 'status'])
        overview = json.loads(output.getvalue())
        self.assertEqual(overview['schema'], 'badi.overview.v2')
        self.assertEqual({key: overview['desktop'][key] for key in ('requests', 'suggestions', 'errors')},
                         {'requests': 4, 'suggestions': 3, 'errors': 1})
        self.assertFalse(overview['desktop']['activity']['enabled'])
        self.assertEqual(overview['desktop']['stats'], STATS)

    def test_overview_survives_a_broker_without_statistics(self):
        self.desktop_socket()
        metrics = {'provider_calls': 4, 'suggestions_shown': 3, 'provider_errors': 1}
        with patch.object(desktop, 'runtime_directory', return_value=self.directory), \
             patch.object(desktop, 'control', side_effect=[json.dumps({'schema': 'badi.overview.v2'}),
                                                           json.dumps({'metrics': metrics}),
                                                           RuntimeError('error_code=rejected:invalid_message')]), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['ctl', 'overview', '--json'])
        self.assertIsNone(json.loads(output.getvalue())['desktop']['stats'])

    def test_stats_show_today_the_week_and_the_leading_apps_without_text(self):
        calls = []

        def control(arguments, endpoint=None, *, input_text=None):
            calls.append(arguments)
            return json.dumps(STATS)
        with patch.object(desktop, 'control', side_effect=control), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['stats'])
        text = output.getvalue()
        self.assertEqual(calls, [['stats', 'status']])
        lines = text.splitlines()
        self.assertEqual(lines[2].split(), ['Today', '3', '2', '5'])
        self.assertEqual(lines[3].split(), ['Last', '7', 'days', '12', '6', '12'])
        self.assertIn('2026-10-14 Wed', text, 'seven days, the empty ones too')
        self.assertEqual([line.split()[:2] for line in lines if line.startswith('  2026-10-19')],
                         [['2026-10-19', 'Mon']])
        leaders = text.split('Apps and sites, last 7 days:\n')[1].splitlines()
        self.assertTrue(leaders[0].strip().startswith('https://example.com'), leaders)
        self.assertIn('≈ 7 words · 9 shown · 4 accepted', leaders[0])
        self.assertTrue(leaders[1].strip().startswith('com.mitchellh.ghostty'), leaders)

        with patch.object(desktop, 'control', side_effect=control), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['stats', '--json'])
        self.assertEqual(json.loads(output.getvalue()), STATS)

        empty = {'available': True, 'summary': dict(STATS['summary'], days=[], subjects=[], totals={
            window: {'shown': 0, 'accepted': 0, 'words': 0} for window in ('today', 'last_7_days', 'retained')})}
        with patch.object(desktop, 'control', return_value=json.dumps(empty)), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['stats'])
        self.assertIn('Nothing counted yet', output.getvalue())

        unreadable = {'available': False, 'summary': empty['summary']}
        with patch.object(desktop, 'control', return_value=json.dumps(unreadable)), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['stats'])
        self.assertIn('badi stats forget', output.getvalue())

        calls.clear()
        with patch.object(desktop, 'control', side_effect=control), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['stats', 'forget'])
        self.assertEqual(calls, [['stats', 'forget']])
        self.assertIn('deleted', output.getvalue())
        for invalid in (['stats', 'clear'], ['stats', 'forget', 'now'], ['stats', '--text']):
            with self.subTest(invalid=invalid), self.assertRaises(RuntimeError):
                desktop.main(invalid)

    def test_an_unknown_form_of_any_command_changes_nothing(self):
        untouched = AssertionError('an unknown form must not reach the broker, settings or systemd')

        def transform_only(transform):
            # badi languages checks its codes inside the transform, before any write.
            transform({'schema': 'badi.settings.v2', 'revision': 1, 'paused': False, 'subjects': []})
            raise untouched
        for word in desktop.COMMANDS:
            with self.subTest(word=word), \
                 patch.object(desktop, 'control', side_effect=untouched), \
                 patch.object(desktop, 'update_settings', side_effect=transform_only), \
                 patch.object(desktop.subprocess, 'run', side_effect=untouched):
                if word == 'ctl':
                    continue  # every ctl form is the broker's to judge
                try:
                    desktop.main([word, 'no', 'such', 'form', 'here'])
                except RuntimeError as error:
                    self.assertRegex(str(error), 'Unknown command|Usage:')
                else:
                    self.fail(f'{word} accepted an unknown form')
        with self.assertRaisesRegex(RuntimeError, 'Unknown command'):
            desktop.main(['no-such-command'])

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
        self.assertEqual(request, ['settings', 'replace', '--if-revision', '7', '--json', '-'])
        document = json.loads(command.call_args.kwargs['input_text'])
        self.assertEqual(document['revision'], 8)
        self.assertTrue(document['paused'])

    def test_pause_toggle_flips_the_persisted_state_with_compare_and_swap(self):
        for paused, message in ((False, 'Predictions paused.'), (True, 'Predictions resumed.')):
            settings = {'schema': 'badi.settings.v2', 'revision': 7, 'paused': paused, 'subjects': []}
            with self.subTest(paused=paused), \
                 patch.object(desktop, 'control', side_effect=[json.dumps(settings), '{}', '{}']) as command, \
                 contextlib.redirect_stdout(io.StringIO()) as output:
                desktop.main(['pause', 'toggle'])
            replace = command.call_args_list[1]
            self.assertEqual(replace.args[0], ['settings', 'replace', '--if-revision', '7', '--json', '-'])
            document = json.loads(replace.kwargs['input_text'])
            self.assertEqual((document['revision'], document['paused']), (8, not paused))
            self.assertEqual(output.getvalue().strip(), message)
            # Resuming also ends every snooze; pausing leaves them.
            self.assertEqual(command.call_count, 3 if paused else 2)
            if paused:
                self.assertEqual(command.call_args.args[0], ['snooze', 'end'])
        settings = {'schema': 'badi.settings.v2', 'revision': 7, 'paused': False, 'subjects': []}
        with patch.object(desktop, 'control', side_effect=[json.dumps(settings), RuntimeError('settings_conflict')]), \
             self.assertRaisesRegex(RuntimeError, 'settings_conflict'):
            desktop.main(['pause', 'toggle'])

    def test_resume_ends_every_snooze_after_the_settings_write(self):
        settings = {'schema': 'badi.settings.v2', 'revision': 7, 'paused': True, 'subjects': []}
        with patch.object(desktop, 'control', side_effect=[json.dumps(settings), '{}', '{}']) as command, \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['resume'])
        self.assertEqual([call.args[0][:2] for call in command.call_args_list],
                         [['settings', 'show'], ['settings', 'replace'], ['snooze', 'end']])
        self.assertEqual(output.getvalue().strip(), 'Predictions resumed.')

    def test_snooze_names_the_app_or_site_and_explains_a_refusal(self):
        origin = {'kind': 'browser_origin', 'adapter': 'chromium', 'scheme': 'https', 'host': 'mail.example.com', 'port': 443}
        reply = {'v': 2, 'type': 'control.result', 'payload': {
            'action': 'snooze', 'accepted': True, 'reason': 'accepted', 'paused': False,
            'snooze': {'subject': origin, 'remaining_ms': 600000}}}
        with patch.object(desktop, 'control', return_value=json.dumps(reply)) as command, \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['snooze'])
        command.assert_called_once_with(['snooze', '10'])
        self.assertEqual(output.getvalue().strip(),
                         'Badi is snoozed in https://mail.example.com for 10 minutes. badi resume ends it sooner.')
        with patch.object(desktop, 'control', return_value=json.dumps(reply)) as command, \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['snooze', '1'])
        command.assert_called_once_with(['snooze', '1'])
        self.assertIn('for 1 minute.', output.getvalue())
        for code, message in (('no_context', 'Nothing to snooze'), ('ambiguous_session', 'more than one app'),
                              ('paused', 'paused already')):
            with self.subTest(code=code), \
                 patch.object(desktop, 'control', side_effect=RuntimeError(f'error_code=rejected:{code}')), \
                 self.assertRaisesRegex(RuntimeError, message):
                desktop.main(['snooze'])
        with patch.object(desktop, 'control', side_effect=RuntimeError('error_code=response_timeout')), \
             self.assertRaisesRegex(RuntimeError, 'response_timeout'):
            desktop.main(['snooze'])
        for invalid in (['snooze', '0'], ['snooze', '1441'], ['snooze', 'ten'], ['snooze', '5', '6']):
            with self.subTest(invalid=invalid), patch.object(desktop, 'control') as command:
                with self.assertRaises(RuntimeError):
                    desktop.main(invalid)
                command.assert_not_called()
        with patch.object(desktop, 'control', return_value='{}') as command, \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['snooze', 'end'])
        command.assert_called_once_with(['snooze', 'end'])
        self.assertIn('Every snooze ended', output.getvalue())

    def test_status_and_doctor_name_each_snooze(self):
        metrics = {'provider_calls': 1, 'suggestions_shown': 0, 'provider_errors': 0, 'no_suggestion': BREAKDOWN}
        snoozes = [{'subject': {'kind': 'linux_app', 'adapter': 'fcitx', 'app_id': 'telegram'}, 'remaining_ms': 540000},
                   {'subject': {'kind': 'browser_origin', 'adapter': 'chromium', 'scheme': 'http',
                                'host': 'localhost', 'port': 4173}, 'remaining_ms': 20000}]
        health = {'paused': False, 'provider': 'local_model', 'metrics': metrics, 'snoozes': snoozes}
        line = 'Snoozed: telegram (9 min left), http://localhost:4173 (1 min left) · badi resume ends it'
        self.assertIn(line, desktop.status_text(health, {}))
        self.assertNotIn('Snoozed', desktop.status_text({**health, 'snoozes': []}, {}))

    def test_battery_mode_is_omitted_when_quiet(self):
        settings = {'schema': 'badi.settings.v2', 'revision': 7, 'paused': False, 'battery_mode': 'normal',
                    'subjects': []}
        for arguments, expected in ((['battery', 'normal'], 'normal'), (['battery', 'explicit'], 'explicit'),
                                    (['battery', 'quiet'], None)):
            with self.subTest(arguments=arguments), \
                 patch.object(desktop, 'control', side_effect=[json.dumps(settings), '{}']) as command, \
                 contextlib.redirect_stdout(io.StringIO()) as output:
                desktop.main(arguments)
            sent = json.loads(command.call_args.kwargs['input_text'])
            self.assertEqual(sent.get('battery_mode'), expected)
            self.assertEqual(sent['subjects'], [])
            self.assertIn('as on mains power' if expected == 'normal' else 'Explicit requests are unchanged',
                          output.getvalue())
        for invalid in (['battery', 'off'], ['battery'], ['battery', 'quiet', 'now']):
            with patch.object(desktop, 'control') as command, self.assertRaises(RuntimeError):
                desktop.main(invalid)
            command.assert_not_called()

    def test_screen_share_hold_is_the_default_and_omitted(self):
        settings = {'schema': 'badi.settings.v2', 'revision': 7, 'paused': False, 'subjects': []}
        for arguments, expected, said in ((['screen-share', 'suggest'], 'suggest', 'can appear on the shared screen'),
                                          (['screen-share', 'hold'], None, 'shows no suggestions')):
            with self.subTest(arguments=arguments), \
                 patch.object(desktop, 'control', side_effect=[json.dumps(settings), '{}']) as command, \
                 contextlib.redirect_stdout(io.StringIO()) as output:
                desktop.main(arguments)
            self.assertEqual(command.call_args.args[0], ['settings', 'replace', '--if-revision', '7', '--json', '-'])
            sent = json.loads(command.call_args.kwargs['input_text'])
            self.assertEqual(sent.get('screen_share'), expected)
            self.assertEqual((sent['revision'], sent['subjects']), (8, []))
            self.assertIn(said, output.getvalue())
        for invalid in (['screen-share'], ['screen-share', 'off'], ['screen-share', 'on'],
                        ['screen-share', 'hold', 'now']):
            with patch.object(desktop, 'control') as command, self.assertRaises(RuntimeError):
                desktop.main(invalid)
            command.assert_not_called()

    def test_status_and_doctor_name_a_screen_share_hold_without_text(self):
        metrics = {'provider_calls': 3, 'suggestions_shown': 1, 'provider_errors': 0, 'no_suggestion': BREAKDOWN}
        settings = {'schema': 'badi.settings.v2', 'revision': 2, 'paused': False, 'subjects': []}
        shared = {'observed': True, 'session': 'active', 'low_power': False, 'automatic': 'normal',
                  'screen_share': 'held'}
        health = {'paused': True, 'provider': 'local_model', 'metrics': metrics, 'desktop': shared}
        text = desktop.status_text(health, settings)
        self.assertTrue(text.startswith('Badi: Held while the screen is shared'), text)
        self.assertIn('Screen sharing: shared, every suggestion held until it stops', text)
        self.assertTrue(desktop.status_text(health, {**settings, 'paused': True}).startswith('Badi: Paused'))
        suggesting = {**health, 'paused': False, 'desktop': {**shared, 'screen_share': 'suggesting'}}
        self.assertIn('Screen sharing: shared, still suggesting', desktop.status_text(suggesting, settings))
        idle = {**health, 'paused': False, 'desktop': {**shared, 'screen_share': 'none'}}
        self.assertIn('Screen sharing: not shared · holds suggestions while shared',
                      desktop.status_text(idle, settings))
        self.assertIn('Screen sharing: not shared · still suggests while shared',
                      desktop.status_text(idle, {**settings, 'screen_share': 'suggest'}))
        self.assertNotIn('Screen sharing', desktop.status_text({**idle, 'desktop': None}, settings))
        gone = {**health, 'desktop': {**shared, 'observed': False}}
        self.assertIn('the session helper stopped during a share; suggestions stay held',
                      desktop.status_text(gone, settings))
        self.assertNotIn('Screen sharing', desktop.status_text({**gone, 'desktop': {**gone['desktop'],
                                                                                     'screen_share': 'none'}}, settings))
        service = {'loaded': True, 'active': 'active', 'state': 'running', 'autostart': True}
        for broker_health, note, absent in (
                (health, 'holds every suggestion until sharing stops', 'Predictions are paused'),
                (suggesting, 'can appear on the shared screen', 'holds every suggestion')):
            def control(arguments, endpoint=None, health=broker_health):
                return json.dumps(settings if arguments[0] == 'settings' else health)
            with self.subTest(note=note), \
                 patch.object(desktop, 'service_state', return_value=service), \
                 patch.object(desktop, 'session_helper_state', return_value={'loaded': True, 'active': 'active'}), \
                 patch.object(desktop, 'accessibility_state', return_value={'loaded': False, 'ready': False}), \
                 patch.object(desktop, 'native_state', return_value={'active': 'active', 'addon_loaded': True}), \
                 patch.object(desktop.shutil, 'which', return_value='/usr/bin/pw-dump'), \
                 patch.object(desktop, 'control', side_effect=control):
                report = desktop.health_report()
            self.assertTrue(any(note in line for line in report['notes']), report['notes'])
            self.assertFalse(any(absent in line for line in report['notes']), report['notes'])
            self.assertEqual(report['problems'], [])

    def test_suggestion_length_is_omitted_when_normal(self):
        settings = {'schema': 'badi.settings.v2', 'revision': 7, 'paused': False, 'suggestion_length': 'short',
                    'subjects': []}
        for arguments, expected, said in ((['length', 'short'], 'short', 'one or two words'),
                                          (['length', 'normal'], None, 'up to four words')):
            with self.subTest(arguments=arguments), \
                 patch.object(desktop, 'control', side_effect=[json.dumps(settings), '{}']) as command, \
                 contextlib.redirect_stdout(io.StringIO()) as output:
                desktop.main(arguments)
            self.assertEqual(command.call_args.args[0], ['settings', 'replace', '--if-revision', '7', '--json', '-'])
            sent = json.loads(command.call_args.kwargs['input_text'])
            self.assertEqual(sent.get('suggestion_length'), expected)
            self.assertEqual((sent['revision'], sent['subjects']), (8, []))
            self.assertIn(said, output.getvalue())
        for invalid in (['length'], ['length', 'long'], ['length', 'medium'], ['length', '4'],
                        ['length', 'short', 'now']):
            with patch.object(desktop, 'control') as command, self.assertRaises(RuntimeError):
                desktop.main(invalid)
            command.assert_not_called()
        metrics = {'provider_calls': 1, 'suggestions_shown': 1, 'provider_errors': 0, 'no_suggestion': BREAKDOWN}
        health = {'paused': False, 'provider': 'local_model', 'metrics': metrics}
        self.assertIn('Length: short, one or two words', desktop.status_text(health, settings))
        self.assertIn('Length: normal, up to four words', desktop.status_text(health, {}))
        self.assertIn('Length: normal, up to four words', desktop.status_text(health))

    def test_status_and_doctor_report_the_desktop_session_helper(self):
        metrics = {'provider_calls': 1, 'suggestions_shown': 1, 'provider_errors': 0, 'no_suggestion': BREAKDOWN}
        settings = {'schema': 'badi.settings.v2', 'revision': 2, 'paused': False, 'battery_mode': 'quiet',
                    'subjects': []}
        deferred = {'observed': True, 'session': 'active', 'low_power': True, 'automatic': 'deferred'}
        health = {'paused': False, 'provider': 'local_model', 'metrics': metrics, 'desktop': deferred}
        self.assertIn('Desktop: session active · on battery or power saver · battery mode quiet · '
                      'automatic after the typing pause', desktop.status_text(health, settings))
        absent = {**health, 'desktop': {'observed': False, 'session': 'active', 'low_power': False,
                                        'automatic': 'normal'}}
        self.assertIn('Desktop: no session helper', desktop.status_text(absent, settings))
        self.assertNotIn('Desktop:', desktop.status_text({**health, 'desktop': None}))
        service = {'loaded': True, 'active': 'active', 'state': 'running', 'autostart': True}
        cases = (
            (health, {'loaded': True, 'active': 'active'}, 'automatic suggestions start after the typing pause'),
            ({**health, 'paused': True, 'desktop': {**deferred, 'session': 'sleeping'}},
             {'loaded': True, 'active': 'active'}, 'every grant is held'),
            (absent, {'loaded': False, 'active': 'inactive'}, 'badi-session) is not installed'),
        )
        for broker_health, helper, note in cases:
            def control(arguments, endpoint=None, health=broker_health):
                return json.dumps(settings if arguments[0] == 'settings' else health)
            with self.subTest(note=note), \
                 patch.object(desktop, 'service_state', return_value=service), \
                 patch.object(desktop, 'session_helper_state', return_value=helper), \
                 patch.object(desktop, 'accessibility_state', return_value={'loaded': False, 'ready': False}), \
                 patch.object(desktop, 'native_state', return_value={'active': 'active', 'addon_loaded': True}), \
                 patch.object(desktop, 'control', side_effect=control):
                report = desktop.health_report()
            self.assertTrue(any(note in line for line in report['notes']), report['notes'])
            self.assertEqual(report['problems'], [])
            text = desktop.doctor_text(report)
            self.assertIn('Session helper: ' + ('active' if helper['loaded'] else 'not installed'), text)

    def test_style_learning_keeps_the_key_out_of_argv_and_is_opt_in(self):
        key = ('ab' * 16, 'cd' * 32)
        keyring = SimpleNamespace(ensure=lambda: key, delete=lambda: True)
        settings = {'schema': 'badi.settings.v2', 'revision': 7, 'paused': False, 'subjects': [
            {'identity': desktop.site_identity('https://mail.example'), 'permissions': {
                'context_read': 'allow', 'display': 'allow', 'suggest': 'allow', 'learn': 'block',
                'retention': {'mode': 'none'}}}]}
        calls = []

        def control(arguments, endpoint=None, *, input_text=None):
            calls.append((arguments, input_text))
            if arguments[:2] == ['settings', 'show']:
                return json.dumps(settings)
            if arguments == ['style', 'status']:
                return json.dumps({'enabled': True, 'available': True, 'key': 'ready', 'samples': 2, 'subjects': 1})
            if arguments == ['style', 'export']:
                return '{"samples": []}\n'
            return '{}'

        with patch.object(desktop, 'style_keyring', return_value=keyring), \
             patch.object(desktop, 'control', side_effect=control), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['style', 'on'])
        delivery, = [call for call in calls if call[0] == ['style', 'key', '-']]
        self.assertEqual(delivery[1], f'{key[0]} {key[1]}\n')
        self.assertFalse(any(key[1] in ' '.join(arguments) for arguments, _ in calls), 'never in argv')
        self.assertLess(calls.index(delivery), len(calls) - 1, 'the key arrives before learning is enabled')
        self.assertTrue(json.loads(calls[-1][1])['style_learning'])
        self.assertIn('badi style site ORIGIN on', output.getvalue())

        for arguments, learn, retention in ((['style', 'site', 'https://mail.example', 'on'], 'allow', 30),
                                            (['style', 'site', 'https://mail.example', 'on', '7'], 'allow', 7),
                                            (['style', 'site', 'https://mail.example', 'off'], 'block', None)):
            calls.clear()
            with self.subTest(arguments=arguments), patch.object(desktop, 'control', side_effect=control), \
                 contextlib.redirect_stdout(io.StringIO()):
                desktop.main(arguments)
            permissions = json.loads(calls[-1][1])['subjects'][0]['permissions']
            self.assertEqual(permissions['learn'], learn)
            self.assertEqual(permissions['retention'], {'mode': 'bounded', 'days': retention} if retention
                             else {'mode': 'none'})
        for invalid in (['style', 'site', 'https://other.example', 'on'], ['style', 'site', 'https://mail.example', 'on', '91'],
                        ['style', 'site', 'https://mail.example/path', 'on'], ['style', 'maybe']):
            with self.subTest(invalid=invalid), patch.object(desktop, 'control', side_effect=control), \
                 self.assertRaises(RuntimeError):
                desktop.main(invalid)

        calls.clear()
        with patch.object(desktop, 'style_keyring', return_value=keyring), \
             patch.object(desktop, 'control', side_effect=control), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['style', 'forget'])
        self.assertEqual(calls[0][0], ['style', 'forget'])
        self.assertIn('keyring key removed', output.getvalue())

        def keyless(arguments, endpoint=None, *, input_text=None):
            return json.dumps({'enabled': True, 'available': True, 'key': 'absent', 'samples': 0, 'subjects': 0})
        with patch.object(desktop, 'control', side_effect=keyless), self.assertRaisesRegex(RuntimeError, 'badi style on'):
            desktop.main(['style', 'export'])

    def test_context_is_opt_in_keeps_the_key_out_of_argv_and_turns_off_per_app_or_site(self):
        key = ('ab' * 16, 'cd' * 32)
        keyring = SimpleNamespace(ensure=lambda: key, delete=lambda: True)
        settings = {'schema': 'badi.settings.v2', 'revision': 7, 'paused': False, 'subjects': []}
        calls = []

        def control(arguments, endpoint=None, *, input_text=None):
            calls.append((arguments, input_text))
            if arguments[:2] == ['settings', 'show']:
                return json.dumps(settings)
            if arguments == ['context', 'status']:
                return json.dumps({'enabled': True, 'chosen': True, 'available': True, 'key': 'ready',
                                   'subjects': 2, 'words': 17})
            return '{}'

        with patch.object(desktop, 'style_keyring', return_value=keyring), \
             patch.object(desktop, 'control', side_effect=control), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['context', 'on'])
        delivery, = [call for call in calls if call[0] == ['style', 'key', '-']]
        self.assertEqual(delivery[1], f'{key[0]} {key[1]}\n')
        self.assertFalse(any(key[1] in ' '.join(arguments) for arguments, _ in calls), 'never in argv')
        self.assertIs(json.loads(calls[-1][1])['context_words'], True)
        self.assertIn('five letters', output.getvalue())

        calls.clear()
        with patch.object(desktop, 'control', side_effect=control), contextlib.redirect_stdout(io.StringIO()):
            desktop.main(['context', 'off'])
        self.assertIs(json.loads(calls[-1][1])['context_words'], False, 'off is a recorded choice')

        def written(arguments):
            calls.clear()
            with patch.object(desktop, 'control', side_effect=control), contextlib.redirect_stdout(io.StringIO()):
                desktop.main(arguments)
            return json.loads(calls[-1][1])
        document = written(['context', 'app', 'com.anthropic.claude', 'off'])
        self.assertEqual(document['context_words_off'],
                         [{'kind': 'linux_app', 'adapter': 'fcitx', 'app_id': 'com.anthropic.claude'}])
        settings.update(document)
        document = written(['context', 'site', 'https://mail.example', 'off'])
        self.assertEqual([item['kind'] for item in document['context_words_off']], ['browser_origin', 'linux_app'],
                         'canonical order: sites before apps')
        settings.update(document)
        document = written(['context', 'app', 'com.anthropic.claude', 'on'])
        self.assertEqual(len(document['context_words_off']), 1)
        settings.update(document)
        document = written(['context', 'site', 'https://mail.example', 'on'])
        self.assertNotIn('context_words_off', document, 'an empty list is omitted')

        with patch.object(desktop, 'control', side_effect=control), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['context', 'status'])
        self.assertIn('learned words: 17 from 2', output.getvalue())
        calls.clear()
        with patch.object(desktop, 'control', side_effect=control), contextlib.redirect_stdout(io.StringIO()):
            desktop.main(['context', 'forget'])
        self.assertEqual(calls[0][0], ['context', 'forget'])
        for invalid in (['context', 'app', 'Bad App', 'off'], ['context', 'site', 'https://x.example/path', 'off'],
                        ['context', 'maybe']):
            with self.subTest(invalid=invalid), patch.object(desktop, 'control', side_effect=control), \
                 self.assertRaises(RuntimeError):
                desktop.main(invalid)

    def test_doctor_notes_style_learning_without_a_usable_key(self):
        health = {'paused': False, 'provider': 'local_model',
                  'metrics': {'provider_calls': 1, 'suggestions_shown': 1, 'provider_errors': 0, 'no_suggestion': BREAKDOWN}}
        service = {'loaded': True, 'active': 'active', 'state': 'running', 'autostart': True}
        for key, noted in (('absent', True), ('mismatch', True), ('ready', False)):
            settings = {'schema': 'badi.settings.v2', 'revision': 2, 'paused': False, 'style_learning': True,
                        'subjects': []}

            def control(arguments, endpoint=None, key=key):
                if arguments[0] == 'settings':
                    return json.dumps(settings)
                if arguments[0] == 'style':
                    return json.dumps({'enabled': True, 'available': True, 'key': key, 'samples': 0, 'subjects': 0})
                return json.dumps(health)
            with self.subTest(key=key), patch.object(desktop, 'service_state', return_value=service), \
                 patch.object(desktop, 'accessibility_state', return_value={'loaded': False, 'ready': False}), \
                 patch.object(desktop, 'native_state', return_value={'active': 'active', 'addon_loaded': True}), \
                 patch.object(desktop, 'control', side_effect=control):
                report = desktop.health_report()
            self.assertEqual(any('cannot seal samples' in note for note in report['notes']), noted)

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
        desktop.set_app(document, 'nvim', True)
        desktop.set_app(document, 'libreoffice-writer', True)
        desktop.set_app(document, 'libreoffice', True)
        # Adapter names sort alphabetically, as the broker's canonical order requires.
        self.assertEqual([(item['identity']['adapter'], item['identity']['app_id']) for item in document['subjects']],
                         [('fcitx', 'libreoffice'), ('libreoffice', 'libreoffice-writer'), ('neovim', 'nvim'),
                          ('obsidian', 'obsidian'), ('shell', 'bash')])

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
        desktop.set_mode(document, 'all_web_origins', True)
        self.assertIs(document['all_web_origins'], True)
        self.assertEqual(document['subjects'], exact)
        desktop.set_mode(document, 'all_web_origins', False)
        self.assertNotIn('all_web_origins', document)
        self.assertEqual(document['subjects'], exact)

        settings = {'schema': 'badi.settings.v2', 'revision': 7, 'paused': False, 'subjects': exact}
        for mode, expected in (('on', True), ('off', None)):
            with patch.object(desktop, 'control', side_effect=[json.dumps(settings), '{}']) as command, \
                 contextlib.redirect_stdout(io.StringIO()) as output:
                desktop.main(['site', 'all', mode])
            request = command.call_args.args[0]
            self.assertEqual(request, ['settings', 'replace', '--if-revision', '7', '--json', '-'])
            sent = json.loads(command.call_args.kwargs['input_text'])
            self.assertEqual(sent['revision'], 8)
            self.assertEqual(sent.get('all_web_origins'), expected)
            self.assertEqual(sent['subjects'], exact)
            if expected:
                self.assertIn('unless its exact site rule blocks it', output.getvalue())
        for invalid in (['site', 'all', 'maybe'], ['site', 'all']):
            with patch.object(desktop, 'control') as command, self.assertRaises(RuntimeError):
                desktop.main(invalid)
            command.assert_not_called()

    def test_mid_line_is_omitted_when_off(self):
        settings = {'schema': 'badi.settings.v2', 'revision': 7, 'paused': False, 'subjects': []}
        for arguments, expected in ((['mid-line', 'on'], True), (['mid-line', 'off'], None)):
            with self.subTest(arguments=arguments), \
                 patch.object(desktop, 'control', side_effect=[json.dumps(settings), '{}']) as command, \
                 contextlib.redirect_stdout(io.StringIO()):
                desktop.main(arguments)
            sent = json.loads(command.call_args.kwargs['input_text'])
            self.assertEqual(sent.get('mid_line'), expected)
            self.assertEqual(sent['subjects'], [])

    def test_escape_quiet_is_a_bounded_number_omitted_when_off(self):
        settings = {'schema': 'badi.settings.v2', 'revision': 7, 'paused': False, 'subjects': []}
        for arguments, expected in ((['escape-quiet', '90'], 90), (['escape-quiet', 'off'], None)):
            with self.subTest(arguments=arguments), \
                 patch.object(desktop, 'control', side_effect=[json.dumps(settings), '{}']) as command, \
                 contextlib.redirect_stdout(io.StringIO()):
                desktop.main(arguments)
            sent = json.loads(command.call_args.kwargs['input_text'])
            self.assertEqual(sent.get('escape_quiet_seconds'), expected)
            self.assertEqual(sent['subjects'], [])
        for invalid in (['escape-quiet', '0'], ['escape-quiet', '3601'], ['escape-quiet', 'soon']):
            with patch.object(desktop, 'control') as command, self.assertRaises(RuntimeError):
                desktop.main(invalid)
            command.assert_not_called()

    def test_list_modes_listing_and_reset(self):
        settings = {'schema': 'badi.settings.v2', 'revision': 7, 'paused': False, 'subjects': []}
        desktop.set_language(settings, 'brave-origin', 'de')
        desktop.set_language(settings, 'chromium-browser', 'en')
        self.assertEqual(settings['language_overrides'], {'brave-origin': 'de', 'chromium': 'en'})
        desktop.set_language(settings, 'chromium', 'reset')
        self.assertEqual(settings['language_overrides'], {'brave-origin': 'de'})
        desktop.set_language(settings, 'omawrite', 'fr')
        self.assertEqual(settings['language_overrides']['omawrite'], 'fr')
        with self.assertRaisesRegex(RuntimeError, 'Use en, de, fa, fr, es, pt or reset'):
            desktop.set_language(settings, 'omawrite', 'it')
        desktop.set_language(settings, 'omawrite', 'reset')
        desktop.set_app(settings, 'discord', False)
        desktop.set_app(settings, 'omawrite', True)
        desktop.set_site(settings, 'https://bank.example', False)
        desktop.set_site(settings, 'http://localhost:8080', True)
        exact = [dict(item) for item in settings['subjects']]
        for arguments, key, expected in ((['apps', 'blocklist'], 'all_linux_apps', True),
                                         (['apps', 'allowlist'], 'all_linux_apps', None),
                                         (['app', 'all', 'on'], 'all_linux_apps', True),
                                         (['sites', 'blocklist'], 'all_web_origins', True),
                                         (['site', 'all', 'off'], 'all_web_origins', None)):
            with self.subTest(arguments=arguments), \
                 patch.object(desktop, 'control', side_effect=[json.dumps(settings), '{}']) as command, \
                 contextlib.redirect_stdout(io.StringIO()):
                desktop.main(arguments)
            sent = json.loads(command.call_args.kwargs['input_text'])
            self.assertEqual(sent.get(key), expected)
            self.assertEqual(sent['subjects'], exact, 'a mode change keeps every exact rule')
            self.assertEqual(sent['language_overrides'], {'brave-origin': 'de'})
        listing = {**settings, 'all_linux_apps': True}
        with patch.object(desktop, 'control', return_value=json.dumps(listing)), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['app', 'list'])
        self.assertEqual(output.getvalue().splitlines(),
                         ['Apps: blocklist: every supported app except blocked ones',
                          '  blocked  discord', '  allowed  omawrite'])
        with patch.object(desktop, 'control', return_value=json.dumps(settings)), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['site', 'list'])
        self.assertEqual(output.getvalue().splitlines(),
                         ['Sites: allowlist: only allowed sites', '  allowed  http://localhost:8080',
                          '  blocked  https://bank.example'])
        for arguments, removed in ((['app', 'discord', 'reset'], 'discord'), (['site', 'https://bank.example', 'reset'], 'bank.example')):
            with self.subTest(arguments=arguments), \
                 patch.object(desktop, 'control', side_effect=[json.dumps(settings), '{}']) as command, \
                 contextlib.redirect_stdout(io.StringIO()) as output:
                desktop.main(arguments)
            sent = json.loads(command.call_args.kwargs['input_text'])
            remaining = [item['identity'].get('app_id') or item['identity'].get('host') for item in sent['subjects']]
            self.assertNotIn(removed, remaining)
            self.assertEqual(len(remaining), len(exact) - 1)
            self.assertIn('follows the', output.getvalue())
        for invalid in (['apps', 'maybe'], ['app', 'list', 'extra']):
            with patch.object(desktop, 'control') as command, self.assertRaises(RuntimeError):
                desktop.main(invalid)
            command.assert_not_called()

    def test_status_and_doctor_state_the_all_sites_default(self):
        health = {'paused': False, 'provider': 'local_model',
                  'metrics': {'provider_calls': 1, 'suggestions_shown': 1, 'provider_errors': 0, 'no_suggestion': BREAKDOWN}}
        settings = {'schema': 'badi.settings.v2', 'revision': 2, 'paused': False,
                    'all_web_origins': True, 'subjects': []}
        self.assertIn('Web sites: blocklist: every http(s) site except blocked ones', desktop.status_text(health, settings))
        self.assertIn('Web sites: allowlist: only allowed sites',
                      desktop.status_text(health, {**settings, 'all_web_origins': False}))
        self.assertIn('Apps: allowlist: only allowed apps', desktop.status_text(health, settings))
        with patch.object(desktop, 'control', side_effect=[json.dumps(health), json.dumps(settings)]), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['status'])
        self.assertIn('Web sites: blocklist: every http(s) site', output.getvalue())
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
            self.assertNotIn(desktop.ALL_APPS_NOTE, report['notes'])
            self.assertEqual(report['problems'], [])
            self.assertTrue(any('use IME-parity' in note for note in report['notes']))
            self.assertFalse(any('writing is disabled' in note for note in report['notes']))
        # Site-all also opens the observed browser path, which has no second
        # host gate; both the note and help must say so.
        # Zen shares the Chromium browser-origin grants, so the disclosure names it.
        self.assertIn('Chromium/Brave/Zen fields, which have no second site gate', desktop.ALL_SITES_NOTE)
        self.assertIn('private windows', desktop.ALL_SITES_NOTE)
        self.assertIn('Chromium/Brave/Zen fields', desktop.HELP)
        self.assertIn('one site grant covers all three browsers', ' '.join(desktop.HELP.split()))

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
            self.assertEqual(desktop.native_state(), {'active': 'active', 'addon_loaded': True, 'addon_update_pending': True,
                                                      'launcher': 'omarchy-fcitx5.service'})

    def test_without_omarchys_fcitx_unit_the_session_bus_owner_is_inspected(self):
        missing = subprocess.CompletedProcess([], 0, 'LoadState=not-found\nActiveState=inactive\nMainPID=0\n', '')
        for owner, maps, expected in (
                (4242, '7f00 r-xp 0 00:01 9 /home/someone/.local/lib/fcitx5/libbadi-fcitx5.so\n',
                 {'active': 'active', 'addon_loaded': True, 'addon_update_pending': False}),
                (4242, '7f00 r-xp 0 00:01 9 /usr/lib/x86_64-linux-gnu/fcitx5/libdbus.so\n',
                 {'active': 'active', 'addon_loaded': False, 'addon_update_pending': False}),
                (0, '', {'active': 'inactive', 'addon_loaded': False, 'addon_update_pending': False}),
                (None, '', {'active': 'unknown', 'addon_loaded': None, 'addon_update_pending': None,
                            'inspection_error': 'native_process_uninspectable'})):
            with self.subTest(owner=owner), \
                 patch.object(desktop.subprocess, 'run', return_value=missing), \
                 patch.object(desktop.badi_install, 'fcitx_bus_process', return_value=owner), \
                 patch.object(desktop.Path, 'read_text', return_value=maps) as read:
                self.assertEqual(desktop.native_state(), {**expected, 'launcher': 'session'})
                if owner:
                    self.assertEqual(read.call_count, 1)

    def test_doctor_names_the_session_way_to_load_the_addon(self):
        service = {'loaded': True, 'active': 'active', 'state': 'running', 'autostart': True}
        broker = {'provider': 'local_model', 'paused': False, 'metrics': {'provider_calls': 3}}
        for native, message, action in (
                ({'active': 'active', 'addon_loaded': False, 'launcher': 'session'},
                 'The running Fcitx has not loaded Badi.', 'Log out and back in'),
                ({'active': 'inactive', 'addon_loaded': False, 'launcher': 'session'},
                 'Fcitx is not running in this session.', 'im-config -n fcitx5')):
            with self.subTest(native=native), \
                 patch.object(desktop, 'service_state', return_value=service), \
                 patch.object(desktop, 'accessibility_state', return_value={'loaded': False, 'ready': False}), \
                 patch.object(desktop, 'native_state', return_value=native), \
                 patch.object(desktop, 'install_state', return_value={}), \
                 patch.object(desktop, 'control', return_value=json.dumps(broker)):
                problem, = desktop.health_report()['problems']
            self.assertEqual((problem['code'], problem['message']), ('native_addon_unavailable', message))
            self.assertIn(action, problem['action'])

    def test_an_observer_skipped_by_its_session_condition_is_a_note_not_a_problem(self):
        skipped = ('LoadState=loaded\nActiveState=inactive\nMainPID=0\nConditionResult=no\n'
                   'ConditionTimestampMonotonic=123456\n')
        never_started = skipped.replace('ConditionTimestampMonotonic=123456', 'ConditionTimestampMonotonic=0')
        for fields, unmet in ((skipped, True), (never_started, False)):
            with self.subTest(fields=fields), \
                 patch.object(desktop.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, fields, '')):
                self.assertEqual(desktop.observer_service_state().get('condition_unmet', False), unmet)
        service = {'loaded': True, 'active': 'active', 'state': 'running', 'autostart': True}
        with patch.object(desktop, 'service_state', return_value=service), \
             patch.object(desktop, 'accessibility_state',
                          return_value={'loaded': True, 'ready': False, 'active': 'inactive', 'condition_unmet': True}), \
             patch.object(desktop, 'native_state', return_value={'active': 'active', 'addon_loaded': True}), \
             patch.object(desktop, 'install_state', return_value={}), \
             patch.object(desktop, 'control', return_value='{"metrics":{"provider_calls":1}}'):
            report = desktop.health_report()
        self.assertEqual(report['problems'], [])
        self.assertIn(desktop.OBSERVER_SKIPPED_NOTE, report['notes'])
        self.assertIn('Field observer: not for this session', desktop.doctor_text(report))

    def test_the_settings_panel_is_omarchys(self):
        with patch.object(desktop.badi_install, 'desktop_platform', return_value=desktop.badi_install.GENERIC), \
             patch.object(desktop.subprocess, 'run') as command:
            with self.assertRaisesRegex(RuntimeError, "part of Omarchy's bar.*badi --help"):
                desktop.main(['settings'])
            command.assert_not_called()
        with patch.object(desktop.badi_install, 'desktop_platform', return_value=desktop.badi_install.OMARCHY), \
             patch.object(desktop.subprocess, 'run') as command:
            desktop.main(['settings'])
        self.assertEqual(command.call_args.args[0], ['omarchy-shell', 'badi-writing', 'toggle'])

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

    def test_corrupt_settings_stop_the_service_instead_of_restarting(self):
        with patch.object(desktop.subprocess, 'run', side_effect=[
            subprocess.CompletedProcess([], 0, 'c' * 32 + '\n', ''),
            subprocess.CompletedProcess([], 0, 'settings path\nerror_code=control_plane: unsafe_file\n', ''),
        ]):
            problem = desktop.startup_problem()
            self.assertEqual(problem['code'], 'settings_unsafe_file')
            self.assertNotIn('settings path', json.dumps(problem))

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
                desktop.main(['doctor', '--json'])
        self.assertEqual(stopped.exception.code, 1)
        report = json.loads(output.getvalue())
        self.assertEqual(report['problems'][0]['code'], 'model_not_installed')
        self.assertIn('service_failed', [problem['code'] for problem in report['problems']])
        with patch.object(desktop, 'health_report', return_value=report), \
             contextlib.redirect_stdout(io.StringIO()) as human:
            with self.assertRaises(SystemExit) as stopped:
                desktop.main(['doctor'])
        self.assertEqual(stopped.exception.code, 1)
        self.assertIn('Model: unreachable', human.getvalue())
        self.assertIn('Next: Provision the pinned model', human.getvalue())
        report['problems'] = []
        with patch.object(desktop, 'health_report', return_value=report), \
             contextlib.redirect_stdout(io.StringIO()) as human:
            desktop.main(['doctor'])
        self.assertIn('no problems found', human.getvalue())

    def test_observer_readiness_is_bound_to_a_stable_service_process(self):
        running = {'loaded': True, 'active': 'active', 'process_id': 123}
        replacement = {**running, 'process_id': 456}
        with patch.object(desktop, 'observer_service_state', side_effect=[running, replacement]), \
             patch.object(desktop, 'runtime_directory', return_value=Path('/private/badi')), \
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
             patch.object(desktop, 'runtime_directory', return_value=Path('/private/badi')), \
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

    def test_a_debug_run_is_deleted_when_it_ends_unless_a_newer_run_replaced_it(self):
        with patch.object(desktop, 'runtime_directory', return_value=self.directory), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['debug', 'on', '--text'])
            run = desktop.private_json(self.directory / 'debug-control.json')['id']
            self.expiry.assert_called_once_with(run)
            self.assertIn('deleted when the run ends', output.getvalue())
            self.assertIn('never a password field', output.getvalue())
            desktop.main(['debug', 'expire', '00000000-0000-4000-8000-000000000000'])
            self.assertTrue((self.directory / 'debug-trace.jsonl').exists(), 'another run is not this one')
            desktop.main(['debug', 'expire', run])
            self.assertFalse((self.directory / 'debug-trace.jsonl').exists())
            self.assertFalse((self.directory / 'debug-control.json').exists())
        # The timer is the user manager's: fixed arguments, the run's own id,
        # and nothing at all without systemd-run.
        with patch.object(desktop.shutil, 'which', return_value='/usr/bin/systemd-run'), \
             patch.object(desktop.subprocess, 'run', return_value=SimpleNamespace(returncode=0)) as command:
            self.assertTrue(self.schedule_debug_expiry('0f1e2d3c-0000-4000-8000-000000000000'))
        argv = command.call_args.args[0]
        self.assertEqual(argv[:6], ['systemd-run', '--user', '--quiet', '--collect',
                                    '--unit=badi-debug-expiry-0f1e2d3c', '--on-active=901'])
        self.assertEqual(argv[-3:], ['debug', 'expire', '0f1e2d3c-0000-4000-8000-000000000000'])
        with patch.object(desktop.shutil, 'which', return_value=None):
            self.assertFalse(self.schedule_debug_expiry('0f1e2d3c-0000-4000-8000-000000000000'))

    def test_trace_lines_escape_any_control_character_a_writer_sent(self):
        line = desktop.format_trace({'t': 0, 'src': 'fcitx\x1b[2J', 'ev': 'key', 'app': 'evil\x1b]0;title\x07',
                                     'reason': 'plain'})
        self.assertNotIn('\x1b', line)
        self.assertNotIn('\x07', line)
        self.assertIn('reason=plain', line)
        self.assertIn('app="evil\\u001b]0;title\\u0007"', line)

    def test_debug_on_creates_a_private_trace_and_text_only_when_asked(self):
        trace = self.directory / 'debug-trace.jsonl'
        with patch.object(desktop, 'runtime_directory', return_value=self.directory), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['debug', 'on'])
            self.assertFalse(desktop.private_json(self.directory / 'debug-control.json')['text'])
            self.assertEqual((trace.stat().st_mode & 0o777, trace.stat().st_size), (0o600, 0))
            self.assertIn('no typed text is stored', output.getvalue())
            trace.write_text('{"ev": "old run"}\n')
            desktop.main(['debug', 'on', '--text'])
            self.assertTrue(desktop.private_json(self.directory / 'debug-control.json')['text'])
            self.assertEqual(trace.stat().st_size, 0, 'a new run starts an empty trace')
            desktop.main(['debug', 'off'])
            self.assertFalse(trace.exists())
            self.assertFalse((self.directory / 'debug-control.json').exists())

    def test_debug_log_prints_complete_lines_with_text_last(self):
        trace = self.directory / 'debug-trace.jsonl'
        trace.write_text(
            json.dumps({'t': 1000, 'src': 'broker', 'ev': 'out suggestion.show', 'text': ' next words',
                        'chars': 11, 'sid': 'abcd1234'}) + '\n'
            + 'not json\n'
            + json.dumps({'t': 2000, 'src': 'observer', 'ev': 'preview drawn', 'placement': 'chip'}) + '\n'
            + '{"t": 3000, "src": "fcitx"')
        trace.chmod(0o600)
        with patch.object(desktop, 'runtime_directory', return_value=self.directory), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            desktop.main(['debug', 'log'])
        shown, drawn = output.getvalue().splitlines()
        self.assertRegex(shown, r'^\d\d:\d\d:\d\d\.000  broker     out suggestion.show +chars=11 sid=abcd1234 '
                                r'text=" next words"$')
        self.assertIn('observer   preview drawn', drawn)
        self.assertIn('placement=chip', drawn)
        trace.chmod(0o644)
        with patch.object(desktop, 'runtime_directory', return_value=self.directory), \
             self.assertRaisesRegex(RuntimeError, 'private'):
            desktop.print_trace()

    def test_watch_reads_the_preview_layer_from_hyprland(self):
        layers = {'eDP-1': {'levels': {'2': [{'namespace': 'omarchy-bar', 'x': 0, 'y': 0, 'w': 1440, 'h': 26}],
                                       '3': [{'namespace': 'badi-writing-preview', 'x': 491, 'y': 312,
                                              'w': 185, 'h': 22}]}}}
        run = lambda *_a, **_k: SimpleNamespace(returncode=0, stdout=json.dumps(layers))
        self.assertEqual(desktop.preview_layers(run=run), [('eDP-1', 491, 312, 185, 22)])
        entry = desktop.compositor_entry(desktop.preview_layers(run=run))
        self.assertEqual((entry['ev'], entry['x'], entry['width']), ('preview on screen', 491, 185))
        self.assertEqual(desktop.compositor_entry([])['ev'], 'preview not on screen')
        failed = lambda *_a, **_k: SimpleNamespace(returncode=1, stdout='')
        self.assertIsNone(desktop.preview_layers(run=failed))

    def test_writing_languages_and_healing_stay_canonical(self):
        document = {}
        desktop.set_writing(document, "pt", True)
        desktop.set_writing(document, "es", True)
        desktop.set_writing(document, "fr", True)
        self.assertEqual(document["writing_languages"], ["en", "de", "fr", "es", "pt"])
        desktop.set_writing(document, "fr", False)
        desktop.set_writing(document, "es", False)
        desktop.set_writing(document, "pt", False)
        self.assertNotIn("writing_languages", document, "English and German are the omitted default")
        desktop.set_writing(document, "fa", True)
        self.assertEqual(document["writing_languages"], ["en", "de", "fa"], "Persian only from the terminal")
        desktop.set_writing(document, "fa", False)
        self.assertNotIn("writing_languages", document)
        with self.assertRaisesRegex(RuntimeError, "Use one of"):
            desktop.set_writing(document, "it", True)
        desktop.set_healing(document, "fr", "on")
        desktop.set_healing(document, "de", "off")
        self.assertEqual(document["trailing_space_healing"], {"fr": True, "de": False})
        report = {entry["language"]: entry for entry in desktop.writing_report(document, ["en", "de", "fa"])}
        self.assertNotIn("fa", report, "Persian is listed only while it is on")
        self.assertIn("fa", {entry["language"] for entry in
                             desktop.writing_report({"writing_languages": ["en", "fa"]}, ["en"])})
        self.assertEqual((report["fr"]["healing"], report["fr"]["healed"], report["fr"]["enabled"]), ("on", True, False))
        self.assertEqual((report["de"]["healing"], report["de"]["healed"]), ("off", False))
        self.assertEqual((report["en"]["healing"], report["en"]["healed"]), ("auto", True))
        self.assertTrue(report["es"]["experimental"] and not report["en"]["experimental"])
        desktop.set_healing(document, "fr", "auto")
        desktop.set_healing(document, "de", "auto")
        self.assertNotIn("trailing_space_healing", document)
        with self.assertRaisesRegex(RuntimeError, "Usage: badi suggest"):
            desktop.set_healing(document, "fr", "maybe")

    def test_state_changes_ask_the_bar_mark_to_look_again(self):
        settings = {'schema': 'badi.settings.v2', 'revision': 3, 'paused': False, 'subjects': []}
        for arguments, replies in ((['pause'], [json.dumps(settings), '{}']),
                                   (['snooze', 'end'], ['{}']),
                                   (['length', 'short'], [json.dumps(settings), '{}'])):
            self.bar.reset_mock()
            with self.subTest(arguments=arguments), \
                 patch.object(desktop, 'control', side_effect=replies), \
                 contextlib.redirect_stdout(io.StringIO()):
                desktop.main(arguments)
            self.bar.assert_called_once_with()
        # One quiet, bounded call where Omarchy runs; nothing anywhere else,
        # and a shell that cannot answer changes nothing.
        with patch.object(desktop.shutil, 'which', return_value='/usr/bin/omarchy-shell'), \
             patch.object(desktop.subprocess, 'run') as command:
            self.refresh_bar()
        command.assert_called_once()
        self.assertEqual(command.call_args.args[0], ['omarchy-shell', '-q', 'badi-writing', 'refresh'])
        self.assertEqual(command.call_args.kwargs['timeout'], 3)
        with patch.object(desktop.shutil, 'which', return_value=None), \
             patch.object(desktop.subprocess, 'run') as command:
            self.refresh_bar()
        command.assert_not_called()
        with patch.object(desktop.shutil, 'which', return_value='/usr/bin/omarchy-shell'), \
             patch.object(desktop.subprocess, 'run', side_effect=subprocess.TimeoutExpired('omarchy-shell', 3)):
            self.refresh_bar()

    def test_languages_set_suggestions_and_typo_fixes_in_one_write(self):
        document = {"writing_languages": ["en", "de", "fa"], "trailing_space_healing": {"de": False}}
        desktop.set_languages(document, ["fr", "en", "it", "pt"])
        self.assertEqual(document["writing_languages"], ["en", "fa", "fr", "pt"], "a terminal Persian choice stays")
        self.assertEqual(document["spelling_languages"], ["en", "fr", "it"], "Portuguese has no dictionary")
        self.assertIs(document["languages_chosen"], True)
        self.assertEqual(document["trailing_space_healing"], {"de": False}, "healing choices are untouched")
        document = {"spelling_languages": ["en", "de"]}
        desktop.set_languages(document, ["de", "en"])
        self.assertNotIn("writing_languages", document, "English and German are the omitted default")
        self.assertEqual(document["spelling_languages"], ["en", "de"])
        desktop.set_languages(document, ["en"])
        self.assertEqual(document["writing_languages"], ["en"])
        self.assertNotIn("spelling_languages", document, "English alone is the omitted default")
        for invalid in ([], ["fa"], ["en", "en"], ["xx"]):
            with self.assertRaisesRegex(RuntimeError, "Usage: badi languages"):
                desktop.set_languages({}, invalid)
        text = desktop.languages_text({"writing_languages": ["en", "pt"], "spelling_languages": ["en", "nl"]})
        self.assertIn("Languages: English, Português, Nederlands · not chosen yet", text)
        self.assertIn("Português   suggestions (typo fixes not offered)", text)
        self.assertIn("Nederlands  typo fixes (suggestions not offered)", text)
        self.assertNotIn("not chosen", desktop.languages_text({"languages_chosen": True}))
        settings = {'schema': 'badi.settings.v2', 'revision': 3, 'paused': False, 'subjects': []}
        with patch.object(desktop, 'control', side_effect=[json.dumps(settings), '{}', json.dumps(settings)]) as command, \
             contextlib.redirect_stdout(io.StringIO()):
            desktop.main(['languages', 'de', 'fr'])
        sent = json.loads(command.call_args_list[1].kwargs['input_text'])
        self.assertEqual((sent['revision'], sent['writing_languages'], sent['spelling_languages'], sent['languages_chosen']),
                         (4, ['de', 'fr'], ['de', 'fr'], True))

    def test_revise_reports_and_installs_only_the_missing_revision_model(self):
        catalog = {"catalog": [{"id": "qwen3-1.7b", "name": "Qwen3 1.7B", "filename": "Qwen3-1.7B-Q4_K_M.gguf",
                                "download_bytes": 1_282_439_264, "license": "Apache-2.0", "fits": True}]}
        with patch.object(desktop, "model_advice", return_value=catalog), \
             patch.object(desktop, "MODEL_DIRECTORY", self.directory), \
             patch.object(desktop.badi_install, "download_model") as download:
            report, _model = desktop.revision_report()
            self.assertEqual((report["model"], report["installed"]), ("qwen3-1.7b", False))
            installed, downloaded = desktop.install_revision_model()
            self.assertTrue(downloaded and installed["installed"])
            download.assert_called_once()
            (self.directory / "Qwen3-1.7B-Q4_K_M.gguf").write_bytes(b"weights")
            self.assertEqual(desktop.install_revision_model()[1], False, "nothing downloads twice")
            download.assert_called_once()
            catalog["catalog"][0]["fits"] = False
            (self.directory / "Qwen3-1.7B-Q4_K_M.gguf").unlink()
            with self.assertRaisesRegex(RuntimeError, "does not fit"):
                desktop.install_revision_model()

    def test_spelling_languages_stay_canonical_and_name_missing_dictionaries(self):
        document = {}
        desktop.set_spelling(document, "fr", True)
        desktop.set_spelling(document, "de", True)
        self.assertEqual(document["spelling_languages"], ["en", "de", "fr"])
        desktop.set_spelling(document, "de", False)
        desktop.set_spelling(document, "fr", False)
        self.assertNotIn("spelling_languages", document, "English alone is the omitted default")
        desktop.set_spelling(document, "en", False)
        self.assertEqual(document["spelling_languages"], [])
        with self.assertRaisesRegex(RuntimeError, "Use one of"):
            desktop.set_spelling(document, "fa", True)
        (self.directory / "de_AT.aff").write_text("SET UTF-8\n")
        (self.directory / "de_AT.dic").write_text("1\nHaus\n")
        with patch.object(desktop, "DICTIONARY_DIRECTORIES", (self.directory,)), \
             patch.object(desktop.badi_install, "distribution_family", return_value="arch"):
            report = {entry["language"]: entry for entry in
                      desktop.spelling_report({"spelling_languages": ["en", "de", "fr"]})}
        self.assertEqual(report["en"]["dictionary"], "built in")
        self.assertEqual(report["de"]["dictionary"], "de_AT")
        self.assertEqual((report["fr"]["dictionary"], report["fr"]["package"]), (None, "hunspell-fr-comprehensive"))
        self.assertEqual(report["fr"]["install"], "sudo pacman -S hunspell-fr-comprehensive")
        self.assertIsNone(report["en"]["install"])
        self.assertFalse(report["es"]["enabled"])
        for family, command in (("debian", "sudo apt install hunspell-fr"),
                                (None, "sudo pacman -S hunspell-fr-comprehensive")):
            with self.subTest(family=family), patch.object(desktop, "DICTIONARY_DIRECTORIES", (self.directory,)), \
                 patch.object(desktop.badi_install, "distribution_family", return_value=family), \
                 contextlib.redirect_stdout(io.StringIO()) as printed:
                desktop.print_spelling(desktop.spelling_report({"spelling_languages": ["en", "fr"]}))
            self.assertIn(f"needs: {command} (or its", printed.getvalue())
        self.assertEqual(set(desktop.DEBIAN_DICTIONARIES), set(desktop.SPELLING) - {"en"})

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
        self.assertIn("Tab accepts a visible suggestion's next word, otherwise stays Tab", text)
        self.assertIn('Ctrl+Right accepts all of it', text)
        self.assertIn('Tab accepts the next word of a visible suggestion', ' '.join(desktop.HELP.split()))
        self.assertIn('IME-parity accepts once, append-only, like typing', ' '.join(desktop.HELP.split()))
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
        self.assertIn('budget_prefill=2: the writing budget (550 ms while typing, 1.2 s after an explicit request) ended before the model started answering', note)
        self.assertLess(note.index('budget_prefill=2'), note.index('budget_stream=1'))
        self.assertIn('Last: budget_prefill.', note)
        self.assertEqual(report['problems'], [])
        gated = {**broker, 'metrics': {**broker['metrics'],
                                       'no_suggestion': {**BREAKDOWN, 'low_confidence': 4, 'last': 'low_confidence'}}}
        with patch.object(desktop, 'service_state', return_value=service), \
             patch.object(desktop, 'accessibility_state', return_value={'loaded': False, 'ready': False}), \
             patch.object(desktop, 'native_state', return_value={'active': 'active', 'addon_loaded': True}), \
             patch.object(desktop, 'control', return_value=json.dumps(gated)):
            note = next(item for item in desktop.health_report()['notes'] if 'showed no suggestion' in item)
        self.assertIn('low_confidence=4: a word of the suggestion was too unlikely', note)
        self.assertIn('Last: low_confidence.', note)
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
