# Badi focus: GNOME Shell extension

`badi-focus@ahuray.github.io` tells Badi's
[field observer](../accessibility/README.md#desktops) which window has focus
on GNOME, where `org.gnome.Shell.Introspect` gives no process id and answers
only the portal backends
([ADR 0016](../../docs/decisions/0016-gnome-shell-extension-reports-focus.md)).
It is the only Badi code that runs inside gnome-shell, so it stays two
members, no settings and no timers.

## Contract

On gnome-shell's own session-bus connection, at
`/io/github/ahuray/badi/Focus`, interface `io.github.ahuray.badi.Focus1`:

| Member | Shape | Meaning |
| --- | --- | --- |
| `GetFocus()` | `(u version, b locked, t window, u pid, s app_id, s client, b minimized)` | Version 1; `Main.sessionMode.isLocked`; the focused window's per-session id (0 for none), process id (0 when unknown), Wayland app id or X11 class, `wayland` or `x11`, and whether it is minimized |
| `FocusChanged(t window)` | signal | Focus moved, or the focused window's app id changed |

It exposes no title, content, geometry or window list and changes nothing.
It takes no bus name of its own, so the observer calls the verified
`org.gnome.Shell` owner and only gnome-shell can answer. Its session modes
are `user` only: GNOME disables it while the screen is locked, and the
observer then fails closed.

## Install and enable

The Debian and Arch packages install it under
`/usr/share/gnome-shell/extensions/`; `scripts/install-desktop.py` installs it
under `~/.local/share/gnome-shell/extensions/` on any desktop but Omarchy.
Nothing enables it for the user. GNOME Shell on Wayland finds a new extension
at the next login; then:

```sh
gnome-extensions enable badi-focus@ahuray.github.io
```

`badi doctor` reports `gnome_extension_inactive` while it does not answer.
It declares GNOME Shell 46 to 50 (Ubuntu 24.04 ships 46, Debian 13 ships 48).

## Tests

`npm run accessibility:check` reads its metadata and interface against the
observer's expectations and parses it with Node when Node is installed.
[`gnome-headless.py`](../accessibility/tests/gnome-headless.py) runs it in a
real headless GNOME Shell; the observer runbook has the
[command and results](../accessibility/README.md#tests).

The observer can follow GNOME's focus, but GNOME's observed fields still get
no suggestions: the Shell relays every Wayland text-input app to Fcitx, and
Fcitx names no program for that input context, so the addon has no app id
to join to the observer's answer. That is an open decision in
[future plans](<../../future plans.md>).
