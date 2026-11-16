# ADR 0017: an unnamed IBus context on GNOME takes its observed app id from the observer

- Status: accepted (user approval, 2026-11-15)
- Date: 2026-11-15
- Scope: the Fcitx addon's authority over unnamed `ibus` input contexts, the
  accessibility observer's `inspect` contract on GNOME, `badi doctor`, and the
  desktop installer notes

## Context

On GNOME Wayland, Mutter speaks `text-input-v3` and relays every Wayland
text-input client (GTK 4, Chromium, Electron, Zen, LibreOffice) to Fcitx
through Fcitx's `ibus` frontend (`InputContext::frontendName() == "ibus"`).
Because GNOME Shell relays every client and blocks `org.gnome.Shell.Introspect`,
Fcitx has no per-client program name and leaves `InputContext::program()` empty
(`""`), verified under headless GNOME Shell 48.7 with Fcitx 5.1.12
([ADR 0016](0016-gnome-shell-extension-reports-focus.md)).

Until now the [Fcitx addon](../../adapters/fcitx5/README.md) required a
non-empty `InputContext::program()` at `focusIn`, so every Wayland text-input
field on GNOME failed closed as `unidentified_app`. Meanwhile Badi's GNOME
Shell extension (`badi-focus@ahuray.github.io`,
[ADR 0016](0016-gnome-shell-extension-reports-focus.md)) answers the focused
window's per-session id, process id, Wayland app id or X11 class, client type
and lock state from the authenticated `org.gnome.Shell` bus owner, and the
[field observer](../../adapters/accessibility/README.md) already checks each
window against an exact executable and window-class rule in `APPS` and binds
the one focused editable AT-SPI object of that exact process.

Opening GNOME's observed fields without weakening named input contexts, other
frontends, other desktops, or the unknown-identity manual contract needed an
explicit user decision.

## Decision

1. **Named input contexts do not change.** Whenever `InputContext::program()`
   is non-empty, `programAppId` stays authoritative: invalid non-empty strings
   still fail closed at `focusIn`, and observed fields must still match Fcitx's
   canonical `app_id`.
2. **Other unnamed frontends do not change.** When `InputContext::program()` is
   empty and `InputContext::frontendName() != "ibus"` (`wayland`, `wayland_v2`,
   `dbus`, `xim`, …), `focusIn` still denies the context as `unidentified_app`
   without contacting the observer or the broker.
3. **Unnamed `ibus` contexts stay observed-only.** When
   `InputContext::program()` is empty and `InputContext::frontendName() ==
   "ibus"`:
   - `NativeEditPath::Manual` and `NativeEditPath::TypedLine` never open on
     that context, before, during or after observation. No policy query or
     broker session runs while the context is unidentified, and plain `Tab` and
     `Ctrl+Shift+Space` pass through to the application.
   - Instead of `{"op": "inspect", "app_id": ...}`, the addon sends
     `{"op": "inspect", "frontend": "ibus"}` (plus `"selection": true` for an
     explicit selection revision). `inspect` accepts exactly one of `app_id` or
     `frontend: "ibus"`; any other shape or frontend name is `invalid_request`.
4. **Only the GNOME observer resolves `frontend: "ibus"`.** On any observer
   backend other than `"gnome"` (`"hyprland"`, `"kwin"`, or unset),
   `{"op": "inspect", "frontend": "ibus"}` is denied as `unsupported_app`. On
   `"gnome"`, inside the same dual lock check (`GetFocus` `locked == false` and
   `org.gnome.ScreenSaver.GetActive == false` on the verified `org.gnome.Shell`
   owner), `Desktop.focused_app()` reads the unminimized focused window (`window
   != 0`, `hidden == false`), matches it against `APPS` (`_shows` and
   `verify_process`), requires **exactly one** rule to match (`unsupported_app`
   when none or more than one matches; `app_mismatch` when only one half of a
   rule matches, such as a spoofed window class or wrong executable), and
   requires the one focused editable AT-SPI object to belong to `window["pid"]`.
5. **Only observed apps may be adopted, for that epoch only.** The addon
   accepts the reply's `binding.app_id` and `target` only when `app_id` is in
   `unnamedIbusObservedApp` — the IME-parity apps plus the native apps that
   carry an observer rule in `APPS` (`omawrite`, `telegram`), checked for
   equality by `observer-identities.py`. Manual-only native apps
   (`org.gnome.texteditor`, `com.github.xournalpp.xournalpp`, …), terminals
   (`com.mitchellh.ghostty`, `foot`) and unavailable apps (`firefox`,
   `obsidian`, …) are rejected in both the observer and the addon. Subsequent
   `snapshot` and `preview` calls carry that canonical `binding.app_id` and
   epoch as usual, and every snapshot/caret agreement check applies unchanged.
6. **Clean retirement.** Any observer invalidation, inspection denial, observer
   unavailability, target mismatch, capability change or `focusOut` closes the
   broker session, drops the adopted `app_id`, and returns an unnamed `ibus`
   binding to its unidentified, closed state under a fresh session UUID — never
   to `NativeEditPath::Manual`.

## Consequences

- Observed fields in Chromium-based apps, Zen, LibreOffice Writer, Telegram and
  Omawrite can bind and suggest in Fcitx's panel on GNOME once the user enables
  `badi-focus@ahuray.github.io` and allows the site or app, verified live in a
  Debian 13.7 GNOME 48.7 Wayland VM for Chromium and GTK3 Writer
  ([Debian tested-with](../../packaging/debian/README.md#tested-with)).
- Manual-only GTK/Qt apps launched over Wayland `text-input-v3` on GNOME still
  hand Fcitx an empty `ibus` program name and have no observer rule, so they
  fail closed rather than opening an unidentified manual contract; apps using
  Fcitx's native GTK/Qt IM modules report their program name and keep their
  existing contract.
- Hyprland, KDE Plasma and every named input context keep Fcitx's own
  `program()` check unchanged.

## Alternatives rejected

- **Allowing `NativeEditPath::Manual` on unnamed `ibus` contexts.** Opens the
  unknown-identity manual contract without an input-method program name.
- **Accepting `frontend: "ibus"` on Hyprland or KWin.** Unnecessary outside
  GNOME's `text-input-v3` IBus relay; keeping other desktops on Fcitx's own
  `program()` bounds the authority transfer to GNOME.
- **Reading AT-SPI's application name without `APPS` process + window-class
  verification.** Self-reported by the client and not canonical app identity.
