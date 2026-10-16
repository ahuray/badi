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
  | Tab | Accepts the next word of a visible suggestion (a spelling correction whole). While that word's remainder is still returning on an observed or typed-line field, one Tab is held for it instead of reaching the app. On the manual path it also requests at the end of a phrase. Otherwise it stays the app's Tab. |
  | Ctrl+Right | Accepts all of Badi's live candidate; otherwise the app's key. |
  | Escape | Dismisses the suggestion or closes a notice. |
  | Ctrl+Shift+Space, Ctrl+Shift+Y | Request or refresh; accept. |

- **Dismissal.** Unchanged native publications keep the context declined.
  A changed publication ends the local until-edit decline, including a fast
  delete/retype whose empty intermediate state the observer never reads.
  Broker-side timed Escape quiet still applies across those edits.
- **Type-through.** A key that types the suggestion's next characters still
  reaches the app and retires the candidate like any edit. Inspection starts
  at once, and the next suggestion waits 30 ms instead of 120 ms. The request
  waits until those characters are in the field, so a snapshot from before
  the key lands does not call the model. A word acceptance does the same.
  The addon remembers the untyped rest until a suggestion shows again, so keys
  typed before the remainder returns, even while the field waits to be observed
  again, still count. The broker then answers with the remainder and no model
  call ([ADR 0004](../../docs/decisions/0004-type-through-carries-text-not-authority.md)).
  After a word acceptance on an observed or typed-line field, a Tab in that
  gap is held for at most 700 ms; when the remainder is displayed and still
  begins with the word that Tab meant, it takes that word through the normal
  broker grant. Any other key, Escape, a focus change or the timeout drops
  the held Tab without replaying it. On the manual path no Tab is held: that
  Tab is the request that brings the remainder.
- **Manual request.** It needs a collapsed, non-composing, non-sensitive
  surrounding-text snapshot, the live `SurroundingText` capability, a valid
  input-method language or explicit app override (English, German or Persian),
  and a surrounding-text update since focus or the last authority change.
  Until a request, text events only invalidate local state.
- **Quiet fields.** When the observer reports a file dialog or a very narrow
  field (`quiet` in the [observer contract](../accessibility/README.md#socket-contract)),
  automatic work stops before a snapshot, so the field's text is not read;
  Ctrl+Shift+Space still asks there. An unrecognized quiet fact also quiets
  automatic work. The manual native path sees only Fcitx purposes and has no
  dialog or width signal, so it is not quieted this way.
- **Display.** The observer's grey inline preview, with a compact grey overflow
  panel below or above the field when needed. If neither fits or geometry is
  unavailable, Badi's Fcitx panel contains one unhighlighted candidate, no
  numeric label and no keyboard-help banner. It keeps the user's Fcitx theme.
  Suggestions expire after at most 15 seconds; notices still expire after five.
- **Acceptance.** The broker authorizes first. A matching `commit.prepare`
  causes exactly one `commitString`, reported as `dispatched-unverified`. The
  candidate must still be present and owned at dispatch, and a local timer
  clears it when its lease expires.
  An acceptance key freezes the chosen version of a growing preview while
  authorization is in flight; later extensions cannot replace its text or
  pending request.
  If a preview still occupies the observer, the commit snapshot waits up to
  500 ms for it; it still takes a fresh snapshot and keeps the original leases.
- **Reconnect.** A closed connection retires every session, revision,
  candidate, grant and pending observer reply. Reconnection retries at most ten
  times, with backoff capped at five seconds, renewed by a key or focus. Unchanged
  authority lets Tab reread Fcitx's current text; automatic requests wait for
  new text.
- **Replacement.** Native exact fields (`omawrite`, Telegram, Xournal++, and
  other granted native ids) may delete the exact typed suffix and then
  `commitString` the correction, still reported as `dispatched-unverified`.
  The suffix must still be the text before the caret. IME-parity apps stay
  append-only: a replacement is not shown or applied there
  ([ADR 0003](../../docs/decisions/0003-ime-parity-append-only.md)). In Omawrite
  0.5.0 on 2026-10-12, accepting `teh ` → `the ` took two native undos: the
  first removed `the ` and the second restored `teh `. Telegram and Xournal++
  were not retried.

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
- **Ctrl+Shift+Space** requests only after the field has an edit path and a
  grant. Before that, and in an app with no grant, the chord reaches the app
  (LibreOffice's non-breaking space, VS Code's parameter hints). Focus still
  asks the observer. `badi debug` records `no_edit_path` or `app_disabled`.
  A denied password, purpose or selection field is `ime_parity_field_denied`;
  an observer the broker cannot reach is `ime_parity_observer_unavailable`.
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

Unavailable apps are never read or inspected. Ctrl+Shift+Space there reaches
the app; `badi debug` records `no_edit_path`.

### Terminals

Terminals report the `Terminal` capability and no surrounding text. For an
exact app rule on a listed terminal (only `com.mitchellh.ghostty` so far), the
`TypedLine` path follows the line typed into that input context and uses it as
the context ([ADR 0009](../../docs/decisions/0009-terminals-use-the-typed-line.md)).
`lineKey()` applies typing, Backspace, Ctrl+W, Ctrl+U, Ctrl+C and Enter, and
stops tracking at any other key the terminal receives (`line_unknown` until
the next empty line). After the typing pause a line of two or more words
requests automatically; the panel shows the suggestion, Tab types one word
and Ctrl+→ all of it, after which the next request follows the line. Tab
without a suggestion or a pending remainder stays the terminal's. A miss is
announced only after an explicit request: an automatic request that finds
nothing shows no notice, on this path and every other (`requestExplicit`
records how the published request was made; `onClear` shows
`clearNoticeText` only for an explicit one).

### Observed fields

The broker target id is random per field visit. It survives reacquisition of
the exact same AT-SPI bus, object, process, app and URI solely to preserve a
timed dismissal. Focus loss, a different field or denied acquisition ends the
visit. Observer epochs, sessions, revisions, salts, policy checks and commit
grants remain fresh after invalidation; the observer's own epoch-bound target
still corroborates every snapshot and preview.

The [accessibility observer](../accessibility/README.md) supplies field identity
and corroboration. On Fcitx 5.1.22 the
[frontend backport](../../packaging/fcitx5-wayland-compat/README.md) keeps
Chromium's surrounding text current. Neither provides an editor transaction.

- **Display.** A preview request carries the input context's `caret_rect`
  when the frontend declares `RelativeRect` (Qt's Fcitx module); the observer
  uses it only where AT-SPI has no glyph geometry. When the observer verifies
  the field but reports `rendered:false` (no calibrated geometry), Badi shows
  its Fcitx panel, which Hyprland places at the app's text-input caret
  rectangle.
- **Inspection.** A keystroke in a field already under observation re-reads
  that field and reuses its session, so the model can run during the pause.
  It does not inspect again unless that read says the accessible changed.
  The suggestion stays hidden until 120 ms after that keystroke, or 30 ms
  after a typed-through key. The first look at a field, and an idle
  invalidation, still wait (120 ms, then 240, 480 and 960 ms). A busy observer
  is retried every 20 ms. Badi's timers ask for 1 ms accuracy, because Fcitx's
  sd-event loop otherwise allows 250 ms of slack. With debug on,
  `inspect_sent` and `inspect_answered` time each inspection in the
  [debug trace](../../README.md#debug-trace), beside each key's action and
  each suggestion shown as the `overlay` or the `panel`.
- **Invalidations.** One without input re-inspects after 240, 480 and 960 ms
  and then waits for input (`observer_awaiting_input`); a change of the field
  itself always waits.
- **Paragraph ends.** Chromium ends every `<p>` with `"\n\n"` and every `<div>`
  line with `"\n"` when anything follows the editor. For an observed IME-parity
  field only, `normalizeObservedParagraphEnd()` treats exactly that suffix as
  end of field: Tab and the broker see `after: ""`, while the fingerprint and
  every observer check use the raw text (`observedAfter()`).
- **Writer's trailing spaces.** Writer's sentence for Fcitx leaves out the
  spaces just typed at a paragraph end. `restoreObservedTrailingSpaces()`
  takes exactly those spaces from the observer's verified text when nothing
  follows the caret; any other difference stays a mismatch.
- **Mid-text.** An observed field requests with text after the caret too; the
  broker suggests only what fits before it, and Tab inserts at the caret. The
  manual Tab-request path still requires the end of the field, so Tab keeps its
  meaning inside native text.
- **Agreement** is exact with two equivalences: an empty Fcitx `after` matches
  an observed block end alone, and a `scope: "block"` snapshot (an opaque list,
  quote or table in the window) matches when Fcitx's `before` ends with it
  right after a line break.

## Tested with

Arch Linux, native Wayland, Hyprland 0.56.2, Fcitx 5.1.22 (5.1.21 for Omawrite
and Xournal++), input method `keyboard-us`. These are probe cells, not
toolkit-wide claims.

On 2026-10-12, the modified `26eb1d9` checkout installed on this desktop
showed and accepted 20/20 warm Chromium 152 suggestions across five disposable
prefixes repeated four times. On 2026-10-13, after the independently checked
weight-loading change, the same twenty trials all showed and accepted:
visible p50 228.0 ms, nearest-rank p95 283.6 ms, zero wrong-field edits.
Two focus-interrupted attempts were retained separately and are not part of
that completed timing sample. Timing ran from last key release
to the addon's display acknowledgement. Three further trials passed timed
Escape quiet across same-field edits, explicit requests during quiet, and
narrow-field quiet before acquisition. Moving focus before Tab dispatched no
commit. Native undo could merge acceptance with preceding typing, as permitted
by IME-parity. These are live samples, not held-out language quality evidence;
Omawrite 0.5.0’s native Open File location field also reported `file_dialog`,
made no automatic model call after typing, and displayed an explicit request.

The 2026-10-13 inline-preview handoff reports grey text and Tab acceptance in
Brave Origin's DuckDuckGo box and a disposable local textarea. Zen's
DuckDuckGo box showed inline text on Ctrl+Shift+Space; automatic suggestions
stayed quiet, and acceptance was not established in that trial. Google was
not tested because of its consent wall; no Google result is inferred from
DuckDuckGo. The earlier Chromium and Omawrite captures recorded zero physical
pixels between typed and ghost glyph-bottom modes at scale 2
([ADR 0006](../../docs/decisions/0006-grey-preview-is-an-overlay.md)).
These are prior live observations, separate from the merge's automated checks.

The merged `25fde96` build was installed on 2026-10-13 and exercised through
the running local model, observer and addon in Chromium 152.0.7977.82 on native
Wayland. Two disposable fields retained their grey suggestion for six seconds
before Tab: a textarea accepted inline text, and a 280 CSS px input accepted
the quiet overflow panel shown above the field. Neither edited the second
field. Undo merged with preceding typing in both cases. A horizontally scrolled
100 CSS px input showed no suggestion in two explicit-request attempts, so
that boundary remains unverified. These are acceptance samples, not latency
or language-quality scores. The disposable browser profile was removed; the
installed settings and keyboard profile remained byte-identical.

Type-through on 2026-10-13 night: the installed `b492c0a` broker, then the
checkout broker run in its place (same socket, real model, not installed),
with isolated profiles of Chromium 152, Brave Origin and Zen and `wtype` keys
on a disposable page. The installed broker carried the typed `l` of "latest",
then made a second model call: Chromium's text change also ended the
observer's epoch, so the addon admitted the same context again under a new
session and the carry, which then needed newly typed characters, declined
(`prefix_mismatch`). The fixed broker answered both admissions from the carry:
the remainder stayed on screen within about 300 ms and `provider_calls` did
not move, in Chromium (a typed letter, then three more), Brave Origin and Zen
(a typed space after "Thank you"). In that fresh Zen profile the suggestion
showed in Fcitx's candidate box, not the overlay. One Ctrl+Z in Chromium
undid the accepted text together with the preceding typing, as IME-parity
allows. `wtype` cannot start a call with a space on this desktop (it is
dropped even while Badi is paused), so the trial sent a Shift tap first.

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

On 2026-10-13, the installed `badi language chromium de` override produced
and accepted German suggestions in three disposable Chromium fields while
Fcitx stayed on `keyboard-us`; no insertion reached the second field. The
previous language setting was restored. This tests the override and native
dispatch, not independent German quality.

With the override removed and the same English keyboard, two unfinished German
phrases selected German from their field text and accepted into the correct
field: one automatic suggestion and one explicit request after no automatic
suggestion appeared. Two earlier complete phrases abstained. This verifies
the installed language path, not a latency distribution or quality score.

Open Omawrite or Xournal++ with `badi launch omawrite` or `badi launch
xournalpp` (in Xournal++, select the **Text** tool and click the page). Type
`Please find attached the` with the caret at the end. Omawrite suggests on its
own; in Xournal++, press Tab to request. Then Tab accepts a word (Ctrl+→ all
of it) and Escape dismisses.

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
| `npm run fcitx5:check` | Build plus deterministic suites: app classes and ids (and their agreement with the observer, `tests/observer-identities.py`), key, Tab, invoke and inspection decisions, type-through, word acceptance and the held Tab, binding and sanitization, framing and strict wire shapes, one-shot commits, foreign-IME yield, debug snapshots, reconnect |
| `npm run fcitx5:integration` | Also needs Cargo and Python 3. The shipped transport against a real broker in temporary roots: context, accept, report, dismiss, a type-through and word-acceptance scenario that must generate once, and recovery after a broker restart |
| `PYTHONDONTWRITEBYTECODE=1 python3 adapters/fcitx5/tests/observed-desktop.py` | The real addon in a private D-Bus session with its own Fcitx, broker and a synthetic observer: every app class, denials, panel and preview display, disagreement at request, display and dispatch, type-through with Tab, the held Tab and Ctrl+Right, a Ghostty typed line with quiet automatic misses, the Zen urlbar, foreign preedit, paragraph ends and recovery. Proves transport and state, not app behavior |
| `PYTHONDONTWRITEBYTECODE=1 python3 adapters/fcitx5/tests/desktop-tab.py` | Opt-in, after installation: synthetic contexts drive the installed addon and model through Tab request, accept and denials; run it while not typing elsewhere |

A staged install leaves the live Fcitx alone:

```sh
DESTDIR="$PWD/adapters/fcitx5/stage" cmake --install adapters/fcitx5/build --prefix /usr
```
