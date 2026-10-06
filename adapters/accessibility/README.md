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
| `chromium` | `/usr/lib/chromium/chromium` | `chromium` | browser origin |
| `chromium-browser` | as above, started without `/usr/bin/chromium`'s `CHROME_DESKTOP` | `chromium-browser` | browser origin |
| `brave-origin` | `/opt/brave-origin-bin/brave` | `brave-origin`, or an `--app` window `brave-<host>__<path>-<profile>` | browser origin |
| `zen` (Gecko) | `/opt/zen-browser-bin/zen-bin` | `zen` | browser origin |
| `chatgpt` (Codex) | `/usr/lib/chatgpt/ChatGPT` | `chatgpt` | desktop app |
| `code` (VS Code) | `/usr/share/code/code` | `code`, or `com.microsoft.VSCode` from 1.140 | desktop app |
| `cursor` | `/usr/lib/electron42/electron` running exactly `/usr/share/cursor/resources/app/cursor.mjs` | `cursor` | desktop app |
| `discord` | `$XDG_CONFIG_HOME/discord/app-<version>/Discord` | `discord` | desktop app |
| `grok-bot` | `/opt/Grok Bot/grok-bot` | `grok-bot` | desktop app |
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
  the snapshot starts at the AT-SPI `SENTENCE` boundary and must still equal it.
  The suggestion shows in the Fcitx panel.

Browser targets share one browser-origin identity (adapter `chromium`), so one
`badi site ... on` grant covers Chromium, Brave and Zen; the binding keeps the
exact app id. Electron apps render web content, so the HTML purpose gates apply
to them too. A missing tag or input type is ineligible, never guessed. Search,
email and URL inputs, combo boxes and ARIA search boxes (`xml-roles:
searchbox`) get no suggestions.

## Authority boundary

`inspect` verifies the unlocked Omarchy session, the active Hyprland window's
process, its identity rule and exactly one focused editable object. It queries
only that process and reads no prose. Every binding comes from an `inspect`;
`snapshot` and `preview` require its unchanged epoch.

Hyprland's active window and monitors come from its IPC socket. The lock check
runs `qs ipc -n -p $OMARCHY_PATH/shell call -- lock status` as a child process
while the field is read, joined at most 150 ms later. Unless all five lock flags
are explicitly false, the lock denial wins, and any error or timeout fails
closed.

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

## Socket contract

`python3 adapters/accessibility/daemon.py` serves
`$XDG_RUNTIME_DIR/badi/accessibility.sock` (socket `0600`, directory `0700`) to
same-user peers, one instance under an exclusive lock.

- Frames are newline-terminated JSON lines of at most 16 KiB (64 KiB buffered).
- Each operation has a 350 ms budget; AT-SPI calls time out after 50 ms and
  Hyprland calls after 150 ms.
- One connection owns acquisition and preview; another gets `observer_busy`.
- No typed text reaches logs or disk.

Requests carry `schema: "badi.accessibility.v1"`, an ASCII `id` and an `op`:

```json
{"schema":"badi.accessibility.v1","id":"a1","op":"inspect","app_id":"chromium"}
```

| Op | Request adds | Reply `focus` adds |
| --- | --- | --- |
| `inspect` | `app_id` | `binding` `{epoch,bus,path,process_id,app_id,uri}`, `target` (browser targets use `app_id: "chromium"` and an origin; `target_id` hashes bus, path and epoch), `purpose`, `caret`, `selection_count`, `geometry: null` |
| `snapshot` | `binding`, `policy_target` | `before`, `after`, `total_chars`, optional `scope: "block"` |
| `preview` | `binding`, `policy_target`, `expected_caret`, `expected_total_chars`, `text` (1–160 characters), `ttl_ms` (1–5000) | `total_chars`, `rendered` |

`preview` compares the current caret and length with the expected ones before
drawing. `hide` returns `hidden:true`; `status` returns readiness without
acquiring anything, and `health.py` probes it for the installer and `badi
doctor`. Readiness does not prove that any app exposes an eligible field.

Failures return `{schema,id,ok:false,error,epoch}` with a content-free code;
every failure except a malformed request or `observer_busy` hides the preview:

| Error | Cause |
| --- | --- |
| `invalid_request`, `invalid_preview` | Malformed envelope, binding or preview |
| `unsupported_app`, `app_mismatch` | No rule for the app id, or the active window is not that app |
| `desktop_unavailable`, `desktop_locked` | Hyprland or the lock query failed, or a lock flag is set |
| `focus_unavailable` | Not exactly one focused editable page field |
| `origin_unavailable`, `unsupported_origin` | No readable address, or not HTTP(S) |
| `sensitive_field`, `ineligible_field` | A password role or type; not focused, editable, showing, visible and enabled |
| `unsupported_field`, `selection_present`, `invalid_caret` | Another role or HTML purpose, browser UI or an unresolvable rich editor; a selection; a bad caret |
| `tree_limit`, `invalid_text` | A search passed 192 nodes; control characters in the text |
| `stale_binding`, `target_mismatch` | The field, epoch, position or text changed, or another policy target |
| `operation_timeout`, `observer_busy`, `accessibility_unavailable` | Budget exceeded; another owner; AT-SPI failed |

Unsolicited `{schema,event:"invalidate",epoch,reason,app_id}` messages name the
previous app; a client must drop its candidate and inspect afresh.

## Caret geometry and preview

Only `preview` calibrates, again each time, from the window frame F, the
nearest web document D, the field E and the glyph G before the caret. Any
inconsistency yields no geometry, and the addon uses the Fcitx panel.

- **Chromium and Electron** report D, E and G in physical pixels offset by F's
  origin times the scale s, where `D.width / F.width` must equal the monitor
  scale within 0.01. The logical caret is `x = (G.x + G.width - F.x*s)/s`,
  `y = (G.y - F.y*s)/s`, with line height `G.height/s`.
- **Caret-wide fields.** VS Code (with `editor.editContext` off) keeps its
  input textarea only as wide as the caret and places it right after G. A
  field at most two logical pixels wide whose left edge meets G's right edge
  on G's line marks the caret; its parent, the editor widget, must contain
  both and bounds the drawing.
- **Gecko** reports F, D and E in logical pixels and G in device pixels, with F
  at the window origin. The caret is `x = (G.x + G.width)/s`, `y = G.y/s`. A tile
  narrower than Zen's minimum content width fails and uses the panel.

Geometry is offered only for left-to-right text. The GTK4 layer-shell overlay
draws the suggestion as translucent grey text right after the caret, with no
background, at a size derived from the glyph height. It draws only suggestions
of at most 64 characters that fit inside the field, window and monitor;
otherwise it returns `rendered: false`. Qt fields (Omawrite, Telegram) report
no character extents and always use the panel. The surface takes no input or
focus, and any invalidation, new render or expiry hides it. The helper forces
software rendering and the simple GTK input module, so it never loads a GPU
driver or connects to an input method.

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

A file that already has the flag is left alone. A reduced mode or
`--disable-renderer-accessibility` is the user's choice and stops the installer
before it changes anything. Changed files are backed up, and running windows
keep their old flags until relaunched. Discord has no reliable mechanism: its
web client resets its `chromiumSwitches` list after one launch, so the
installer writes nothing for it.

## Tests

```sh
npm run accessibility:check         # source tests, no desktop
npm run accessibility:integration   # private D-Bus and XDG environments
```

The source tests cover privacy gates, malformed frames, identity rules, epochs,
stale snapshots, bounded Unicode reads, rich-editor flattening, calibration
against recorded live extents, the preview lifecycle and the lock query. The
integration lane proves that other senders and fields cannot deliver events and
that invalidation unsubscribes the authorized field.
