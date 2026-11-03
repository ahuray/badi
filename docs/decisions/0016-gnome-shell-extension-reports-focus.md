# ADR 0016: a GNOME Shell extension of Badi's own answers focus on GNOME

- Status: accepted (user approval, 2026-11-03)
- Date: 2026-11-03
- Scope: a GNOME Shell extension shipped with Badi, the accessibility
  observer's GNOME backend, its user unit's start condition, the installers
  and `badi doctor`

## Context

The [field observer](../../adapters/accessibility/README.md) binds every
field to the focused window's process and app id, checked against an exact
identity rule, and denies while the session is locked. It has that source on
Hyprland and, through a KWin script, on KDE Plasma 6 (ADR 0015, proposed in
its own change). GNOME had none
([future plans](../../future%20plans.md), step 7):

- `org.gnome.Shell.Introspect` (`GetWindows`, `GetRunningApplications`)
  gives no process id, and since GNOME 41 answers only callers on its
  allowlist (the portal backends) unless unsafe mode is on.
- `org.gnome.Shell.Eval` is disabled outside unsafe mode, and unsafe mode
  would open far more than Badi needs.
- Mutter offers no Wayland protocol for foreign toplevels, and AT-SPI's
  application name is not the app id, nor is its process id meant as an
  identity.
- Only code inside gnome-shell can read `global.display.focus_window`, whose
  `Meta.Window` gives `get_pid()`, the Wayland app id (`get_wm_class()`),
  `get_client_type()`, `minimized` and a per-session `get_id()`.

So GNOME needs a Shell extension that the user installs and enables. GNOME
48 (Debian 13) and 46 (Ubuntu 24.04) both load ESM extensions (45 and later);
an extension declares the Shell versions it supports.

## Decision

1. **One small extension**, `badi-focus@ahuray.github.io`, shipped in the
   packages under `/usr/share/gnome-shell/extensions/` and by the user-local
   installer under `~/.local/share/gnome-shell/extensions/`. Installing it
   never enables it: `gnome-extensions enable badi-focus@ahuray.github.io` is
   the user's own step, which the installer and `badi doctor` name. It runs
   in the `user` session mode only, so GNOME disables it while the screen is
   locked.
2. **What it exposes.** On gnome-shell's own session-bus connection, at
   `/io/github/ahuray/badi/Focus`, interface `io.github.ahuray.badi.Focus1`:
   - `GetFocus() → (u version, b locked, t window, u pid, s app_id, s client,
     b minimized)`: the focused window's per-session id, process id, Wayland
     app id or X11 class, `wayland` or `x11`, and whether it is minimized;
     `window` 0 when nothing has focus, `locked` from
     `Main.sessionMode.isLocked`;
   - signal `FocusChanged(t window)` on each focus change, and on a change of
     the focused window's app id.
   It exposes no title, content, geometry or window list, and no method that
   changes anything. It takes no well-known name of its own: it lives on the
   connection that owns `org.gnome.Shell`.
3. **Who may call it.** Any process on the user's session bus, which only the
   user's processes reach. What it answers is less than a same-UID process
   can learn already ([ADR 0001](0001-same-uid-trust-boundary.md)), so it adds
   no caller check that a same-UID process could not pass.
4. **How the observer authenticates it.** The observer resolves the owner of
   `org.gnome.Shell`, requires the bus daemon's credentials for it to name
   this user and a process the kernel names `gnome-shell`, and then calls
   `GetFocus` on that unique name, so only gnome-shell can answer. It accepts
   `FocusChanged` only from that unique name. A new owner of
   `org.gnome.Shell` (a Shell restart) stops the observer, and systemd
   restarts it.
5. **How it fails closed.** Every inspection calls `GetFocus` afresh, within
   the 150 ms desktop budget; nothing is cached. A missing object or method
   (the extension not installed, not enabled, or disabled at lock) is
   `desktop_unavailable`, and so is an unknown `version` (an extension
   newer or older than the observer), which `badi doctor` reports; `locked`
   true, or `org.gnome.ScreenSaver.GetActive` on the same unique name
   not an explicit false, is `desktop_locked`; window 0 is
   `focus_unavailable`; a minimized window fails the identity rule. The
   identity rules, epochs and AT-SPI focus events apply unchanged. An X11
   window is reported as XWayland, so no caret is calibrated for it.
6. **Selection and unit.** The observer picks GNOME when
   `XDG_CURRENT_DESKTOP` lists `GNOME` and `WAYLAND_DISPLAY` is set, after
   Hyprland and KWin, whatever else the list holds (`ubuntu:GNOME`,
   `zorin:GNOME`); the unit leaves that choice to the observer (ADR 0015).
   A session that lists GNOME but shows no `org.gnome.Shell` owner within 10
   seconds (Budgie calls itself `Budgie:GNOME`) has no backend: exit 78.
   `badi doctor` names the source (`ready · GNOME Shell`) or says the
   extension is missing, disabled or another version.
7. **No geometry at first.** As on Plasma, GNOME fields show suggestions in
   the input method's panel until a later change maps Mutter's frame and
   monitor scale onto the caret calibration.

## Evidence after implementing

- **The extension and backend** pass against a real headless GNOME Shell
  48.7 (Debian 13) and 46.0 (Ubuntu 24.04), with a minimal GDM and logind stand-in so the Shell
  builds its real screen shield: each focused GTK 4 window's process and app
  id; the identity rule accepting only the focused one; a locked session
  denied, the extension unexported in the unlock-dialog mode; answers back
  after logind's Unlock; a disabled extension denied
  ([observer runbook](../../adapters/accessibility/README.md#tested-with)).
- **The addon's app id on GNOME is empty.** Under the same headless Shell,
  with Fcitx 5.1.12 standing in for IBus, a focused GTK 4 Wayland entry's
  input context was created through Fcitx's IBus frontend with no program
  name (`program:` empty in Fcitx's `DebugInfo`): the Shell relays every
  text-input-v3 app, and Fcitx does not name it. The addon then has no app
  id, so it denies the field, as it does for any unnamed context. The
  observer can tell which app has focus, but nothing yet joins that to the
  addon's field. Taking the app id from the observer for an unnamed IBus
  context changes the addon's authority contract and is an open user
  decision ([future plans](../../future%20plans.md)); until then GNOME's
  observed fields get no suggestions.
- **Extension review.** Publishing on extensions.gnome.org is optional and a
  separate decision; the packages can ship it without that.

## Consequences

- GNOME users install one more piece and enable it themselves; until they do,
  `badi doctor` explains why observed fields get no suggestions. Even then,
  observed fields stay without suggestions until the app-id decision above.
- A Shell upgrade can disable the extension (an undeclared Shell version);
  the observer then fails closed and `badi doctor` says so.
- The extension is the only Badi code that runs inside gnome-shell, so it
  stays minimal: two members, no settings, no timers.

## Alternatives rejected

- **`org.gnome.Shell.Introspect`.** No process id, and refused to Badi.
- **Unsafe mode or `Eval`.** Opens the whole Shell to every same-UID caller.
- **AT-SPI alone.** Its application name is not the app id, and its process
  id is not an identity the bus vouches for.
- **Polling a window list from the extension.** Exposes every window to
  answer a question about one.
