# Badi accessibility observer

This helper gives the Fcitx addon **field identity, bounded context and an
optional grey preview** at the caret. It never inserts, deletes, selects or
types text, and it changes no accessibility setting or app policy.

## App rules

Each supported Fcitx app id maps to one identity rule. The active Hyprland
window's process must belong to this user, run the listed executable (from
`/proc/PID/exe`, so a deleted or replaced binary fails) and show the listed
window class. A rule makes a field eligible; it does not prove the app's
editing works ([coverage](../../README.md#where-it-works)).

| App id | Executable | Window class | Broker target |
| --- | --- | --- | --- |
| `chromium` | `/usr/lib/chromium/chromium` | `chromium`, or an `--app` window `chrome-<host>__<path>-<profile>` (Omarchy's web apps) | browser origin |
| `chromium-browser` | as above, started without `/usr/bin/chromium`'s `CHROME_DESKTOP` | `chromium-browser` | browser origin |
| `brave-origin` | `/opt/brave-origin-bin/brave` | `brave-origin`, or an `--app` window `brave-<host>__<path>-<profile>` | browser origin |
| `zen` (Gecko) | `/opt/zen-browser-bin/zen-bin` | `zen` | browser origin |
| `chatgpt` (Codex) | `/usr/lib/chatgpt/ChatGPT` | `chatgpt` | desktop app |
| `code` (VS Code) | `/usr/share/code/code` | `code`, or `com.microsoft.VSCode` from 1.140 | desktop app |
| `cursor` | `/usr/lib/electron42/electron` running exactly `/usr/share/cursor/resources/app/cursor.mjs` | `cursor` | desktop app |
| `discord` | `$XDG_CONFIG_HOME/discord/app-<version>/Discord` | `discord` | desktop app |
| `grok-bot` | `/opt/Grok Bot/grok-bot` | `grok-bot` | desktop app |
| `com.anthropic.claude` (Claude) | `/usr/lib/claude-desktop/claude-desktop` | `com.anthropic.Claude` | desktop app |
| `telegram` | `/usr/bin/Telegram` | `org.telegram.desktop` | desktop app |
| `omawrite` | `/usr/bin/omawrite` | `omawrite` | desktop app |
| `libreoffice` | `/usr/lib/libreoffice/program/soffice.bin` | `libreoffice-writer`, `paragraph` fields only | desktop app |

- **Cursor** shares the Electron runtime, so its command line (read up to
  64 KiB) must name Cursor's entry as Chromium parses switches. Options that
  load other code or run no app (`--app=`, `-r`, `-i`, `-v`, `-a`, …) make it
  unrecognized. The rule separates apps; it is no defence against same-user
  malware.
- **Discord** updates itself in the user's configuration. Every path component
  must be a real, user-owned directory or file without group or other write
  permission, and the binary must be the inode the process runs.
- **Brave web apps** take their page origin's policy, as a tab does.
- **LibreOffice** reports one program id for every module, so only Writer's
  `paragraph` fields qualify. Writer gives Fcitx only the caret's sentence, so
  the snapshot, after the binding is authorized, starts at the AT-SPI
  `SENTENCE` boundary and must still equal it. Inspect does not read that sentence.
  At a trailing space that ends the paragraph, Writer 26.8 answers an empty
  range (0, 0); the lookup steps back over empty answers only, and the
  sentence it finds must still contain the caret's character. Fcitx's
  sentence there ends before the space; the addon restores only that space
  ([Fcitx runbook](../fcitx5/README.md#observed-fields)).

Browser targets share one browser-origin identity (adapter `chromium`), so one
`badi site ... on` grant covers Chromium, Brave and Zen. The target and binding
keep the exact app id so per-app language overrides stay separate. Update the
broker and observer together: older brokers recognize only `chromium` browser
targets. Electron apps render web content, so the HTML purpose gates apply
to them too. A missing tag or input type is ineligible, never guessed. Search,
email and URL inputs, combo boxes and ARIA search boxes (`xml-roles:
searchbox`) get no suggestions.

Two further facts quiet **automatic** suggestions only; the field stays
eligible and an explicit request still reads it. A field inside a
`file chooser` ancestor reports `quiet: "file_dialog"`. A field under twelve
glyphs wide and no taller than one padded line (three glyph heights), judged
from its own SCREEN extents against the glyph before the caret, reports
`quiet: "narrow_field"`. An unreadable fact is unknown and quiets nothing: a
field narrower than two glyphs is a caret-wide proxy (VS Code), zero glyph
boxes (Qt) say nothing, and a taller box is not judged narrow. The addon
honors the fact before any automatic snapshot, so a quiet field's prose is
not read until the user asks. The manual native Fcitx path has no dialog or
width signal and cannot honor this; only observed fields are quieted.

### Sensitive fields

Detection is the toolkit's mark, not the words in the field.

- The Fcitx addon reads no context when the input method reports
  `PasswordOrSensitive`, or a special purpose such as email, number, URL,
  date, or time.
- The observer refuses an accessible whose role is `password text`, or whose
  `text-input-type` is `password`, before it reads the caret or any text.
- It also refuses one whose accessible name or description names a secret
  in Badi's languages (`contract.CREDENTIAL_LABEL`: password, Passwort, mot
  de passe, contraseña, senha, PIN, one-time or verification code, CVV,
  card number, …), so a password a page's "show password" button turned
  into `type=text`, or a plain-text card or code field, is never read. The
  placeholder and the HTML `autocomplete` value are not exposed and not
  used; an unlabelled secret field the toolkit does not mark is still
  handled like prose.

## Authority boundary

`inspect` verifies the unlocked Omarchy session, the active Hyprland window's
process, its identity rule and exactly one focused editable object. It queries
only that process and reads no prose. Every binding comes from an `inspect`;
`snapshot` and `preview` require its unchanged epoch.

Hyprland's active window and monitors come from its IPC socket. Each inspection
asks the running Omarchy shell for fresh `lock.status`, overlapping the field
inspection and waiting at most 150 ms within its operation budget. Unless all
five lock flags are explicitly false, the lock denial wins; errors fail closed.

On Quickshell 0.3.1 and 0.3.2, the observer caches the discovered IPC endpoint and sends
the query directly. It never caches an unlocked answer. Peer UID and PID must
match discovery; instance-directory or executable changes invalidate the
endpoint and an overlapping reply. Other versions use `qs ipc -n -p
$OMARCHY_PATH/shell call -- lock status` as before. The small wire subset is
pinned to upstream [command tags](https://github.com/quickshell-mirror/quickshell/blob/v0.3.1/src/ipc/ipccommand.hpp),
[call replies](https://github.com/quickshell-mirror/quickshell/blob/v0.3.1/src/io/ipccomm.cpp)
and Qt's length-prefixed UTF-16 `QString`; 0.3.2, checked at Omarchy's
revision 2b4501a, changed neither file. Unknown replies are denied.

### Browser fields

A browser field needs an HTTP(S) address: Chromium's Document `URI`, Gecko's
`DocURL`. Cross-origin frames keep their own; an `about:blank` or `about:srcdoc`
frame takes its creator's origin.

- **Chromium** reports `FOCUSED` on several fields at once (the page field and
  the omnibox or its popup). Browser UI (nodes with no document, and
  `chrome://*.top-chrome` WebUI) is ignored, and exactly one HTTP(S) field must
  remain. The omnibox's Fcitx context carries the `Url` purpose, which the
  addon denies before any inspection.
- **Gecko** (Zen) exposes web content without a flag. The urlbar shares the
  page's Fcitx context without a URL purpose, so the observer denies it: the
  focused node's nearest Document must be a `document web` with an HTTP(S)
  `DocURL`. Page fields are `textarea`, `input` with an explicit
  `type=text`, or a `role=textbox` contenteditable `div`. Gecko's surrounding
  text is only the caret's paragraph, so a newline before the caret fails
  agreement (`observer_context_mismatch`).

### Rich editors

A rich editor root (ProseMirror, Lexical, Quill, Draft.js, Slate, CKEditor 5,
TinyMCE, CodeMirror in contenteditable mode, or `<p>`/`<div>` lines) is one
focused object whose text is one U+FFFC per block. It is an `entry`, or a
`section` whose tag is `div` or an editing iframe's `body`.

For Chromium and Electron content, the root is flattened the way Chromium
serializes surrounding text:
- **Block ends.** A `paragraph` (`<p>`) or `heading` ends with `"\n\n"`, a
  `section` (`<div>` line) with `"\n"`. A block whose own text ends in a line
  break gets one break fewer.
- **Inline objects** (`<strong>`, `<em>`, `<code>`, `<span>`, links, `<mark>`,
  `<sub>`, `<sup>`) are written as their own text.
- **Opaque blocks:** any other block (a list, quote, table or image).

`inspect` reads only roles, offsets, lengths and selection counts. `snapshot`
reads, after the policy grant, the blocks in a window one character per block
wider than 512/128, rereads them for the stale check and keeps the last 512
characters before and 128 after the caret. When an opaque block overlaps that
window, the snapshot covers only the caret's own block and says
`scope: "block"`.

Fails closed before prose:
- `unsupported_field`: text directly in the root, more than 64 blocks, a caret
  inside an opaque block or next to a non-text object, or an unknown U+FFFC.
- `invalid_caret`: an ambiguous caret.
- `selection_present`: any selection.

EditContext editors (VS Code's default, CodeMirror on recent Chromium) expose
no editable node and stay unavailable.

## Selection for a revision

An explicit revision ([ADR 0012](../../docs/decisions/0012-explicit-revision-of-one-selection.md))
reads one selection instead of a caret. `inspect` with `"selection": true`
accepts a field with exactly one selection (`selection_absent` otherwise) and,
like every inspect, reads no text. A `snapshot` of that binding with
`"selection": true` (never with `window`) returns `before` (≤512
characters), `selected` (1–600, one line) and `after` (≤128), read with the
same stable-text rule and the selection range read again afterwards
(`stale_binding` if it moved). Rich editors, sentence-bounded apps, a
selection across lines and a longer one are `unsupported_selection`. Without
the flag a selection still denies the field (`selection_present`).

## Snapshots and field events

`snapshot` must repeat the exact binding (bus, path, process, app, URI, epoch)
and policy target, and its Fcitx caller must hold a current context-read grant.
Sensitive, noneditable, invisible, selected, unsupported and stale fields fail
before any text read. The caret, length, selections and text are reread to
detect changes; the result is a snapshot, never an atomic edit transaction, so
Fcitx compares its live text again before dispatch.

Focus and window changes and the field's own events invalidate the epoch and
hide the preview. After an approved snapshot, the field's events (text, caret,
selection, editable, showing, defunct, role) are subscribed on a separate
D-Bus connection, matched to that exact sender and path, so other fields'
events never arrive. Their payloads, including typed text, are never unpacked.
A bounds or visible-data event is a scroll that moves the preview. A text
attributes event changes neither text nor caret and is ignored: Chromium's
spellchecker sends one about 150 ms after a misspelled word, and it must not
end the suggestion for that word, a correction above all. Each event the
observer acts on is an operation with its own 350 ms budget, like a request;
an unexpected failure ends the suggestion and logs only its class.

## Same-window context (Lab only)

`FieldBackend.window_context()` finds the nearest heading above a web field and
the end of the paragraph just before it, in the field's own document. It is
not part of the socket contract and no snapshot carries it: production
prompts use no window context until the Lab shows it helps without harm
([backlog](<../../future plans.md>)).

- **Scope.** Web fields only. The binding's address must equal the field's
  nearest document address, and the result counts only once a `document web`
  ancestor confirms the walk stayed in that document. Native apps get nothing.
- **Walk.** Backwards from the field: each ancestor's earlier siblings, last
  first, at most 96 nodes and 12 levels. The first heading ends the search;
  the paragraph must be nearer than that heading.
- **Never read:** anything editable (another field or editor), a password
  node, a node that is not showing, or a nested `document web` (another
  frame, possibly another origin).
- **Bounds.** A heading of at most 120 and a paragraph tail of at most 240
  characters, cut at a word, with embedded objects, format and control
  characters handled as for window words and spacing collapsed.

`python3 evaluation/writing/window-context.py APP_ID --delay 3` prints, for
the focused field of an allowed web app, the window context and the text
before the caret as JSON, under the observer's identity and lock rules. It is
a capture aid for Lab cases from disposable pages; it writes no file.

## Window words (context on)

With `window: true`, which the Fcitx module sends once per field visit while
the broker's policy reply says `context_words`, a `snapshot` adds
`FieldBackend.vocabulary_text()` as `window_text`
([ADR 0010](../../docs/decisions/0010-context-finishes-words.md)). It reads
the text displayed near a web field so the broker can finish a typed word with
a word in view, such as a name from the thread or a term from a coding session.

- **Scope.** Web fields only, in their own `document web`, named by
  Chromium's `URI` or Gecko's `DocURL`. In a browser that document must have
  the field's page address; a Chromium-based app (Claude, Codex, VS Code) has
  no page address, so its own document is the bound. Native apps get nothing.
- **Walk.** From the field: each ancestor's earlier siblings, last first, then
  its later siblings, until that document. Each node's text is read with its
  embedded objects dropped, then its children's.
- **Never read:** anything editable (another field, such as a subject line),
  a password, a node that is not showing, or a nested `document web`.
- **Characters.** The addon refuses window text that holds a format or
  control character, so format characters (Unicode Cf: a soft hyphen, the
  zero-width joiner inside an emoji sequence, a byte-order mark, bidi
  controls) are deleted, except a zero-width non-joiner between two Arabic
  letters, and control characters become spaces.
- **Bounds.** 2,000 characters, 400 nodes and 120 ms, always leaving 50 ms of
  the operation's deadline for the climb back to the document (two calls per
  level, no more siblings), so a slow page shortens the text. A snapshot
  that still overruns its 350 ms fails as `operation_timeout`. The text is
  spacing-collapsed and cut at a word.

The debug trace records only `window_text_chars`. Live results are in
[Tested with](#tested-with); a real mail client and the Claude desktop app are
not yet tested ([backlog](<../../future plans.md>)).

## Socket contract

`python3 adapters/accessibility/daemon.py` serves
`$XDG_RUNTIME_DIR/badi/accessibility.sock` (socket `0600`, directory `0700`) to
same-user peers, one instance under an exclusive lock.

- Frames are newline-terminated JSON lines of at most 16 KiB (64 KiB buffered).
- Each operation (a request, or a field event that moves the preview) has a
  350 ms budget; AT-SPI calls time out after 50 ms and Hyprland calls after
  150 ms.
- One connection owns acquisition and preview; another gets `observer_busy`.
- No typed text reaches logs or disk.

Requests carry `schema: "badi.accessibility.v1"`, an ASCII `id` and an `op`:

```json
{"schema":"badi.accessibility.v1","id":"a1","op":"inspect","app_id":"chromium"}
```

| Op | Request adds | Reply `focus` adds |
| --- | --- | --- |
| `inspect` | `app_id`, optional `selection: true` | `binding` `{epoch,bus,path,process_id,app_id,uri}`, `target` (browser targets keep the exact app id and origin; `target_id` hashes bus, path and epoch), `purpose`, `caret`, `selection_count`, `geometry: null`, optional `quiet` |
| `snapshot` | `binding`, `policy_target`, optional `window: true` or `selection: true` (never both) | `before`, `after`, `total_chars`, optional `scope: "block"`, optional `quiet`, with `window` an optional `window_text`, with `selection` the `selected` text ([selection](#selection-for-a-revision)) |
| `preview` | `binding`, `policy_target`, `expected_caret`, `expected_total_chars`, `text` (1–66 characters: a suggestion of at most 64, or `→ ` and a correction), `ttl_ms` (1–15000), optional `caret_rect` `{x,y,width,height,scale}` | `total_chars`, `rendered`, optional `quiet` |

`preview` compares the current caret and length with the expected ones before
drawing. `hide` returns `hidden:true`; `status` returns readiness without
acquiring anything, and `health.py` probes it for the installer and `badi
doctor`. Readiness does not prove that any app exposes an eligible field.

Failures return `{schema,id,ok:false,error,epoch}` with a content-free code;
every failure except a malformed request or `observer_busy` hides the preview:

| Error | Cause |
| --- | --- |
| `invalid_request`, `invalid_preview` | Malformed envelope (a duplicate key too), binding or preview |
| `unsupported_app`, `app_mismatch` | No rule for the app id, or the active window is not that app |
| `desktop_unavailable`, `desktop_locked` | Hyprland or the lock query failed, or a lock flag is set |
| `focus_unavailable` | Not exactly one focused editable page field |
| `origin_unavailable`, `unsupported_origin` | No readable address, or not HTTP(S) |
| `sensitive_field`, `ineligible_field` | A password role or type; not focused, editable, showing, visible and enabled |
| `unsupported_field`, `selection_present`, `invalid_caret` | Another role or HTML purpose, browser UI or an unresolvable rich editor; a selection; a bad caret |
| `selection_absent`, `unsupported_selection` | With `selection`: not exactly one selection; a rich editor, a sentence-bounded app, or a selection across lines, blank or over 600 characters |
| `tree_limit`, `invalid_text` | A search passed 192 nodes; control characters in the text |
| `stale_binding`, `target_mismatch` | The field, epoch, position or text changed, or another policy target |
| `operation_timeout`, `observer_busy`, `accessibility_unavailable` | Budget exceeded; another owner; AT-SPI failed |

Unsolicited `{schema,event:"invalidate",epoch,reason,app_id}` messages name the
previous app; a client must drop its candidate and inspect afresh. When the armed field, or a scroll-pane ancestor, reports
`object:bounds-changed` or `object:visible-data-changed`, the grey preview
moves to the caret's new position if the text offset is unchanged. If the
caret moved or can no longer be drawn, the suggestion ends. A scroll the
toolkit reports on neither the field nor a scroll pane does not.

## Caret geometry and preview

Only `preview` calibrates, again each time, from the window frame F, the
nearest web document D, the field E and the glyph G before the caret. Any
inconsistency yields no geometry, and the addon uses the Fcitx panel. Every
rule yields the window-local logical caret, the field bounds, the window and
its monitor.

- **Chromium and Electron** report D, E and G in physical pixels offset by F's
  origin times the scale s, where `D.width / F.width` must equal the monitor
  scale within 0.01. In an iframe the outermost page document proves the scale
  instead and must contain the iframe's document. The logical caret is `x = (G.x + G.width - F.x*s)/s`,
  `y = (G.y - F.y*s)/s`, with line height `G.height/s`.
- **Caret-wide fields.** VS Code (with `editor.editContext` off) keeps its
  input textarea only as wide as the caret and places it right after G. A
  field at most two logical pixels wide whose left edge meets G's right edge
  on G's line marks the caret; its parent, the editor widget, must contain
  both and bounds the drawing.
- **Gecko** reports F, D and E in logical pixels and G in device pixels, with F
  at the window origin. The caret is `x = (G.x + G.width)/s`, `y = G.y/s`. A tile
  narrower than Zen's minimum content width fails and uses the panel.
- **Native windows (LibreOffice).** AT-SPI `WINDOW` coordinates are logical
  pixels relative to the toplevel, so F must be exactly `(0, 0)` and the
  window's size. E must contain G and F must contain E; the caret is
  `x = G.x + G.width`, `y = G.y`. LibreOffice's `SCREEN` coordinates are offset
  by its toolbars and are never used.
- **Input-method rectangle (Qt).** Qt fields (Telegram, Omawrite) report zero
  glyph boxes. When no rule above applies, the addon's optional `caret_rect`
  places the caret: Qt's Fcitx module declares `RelativeRect` and reports the
  caret relative to its window in physical pixels, so the caret is
  `x = rect.x/s`, `y = rect.y/s` with line height `rect.height/s`, and
  `rect.scale` must equal the monitor scale. The field's `WINDOW` extents bound
  the drawing when its frame spans the window and the field holds the caret;
  otherwise the window does. A rectangle outside the visible window (a clipped
  tile) yields nothing.

Geometry is offered only for left-to-right text: the caret's line must hold
no right-to-left character and the field must report the `lr` direction or
glyphs ordered left to right. Qt reports neither; a native field without a
direction needs a strong left-to-right letter on the caret's line, because Qt
takes a paragraph's direction from its first strong character.

The GTK4 layer-shell overlay draws the suggestion as translucent grey text right
after the caret, with no background. When the caret's text run reports a font
family, a size (`size` in points, or `font-size` in px) or a weight, the
overlay uses those, and the baseline is anchored at that font's real Pango
ascent below the glyph box top; without a reported family the size comes
from the glyph height, the face stays the overlay's own and its ascent as a
fraction of ascent plus descent places the baseline within the caret height.
If those metrics are unavailable, the layout-ratio heuristic remains.
Integer layer margins favor the monitor's physical pixel grid and are checked
after snapping against the drawing bounds, including the following glyph.
Some fractional scales cannot put every margin on an exact physical pixel.
On Hyprland the observer asks the
compositor not to animate its layer (`no_anim` rule for the
`badi-writing-preview` namespace, re-applied at most every 30 seconds since a
config reload drops runtime rules). Refreshes run outside the rendering
thread so compositor IPC cannot delay expiry or field invalidation; the
desktop installer persists the same rule
as `~/.config/hypr/badi.lua`, loaded from `hyprland.lua`. A field box that ends before the caret still
keeps that text on the caret's baseline inside the window, so it is not dropped onto the chrome under the
input. A box that still contains the caret is a real edge: text that would
pass it is not drawn over the controls beside the field. When the last
snapshot had words after the caret on its line
(`text_follows`), the glyph after the caret bounds the inline room: text
that ends before it still draws inline, and only text that would cover it
draws a compact chip (grey text on a dark background) under the caret
instead while it fits inside the field, 2 px clear of its border. When
nothing but whitespace follows the caret in a plain (not rich-editor) field
(`text_ends`), the lines under the caret are empty, so text too long for the
line draws as that chip under the caret's line, never above it. Any other
overflow uses the same chip wholly below or above the field, bounded by the
window and monitor, so it does not cover controls beside the field. The
chip's padding is a label margin, not CSS: GTK restyles a class toggled just
before measuring only on the next frame, so a CSS padding measured the chip
12 px narrow and let it cross the field's edge. There is no title, key
banner or selection highlight.
One rule (`contract.suggestion_text`) checks the preview text when the
request arrives, and the overlay checks it and the 15-second lease bound
again before drawing; text that fits nowhere returns `rendered: false` for
the native single-candidate fallback. The surface takes
no input or focus, and any invalidation, new render or expiry hides it. The
preview accepts the broker's remaining lease up to 15 seconds; scrolling
repositions it with only the remaining time and never renews it. The
helper forces software rendering and the simple GTK input module, so it never
loads a GPU driver or connects to an input method.

With `badi debug on`, `debugtrace.py` adds each request, each preview drawn
(placement and monitor-local box), declined (`uncalibrated_caret`,
`right_to_left_text`, `monitor_unmatched`, `no_room`), mapped and hidden
(`replaced`, `expired`, `withdrawn`) to the
[debug trace](../../README.md#debug-trace).

## Installation and the renderer flag

The full desktop install copies the helper and its service and waits for a
health reply before restarting Fcitx. The system accessibility bus must be
enabled; setting its `IsEnabled` property can persist GNOME's
`toolkit-accessibility`, so the installer records the prior values.

Chromium exposes web content over AT-SPI only with
`--force-renderer-accessibility=complete`. That is the lightest mode with the
HTML tag and character extents the observer needs; it costs some browser CPU
and memory on large pages. Gecko needs no flag.
`install-desktop.py --observed-app APP` appends it, with a comment, to that
app's flags file:

| App | File in `$XDG_CONFIG_HOME` |
| --- | --- |
| `chromium` | `chromium-flags.conf` |
| `brave-origin` | `brave-origin-flags.conf` |
| `chatgpt` | `codex-flags.conf` |
| `code` | `code-flags.conf` |
| `cursor` | `cursor-flags.conf` |
| `grok-bot` | `grok-bot-flags.conf` |
| `com.anthropic.claude` | `claude-desktop-flags.conf` |

A file that already has the flag is left alone. A reduced mode or
`--disable-renderer-accessibility` is the user's choice and stops the installer
before it changes anything. Changed files are backed up, and running windows
keep their old flags until relaunched. Discord has no reliable mechanism: its
web client resets its `chromiumSwitches` list after one launch, so the
installer writes nothing to that file. The desktop-wide alternative is:

```sh
python3 scripts/install-desktop.py --screen-reader-accessibility
```

This explicitly enables AT-SPI `ScreenReaderEnabled` and the persistent
`org.gnome.desktop.a11y.applications screen-reader-enabled` setting. It
advertises accessibility to all desktop applications, including Discord;
it does not install a screen reader or change Badi's app/site permissions.
Applications may need relaunching. Prior values are saved privately in
`accessibility-screen-reader.json` beside that install's backup, or under
`install-backups/notes` if no file changed. An already enabled setting leaves
the original note intact. File rollback does not change desktop settings.
To undo this opt-in, use the two recorded values, in this order:

```sh
gsettings set org.gnome.desktop.a11y.applications screen-reader-enabled false
busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status ScreenReaderEnabled b false
```

Replace each `false` if its recorded prior value was `true`. This setting
alone does not prove Discord composer acquisition, suggestions or acceptance;
those need a disposable private composer trial.

## Tests

```sh
npm run accessibility:check         # source tests, no desktop
npm run accessibility:integration   # private D-Bus and XDG environments
```

The source tests cover privacy gates, quiet facts, malformed frames, identity rules, epochs,
stale snapshots, bounded Unicode reads and window-text characters,
rich-editor flattening, calibration against recorded live extents, the
preview lifecycle, field events under the real operation deadline and the
lock query. The integration lane proves that other senders and fields cannot
deliver events and that invalidation unsubscribes the authorized field.

### Tested with

Live on the development workstation (Arch Linux, Hyprland 0.56) with
disposable text in isolated profiles; the
[Fcitx matrix](../fcitx5/README.md#tested-with) has suggestion and
acceptance results per app.

| What (installed commit, date) | Result | Boundary |
| --- | --- | --- |
| Window words, Chromium 154 (`26f4366`, 2026-10-20), a disposable mail-style page | The reply field read 385 characters of the thread; "Marin" finished as "Marinella" and "Okonk" as "Okonkwo"; a name only in the To input was never offered; a word learned in two note fields finished in the reply; `badi context site … off` stopped window text and learned words; the panel asked before context was chosen | Not a real mail client or the Claude desktop app |
| Window words, Zen 1.23.3b, Brave Origin 1.97 (tab and `--app`), VS Code 1.142 (`f180653`, 2026-10-20) | Zen read 197 characters and finished 3 of 3; Brave finished names from the thread; VS Code read 175 characters (explorer and tab labels) and finished names from its screen-reader textarea | Isolated profiles, one page or scratch file each |
| Preview renderer, GTK layer-shell fixture at scale 2 (2026-10-13) | Inline and field-edge overflow drawn; the overflow stayed wholly below the field, clear of its neighbouring button, and hid after its 15 s lease without renewal | The real renderer, not app editing or fractional scales |
