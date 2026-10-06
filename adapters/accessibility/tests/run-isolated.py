#!/usr/bin/env python3
"""Run real accessibility signal tests on task-owned buses without a desktop."""
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import unittest


def session():
    root = Path(os.environ['BADI_A11Y_TEST_ROOT'])
    if os.environ.get('XDG_RUNTIME_DIR') != str(root / 'runtime') or os.environ.get('HOME') != str(root / 'home'):
        raise RuntimeError('The integration session must use its private runtime and home')
    import gi
    gi.require_version('Atspi', '2.0')
    from gi.repository import Gio, GLib

    # Warm only this fixture's infrastructure before testing the helper's short
    # operation deadlines. A cold CI service activation is not a field read.
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    address = bus.call_sync('org.a11y.Bus', '/org/a11y/bus', 'org.a11y.Bus', 'GetAddress', None,
                            GLib.VariantType.new('(s)'), Gio.DBusCallFlags.NONE, 3000, None).unpack()[0]
    accessibility = Gio.DBusConnection.new_for_address_sync(address,
        Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION, None, None)
    try:
        accessibility.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
            'StartServiceByName', GLib.Variant('(su)', ('org.a11y.atspi.Registry', 0)), None,
            Gio.DBusCallFlags.NONE, 3000, None)
    finally:
        accessibility.close_sync(None)
    suite = unittest.defaultTestLoader.discover(str(Path(__file__).parent), pattern='test_signal_scope.py')
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    if result.skipped or result.testsRun == 0:
        raise RuntimeError('The explicit accessibility integration may not skip its real bus checks')
    return 0 if result.wasSuccessful() else 1


def main():
    if sys.argv[1:] == ['--session']:
        return session()
    if sys.argv[1:]:
        raise RuntimeError('This runner takes no arguments')
    if not shutil.which('dbus-run-session'):
        raise RuntimeError('Install dbus-run-session before running accessibility integration')
    service = Path('/usr/share/dbus-1/services/org.a11y.Bus.service')
    if not service.is_file():
        raise RuntimeError('Install at-spi2-core before running accessibility integration')
    with tempfile.TemporaryDirectory(prefix='badi-a11y-bus-') as temporary:
        root = Path(temporary)
        environment = dict(os.environ)
        for name, folder in [('HOME', 'home'), ('XDG_RUNTIME_DIR', 'runtime'), ('XDG_CONFIG_HOME', 'config'),
                             ('XDG_DATA_HOME', 'data'), ('XDG_CACHE_HOME', 'cache'), ('XDG_STATE_HOME', 'state')]:
            (root / folder).mkdir(mode=0o700)
            environment[name] = str(root / folder)
        for name in ['DISPLAY', 'WAYLAND_DISPLAY', 'AT_SPI_BUS_ADDRESS', 'DBUS_SESSION_BUS_ADDRESS',
                     'HYPRLAND_INSTANCE_SIGNATURE', 'DBUS_STARTER_ADDRESS', 'DBUS_STARTER_BUS_TYPE']:
            environment.pop(name, None)
        environment.update(BADI_A11Y_TEST_ROOT=str(root), BADI_A11Y_REQUIRE_SESSION_BUS='1',
                           PYTHONDONTWRITEBYTECODE='1', ATSPI_DBUS_IMPLEMENTATION='dbus-daemon',
                           GSETTINGS_BACKEND='memory', XDG_CONFIG_DIRS='/etc/xdg', XDG_DATA_DIRS='/usr/share')
        services = root / 'services'
        services.mkdir(mode=0o700)
        shutil.copy2(service, services / service.name)
        config = root / 'bus.conf'
        config.write_text('<busconfig><type>session</type><keep_umask/><listen>unix:tmpdir=/tmp</listen>'
            '<auth>EXTERNAL</auth><servicedir>' + str(services) + '</servicedir>'
            '<policy context="default"><allow send_destination="*"/><allow receive_sender="*"/>'
            '<allow own="*"/></policy></busconfig>')
        child = subprocess.Popen(['dbus-run-session', f'--config-file={config}', '--', sys.executable,
                                  str(Path(__file__).resolve()), '--session'], env=environment, start_new_session=True)
        try:
            return child.wait(timeout=30)
        finally:
            # All bus activation descendants belong to this task's process
            # group. Retire stragglers even when the session command has exited.
            try:
                os.killpg(child.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            if child.poll() is None:
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait(timeout=5)


if __name__ == '__main__':
    sys.exit(main())
