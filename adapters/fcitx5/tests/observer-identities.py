#!/usr/bin/env python3
"""Fail when the addon's IME-parity app ids and the observer's web-app and observed-only rules differ.

IME-parity apps accept only through an observed field, so an id the observer
has no rule for can never accept, and an observed web app the addon does not
classify as IME-parity would take the native exact contract instead.
"""
import ast
from pathlib import Path
import re
import sys

ADAPTERS = Path(__file__).resolve().parents[2]


def addon_list(source, name):
    match = re.search(r'\b' + name + r'\s*\{([^}]*)\}', source)
    if not match:
        raise SystemExit(f'state.cpp no longer defines {name}; update {Path(__file__).name}')
    return set(re.findall(r'"([^"]+)"', match.group(1)))


def observer_apps(source):
    for node in ast.parse(source).body:
        if isinstance(node, ast.Assign) and any(getattr(target, 'id', None) == 'APPS' for target in node.targets):
            return {ast.literal_eval(key): {flag.arg: ast.literal_eval(flag.value)
                                            for flag in rule.keywords if flag.arg in ('browser', 'web', 'observed_only')}
                    for key, rule in zip(node.value.keys, node.value.values)}
    raise SystemExit(f'daemon.py no longer defines APPS; update {Path(__file__).name}')


def mismatches(state_source, observer_source):
    browsers = addon_list(state_source, 'kImeParityBrowsers')
    desktop = addon_list(state_source, 'kImeParityDesktopApps')
    native = addon_list(state_source, 'kImeParityNativeApps')
    apps = observer_apps(observer_source)
    web = {app for app, flags in apps.items() if flags.get('web')}
    observed_native = {app for app, flags in apps.items() if flags.get('observed_only') and not flags.get('web')}
    observed_browsers = {app for app in web if apps[app].get('browser')}
    problems = []
    if missing := sorted((browsers | desktop) - web):
        problems.append(f'IME-parity ids without an observer web rule, which can never accept: {missing}')
    if unclassified := sorted(web - browsers - desktop):
        problems.append(f'observer web rules the addon does not classify as IME-parity: {unclassified}')
    if native != observed_native:
        problems.append(f'IME-parity native ids and observed-only native rules differ: {sorted(native ^ observed_native)}')
    if wrong_kind := sorted(((browsers & web) - observed_browsers) | (desktop & observed_browsers)):
        problems.append(f'IME-parity class differs from the observer browser flag: {wrong_kind}')
    return problems


def main():
    state = Path(sys.argv[1]) if len(sys.argv) > 1 else ADAPTERS / 'fcitx5/src/state.cpp'
    observer = Path(sys.argv[2]) if len(sys.argv) > 2 else ADAPTERS / 'accessibility/daemon.py'
    problems = mismatches(state.read_text(), observer.read_text())
    if problems:
        raise SystemExit('\n'.join([f'{state} and {observer} disagree:', *problems]))
    print('IME-parity app ids match the observer web-app and observed-only rules')


if __name__ == '__main__':
    main()
