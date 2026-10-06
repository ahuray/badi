# Badi Fcitx5 module

`libbadi-fcitx5.so` is a cooperative Fcitx5 **module** (`Category=Module`), not
an input method. It runs after the active input method, with a pre-input hook
for Badi's own keys, and yields whenever that input method owns preedit or
candidates. It never uses evdev, `wtype`, the clipboard, a virtual keyboard or
global capture. [WIRE_PROTOCOL.md](WIRE_PROTOCOL.md) holds the broker and
observer wire details.

## Contract

- **Identity.** `InputContext::program()` counts only as an ASCII identifier
  (`^[A-Za-z][A-Za-z0-9_-]*(\.[A-Za-z][A-Za-z0-9_-]*)*$`, at most 128 bytes),
  folded to lowercase once at focus-in (`Telegram` becomes `telegram`). Brave
  `--app` windows (`brave-<host>__<path>-<profile>`) map to `brave-origin`, VS
  Code 1.140's `com.microsoft.VSCode` to `code` and every `libreoffice-*` to
  `libreoffice`. That one id serves policy, sessions, debug and the observer.
- **Policy.** Each focus or authority epoch queries broker policy before any
  text is read; only a current grant opens a session.
- **Keys.** Only Badi's own UI consumes a key:

  | Key | When Badi takes it |
  | --- | --- |
  | Tab | Accepts a visible suggestion. On the manual path it also requests at the end of a phrase. Otherwise it stays the app's Tab. |
  | Ctrl+Right | Accepts the next word of Badi's live candidate; otherwise the app's key. |
  | Escape | Dismisses the suggestion or closes a notice. |
  | Ctrl+Shift+Space, Ctrl+Shift+Y | Request or refresh; accept. |

- **Type-through.** A key that types the suggestion's next characters still
  reaches the app and retires the candidate like any edit, but the next
  inspection waits 30 ms instead of 120 ms. A word acceptance does the same.
  The addon remembers the untyped rest until a suggestion shows again, so keys
  typed before the remainder returns, even while the field waits to be observed
  again, still count. The broker then answers with the remainder and no model
  call ([ADR 0004](../../docs/decisions/0004-type-through-carries-text-not-authority.md)).
- **Manual request.** It needs a collapsed, non-composing, non-sensitive
  surrounding-text snapshot, the live `SurroundingText` capability, a valid
  input-method language (English, German or Persian) and a surrounding-text
  update since focus or the last authority change. Until a request, text events
  only invalidate local state.
- **Display.** The observer's grey inline preview, or else Badi's own Fcitx
  panel: one candidate under “Badi · Tab to accept · Ctrl+→ next word · Escape
  to dismiss”. Notices expire after five seconds.
- **Acceptance.** The broker authorizes first. A matching `commit.prepare`
  causes exactly one `commitString`, reported as `dispatched-unverified`. The
  candidate must still be present and owned at dispatch, and a local timer
  clears it when its lease expires.
- **Reconnect.** A closed connection retires every session, revision,
  candidate, grant and pending observer reply. Reconnection retries at most ten
  times, with backoff capped at five seconds, renewed by a key or focus. Unchanged
  authority lets Tab reread Fcitx's current text; automatic requests wait for
  new text.
- **No replacement.** The module never negotiates `text_replacement` or sends a
  deletion: an `input` handler can move focus between a deletion and an
  insertion ([ADR 0003](../../docs/decisions/0003-ime-parity-append-only.md)).

## App classes

`classifyNativeApp` (`src/state.cpp`) decides every path from the canonical id:

| Class | App ids | Edit path |
| --- | --- | --- |
| Native exact | Any other granted id, e.g. `omawrite`, `com.github.xournalpp.xournalpp`, `telegram` | The manual unknown-identity contract, or an observed desktop field |
| IME-parity browser | `chromium`, `chromium-browser`, `brave-origin`, `zen` | Observed browser field only; origin policy; append-only |
| IME-parity desktop | `chatgpt` (Codex), `code`, `cursor`, `discord`, `grok-bot`, `libreoffice` | Observed field of the same app only; app policy; append-only |
| Unavailable | Other Gecko browsers (Firefox, other Zen builds, LibreWolf, Floorp, PWAsForFirefox, …), Chromium-family ids without an observer rule (Chrome, `brave`, `chrome-*`/`crx_*` web apps, Flatpak ids, Edge, Vivaldi, Opera, `electron*`, `code-oss`, `vscodium`, `vesktop`, …) and `obsidian`, whose plugin owns its fields | None |

Gecko mail clients such as Thunderbird stay native exact. An Electron app with
an unrelated id can only use the native exact manual contract.

IME-parity apps follow [ADR 0003](../../docs/decisions/0003-ime-parity-append-only.md).
In this module that means:
- **No manual path.** They never use the unknown-identity manual path, even
  with an app grant.
- **Ctrl+Shift+Space without an observed field** asks the observer instead and
  explains a failure: "Badi cannot see this text field — check badi doctor"
  (`ime_parity_observer_unavailable`), or, for a denied password, purpose or
  selection field, "Badi cannot read this text field — run badi debug status"
  (`ime_parity_field_denied`).
- **Tab on an observed field** only accepts; without a suggestion it stays the
  app's Tab.
- **Target kind** must match the class (`ime_parity_target_mismatch`,
  `ime_parity_target_invalid`).
- **Browser fields** need an exact origin rule or `badi site all on`, which
  also covers private windows.
- **Blocklist mode.** An app allowed only by the app blocklist mode (policy
  reason `matched_default`) opens observed fields only. On the manual path it
  counts as denied (`app_rule_required`) until it gets an exact rule.
- **Snapshot agreement.** Before publication, display and `commitString`, a
  fresh observer snapshot must equal Fcitx's live text, caret and document
  length. Otherwise it fails closed as `observer_context_mismatch`,
  `observer_display_mismatch`, `observer_display_unavailable`,
  `observer_dispatch_mismatch` or `observer_dispatch_unavailable`.

Unavailable apps are never read or inspected. Ctrl+Shift+Space there shows
“Badi cannot safely insert suggestions in this app yet”.

### Observed fields

The [accessibility observer](../accessibility/README.md) supplies field identity
and corroboration. On Fcitx 5.1.22 the
[frontend backport](../../packaging/fcitx5-wayland-compat/README.md) keeps
Chromium's surrounding text current. Neither provides an editor transaction.

- **Display.** When the observer verifies the field but reports
  `rendered:false` (no calibrated geometry), Badi shows its Fcitx panel, which
  Hyprland places at the app's text-input caret rectangle.
- **Inspection.** It runs 120 ms after input pauses, or 30 ms after a
  typed-through key.
- **Invalidations.** One without input re-inspects after 240, 480 and 960 ms
  and then waits for input (`observer_awaiting_input`); a change of the field
  itself always waits.
- **Paragraph ends.** Chromium ends every `<p>` with `"\n\n"` and every `<div>`
  line with `"\n"` when anything follows the editor. For an observed IME-parity
  field only, `normalizeObservedParagraphEnd()` treats exactly that suffix as
  end of field: Tab and the broker see `after: ""`, while the fingerprint and
  every observer check use the raw text (`observedAfter()`).
- **Agreement** is exact with two equivalences: an empty Fcitx `after` matches
  an observed block end alone, and a `scope: "block"` snapshot (an opaque list,
  quote or table in the window) matches when Fcitx's `before` ends with it
  right after a line break.

## Tested with

Arch Linux, native Wayland, Hyprland 0.56.2, Fcitx 5.1.22 (5.1.21 for Omawrite
and Xournal++), input method `keyboard-us`. These are probe cells, not
toolkit-wide claims.

| Application | Reaches Fcitx through | Boundary |
| --- | --- | --- |
| Omawrite 0.5.0 (Qt 6) | `QT_IM_MODULE=fcitx` | 20/20 trials; one native undo restores the exact prefix |
| Xournal++ 1.3.7 (GTK 3 text tool) | `GTK_IM_MODULE=fcitx` launcher override | 20/20 trials; after Escape leaves text editing, one document undo restores the prefix |
| Chromium 152, Brave Origin, Electron apps | text-input-v3 (`wayland_v2`), Wayland app id | Page fields carry no purpose hints, so the observer's checks deny passwords. The omnibox sends the `Url` purpose and is denied before any observer request. Surrounding text is exact only with the frontend backport. |
| VS Code 1.140 | as above | Needs `"editor.editContext": false`; its default EditContext sends corrupted text, which fails closed. Its caret-wide hidden textarea gets the inline preview bounded by the editor widget. |
| Zen 1.23b (Gecko 157) | `wayland_v2`, `zen` | Fields and the urlbar share one context with no `Url` hint, so the observer denies the urlbar. Gecko sends only the caret's paragraph. One Ctrl+Z undoes an acceptance alone. |
| Telegram (Qt) | `dbus`, `Telegram` | Canonicalized to `telegram` |

Fcitx supplies process identity and text but no stable widget identity, so
manual context declares field identity and purpose unknown, and only the
broker's explicit-manual path can authorize it.

## Install and roll back

`python3 scripts/install-desktop.py` builds the release broker and addon,
installs them under `~/.local/lib`, and loads the addon into
`omarchy-fcitx5.service` through a user drop-in. It preserves autostart choices,
verifies the observer before restarting Fcitx, and reports success only after
model health, addon loading and an unchanged keyboard profile. It needs the
provisioned model, an active Omarchy session and an unlocked desktop. Xournal++
gets a launcher override with `GTK_IM_MODULE=fcitx` for its exact identity.
`--broker-only` updates only the broker and commands, while locked.

Each run that changes something backs up what it replaced under
`~/.local/state/badi/install-backups/<UTC time>/`. There, `changes.json` lists
every path, its action (`create`, `replace` or `remove`), the saved original
and the installed hash. The newest three backups are kept. To roll back from
the unlocked session:

1. Stop `badi-accessibility.service` and `badi-broker.service` (a broker-only
   rollback leaves the observer and Fcitx running).
2. Restore, newest backup first. A path changed since that install is listed
   and left alone (exit 1); `--only PATH` limits the restore:

   ```sh
   python3 scripts/badi_install.py restore ~/.local/state/badi/install-backups/<UTC time>
   ```

3. Run `systemctl --user daemon-reload`, restart `omarchy-fcitx5.service` and
   the services you stopped, and relaunch affected apps. To remove only the
   renderer accessibility flag, delete the `# Badi: renderer accessibility for
   the focused-field observer` comment and the flag line after it from that
   app's flags file.
4. An observed-app install also saved `accessibility-setting.json` with the
   prior `bus_enabled` and `toolkit_accessibility` values. Restore them only
   from that receipt; without one, leave these desktop-wide settings alone:

   ```sh
   busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled b <bus_enabled>
   gsettings set org.gnome.desktop.interface toolkit-accessibility <toolkit_accessibility>
   ```

Settings and model files stay. A recovered service proves no prediction,
insertion or undo; check those in an app.

## Try it

Open Omawrite or Xournal++ with `badi launch omawrite` or `badi launch
xournalpp` (in Xournal++, select the **Text** tool and click the page). Type
`Please find attached the` with the caret at the end. Omawrite suggests on its
own; in Xournal++, press Tab to request. Then Tab accepts and Escape dismisses.

Judge usefulness and editing safety separately. Try at least 20 prefixes from
your own writing, and count late, irrelevant and invented suggestions apart
from useful ones. Check that acceptance inserts exactly once, Escape leaves the
text intact, undo removes the accepted text, and typing or moving focus never
inserts a visible suggestion. `badictl probe -` runs text through the model
without an editor.

## Build and test

Needs CMake, Ninja, a C++20 compiler, Fcitx5Core and nlohmann-json.

| Command | Covers |
| --- | --- |
| `npm run fcitx5:check` | Build plus deterministic suites: app classes and ids (and their agreement with the observer, `tests/observer-identities.py`), key, Tab, invoke and inspection decisions, type-through and word acceptance, binding and sanitization, framing and strict wire shapes, one-shot commits, foreign-IME yield, debug snapshots, reconnect |
| `npm run fcitx5:integration` | Also needs Cargo and Python 3. The shipped transport against a real broker in temporary roots: context, accept, report, dismiss, a type-through and word-acceptance scenario that must generate once, and recovery after a broker restart |
| `PYTHONDONTWRITEBYTECODE=1 python3 adapters/fcitx5/tests/observed-desktop.py` | The real addon in a private D-Bus session with its own Fcitx, broker and a synthetic observer: every app class, denials, panel and preview display, disagreement at request, display and dispatch, type-through, Ctrl+Right, the Zen urlbar, foreign preedit, paragraph ends and recovery. Proves transport and state, not app behavior |
| `PYTHONDONTWRITEBYTECODE=1 python3 adapters/fcitx5/tests/desktop-tab.py` | Opt-in, after installation: synthetic contexts drive the installed addon and model through Tab request, accept and denials; run it while not typing elsewhere |

A staged install leaves the live Fcitx alone:

```sh
DESTDIR="$PWD/adapters/fcitx5/stage" cmake --install adapters/fcitx5/build --prefix /usr
```
