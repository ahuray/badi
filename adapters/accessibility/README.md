# Badi focused accessibility observer

This helper supplies **field identity and bounded context**, and an optional
caret preview, to the cooperative Fcitx addon. It never inserts, deletes, selects,
or types text and never changes accessibility settings or application policy.

Each supported Fcitx app id maps to one identity rule. The active Hyprland
window's process must belong to this user, match the executable rule and report
the listed window class. Rules were inspected on this workstation; a matching
rule is an acquisition eligibility check, not a claim that an application's
complete editing flow works.

| App id | Executable rule | Window class | Broker target |
| --- | --- | --- | --- |
| `chromium` | `/usr/lib/chromium/chromium` | `chromium` | browser origin |
| `chromium-browser` | `/usr/lib/chromium/chromium` started without `/usr/bin/chromium`'s `CHROME_DESKTOP` | `chromium-browser` | browser origin |
| `brave-origin` | `/opt/brave-origin-bin/brave` (its Bash wrapper is only the parent) | `brave-origin` | browser origin |
| `zen` (Gecko) | `/opt/zen-browser-bin/zen-bin` (`/usr/bin/zen-browser` only `exec`s it) | `zen` | browser origin |
| `chatgpt` (Codex desktop) | `/usr/lib/chatgpt/ChatGPT` | `chatgpt` | desktop app |
| `code` (VS Code) | `/usr/share/code/code` | `code` | desktop app |
| `cursor` | `/usr/lib/electron42/electron`, whose first non-switch argument is exactly `/usr/share/cursor/resources/app/cursor.mjs` | `cursor` | desktop app |
| `discord` | `$XDG_CONFIG_HOME/discord/app-<version>/Discord` | `discord` | desktop app |
| `telegram` | `/usr/bin/Telegram` | `org.telegram.desktop` | desktop app |
| `omawrite` | `/usr/bin/omawrite` | `omawrite` | desktop app |

Executables come from `/proc/PID/exe` without further symlink resolution; a
deleted or replaced binary does not match. Cursor shares the Electron runtime,
so its `/proc/PID/cmdline` (read up to 64 KiB) must name Cursor's entry the way
Chromium parses switches. Electron's `default_app` options that load other code
(`--app=`, `-r`/`--require`) or run no app (`-i`/`--interactive`/`-repl`,
`-v`/`--version`, `-a`/`--abi`) before that entry make the process
unrecognized. A same-user process can rewrite its own command line; the rule
separates apps and does not defend against same-user malware.
Discord updates itself inside the user's configuration. The kernel path must be
`<realpath(config)>/discord/app-<digits and dots>/Discord` in normalized form.
`discord`, the version directory and the binary must be a real directory,
directory and regular file owned by this user, without group/other write
permission. The file must also be the inode the process is running.
Symlinked components, other owners, traversal spellings, `(deleted)` and other
names fail.

Browser targets use the broker's single browser-origin identity (adapter
`chromium`), so one `badi site ... on` grant covers Chromium, Brave and Zen
alike; the binding keeps the exact app id. Other apps are `desktop_application`
targets granted with `badi app APP_ID on`. Electron apps render web content, so
the HTML purpose gates (tag and input type) apply to them as well as to
browsers. A missing tag or input type is ineligible, never guessed.

## Authority boundary

`inspect` verifies the unlocked Omarchy session, current Hyprland window PID,
the identity rule above, and exactly one focused editable accessibility object.
Only that process is queried. Every binding comes from an `inspect`; `snapshot`
and `preview` require its unchanged epoch and do not query the lock again.
Hyprland's active window and monitors are read over its IPC socket
(`$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock`) within the
operation budget; only the lock check starts a process. It makes the Quickshell
IPC call behind `omarchy-shell lock status` directly (`qs ipc -n -p
$OMARCHY_PATH/shell call -- lock status`) as a child process that runs while the
field is read, and `inspect` answers only after joining it: at most 150 ms after
the field read, within the operation budget. A late child is killed, and every
child is reaped. The query denies unless all five lock flags are explicitly
false, and that denial replaces any field result or error; a missing
`OMARCHY_PATH`, an IPC error reply, a timeout or any unparsable answer fails
closed. No lock state is kept between operations. Chromium browser nodes must supply a valid
HTTP(S) Document `URI`, Gecko ones a `DocURL` (below); cross-origin frames
retain their own address. Desktop apps need
exactly one focused editable node. The helper reads no field prose during
inspection.

Chromium 152 reports `FOCUSED` on several fields at once, so AT-SPI cannot say
which field receives keys (live, 2026-09-26). With the page focused, the
omnibox popup's WebUI combo box (`chrome://omnibox-popup.top-chrome/`) also
reports focused, editable and showing. With the omnibox focused, the omnibox
entry and the page field both do. Browsers therefore ignore browser UI: nodes
with no document ancestor (the omnibox) and `chrome://*.top-chrome` WebUI
documents. Exactly one field must remain, and it must have an HTTP(S) origin.
Any other document counts as page content, including DevTools, `chrome://`
pages, extension pages, `about:blank` and unreadable URIs. Two such fields, or
none, fail closed. Selecting the stale page field while the omnibox is being
typed into is safe for two reasons. First, the omnibox's Fcitx input context
carries the `Url` purpose (capabilities `0x1072` against `0x80072` for the page
textarea), which the addon's `allowsNativeContext()` denies before any
inspection. One disposable character typed there produced `field_denied`
contexts, one `ime_parity_unobserved` block and no request. Second, every
display and dispatch requires the snapshot's text around the caret to equal
Fcitx's live surrounding text for the field that really has the keyboard.
With its page focused, Brave 1.96 reported only the page field; its omnibox was
not measured.

Zen 1.22.3b (Gecko 156.0.1) differs, as a live probe with disposable text
showed on 2026-09-26. Gecko exposes web content with no flag. Its Document attributes
are exactly `DocURL` and `MimeType`, and `URI` is empty. The browser window
(role `frame`) is itself a Document, `chrome://browser/content/browser.xhtml`.
Every node reports the parent process, which is also the Hyprland window's PID.
Exactly one node was focused and editable in every state: the urlbar while it
had the keyboard (role `combo box`, no `document web` ancestor), otherwise the
page field. The urlbar shares the page's Fcitx input context and has no `Url`
purpose (capabilities `0x72` for both), so the addon cannot deny it. For Gecko
the helper therefore filters nothing. Exactly one focused editable node must
exist, and its nearest Document must be a `document web` with an HTTP(S)
`DocURL`. A second focused field fails closed as `focus_unavailable`. The
urlbar and other browser UI, and `about:`, `moz-extension:`, `resource:`,
`file:` or empty addresses, fail as `unsupported_field` before any text read.
Only the nearest Document counts, so an `about:blank` or loading frame never
borrows its parent page's origin. Page fields were role `entry`, with `tag`
`textarea`, `input` plus `text-input-type=text`, or `div` for a
`role=textbox` contenteditable. The password was role `password text` with
`text-input-type=password`, and Fcitx also marked it Password and Sensitive.
The first attribute query on a new document once lacked `tag`; that field is
ineligible until the next input re-inspects it. A contenteditable without
`role=textbox` and email, URL, telephone, number and search inputs were not
probed; the purpose gate rejects the latter. Gecko sets `text-input-type` only
from an explicit `type` attribute (`HTMLTextFieldAccessible::NativeAttributes`
and its remote cache, Firefox source 2026-09), so an `<input>` without one is
always ineligible in Zen; the gate cannot tell it from an unexposed purpose.
Gecko's IME surrounding text is only the caret's paragraph
(`IMContextWrapper::GetCurrentParagraph`), while a snapshot spans up to 512
characters across lines. Once a newline precedes the caret, or the caret ends a
paragraph with more text after it, the Fcitx agreement check fails closed as
`observer_context_mismatch`.

### Rich editors

Codex desktop's composer (Chromium 153, `chatgpt`, live 2026-09-27) is a
ProseMirror contenteditable. The one focused editable node is role `entry`, tag
`div`, `xml-roles=textbox`. Its text is one U+FFFC embedded object per
paragraph, and its caret is the offset of the paragraph that holds the caret.
Each `paragraph` child holds the typed text and the caret (the others report
-1). Empty, the paragraph's text is `"\n"` plus a U+FFFC for the placeholder's
`::after` pseudo-element; typed, it was exactly the phrase with no trailing
break.

For Chromium and Electron web content (never Gecko or native toolkits), a root
with embedded objects is flattened the way Chromium 152 serializes text-input
surrounding text (`WAYLAND_DEBUG` `set_surrounding_text`, 2026-09-27): each
paragraph's text followed by `"\n\n"`, margins or not, the last paragraph
included. `caret` and `total_chars` are in those flattened coordinates.
`inspect` reads only link offsets, roles, lengths, caret offsets and selection
counts, never prose. `snapshot` reads only the paragraphs that overlap its
512/128-character window, and rereads them for the stale check. Geometry and
direction come from the caret paragraph's glyph, and `character_offset` still
names the flattened caret. A caret at a paragraph's start has no glyph, so the
addon uses the Fcitx panel.

Anything unresolvable fails closed before prose:

- `unsupported_field`: text directly in the root, more than 64 blocks, a
  non-`paragraph` or foreign-parent block, or a paragraph with its own embedded
  objects (placeholder, image, mention, link, code span).
- `invalid_caret`: an ambiguous or out-of-range caret.
- `selection_present`: a selection in any paragraph.

Two cases fail closed at `snapshot`, as `unsupported_field`. A U+FFFC read as
text is one. The other is a paragraph that ends in a line break (an empty
ProseMirror paragraph, or a trailing hard break) whose end is in the window:
Chromium adds only one `"\n"` after it, which these lengths cannot express
without reading prose at inspection. The single-paragraph Codex shape was observed live, but not
with this build: its window was closed before the acceptance run. An isolated
Chromium 152 fixture with the same `<div role=textbox><p>` shape and this build
displayed, accepted and dismissed suggestions. Multi-paragraph rules come from
the recorded serialization and fakes.

`snapshot` requires the exact prior object bus name/path, process, application,
URI, epoch and broker-policy target. Its trusted Fcitx caller must already hold a
current broker context-read grant for that target. Password/sensitive,
noneditable, invisible, selected, unsupported field purposes and stale bindings
fail before `Text.GetText`. Reads are at most 512 Unicode characters before the
caret and 128 after it; the same field's caret, length and selections and the
bounded text are reread to detect changes. Neither the accessible object nor this helper provides an atomic edit
transaction. Fcitx must still compare its exact live surrounding text, caret,
focus epoch and one-shot commit authority before dispatch.

Accessibility events and compositor focus/window changes invalidate the current
epoch and clear the preview. The only global listener is for focus changes; any
focus change invalidates without querying its source. After an approved
snapshot, the field's own events (text, caret, selection, editable, showing,
defunct and role changes) are subscribed on a separate D-Bus connection with a
match restricted to the exact unique sender and field object path, so events
from other fields or a busy page never arrive. Any of them invalidates, and
invalidation or disconnect removes that match; event payloads, including typed
text, are never unpacked. The application is asked to produce these events
(`RegisterEvent` for its bus name) after the first grant in it; that interest
stays while its window stays active and is withdrawn when the active window or
armed application changes, or the helper exits. Notifications do not retain
event text, document labels, or URIs. Browser origin policy stays centralized in
the broker; this helper does not grant blanket browser access.

## Socket contract

Run with the system Python that supplies PyGObject and the Atspi typelib:

```sh
python3 adapters/accessibility/daemon.py
npm run accessibility:check
npm run accessibility:integration
```

The default endpoint is `$XDG_RUNTIME_DIR/badi/accessibility.sock`, with socket
mode `0600` and directory mode `0700`. A same-user peer check, exclusive instance
lock, bounded frames, and fixed error codes apply. Stale owned sockets are
recovered only after the exclusive lock and a refused connection verify there
is no live observer. No typed text is written to logs or disk. Coalesced frames and partial tails are supported, with at most 64 KiB buffered
per connection and 16 KiB per frame including the newline. One operation runs per
GLib dispatch so authority events can invalidate between queued requests.
Each operation has a 350 ms overall budget; AT-SPI calls have 50 ms timeouts and
Hyprland IPC and the lock check at most 150 ms.
Only one connection owns acquisition and preview authority. A second acquisition
client receives `observer_busy`; its disconnect cannot alter the owner's state.
Queued callbacks bind to a connection object, preventing reuse of a file
descriptor from inheriting an earlier connection's work.

Every line is a JSON object with `schema: "badi.accessibility.v1"`. Requests have
an ASCII `id` of 1–64 letters/digits/underscore/dot/hyphen:

```json
{"schema":"badi.accessibility.v1","id":"a1","op":"inspect","app_id":"chromium"}
```

Success returns `{schema,id,ok:true,focus}`. The `focus` object contains:

- `binding`: exactly `{epoch,bus,path,process_id,app_id,uri}`.
- `target`: the broker target descriptor, including the canonical origin for a
  browser. Browser targets use `app_id: "chromium"`; binding identity retains the
  exact verified incoming native program. `target_id` hashes bus/path/epoch.
- `purpose: "plain_text"`, `caret`, `selection_count`, and optional `geometry`.

`snapshot` requests repeat `binding` and `policy_target` verbatim. Successful
responses add `before`, `after`, and `total_chars` **inside `focus`**.

`preview` requests repeat `binding` and `policy_target`, adding the last verified
snapshot's `expected_caret` and `expected_total_chars`, plus `text` (1–160
characters) and `ttl_ms` (1–5000). Both position fields are required integers,
with `0 <= expected_caret <= expected_total_chars <= 2147483647`; there is no
unbound preview request. The helper compares current eligible metadata with both
snapshot values **before calling the renderer**, without acquiring prose. A
missed event or repeated bounded text cannot hide a changed caret or document
length. Success includes `focus.total_chars` and `focus.rendered`; the latter
reports whether the optional GTK preview rendered. Fcitx verifies the reply's
identity, caret and document length before making acceptance available.
Unsupported coordinate conventions or unavailable GTK4LayerShell return false.
`hide` requires only schema/id/op and returns `{schema,id,ok:true,hidden:true,epoch}`.
Invalidation, client disconnect and expiry clear the preview. Display uses plain
text and never accepts pointer/keyboard input. Only `preview` calibrates
geometry, so `inspect` and `snapshot` return `geometry: null`.

`status` takes only schema/id/op and returns
`{schema,id,ok:true,status:{ready:true,process_id,protocol_version:1}}`. It acquires
no application metadata or text, receives no field events, and does not hide or
disarm another connection's preview when the diagnostic client disconnects.
The stdlib-only `health.py` probe verifies the private socket, exact service
PID/UID, bounded reply and unchanged socket identity. The installer and
`badi doctor` separately check stable systemd service identity. Readiness does
not prove that accessibility is enabled or that any app exposes an eligible field.

Failures return `{schema,id,ok:false,error,epoch}` with fixed metadata-only error
codes. Unsolicited messages are
`{schema,event:"invalidate",epoch,reason,app_id}`; `app_id` identifies the previous
tracked application. A client must clear any candidate and obtain fresh
inspection/policy after invalidation or transport loss.

## Caret geometry and inline preview

Each `preview` request calibrates again; nothing is cached across requests or
tied to a Chromium version. The helper reads the outermost `frame` F, the
field's nearest `document web` D, the field E and the glyph G before the caret in
`SCREEN` coordinates. Chromium and Electron report D, E and G in physical pixels
offset by F's origin times the scale, while F is logical. Electron's frame
origin is arbitrary but cancels. Chromium's `WINDOW` coordinates are relative to
the web viewport and are not used. `D.width / F.width` must equal the Hyprland
monitor scale within 0.01. With that scale s, the window-local logical caret is
`x = (G.x + G.width - F.x*s)/s`, `y = (G.y - F.y*s)/s`, with line height
`G.height/s`. The helper rejects G outside E, E outside D, a caret outside the
window, XWayland, a transformed or ambiguous monitor, and zero extents.

This matched each app's own caret rectangle within 0.5 logical pixels in the
private nested session for Chromium at scales 1 and 2, and for Brave and VS Code
at scale 2. Fractional scales, iframes and other Electron apps remain untested.

Gecko needs its own rule, because `D.width / F.width` was 671/687 at scale 2.
In Zen, `SCREEN` and `WINDOW` extents were identical and relative to the window.
F sits at (0, 0), and F, D and E are logical pixels, but G is in device pixels.
The helper therefore requires F at the origin and as wide as the Hyprland
window, D inside F, E inside D, and G/s inside E. The window-local caret is then
`x = (G.x + G.width)/s`, `y = G.y/s`, with line height `G.height/s`; E is used
unscaled. Zen's surface (687x495) was taller than the tile Hyprland showed
(687x431), so Hyprland clipped its bottom 64 pixels. The caret must therefore
fit Hyprland's window size, never F's height. A marker screenshot put the
viewport exactly at the window origin plus D's offset, and the predicted caret
was within 0.3 logical pixels of the page's own (x 289.5 against 289.77). Only
scale 2 was measured, and scale 1 is the identity. Every other scale, a frame
away from the origin and any failed containment check yield no geometry, so the
addon uses the Fcitx panel. The inline preview also needs the LTR `direction`
text attribute, and Gecko has none: its text attributes are language, invalid,
colors, font family, size, style and weight, auto-generated, text decoration and
position (`TextAttrsMgr`, Firefox source 2026-09). A Zen snapshot therefore
never sets `caret_edge`, and Zen suggestions always use the Fcitx panel; the
calibration above is kept, tested, for a Gecko that exposes direction. Gecko's
component extents divide device pixels by the window's scale factor, not the
page's zoom, while character extents stay device pixels, which matches the
probe.
Qt fields, including Omawrite and Telegram, return zero character extents, so no
overlay is drawn and the addon uses the Fcitx panel instead.

A successful snapshot adds `caret_edge: "right"` only when its approved context
and text-run metadata establish LTR direction. The GTK4 layer-shell overlay then
draws the suggestion as translucent grey text right after the caret. Its font
size is derived from the glyph height and its baseline is aligned with the glyph
box. There is no pill or background. It renders only LTR suggestions that end
inside the field, window and monitor; otherwise it returns `rendered: false` and
the addon falls back to the Fcitx panel. The surface uses the overlay layer, no
keyboard interactivity, an empty input region and no focus. Any invalidation,
new render or expiry hides it. The helper sets `GSK_RENDERER=cairo`,
`GDK_DISABLE=gl,vulkan` and `GTK_IM_MODULE=gtk-im-context-simple` for itself
before GTK loads, so one grey label loads no GPU driver and the helper never
connects to an input method. It builds the preview and lays out sample text
once from an idle callback after startup, so font setup does not count against
the first preview's deadline. If the preview cannot be built (no layer-shell
support), the helper remembers that and returns `rendered: false` until it
restarts. Software rendering at a fractional monitor scale may look slightly
softer than GPU rendering; it has not been compared live.

## Verified evidence and limits

The 2026-09-07 isolated Chromium 151.0.7922.173 native Wayland fixture loaded **no
extensions**. AT-SPI exposed a textarea, contenteditable, password role and
cross-origin iframe field. Two fields with exactly identical text had different
object paths. The password's `GetText` was never called. Fcitx metadata recorded
native input/context events and app identity `chromium`; no app policy was added
and no suggestions or editing were claimed by that acquisition probe.
Artifacts: `output/extensionless/chromium-probe/report.json` (ignored local
receipts). Initial viewport-emulation evidence is retained separately and does
not establish physical visibility of offscreen fields.

The source-level observer/IPC tests cover privacy gates, malformed frames,
identity and epoch changes, stale snapshots, bounded Unicode reads and preview
lifecycle. Private socket and actual-addon cases also prove that missed caret
and length changes cannot reach the renderer, even with identical bounded text.
The explicit session-bus lane uses task-created emitters to prove that
unrelated senders and fields cannot deliver typed events, and that invalidation
unsubscribes the authorized field. Its runner creates private D-Bus and XDG
environments and never connects to the normal desktop's accessibility bus.
A 2026-09-07 live Chromium 151 native Wayland trial verified the complete
private-socket snapshot and a visibly aligned grey caret pill, since replaced by
inline text, using fixed presentation text. Broker origin authorization preceded the text read; the
focused Chromium window remained unchanged. Evidence is retained in
`output/extensionless/installed-browser-01/preview-result.json` and `preview.png`.
Later sandbox-enabled Chromium trials show real local-model text in the same grey
pill and accept it through Fcitx, but undo also removes the preceding typed prefix.
Native correction failed when an `input` handler moved focus after deletion.
Even a single native append failed: a `beforeinput` focus change redirected it to
another field, and a caret change placed it before the original prefix. Cancelling
that event prevented mutation, while a focus change after insertion preserved
the correct result. Evidence is in `output/extensionless/installed-browser-03/`
and `installed-browser-04/`; an earlier sandbox-disabled append trial remains
separately recorded in `installed-browser-02/`.

Native replacement is disabled in negotiation, parsing, authorization and
dispatch; editor-owned correction is separate. Matching observation never grants
an atomic editor operation. AT-SPI cannot supply one because Chromium does not
expose `EditableText`, and identical caret/selection calls return without closing
the typing group. Under the 2026-09-26
[IME-parity decision](../../docs/decisions/0003-ime-parity-append-only.md),
Chromium, Brave, Codex, VS Code, Cursor and Discord may nonetheless accept
through one append-only Fcitx `commitString` that behaves like typed text. Undo may coalesce with preceding typing, and page
script that moves focus or caret during `beforeinput` may redirect it like a
keystroke. Neither exact undo nor verified field authority is claimed. This
helper's share is exact identity, snapshot/caret agreement before display and
dispatch, and fail-closed purpose and sensitivity gates. For those apps the
identity rules, calibration and preview have source tests and private
nested-session evidence only, until live trials. Zen joined the same decision
by user request. Its identity, focus, `DocURL` and calibration rules come from
the live probe above; acceptance through a Fcitx `commitString` into
Gecko and its undo grouping are not live-tested, and it has no inline preview. There, one Ctrl+Z
removed a whole typed 24-character phrase.

Full desktop installation copies the helper/service, preserves an existing
autostart choice, and waits for an exact-process health reply before restarting
Fcitx. On Fcitx 5.1.22 it also selects the pinned
[compatibility frontend](../../packaging/fcitx5-wayland-compat/README.md), without
which Chromium-based text-input-v3 clients stop publishing surrounding text.
`--broker-only` leaves this helper and the frontend untouched. In the private
nested session, VS Code published its editor's surrounding text with
`"editor.editContext": false`. Set it in the VS Code user settings; `badi doctor`
names this one-line fix while the setting is missing (a blank file counts as
`{}`, as in VS Code).

### Renderer accessibility flag

Chromium exposes no web content over AT-SPI unless it starts with
`--force-renderer-accessibility`. This holds even with `org.a11y.Status`
`IsEnabled` and GNOME toolkit accessibility both true, so every Chromium-based
IME-parity app needs the flag. Gecko needs none: Zen exposed its pages without
one, so `--observed-app` has no Zen entry. Measured on Chromium 152.0.7977.82
(2026-09-26) with a disposable profile and a local textarea:

| Mode | Observer result |
| --- | --- |
| no flag | no web nodes; `focus_unavailable` |
| `=basic`, `=form-controls` | URI, caret and text are available, but the HTML `tag` is empty and there are no character extents, so the purpose gate rejects the field and there is no calibrated caret |
| bare switch, `=complete` | tag, URI, caret, bounded text and calibrated geometry; inspection took 32–43 ms of the 350 ms budget |

The lightest working mode is therefore `complete` (the bare switch is the same
mode). It makes Chromium build the full accessibility tree for every page. That
costs some browser CPU and memory, most visibly on large pages, for as long as
the app runs. In these runs Chromium 152 gave Fcitx a native Wayland text-input
context with surrounding text without any input-method flag. Electron 42 is
expected to share that default but was not relaunched here. The installer adds
no input-method flags.

`install-desktop.py --observed-app APP` (repeatable) appends only
`--force-renderer-accessibility=complete`, with a comment, to that launcher's
user flags file under `$XDG_CONFIG_HOME`. Each file is parsed the way its
wrapper reads it:

| App | File | Wrapper parsing |
| --- | --- | --- |
| `chromium` | `chromium-flags.conf` | `/usr/bin/chromium`: GLib shell quoting, `#` comments |
| `brave-origin` | `brave-origin-flags.conf` | `/usr/bin/brave-origin`: each non-comment line is one argument |
| `chatgpt` | `codex-flags.conf` | `/usr/bin/chatgpt`: text after `#` removed, split on whitespace |
| `code` | `code-flags.conf` | `/usr/bin/code`: text after `#` removed, split on whitespace |
| `cursor` | `cursor-flags.conf` | `/usr/share/cursor/cursor`: each non-comment line is one argument, after Cursor's entry |

A file that already has the bare or `=complete` switch is left untouched. Any
other value, or `--disable-renderer-accessibility`, stops the installer before
it builds or changes anything, because a reduced mode is the user's choice.
Changed files keep their mode (new files are `0600`), are backed up and listed
in the desktop backup's `changes.json`, and are recorded in the desktop install
receipt. Existing
app windows keep the flags they started with; relaunch the app.

Discord has no supported mechanism. `/usr/bin/discord` reads no flags file and
forwards only its own arguments; its desktop entry passes `--url -- %u`. Discord's `settings.json` `chromiumSwitches` is an undocumented
internal list: its allowlist check accepts unlisted names only because
`0 !== undefined`, and it appends names without values. Whether it applies
before Chromium reads the accessibility mode is untested. The installer does
not write it. Starting `discord --force-renderer-accessibility=complete` after
quitting any running instance passes a real switch, but this is not live-tested.

The system accessibility bus also needs to be enabled. Enabling its `IsEnabled`
property can persist the corresponding GNOME toolkit-accessibility setting;
treat this as a desktop setting with a recorded prior value, not a
process-local toggle.
