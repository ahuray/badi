// Badi focus (ADR 0016): answers which window has focus on gnome-shell's own
// session-bus connection, for Badi's field observer. It exposes the focused
// window's process id, app id, client type and minimized state, and a signal
// when focus or that app id changes; never a title, content, geometry or
// window list, and nothing that changes the desktop. It runs in the user
// session mode only, so GNOME disables it while the screen is locked.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const VERSION = 1;
const PATH = '/io/github/ahuray/badi/Focus';
const INTERFACE = `<node>
  <interface name="io.github.ahuray.badi.Focus1">
    <method name="GetFocus">
      <arg type="u" name="version" direction="out"/>
      <arg type="b" name="locked" direction="out"/>
      <arg type="t" name="window" direction="out"/>
      <arg type="u" name="pid" direction="out"/>
      <arg type="s" name="app_id" direction="out"/>
      <arg type="s" name="client" direction="out"/>
      <arg type="b" name="minimized" direction="out"/>
    </method>
    <signal name="FocusChanged">
      <arg type="t" name="window"/>
    </signal>
  </interface>
</node>`;

export default class BadiFocus extends Extension {
    enable() {
        this._exported = Gio.DBusExportedObject.wrapJSObject(INTERFACE, this);
        this._exported.export(Gio.DBus.session, PATH);
        this._window = null;
        this._classChanged = 0;
        this._focusChanged = global.display.connect('notify::focus-window', () => this._follow());
        this._follow();
    }

    disable() {
        global.display.disconnect(this._focusChanged);
        this._watch(null);
        this._window = null;
        this._exported.unexport();
        this._exported = null;
    }

    GetFocus() {
        const window = global.display.focus_window;
        const locked = Main.sessionMode.isLocked;
        if (!window)
            return [VERSION, locked, 0, 0, '', '', false];
        const pid = window.get_pid();
        const client = window.get_client_type() === Meta.WindowClientType.WAYLAND ? 'wayland' : 'x11';
        return [VERSION, locked, window.get_id(), pid > 0 ? pid : 0, window.get_wm_class() ?? '', client,
            window.minimized];
    }

    _follow() {
        const window = global.display.focus_window;
        this._watch(window);
        this._exported.emit_signal('FocusChanged', new GLib.Variant('(t)', [window ? window.get_id() : 0]));
    }

    _watch(window) {
        if (window === this._window)
            return;
        if (this._window && this._classChanged) {
            try {
                this._window.disconnect(this._classChanged);
            } catch {
                // The previous window is already gone, and its handler with it.
            }
        }
        this._window = window;
        this._classChanged = window ? window.connect('notify::wm-class', () => this._follow()) : 0;
    }
}
