# Screen-share hold

While the screen is being shared through the desktop portal, Badi shows no suggestions anywhere, so grey text with words from the window or learned ones never appears on someone else's screen. Suggestions return about a second after sharing stops. `badi screen-share suggest` keeps suggesting during a share instead; `hold` is the default.

## Sub-features

- `share-hold` withdraws a visible suggestion and holds every grant while a portal screencast stream lives in PipeWire.
- `share-release` ends the hold about a second after the last stream, never in between when a share swaps streams.
- `share-suggest` keeps suggesting while shared, after `badi screen-share suggest` or the panel's While sharing the screen row.
- `share-named` names the hold in `badi status`, `badi doctor`, the panel, the bar mark and the debug trace, without text.

## How to get to it (user POV)

- Share the screen from a browser (`getDisplayMedia`), OBS or a meeting app; Omarchy's picker chooses what to share.
- While it is shared, `badi status` starts with `Badi: Held while the screen is shared`, and the panel's hero reads `HELD · SCREEN SHARED`.
- `badi screen-share suggest` keeps suggesting; `badi screen-share hold` returns to the default.

## Driving it with verify-badi

Preconditions:

- Headless: none of the helper's drives send a session helper report; the headless proof is `npm run session:integration`, which runs the real helper against a phrase broker, a private bus and a fake `pw-dump`.
- Desktop: the installed session helper (`badi doctor` lists `Session helper: active`), `pw-dump` from PipeWire, and xdg-desktop-portal-hyprland.

- **Headless.** `npm run session:integration` exits 0; its screen-share test holds on a fake screencast node, lifts the hold with settings `screen_share: suggest`, restores it, ends it on the node's removal, and drops it when the helper exits.
- **Desktop.** `badi debug on`, a disposable local page in an isolated Chromium profile calling `getDisplayMedia`, pick the screen in Omarchy's picker and press Share. Typing in that page shows nothing while shared and grey text again a second after stopping. The trace shows `in desktop.state` with `screen_shared: true` and `out desktop.status` with `screen_share: held`. Omarchy's `omarchy-shell notifications probe` reports `sharingActive: true` at the same time. Screenshots of the quiet field and of the grey text afterwards are the desktop proof. A VM cannot run it.

## Gotchas

- Chromium's share dialog previews the screen through a live portal stream, so the hold starts when the picker closes, before Share is pressed. That is intended: the preview is a capture too.
- The picker takes focus, so a ghost visible before the share is usually gone before the hold starts. Withdrawing a visible suggestion is proven by the broker test, not by this trial.
- Only xdg-desktop-portal-hyprland streams were tried. Other portal backends' node names are not checked.
- A share hold makes `paused` true in `badictl status` although settings are not paused; read `desktop.screen_share`, not `paused`, to tell a hold from a pause.
