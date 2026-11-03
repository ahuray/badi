# ADR 0015: a KWin script of the observer's own reports focus on KDE Plasma

- Status: accepted
- Date: 2026-11-03
- Scope: the accessibility observer's focused-window and lock sources outside
  Hyprland, its user unit's start condition, and `badi doctor`

## Context

The [field observer](../../adapters/accessibility/README.md) binds every
field to the active window's process and app id, checked against an exact
identity rule, and denies while the session is locked. It learned both only
from Hyprland's IPC and Omarchy's lock status, so on KDE Plasma its unit was
skipped and Chromium-based apps, Zen, VS Code and Writer's grey preview got
no observed fields ([future plans](../../future%20plans.md), step 7).

Plasma offers two sources of the active window's process and app id:

1. **`org_kde_plasma_window_management`**, the Wayland protocol Fcitx's own
   app monitor uses (app id; process id since version 8). KWin advertises it
   only to a client whose executable a desktop file names in
   `X-KDE-Wayland-Interfaces`. Checked on KWin 6.3.6 (Debian 13): an ordinary
   client's registry lists `org_kde_plasma_virtual_desktop_management` and
   `org_kde_plasma_shell` but not the window management global. The
   observer runs as `/usr/bin/python3`, so granting it would grant every
   Python program the user runs, or need a native helper binary of its own,
   plus a Wayland client written by hand (no Wayland binding is a dependency).
2. **A KWin script** reading `workspace.activeWindow`, loaded over D-Bus
   (`org.kde.KWin /Scripting loadScript`, then `run`), which sends what it
   reads back with `callDBus`. No extra binary, permission or desktop file;
   the window model is KWin's own. On KWin 6.3.6 headless
   (`kwin_wayland --virtual`): `windowActivated` fires with no window, then
   with the new one; `pid`, `resourceClass` (the Wayland app id),
   `internalId`, `minimized` and `hidden` are readable; `loadScript` returns
   -1 for a name already loaded and accepts a missing file, failing only
   when the script runs; the session's `org.freedesktop.ScreenSaver` is owned
   by KWin's own connection. While locked, `GetActive` answered true and
   `activeWindow` still named the app window (the greeter did not start in
   the container), so the window alone cannot prove the session is unlocked.
   Distributions give `kwin_wayland` `CAP_SYS_NICE`, which makes it
   undumpable: its `/proc` exe link cannot be read.

## Decision

1. **Selection.** The observer follows Hyprland when
   `HYPRLAND_INSTANCE_SIGNATURE` is set, otherwise KWin when
   `XDG_CURRENT_DESKTOP` lists `KDE`, `KDE_SESSION_VERSION` is 6 or later
   and `WAYLAND_DISPLAY` is set. Anywhere else it exits with status 78, which
   its unit does not restart; the unit's start condition already skips it
   outside a Hyprland or `XDG_CURRENT_DESKTOP=KDE` session. Hyprland
   behavior does not change.
2. **The script.** At start the observer writes a script naming its own
   private connection's unique bus name and a random per-run token into its
   `0700` runtime directory (`0600`), unloads any earlier
   `badi-field-observer` script, loads and runs this one and deletes the
   file once the script has answered. The script reports, as JSON, the
   active window's `pid`, app id, window id and minimized, hidden, managed
   and deleted flags on each activation and app id change, with a rising
   serial. It reads no title, content or geometry. A KWin without the Plasma
   6 API (`workspace.activeWindow`, `windowActivated`) reports that and the
   observer exits with 78.
3. **Authentication.** A report counts only from the unique name that owns
   `org.kde.KWin`, which the bus daemon says runs as this user
   (`GetConnectionCredentials`) in a process the kernel names `kwin_wayland`
   (`/proc/PID/status`, which stays readable for an undumpable process),
   with this run's token and a higher serial. Any other caller gets an
   access-denied reply and changes nothing. A report in an unknown shape
   from KWin means no window. Same-UID processes stay inside the trust
   boundary ([ADR 0001](0001-same-uid-trust-boundary.md)).
4. **Fresh reads.** Each inspection first calls KWin's `isScriptLoaded` for
   that script. KWin answers on its main thread, after sending the reports
   of every activation it already handled, and the observer's bus thread
   applies reports in arrival order, so the window read afterwards is
   current. `false` means the script was unloaded: the observer stops and
   systemd restarts it, which loads the script again. A new or vanished
   `org.kde.KWin` owner does the same.
5. **Lock.** After the field is read, KWin's own `/ScreenSaver` object
   (called by KWin's unique name, so no other process can answer) must
   return an explicit `false` from `GetActive`; true is `desktop_locked`,
   anything else `desktop_unavailable`. A window whose app id or process is
   `kscreenlocker_greet` is `desktop_locked` too.
6. **Everything else is unchanged.** The same identity rules (`/proc` exe,
   app id as the window class, mapped and not hidden), epochs, focus and
   field events apply. Any report that changes the window invalidates the
   tracked field, as a Hyprland window event does.
7. **No geometry yet.** The KWin backend reports no monitors and leaves
   XWayland unknown, so no caret is calibrated and observed fields show
   their suggestion in the Fcitx panel. A grey preview on Plasma needs
   KWin's client geometry and output scale mapped onto the calibration and a
   live check; it is a later step.

## Consequences

- On Plasma 6, an app becomes eligible for observed fields where its
  identity rule's executable matches the install: on Debian 13 that is
  Chromium (`/usr/lib/chromium/chromium`), VS Code from Microsoft's package
  and LibreOffice Writer; the other rules name Arch paths. No Plasma desktop
  has run that flow live yet, so README coverage stays unclaimed until it
  does.
- `badi doctor` names the focused-window source (`ready · KWin (KDE
  Plasma 6)`), and says when the observer found none.
- A same-UID process can unload the script; the next inspection notices
  and the observer reloads it. It could also impersonate KWin's reports only
  by running inside KWin, which ADR 0001 already accepts.
- GNOME still has no backend: `org.gnome.Shell.Introspect` gives no process
  id and answers only portal backends, so it needs a Shell extension.

## Alternatives rejected

- **`org_kde_plasma_window_management` from the observer.** Needs a desktop
  file granting the interface to the Python interpreter, or a new native
  binary, and a hand-written Wayland client; KWin's own model is simpler and
  equally authoritative.
- **Trusting the last report without a round trip.** A request arriving just
  after a focus change could read the previous window.
- **Verifying KWin by its `/proc` exe link.** Unreadable for the
  capability-carrying `kwin_wayland` that distributions ship, so the
  observer would never start there.
