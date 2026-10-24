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
  `--app` windows (`brave-<host>__<path>-<profile>`) map to `brave-origin` and
  Chromium's (`chrome-<host>__<path>-<profile>`, how Omarchy opens web apps
  when Chromium is the default browser) to `chromium`; the observer admits only
  that browser's own executable under each, so Google Chrome stays out. VS
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
  | Ctrl+Shift+Tab | Wherever Badi would take Tab (a suggestion shows, a Tab is held for its remainder, or Tab would request on the manual path: a granted app at an eligible caret): retires the suggestion and sends the app a real Tab. Otherwise, as in an app without a grant, it stays the app's key. |

- **Literal Tab.** Ctrl+Shift+Tab is Cotypist's Option+Tab; Omarchy 4.0.6
  keeps Alt+Tab, Alt+Shift+Tab and Super+Tab for windows and workspaces, and
  a lone Alt tap opens the menu bar in Zen, VS Code and Writer, so the chord
  is the Ctrl+Shift family of Badi's other chords. Neither Omarchy's default
  bindings nor Fcitx bind it. The chord retires the suggestion like any edit
  (no dismissal is recorded) and drops a held Tab. The Tab then waits for the
  release that leaves no Ctrl, Alt, Shift or Super down, plus 20 ms for the
  compositor's modifier update: over Wayland's input-method-v2,
  `forwardKey` goes through Fcitx's virtual keyboard under the modifiers
  still held, so an immediate Tab would reach the app as Ctrl+Shift+Tab. A
  keyboard that sets modifiers as state without key events (wtype, an
  on-screen keyboard) sends no such release, so there the chord key's own
  release starts the wait. The Tab is forwarded with the chord key's own
  keycode: Fcitx 5.1.23 otherwise looks Tab up under the modifier state it
  last heard, where Shift turns it into ISO_Left_Tab and no key is sent.
  Another key typed first sends the waiting Tab ahead of it; a key pressed
  while the chord's modifiers are still down, or a focus change, drops it
  (`literal_tab_dropped`). A suggestion shown again while the Tab waited is
  retired when it leaves. The trace records `literal_tab_waiting` and
  `literal_tab_sent`. The Tab is the user's own key for the focused input
  context, never a suggestion's text, so it needs no grant.
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
  the held Tab without replaying it. At most one Tab is held per acceptance:
  the timeout, and a broker clear answering the request for the remainder,
  end its automatic return (`typeThroughAutomatic`), so the next Tab reaches
  the application. On the manual path no Tab is held: that Tab is the
  request that brings the remainder.
- **Manual request.** It needs a collapsed, non-composing, non-sensitive
  surrounding-text snapshot, the live `SurroundingText` capability, a valid
  input-method language or explicit app override (English, German or Persian),
  and a surrounding-text update since focus or the last authority change.
  Until a request, text events only invalidate local state.
- **fcitx5-qt's surrounding echo.** Since fcitx5-qt 5.1.16
  ([upstream change](https://github.com/fcitx/fcitx5-qt/commit/bac684ce07d9df78ee37dab1cbdcebfa71bf24ec)),
  fcitx5-qt withdraws `SurroundingText` after every key: its own
  cursor-rectangle update follows the key's text update, and an update that
  does not ask for the text now clears it. Fcitx keeps that text.
  `SurroundingEcho` (`state.h`) recognizes exactly that: `SurroundingText` alone withdrawn after text
  arrived under it, and alone granted back while withdrawn. Any other
  capability change or a focus change ends it, and the trace records the
  echo as `surrounding_echo`. Only an observed field keeps reading the text,
  so its observer still has to agree with the text and caret before request,
  display and dispatch. The manual path above keeps Fcitx's flags and stays
  closed. fcitx5-qt applies Fcitx's delete request to its own copy of the
  text, which the echo cleared, so a spelling replacement would only append:
  while the echo holds, a replacement is not shown and its commit is refused
  (`replacement_unavailable`).
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
  new text. A broker refusal of one request (an `error` frame, such as an
  acceptance of a suggestion that just expired) keeps the connection and ends
  only that request ([wire](WIRE_PROTOCOL.md#refusals)).
- **Replacement.** Native exact fields (`omawrite`, Telegram, Xournal++, and
  other granted native ids) may delete the exact typed suffix and then
  `commitString` the correction, still reported as `dispatched-unverified`.
  The suffix must still be the text before the caret. IME-parity apps but
  LibreOffice take a spelling correction the same way, after the observer
  re-confirms the field ([ADR 0011](../../docs/decisions/0011-checked-corrections-in-append-only-apps.md)).
  The preview draws it as `→ address`, and the panel fallback as
  `adress → address`. For a second after the commit, the addon waits for the
  app's surrounding text to end as the correction left it, and records
  `correction_confirmed` or `correction_unconfirmed`; an unconfirmed one also
  gets the notice "Badi could not confirm that correction — check the text".
  An update between the two edits gives no verdict. Native undo may take one
  step per edit (Omawrite 0.5.0, [tested with](#tested-with)).

### Revising a selection

Ctrl+Shift+Space on a selection asks for a grammar check of it, and pressing
it again while the revision shows asks for a refined wording (and back);
without a selection the chord keeps its meaning
([ADR 0012](../../docs/decisions/0012-explicit-revision-of-one-selection.md),
[wire](WIRE_PROTOCOL.md#revision)). The selection must be one line of 1–600
characters that Fcitx reports with its surrounding text. In an app the
observer reads, the observer also binds the field: inspect with `selection`
(no text), then, once policy allows the app, a selection snapshot whose
selection and surrounding text Fcitx's must match. Native exact apps the
observer cannot read use the manual contract; IME-parity apps have no
fallback, and the observer reads only plain fields there, not rich editors.
While the request runs, the panel says Badi is checking, and no inspection
runs; an edit, a changed selection, a focus change or an observer
invalidation retires the request.

The revision shows as Badi's candidate list: lines of up to 64 characters,
removed words struck through, added ones bold in the highlight colour, and a
help line (`Grammar · Tab apply · Esc dismiss · Ctrl+Shift+Space refine`).
Tab applies it, Escape dismisses it, and any other key retires it and keeps
its own meaning, so typing still replaces the selection. Applying checks the
selection again (the observer too, for an observed field), commits the
revision once and reads the field back for a second: the text before the
caret must end as the selection's prefix plus the revision, and the text
after it must be unchanged, or a notice names an unconfirmed revision. One
Ctrl+Z restores the selection where the app merges the commit into one undo
step, as it does for typing over a selection.

## App classes

`classifyNativeApp` (`src/state.cpp`) decides every path from the canonical id:

| Class | App ids | Edit path |
| --- | --- | --- |
| Native exact | Any other granted id, e.g. `omawrite`, `com.github.xournalpp.xournalpp`, `telegram` | The manual unknown-identity contract, or an observed desktop field |
| IME-parity browser | `chromium`, `chromium-browser`, `brave-origin`, `zen`, `zen-twilight` | Observed browser field only; origin policy; append-only |
| IME-parity desktop | `chatgpt` (Codex), `code`, `com.anthropic.claude` (Claude), `cursor`, `discord`, `grok-bot`, `libreoffice` | Observed field of the same app only; app policy; append-only |
| Unavailable | Other Gecko browsers (Firefox, other Zen builds and Flatpak ids, LibreWolf, Floorp, PWAsForFirefox, …), Chromium-family ids without an observer rule (Chrome, `brave`, `chrome-*`/`crx_*` web apps, Flatpak ids, Edge, Vivaldi, Opera, `electron*`, `code-oss`, `vscodium`, `vesktop`, …) and `obsidian`, whose plugin owns its fields | None |

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
  also covers private windows. A site allowed only by `site all on` also
  reports `matched_default`; browser targets come only from observed fields,
  which that reason opens.
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
`lineKey()` classifies each key with `lineEditFor()` and `TypedLine` applies
it at a tracked caret: typing inserts, Backspace, Ctrl+W, Delete, Ctrl+D and
Ctrl+K edit, and Left, Right, Home, End and Ctrl+A/B/E/F move, so the rest of
the line is the request's `after`. Enter, Ctrl+C and Ctrl+U start an empty
line. Word or vertical motion (Ctrl/Alt+arrows, Alt+B/F/D, Up, Down, Page
keys), or Left/Right past an end, stops tracking (`line_unknown`) until End
or an empty line; any other key, such as a paste or completion, restarts it
from the caret. A correction shows as `teh → the`; Tab commits one DEL per
tracked scalar of the typo followed by the fix, because a terminal ignores
`deleteSurroundingText`.
After the typing pause a line of two or more words
requests automatically; the panel shows the suggestion with the word Tab
takes in the theme's bold highlight and the rest plain, without the leading
space ([panel theme](../../ui/omarchy-plugin/README.md#suggestion-panel-theme)),
Tab types one word
and Ctrl+→ all of it, after which the next request follows the line. Tab
without a suggestion or a pending remainder stays the terminal's. A miss is
announced only after an explicit request: an automatic request that finds
nothing shows no notice, on this path and every other (`requestExplicit`
records how the published request was made; `onClear` shows
`clearNoticeText` only for an explicit one). When Neovim with
the Badi plugin holds the terminal's focus, the broker answers the terminal's
policy with `editor_focused` (trace `policy editor_focused`): the typed line
requests nothing, and Tab and Ctrl+Shift+Space reach Neovim, whose own
integration suggests with exact undo. The broker, not the addon, owns the
prompt window, so the typed line itself is not trimmed here.

### Observed fields

The broker target id is random per field visit. It survives reacquisition of
the exact same AT-SPI bus, object, process, app and URI solely to preserve a
timed dismissal. Focus loss, a different field or denied acquisition ends the
visit. Observer epochs, sessions, revisions, salts, policy checks and commit
grants remain fresh after invalidation; the observer's own epoch-bound target
still corroborates every snapshot and preview.

The [accessibility observer](../accessibility/README.md) supplies field identity
and corroboration. Fcitx 5.1.23 and later keep Chromium's surrounding text
current on Hyprland: the frontend answers each `done` with a commit
([upstream PR #1690](https://github.com/fcitx/fcitx5/pull/1690)). Neither
provides an editor transaction.

- **Display.** A preview request carries the input context's `caret_rect`
  when the frontend declares `RelativeRect` (Qt's Fcitx module); the observer
  uses it only where AT-SPI has no glyph geometry. When the observer verifies
  the field but reports `rendered:false` (no calibrated geometry), Badi shows
  its Fcitx panel, which Hyprland places at the app's text-input caret
  rectangle.
- **Inspection.** A keystroke in a field already under observation re-reads
  that field and reuses its session, so the model can run during the pause.
  It does not inspect again unless that read says the accessible changed.
  Any other field is inspected on the keystroke too, and so is a field the
  user's own typing invalidated: Chromium and Writer report each edit of the
  field, which ends the observation until the next inspection. Once a field
  has reported an edit in this focus visit, a keystroke waits for that report
  instead of re-reading the field first, whose model run the report would
  cancel; the typing pause is the fallback. The suggestion
  stays hidden until 120 ms after that keystroke, or 30 ms after a
  typed-through key. After the observer declined the field in this focus
  visit, input waits for the typing pause again; so does an idle invalidation
  (120 ms, then 240, 480 and 960 ms). A busy observer is retried every 20 ms. Badi's timers ask for 1 ms accuracy, because Fcitx's
  sd-event loop otherwise allows 250 ms of slack. With debug on,
  `inspect_sent` and `inspect_answered` time each inspection in the
  [debug trace](../../README.md#debug-trace), beside each key's action and
  each suggestion shown as the `overlay` or the `panel`.
- **Invalidations.** One after input re-inspects at once. One without input
  re-inspects after 240, 480 and 960 ms and then waits for input
  (`observer_awaiting_input`); a change of the field itself waits, except the
  first after an inspection within the typing pause, which can be that key's
  own late event and re-inspects after 120 ms.
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
  quote, table or inline-markup paragraph in the window) matches when Fcitx's
  `before` ends with its block text right after a line break and Fcitx's
  `after` begins with its rest of block and block end.

## Tested with

Live on the development workstation: Arch Linux, native Wayland, Hyprland
0.56.2, Fcitx 5.1.23, input method `keyboard-us`, the real local model and
disposable text in isolated profiles or scratch files, traced with
`badi debug on --text`. A cell is evidence for that app version, not for its
toolkit in general.

| App (version, date) | Reaches Fcitx through | Result | Boundary |
| --- | --- | --- | --- |
| Chromium 154 (2026-10-19/20), 155 (2026-10-23/24) | text-input-v3 (`wayland_v2`); observed, IME-parity | On 155 (2026-10-24): a textarea, a contenteditable after a trailing space, Quill, ProseMirror and Lexical on their own sites, a typo fix, a selection revision, and a password field and search boxes refused. With inspection on the keystroke, a textarea showed 8/8 inline in 189–308 ms after the last key on a fresh focus (median 215, against 276 before). The installed-flow matrix below; as an Omarchy web app (`chrome-<site>__-Default`) 4/4 inline in 168–321 ms, with window words. On 155, `adress ` showed `→ address`; Tab corrected it, the read-back confirmed it and one Ctrl+Z restored the typo. A page that moves focus on the deletion split it, and the notice named it ([ADR 0011](../../docs/decisions/0011-checked-corrections-in-append-only-apps.md)) | Page fields carry no purpose hints, so the observer's checks deny passwords; the omnibox's `Url` purpose is denied before any observer request. Needs Fcitx 5.1.23 |
| Brave Origin 1.97 (2026-10-20), 1.98 (2026-10-24) | as above | On 1.98: a textarea, a contenteditable and a confirmed typo fix; a password field sent nothing. On 1.97: 5/5 in a textarea, 3/3 in a contenteditable, 4/4 as an `--app` web app; Ctrl+→ takes all, Escape hides without editing | as above |
| Zen 1.23.3b, Gecko 158 (2026-10-20) | `wayland_v2`, `zen` | Suggestions and window words | The urlbar shares the page's context with no `Url` hint, so the observer denies it. Gecko sends only the caret's paragraph. One Ctrl+Z undoes an acceptance alone. A fresh profile once showed the suggestion in Fcitx's candidate box |
| Zen Twilight 1.24t (2026-10-24) | `wayland_v2`, `zen-twilight` | A textarea and a contenteditable suggested; a typo fix was confirmed by read-back | As Zen: the observer denies the urlbar; password and search fields refused |
| VS Code 1.142 (2026-10-20), 1.143 (2026-10-24) | as Chromium | 3/3; the saved file read back exactly; on 1.143 a markdown buffer suggested and Tab committed a word | Needs `"editor.editContext": false`; its default EditContext sends corrupted text, which fails closed. `wtype` trials need `"keyboard.dispatch": "keyCode"` |
| LibreOffice Writer 26.8.1, GTK 3 (2026-10-20/24) | observed sentence | 2/3 inline in 169 and 241 ms; Tab commits one word; on 2026-10-24 inline again, and Alt+F7 turned `lsit` into `list`, undone by one Ctrl+Z; with inspection on the keystroke, 8/8 inline in 216–300 ms after the last key (median 241, against 359 before) | Writer sends only the caret's sentence. Spelling goes through the [extension](../libreoffice/README.md): Alt+F7 replaces, one Ctrl+Z restores |
| Ghostty 1.3.1 (2026-10-20/24) | typed line ([ADR 0009](../../docs/decisions/0009-terminals-use-the-typed-line.md)) | 3/3 in the panel; one word per Tab; corrections erase the typo with DELs (`teh → the` on 2026-10-24) | Exact opt-in rule |
| foot 1.28.0 (2026-10-20) | typed line | 4/4 in the panel in 118–307 ms; `adress → address` with DELs | Exact opt-in rule |
| Omawrite 0.7.0 (2026-10-20), 0.7.2 (2026-10-24), Qt 6.12, fcitx5-qt 5.1.16 | `QT_IM_MODULE=fcitx`; observed native exact | 6/7 in 248–426 ms; Ctrl+→ takes all; one Ctrl+Z restores the exact prefix; on 0.7.2 a suggestion outlives the autosave's title change and Tab commits | Suggestions through the [surrounding echo](#contract); no spelling replacement until fcitx5-qt is fixed. 0.5.0 replaced a typo in two undo steps |
| Xournal++ 1.3.7, 1.3.8 (2026-10-24), GTK 3 text tool | `GTK_IM_MODULE=fcitx` launcher override | 20/20; on 1.3.8 a Tab request showed in the panel and Tab committed it | Tab requests; after Escape leaves text editing, one document undo restores the prefix |
| Telegram 7.3.2, Qt 6, fcitx5-qt 5.1.16 (2026-10-24) | `dbus`, canonicalized to `telegram`; observed | The composer suggested inline through the [surrounding echo](#contract) and Tab committed a word | As Omawrite: no spelling replacement until fcitx5-qt is fixed |
| Discord 1.0.166 (2026-10-24) | as Chromium; the flag on its launcher | Without the renderer flag the observer found no field; started with it, also from the launcher `--observed-app discord` writes, the composer suggested inline and Tab committed a word | Typing shows Discord's typing indicator, so trials use a private channel |

The 2026-10-24 cells, their drivers and screenshots are in
`output/native-trials/2026-10-24-full-trial/` (`results.jsonl`).

What the live trials established, beyond those cells:

- **Selection revision** (Chromium 155, installed `2956224`, 2026-10-23,
  [ADR 0012](../../docs/decisions/0012-explicit-revision-of-one-selection.md)):
  an English grammar fix (4.1 s cold) and its refinement showed with marked
  changes, applied once with a confirmed read-back, and one Ctrl+Z restored
  the selection; a German clause kept its verb last; a correct selection said
  nothing to change, and a password field sent no request. Writer refuses
  (Fcitx gets only the caret's sentence); native GTK editors were not tried.

- **Display and timing** (Chromium 152, 2026-10-13): 20/20 warm suggestions
  shown and accepted, visible p50 228 ms and p95 284 ms from the last key
  release; a 280 CSS px input used the overflow panel above the field.
- **Type-through** (Chromium, Brave Origin, Zen, 2026-10-13): the remainder
  returned within about 300 ms with no model call.
- **Tab** (Ghostty, Chromium, Omawrite, 2026-10-16): Tab takes one word,
  quick Tabs are held for the remainder, and Tab with nothing showing stays
  the app's.
- **Installed-flow matrix** (Chromium 154 and Ghostty, 2026-10-19), with no
  wrong-field or stale edit:
  - `badi pause` hides a suggestion and lets Tab reach the page.
  - Tab 0.2, 1 or 3 s into a restart of the broker, observer, session
    helper or Fcitx commits nothing stale.
  - Page script moving focus closes the first field's session.
  - Fcitx's Unicode composition hides the suggestion and Tab commits
    nothing.
  - One Ctrl+Z after an acceptance also removes the preceding typing, as
    IME-parity allows.
  - Ghostty's typed line survives a broker restart; an Fcitx restart starts
    a new line.
  - At fractional scale 1.5 the grey text sits on the caret's baseline.
- **Literal Tab** (Chromium 154, 2026-10-20): Ctrl+Shift+Tab with a
  suggestion showing reached the page as one plain Tab; with none showing,
  the chord stayed Chromium's.
- **Quiet**: Escape quiet across edits, a request during quiet,
  narrow-field quiet before acquisition, and Omawrite's Open File dialog
  (`file_dialog`, explicit requests only).
- **Neovim in Ghostty** (the user's LazyVim, 2026-10-17): with the Badi
  plugin focused only Neovim asked, Tab and the chord reached Neovim, and
  after `:q` the shell's typed line suggested again.
- **A prompt window's run-on line** (Claude Code in Ghostty, 2026-10-17):
  with the broker moving the prompt's sentence start in 64-scalar steps, 50
  of 51 requests were proposals with no cancellation or budget outcome,
  where the sliding start had cost 14 cancellations and eight budget
  outcomes in 31.
- **Language override** (Chromium, 2026-10-13): `badi language chromium de`
  gave German suggestions under `keyboard-us`; without it, unfinished German
  phrases chose German from their own text. Not a quality score.

Not yet tried live:
- a real suspend and resume, and a VT switch;
- several monitors, and non-US keyboard groups;
- a horizontally scrolled 100 CSS px input, which showed nothing on two
  explicit requests;
- Google's search box (consent wall).

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

On other desktops (GNOME, KDE Plasma, Debian and Ubuntu), the desktop
starts Fcitx, so the installer never restarts it. It writes
`~/.config/environment.d/50-badi-fcitx5.conf` with the same two directories
as Omarchy's drop-in and this system's own addon directory
(`/usr/lib/x86_64-linux-gnu/fcitx5` there), and Fcitx loads the addon at the
next login ([Debian and Ubuntu](../../packaging/debian/README.md#install-from-a-checkout)).

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

The addon builds and passes its suites against Fcitx 5.1.23 (Arch), 5.1.12
(Debian 13) and 5.1.7 (Ubuntu 24.04), and those Fcitx versions load it
([tested with](../../packaging/debian/README.md#tested-with)). Fcitx 5.1.7
predates `add_fcitx5_addon`, `FCITX_ADDON_FACTORY_V2` and
`eventloopinterface.h`. There CMake builds the same `MODULE` library the
macro builds, the addon exports the unnamed factory symbol that 5.1.7's loader
resolves, and [`fcitx_event.h`](src/fcitx_event.h) includes the event-source
header only where it exists. The literal-Tab chord is matched on the key as
the frontend sent it (Tab or ISO_Left_Tab under exactly Ctrl and Shift),
because 5.1.7's `Key::normalize()` turns Ctrl+Shift+Tab into Ctrl+Tab. No
other difference is known; neither older version has run in a live session.

| Command | Covers |
| --- | --- |
| `npm run fcitx5:check` | Build plus deterministic suites: app classes and ids (and their agreement with the observer, `tests/observer-identities.py`), key, Tab, literal-Tab, invoke and inspection decisions, type-through, word acceptance and the held Tab, binding and sanitization, framing and strict wire shapes, broker refusals, one-shot commits, foreign-IME yield, debug snapshots, reconnect, the revision core and its observer gates, and the broker and observer sockets |
| `npm run fcitx5:integration` | Also needs Cargo and Python 3. The shipped transport against a real broker in temporary roots: context, accept, report, dismiss, a type-through and word-acceptance scenario that must generate once, and recovery after a broker restart |
| `PYTHONDONTWRITEBYTECODE=1 python3 adapters/fcitx5/tests/observed-desktop.py` | The real addon in a private D-Bus session with its own Fcitx, broker and a synthetic observer: every app class, denials, panel and preview display, disagreement at request, display and dispatch, type-through with Tab, the held Tab and Ctrl+Right, a Ghostty typed line with quiet automatic misses, the Zen urlbar, foreign preedit, paragraph ends, recovery, the literal Tab with and without a grant, a stream moving from the panel to the preview, a refused acceptance, and a revision applied once despite a second Tab. Proves transport and state, not app behavior. Each run writes `output/observed-desktop/observed-dbus-*/` with `result.json`, the broker and Fcitx logs and the session's own output (`session.log`), even when the session dies before its logs open; it needs a python3 that imports PyGObject |
| `PYTHONDONTWRITEBYTECODE=1 python3 adapters/fcitx5/tests/desktop-tab.py` | Opt-in, after installation: synthetic contexts drive the installed addon and model through Tab request, accept and denials; run it while not typing elsewhere |

A staged install leaves the live Fcitx alone:

```sh
DESTDIR="$PWD/adapters/fcitx5/stage" cmake --install adapters/fcitx5/build --prefix /usr
```
