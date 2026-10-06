# Badi focused accessibility observer

This helper supplies **field identity and bounded context**, and an optional
caret preview, to the cooperative Fcitx addon. It never inserts, deletes, selects,
or types text and never changes accessibility settings or application policy.

## Application rules

Each supported Fcitx app id maps to one identity rule. The active Hyprland
window's process must belong to this user, match the executable rule and report
the listed window class. A matching rule is an acquisition eligibility check,
not a claim that an application's complete editing flow works; the
[coverage summary](../../README.md) lists what was tested live.

| App id | Executable rule | Window class | Broker target |
| --- | --- | --- | --- |
| `chromium` | `/usr/lib/chromium/chromium` | `chromium` | browser origin |
| `chromium-browser` | `/usr/lib/chromium/chromium` started without `/usr/bin/chromium`'s `CHROME_DESKTOP` | `chromium-browser` | browser origin |
| `brave-origin` | `/opt/brave-origin-bin/brave` (its Bash wrapper is only the parent) | `brave-origin`, or an `--app` window `brave-<host>__<path>-<profile>` | browser origin |
| `zen` (Gecko) | `/opt/zen-browser-bin/zen-bin` (`/usr/bin/zen-browser` only `exec`s it) | `zen` | browser origin |
| `chatgpt` (Codex desktop) | `/usr/lib/chatgpt/ChatGPT` | `chatgpt` | desktop app |
| `code` (VS Code) | `/usr/share/code/code` | `code`, or `com.microsoft.VSCode` from 1.140 | desktop app |
| `cursor` | `/usr/lib/electron42/electron`, whose first non-switch argument is exactly `/usr/share/cursor/resources/app/cursor.mjs` | `cursor` | desktop app |
| `discord` | `$XDG_CONFIG_HOME/discord/app-<version>/Discord` | `discord` | desktop app |
| `grok-bot` | `/opt/Grok Bot/grok-bot` | `grok-bot` | desktop app |
| `telegram` | `/usr/bin/Telegram` | `org.telegram.desktop` | desktop app |
| `omawrite` | `/usr/bin/omawrite` | `omawrite` | desktop app |
| `libreoffice` | `/usr/lib/libreoffice/program/soffice.bin` | `libreoffice-writer`, `paragraph` fields only | desktop app |

Executables come from `/proc/PID/exe` without further symlink resolution; a
deleted or replaced binary does not match.

- **Cursor** shares the Electron runtime, so its `/proc/PID/cmdline` (read up
  to 64 KiB) must name Cursor's entry the way Chromium parses switches.
  Electron `default_app` options before that entry that load other code
  (`--app=`, `-r`/`--require`) or run no app (`-i`/`--interactive`/`-repl`,
  `-v`/`--version`, `-a`/`--abi`) make the process unrecognized. A same-user
  process can rewrite its own command line; the rule separates apps and does not
  defend against same-user malware.
- **Discord** updates itself inside the user's configuration. The kernel path
  must be exactly `<realpath(config)>/discord/app-<digits and dots>/Discord`;
  `discord`, the version directory and the binary must be a real directory,
  directory and regular file owned by this user without group/other write
  permission, and the file must be the inode the process runs. Symlinked
  components, other owners, traversal spellings and `(deleted)` fail.
- **Brave web apps** (Omarchy's HEY, X, Basecamp, Zoom) are Brave Origin
  `--app` windows named `brave-<host>__<path>-<profile>`. The addon maps that
  program id to `brave-origin` before folding, since host labels may start
  with a digit, and the rule accepts such a class (printable ASCII, at most 255
  bytes) for the same executable. The page origin selects policy as in a tab.
- **LibreOffice** reports one Fcitx program id for every module, so the rule
  admits only `libreoffice-writer` windows and `paragraph` fields; Calc,
  Impress and dialog entries fail closed. Writer gives Fcitx only the caret's
  sentence as surrounding text, so the snapshot starts at that sentence's AT-SPI
  `SENTENCE` boundary and must still equal it exactly. Its frame has no web
  document, so the suggestion shows in the Fcitx panel.

Browser targets use the broker's single browser-origin identity (adapter
`chromium`), so one `badi site ... on` grant covers Chromium, Brave and Zen
alike; the binding keeps the exact app id. Other apps are `desktop_application`
targets granted with `badi app APP_ID on`. Electron apps render web content, so
the HTML purpose gates (tag and input type) apply to them as to browsers. A
missing tag or input type is ineligible, never guessed. Search, email and URL
inputs, combo boxes and ARIA search boxes (`xml-roles: searchbox`, which
accessibility reports as a plain entry) get no suggestions.

## Authority boundary

`inspect` verifies the unlocked Omarchy session, the current Hyprland window
PID, the identity rule above and exactly one focused editable accessibility
object, queries only that process, and reads no field prose. Every binding comes
from an `inspect`; `snapshot` and `preview` require its unchanged epoch and do
not query the lock again.

Hyprland's active window and monitors come from its IPC socket
(`$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock`). The lock
check is the Quickshell IPC call behind `omarchy-shell lock status`, made
directly (`qs ipc -n -p $OMARCHY_PATH/shell call -- lock status`) as a child
process that runs while the field is read. `inspect` answers only after joining
it, at most 150 ms after the field read and within the operation budget; a late
child is killed, and every child is reaped. Unless all five lock flags are
explicitly false, the lock denial replaces any field result or error. A missing
`OMARCHY_PATH`, an IPC error reply, a timeout or an unparsable answer fails
closed, and no lock state is kept between operations.

### Browser fields

Browser fields need an HTTP(S) address: Chromium's Document `URI`, Gecko's
`DocURL`. Cross-origin frames keep their own.

Chromium 152 reports `FOCUSED` on several fields at once: the page field and the
omnibox popup's WebUI combo box (`chrome://omnibox-popup.top-chrome/`) while the
page has the keyboard, the page field and the omnibox entry while the omnibox
has it. Browsers therefore ignore browser UI (nodes with no document ancestor,
like the omnibox, and `chrome://*.top-chrome` WebUI documents), and exactly one
field with an HTTP(S) origin must remain. Any other document counts as page
content, including DevTools, `chrome://` and extension pages, `about:blank` and
unreadable URIs; two such fields, or none, fail closed. The stale page field
chosen while the omnibox has the keyboard never receives text: the omnibox's
Fcitx context carries the `Url` purpose, which the addon's
`allowsNativeContext()` denies before any inspection, and every display and
dispatch requires the snapshot around the caret to equal Fcitx's live
surrounding text.

Gecko (Zen 1.22.3b and 1.23b, Gecko 156 and 157) exposes web content without a
flag. Its Documents carry `DocURL` and an empty `URI`; the browser window (role
`frame`) is itself a Document, `chrome://browser/content/browser.xhtml`, and
every node reports the window's process. Exactly one node is focused and
editable: the urlbar (role `combo box`, no `document web` ancestor) while it has
the keyboard, otherwise the page field. The urlbar shares the page's Fcitx
context without a `Url` purpose, so the observer, not the addon, denies it: the
one focused editable node's nearest Document must be a `document web` with an
HTTP(S) `DocURL`. The urlbar and other browser UI, and `about:`,
`moz-extension:`, `resource:`, `file:` or empty addresses, fail as
`unsupported_field` before any text read; a second focused field fails as
`focus_unavailable`. Only the nearest Document counts, so an `about:blank` or
loading frame never borrows its parent page's origin.

Zen page fields are role `entry` with tag `textarea`, `input` plus
`text-input-type=text`, or `div` for a `role=textbox` contenteditable. Gecko
sets `text-input-type` only from an explicit `type` attribute, so an `<input>`
without one is ineligible, and a new document's first attribute query can lack
`tag` until the next input re-inspects it. Gecko's IME surrounding text is only
the caret's paragraph (`IMContextWrapper::GetCurrentParagraph`), while a
snapshot spans up to 512 characters across lines. Once a newline precedes the
caret, or the caret ends a paragraph with more text after it, the Fcitx
agreement check fails closed as `observer_context_mismatch`.

### Rich editors

A rich editor root (Codex's ProseMirror composer, Lexical, Quill, Draft.js,
Slate, CKEditor 5, TinyMCE, CodeMirror in contenteditable mode, or a
contenteditable of `<p>` or `<div>` lines) is one focused editable whose text is
one U+FFFC embedded object per block, and whose caret is the offset of the block
holding the caret. The root is an `entry` (`role=textbox`) or, without that
role, a `section` whose tag is `div` (Lexical, Quill) or an editing iframe's
`body` (TinyMCE). Each block's caret is -1 except the caret's block.

For Chromium and Electron web content (never Gecko or native toolkits), the
root is flattened the way Chromium serializes text-input surrounding text when
anything is rendered after the editor: a `paragraph` (`<p>`) or `heading` ends
with `"\n\n"`, a `section` (`<div>` line) with `"\n"`, and a block whose own
text already ends in a line break (an empty line, a trailing `<br>`, Slate's
U+FEFF line) with one break fewer. Inline objects inside a block (`<strong>`,
`<em>`, `<code>`, `<span>`, links, `<mark>`, `<sub>`, `<sup>`; roles `static`,
`link`, `mark`, `subscript`, `superscript`) are written as their own text. Any
other block (a list, quote, table, image or figure) is opaque.

`inspect` reads only roles, link offsets, lengths, caret offsets and selection
counts, so `caret`, `total_chars` and `character_offset` count each block's raw
text and full end. `snapshot`, after the policy grant, reads the blocks in its
window one character per block wider than 512/128 (Chromium can drop one break
per block), rereads them for the stale check, serializes them as above and
keeps the last 512 characters before and 128 after the caret. When an opaque
block overlaps that window, the snapshot covers only the caret's own block and
says `scope: "block"`; the addon then requires Fcitx's text to end with exactly
that block from a line start. Geometry and direction come from the caret
block's glyph; a caret at a block's start has none, so the addon uses the Fcitx
panel.

Anything unresolvable fails closed before prose:

- `unsupported_field`: text directly in the root, more than 64 blocks, a
  foreign-parent block, a caret in an opaque block (inside a list or quote), or
  a caret block whose embedded objects are not inline text (an image).
- `invalid_caret`: an ambiguous or out-of-range caret.
- `selection_present`: a selection in any block.

`snapshot` also fails as `unsupported_field` on a U+FFFC that is not one of the
block's inline objects. An `about:blank` or `about:srcdoc` frame has its
creator's origin, so a TinyMCE or srcdoc field takes its enclosing page's URI.
Editors on Chromium's EditContext API (CodeMirror on recent Chromium, VS Code's
default) expose no editable DOM node and stay unavailable.

Live on 2026-10-06 in Chromium 152, both in an empty editor and in a second
line under an existing one: textarea, text input, contenteditable plain, `<p>`,
`<div>` and `<br>` lines, Quill, CKEditor 5, Draft.js, Slate, CodeMirror,
TinyMCE, an input in a shadow root and a textarea in an iframe; and on the
official demo pages of Lexical, Slate, CKEditor 5, ProseMirror and TinyMCE,
each with one exact append.

## Snapshots and field events

`snapshot` requires the exact prior bus name, object path, process, application,
URI, epoch and broker-policy target, and its trusted Fcitx caller must already
hold a current context-read grant for that target. Password or sensitive,
noneditable, invisible, selected and unsupported fields and stale bindings fail
before `Text.GetText`. Reads are at most 512 Unicode characters before the caret
and 128 after it; the field's caret, length, selections and bounded text are
reread to detect changes. Neither the accessible object nor this helper provides
an atomic edit transaction, so Fcitx still compares its exact live surrounding
text, caret, focus epoch and one-shot commit authority before dispatch.

Accessibility events and compositor focus or window changes invalidate the
current epoch and clear the preview. The only global listener is for focus
changes, which invalidate without querying their source. After an approved
snapshot, the field's own events (text, caret, selection, editable, showing,
defunct and role changes) are subscribed on a separate D-Bus connection, matched
to the exact unique sender and object path, so other fields' events never
arrive. Any of them invalidates, and invalidation or disconnect removes the
match; payloads, including typed text, are never unpacked. The application is
asked to produce these events (`RegisterEvent` for its bus name) after the first
grant in it, until the active window or armed application changes or the helper
exits. Notifications carry no event text, document labels or URIs. Browser
origin policy stays in the broker; this helper grants no blanket browser access.

## Socket contract

Run it with the system Python that supplies PyGObject and the Atspi typelib:
`python3 adapters/accessibility/daemon.py`.

- The endpoint is `$XDG_RUNTIME_DIR/badi/accessibility.sock` (socket `0600`,
  directory `0700`), for same-user peers only, with one instance under an
  exclusive lock. A stale socket is removed only after the lock and a refused
  connection prove that no observer is running.
- Frames are newline-terminated JSON lines of at most 16 KiB, with 64 KiB
  buffered per connection; coalesced frames and partial tails are supported.
  One operation runs per GLib dispatch, so authority events can invalidate
  between queued requests.
- Each operation has a 350 ms budget. AT-SPI calls time out after 50 ms,
  Hyprland IPC calls after at most 150 ms, and the lock query at most 150 ms
  after the field read.
- One connection owns acquisition and preview. A second acquisition client
  receives `observer_busy`, and its disconnect cannot alter the owner's state.
  Queued callbacks bind to a connection object, so a reused file descriptor
  inherits no earlier connection's work.
- No typed text is written to logs or disk.

Every line is a JSON object with `schema: "badi.accessibility.v1"`. Requests have
an ASCII `id` of 1–64 letters/digits/underscore/dot/hyphen:

```json
{"schema":"badi.accessibility.v1","id":"a1","op":"inspect","app_id":"chromium"}
```

Success returns `{schema,id,ok:true,focus}`, where `focus` contains:

- `binding`: exactly `{epoch,bus,path,process_id,app_id,uri}`.
- `target`: the broker target descriptor, including the canonical origin for a
  browser. Browser targets use `app_id: "chromium"`, while the binding keeps the
  exact verified app id. `target_id` hashes bus/path/epoch.
- `purpose: "plain_text"`, `caret`, `selection_count`, and `geometry`.

`snapshot` requests repeat `binding` and `policy_target` verbatim; replies add
`before`, `after` and `total_chars` **inside `focus`**.

`preview` requests repeat `binding` and `policy_target` and add the last
verified snapshot's `expected_caret` and `expected_total_chars` (required
integers, `0 <= expected_caret <= expected_total_chars <= 2147483647`), `text`
(1–160 characters) and `ttl_ms` (1–5000). The helper compares the current
eligible metadata with both positions **before calling the renderer**, without
acquiring prose, so a missed event or repeated bounded text cannot hide a
changed caret or document length. Replies add `focus.total_chars` and
`focus.rendered`, whether the GTK preview drew. Fcitx verifies the reply's
identity, caret and length before making acceptance available.

`hide` takes only schema/id/op and returns
`{schema,id,ok:true,hidden:true,epoch}`. `status` takes the same and returns
`{schema,id,ok:true,status:{ready:true,process_id,protocol_version:1}}`; it
acquires nothing, receives no field events, and its client's disconnect does not
hide or disarm the owner's preview. The stdlib-only `health.py` probe verifies
the private socket, exact service PID/UID, bounded reply and unchanged socket
identity; the installer and `badi doctor` also check systemd service identity.
Readiness does not prove that accessibility is enabled or that any app exposes
an eligible field.

Failures return `{schema,id,ok:false,error,epoch}` with a fixed metadata-only
code. A malformed request or `observer_busy` leaves field state unchanged;
every other failure clears the preview and the field's event match.

| Error | Cause |
| --- | --- |
| `invalid_request`, `invalid_preview` | Malformed envelope, app id, binding, or preview text, lifetime or position |
| `unsupported_app` | No identity rule for the app id |
| `app_mismatch` | The active window is not a mapped, visible window of this user's exact app |
| `desktop_unavailable` | Hyprland IPC or the lock query failed, timed out or answered unexpectedly |
| `desktop_locked` | A lock flag is not explicitly false |
| `focus_unavailable` | No active window, or not exactly one focused editable page field in its process |
| `origin_unavailable`, `unsupported_origin` | No readable document address, or not an HTTP(S) origin |
| `sensitive_field` | A password role or password `text-input-type`, before any caret, extent or text call |
| `ineligible_field` | Not focused, editable, showing, visible and enabled |
| `unsupported_field` | No Text interface, another role or HTML purpose, Gecko browser UI, or an unresolvable rich editor |
| `selection_present`, `invalid_caret` | A selection, or an out-of-range or ambiguous caret |
| `tree_limit` | A search without the Collection interface passed 192 nodes |
| `stale_binding`, `target_mismatch` | The field, epoch, position or text changed, or another policy target |
| `invalid_text` | Control characters other than tab and line breaks in the bounded text |
| `operation_timeout` | The 350 ms operation budget ran out |
| `observer_busy` | Another connection owns acquisition and preview |
| `accessibility_unavailable` | AT-SPI or the field event connection failed |

Unsolicited `{schema,event:"invalidate",epoch,reason,app_id}` messages name the
previously tracked application. `reason` is `focus_changed`, `field_changed`,
`window_changed`, `client_disconnected`, `operation_timeout`,
`field_unavailable`, `accessibility_unavailable` or `desktop_unavailable` (the
Hyprland event stream ended and the helper exits). A client must clear any
candidate and inspect afresh after invalidation or transport loss.

## Caret geometry and inline preview

Only `preview` calibrates, so `inspect` and `snapshot` return `geometry: null`.
Each `preview` calibrates again; nothing is cached across requests or tied to a
Chromium version. The helper reads the outermost `frame` F, the field's nearest
`document web` D, the field E and the glyph G before the caret in `SCREEN`
coordinates. Any inconsistency yields no geometry, and the addon uses the Fcitx
panel.

Chromium and Electron report D, E and G in physical pixels offset by F's origin
times the scale, while F is logical; Electron's arbitrary frame origin cancels.
Chromium's `WINDOW` coordinates are relative to the web viewport and are not
used. `D.width / F.width` must equal the Hyprland monitor scale s within 0.01.
The window-local logical caret is then `x = (G.x + G.width - F.x*s)/s`,
`y = (G.y - F.y*s)/s`, with line height `G.height/s`. G outside E, E outside D,
a caret outside the window, XWayland, a transformed or ambiguous monitor and
zero extents are rejected. This matched each app's own caret within 0.5 logical
pixels for Chromium at scales 1 and 2 and for Brave and VS Code at scale 2;
fractional scales, iframes and other Electron apps are untested.

Gecko's `D.width / F.width` is not the scale (671/687 at scale 2), so it has its
own rule. In Zen, `SCREEN` and `WINDOW` extents are identical and relative to
the window. F sits at (0, 0), and F, D and E are logical pixels (Gecko divides
component extents by the window's scale factor, not the page zoom), while G
stays in device pixels. The helper requires F at the origin and as wide as the
Hyprland window, D inside F, E inside D, and G/s inside E; the caret is then
`x = (G.x + G.width)/s`, `y = G.y/s`, with line height `G.height/s`. Gecko's
surface can be taller than the tile Hyprland shows, so the caret must fit
Hyprland's window size, never F's height. At scale 2 the prediction was within
0.3 logical pixels of the page's own caret; scale 1 is the identity, and other
scales use the Fcitx panel. A tile narrower than Zen's minimum content width
(F 500 wide in a 341-pixel tile) fails the width rule and also uses the panel.

Preview geometry carries `caret_edge: "right"` only when the last approved
snapshot's caret run is left-to-right and its line before the caret holds
left-to-right text and no right-to-left characters. The run direction is the
`direction` text attribute or, where the toolkit exposes none (Gecko), the order
of the two glyphs before the caret on one line.

The GTK4 layer-shell overlay then draws the suggestion as translucent grey text
right after the caret, with no background, its font size derived from the glyph
height and its baseline aligned with the glyph box. It renders only
left-to-right suggestions of at most 64 characters that end inside the field,
window and monitor; otherwise it returns `rendered: false` and the addon uses
the Fcitx panel. Qt fields, including Omawrite and Telegram, report zero
character extents and always use the panel. The surface uses the overlay layer,
no keyboard interactivity, an empty input region and no focus. Any invalidation,
new render or expiry hides it.

The helper sets `GSK_RENDERER=cairo`, `GDK_DISABLE=gl,vulkan` and
`GTK_IM_MODULE=gtk-im-context-simple` before GTK loads, so the label loads no GPU
driver and the helper never connects to an input method. It builds the preview
and lays out sample text once from an idle callback after startup, so font setup
does not count against the first preview's deadline. If the preview cannot be
built (no layer-shell support), it returns `rendered: false` until restart.

## Limits

Observation never grants an atomic editor operation, and AT-SPI cannot supply
one (Chromium exposes no `EditableText`), so native replacement stays disabled
and correction stays editor-owned. The observed apps accept through one
append-only Fcitx `commitString` that behaves like typed text, without exact
undo or verified field authority
([IME-parity decision](../../docs/decisions/0003-ime-parity-append-only.md)).
This helper's share is exact identity, snapshot/caret agreement before display
and dispatch, and fail-closed purpose and sensitivity gates.

## Installation

Full desktop installation copies the helper and its service, preserves an
existing autostart choice, and waits for an exact-process health reply before
restarting Fcitx. On Fcitx 5.1.22 it also selects the pinned
[compatibility frontend](../../packaging/fcitx5-wayland-compat/README.md), without
which Chromium-based text-input-v3 clients stop publishing surrounding text.
`--broker-only` leaves this helper and the frontend untouched. VS Code publishes
its editor's surrounding text only with `"editor.editContext": false` in its user
settings; `badi doctor` names this fix while the setting is missing (a blank file
counts as `{}`, as in VS Code).

The system accessibility bus must be enabled. Enabling its `IsEnabled` property
can persist the GNOME toolkit-accessibility setting, so treat it as a desktop
setting with a recorded prior value, not a process-local toggle.

### Renderer accessibility flag

Chromium exposes no web content over AT-SPI unless it starts with
`--force-renderer-accessibility`, even with `org.a11y.Status` `IsEnabled` and
GNOME toolkit accessibility both true. Gecko needs no flag. Measured on Chromium
152.0.7977.82 with a local textarea:

| Mode | Observer result |
| --- | --- |
| no flag | no web nodes; `focus_unavailable` |
| `=basic`, `=form-controls` | URI, caret and text are available, but the HTML `tag` is empty and there are no character extents, so the purpose gate rejects the field and there is no calibrated caret |
| bare switch, `=complete` | tag, URI, caret, bounded text and calibrated geometry; inspection took 32–43 ms of the 350 ms budget |

The lightest working mode is therefore `complete` (the bare switch is the same
mode). It builds the full accessibility tree for every page, which costs some
browser CPU and memory, most visibly on large pages, while the app runs.
Chromium already gives Fcitx a native Wayland text-input context with
surrounding text, so no input-method flag is needed.

`install-desktop.py --observed-app APP` (repeatable) appends only
`--force-renderer-accessibility=complete`, with a comment, to that launcher's
user flags file under `$XDG_CONFIG_HOME`, parsed the way its wrapper reads it:

| App | File | Wrapper parsing |
| --- | --- | --- |
| `chromium` | `chromium-flags.conf` | `/usr/bin/chromium`: GLib shell quoting, `#` comments |
| `brave-origin` | `brave-origin-flags.conf` | `/usr/bin/brave-origin`: each non-comment line is one argument |
| `chatgpt` | `codex-flags.conf` | `/usr/bin/chatgpt`: text after `#` removed, split on whitespace |
| `code` | `code-flags.conf` | `/usr/bin/code`: text after `#` removed, split on whitespace |
| `cursor` | `cursor-flags.conf` | `/usr/share/cursor/cursor`: each non-comment line is one argument, after Cursor's entry |
| `grok-bot` | `grok-bot-flags.conf` | `/usr/bin/grok-bot`: text after `#` removed, split on whitespace |

A file that already has the bare or `=complete` switch is left untouched. Any
other value, or `--disable-renderer-accessibility`, stops the installer before
it builds or changes anything, because a reduced mode is the user's choice.
Changed files keep their mode (new files are `0600`), are backed up, listed in
the desktop backup's `changes.json` and recorded in the install receipt.
Running app windows keep their old flags until relaunched.

Discord has no supported mechanism: its updater drops command-line flags and
environment, and `/usr/bin/discord` reads no flags file. Its startup code does
append every unlisted name in `settings.json` `chromiumSwitches` as a bare
switch, and with `force-renderer-accessibility` listed Discord 1.0.159 exposed
its web tree over AT-SPI (2026-10-06). The setting does not persist: about six
seconds after start the web client replaced the list through the desktop
core's `setChromiumSwitches`, so the next launch, including Discord's own
relaunch after an update, starts without it. The installer therefore writes
nothing for Discord.

## Tests

```sh
npm run accessibility:check
npm run accessibility:integration
```

`accessibility:check` runs the source tests without a desktop: privacy gates,
malformed frames, identity rules, epoch changes, stale snapshots, bounded
Unicode reads, rich-editor flattening, calibration against recorded live
extents, the preview lifecycle, and the lock query's child lifecycle against a
task-owned `qs`. `accessibility:integration` creates private D-Bus and XDG
environments, never connects to the desktop's accessibility bus, and uses
task-created emitters to prove that unrelated senders and fields cannot deliver
events and that invalidation unsubscribes the authorized field. The
[Fcitx addon's](../fcitx5/README.md) private-socket and actual-addon cases
prove that missed caret and length changes cannot reach the renderer, even with
identical bounded text.
