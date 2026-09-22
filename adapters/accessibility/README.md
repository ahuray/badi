# Badi focused accessibility observer

This helper supplies **field identity and bounded context**, and an optional
caret preview, to the cooperative Fcitx addon. It never inserts, deletes, selects,
or types text and never changes accessibility settings or application policy.

The supported identity mappings are exact executable/class pairs inspected on
this workstation: Chromium (`/usr/lib/chromium/chromium`, class `chromium`),
ChatGPT/Codex (`/usr/lib/chatgpt/ChatGPT`, class `chatgpt`), and Omawrite
(`/usr/bin/omawrite`, class `omawrite`). A mapping is an acquisition eligibility
check, not a claim that the application's complete editing flow works. Brave
Origin 152 was also found and launched in an isolated profile: its actual
ELF is `/opt/brave-origin-bin/brave`, with class `brave-origin`. Its Fcitx program
was physically observed as `brave-origin` and now has an exact helper mapping.
Its complete prediction/editing flow and caret geometry remain unverified; it
does not inherit Chromium 151 preview coordinates.

## Authority boundary

`inspect` verifies the unlocked Omarchy session, current Hyprland window PID,
owned process executable, and exactly one focused editable accessibility object.
Only that process is queried. Chromium browser chrome/internal pages and
ambiguous focused nodes are excluded before collecting a field. Browser nodes
must supply a valid HTTP(S) Document `URI`; cross-origin frames retain their own
URI. The helper reads no field prose during inspection.

`snapshot` requires the exact prior object bus name/path, process, application,
URI, epoch and broker-policy target. Its trusted Fcitx caller must already hold a
current broker context-read grant for that target. Password/sensitive,
noneditable, invisible, selected, unsupported field purposes and stale bindings
fail before `Text.GetText`. Reads are at most 512 Unicode characters before the
caret and 128 after it; metadata and the bounded text are reread to detect
changes. Neither the accessible object nor this helper provides an atomic edit
transaction. Fcitx must still compare its exact live surrounding text, caret,
focus epoch and one-shot commit authority before dispatch.

Relevant accessibility events and compositor focus/window changes invalidate the
current epoch and clear the preview. Global listeners receive only metadata events. Text-change notifications are
subscribed on a separate D-Bus connection only after an approved snapshot, with
a match restricted to the exact unique sender and field object path.
Invalidation/disconnect removes that match; event prose is never unpacked.
Notifications do not retain event text, document labels, or URIs. Browser origin policy stays centralized in the broker;
this helper does not grant blanket browser access.

Libatspi also emits local, sender-null `defunct` events when its temporary proxy
objects are disposed. The helper ignores that exact disposal signal only for a
positively identified different bus/object path. Disposal of the tracked object,
unknown proxy identity, and genuine remote events retain invalidation. This
prevents inspection itself from repeatedly invalidating a stable Chromium field;
snapshot identity and text rereads remain unchanged.

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
Each operation has a 350 ms overall budget and AT-SPI calls have 50 ms timeouts.
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
text and never accepts pointer/keyboard input.

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

On this Chromium 151/Wayland/scale-2 cell, glyph `SCREEN` extents are **window-local
physical pixels**, while the top-level frame gives window-local logical bounds.
Hyprland supplies the actual global logical origin. The helper returns raw
extents and the active window metadata; it marks the tested convention only for
the verified Chromium major version, native Wayland, and an untransformed monitor.
A successful snapshot adds `caret_edge: "right"` only when its approved context
and text-run metadata establish LTR direction. RTL/mixed/unknown runs remain
unrenderable. The preview independently validates this edge, scale, monitor and
window bounds before mapping coordinates. Other applications retain unnormalized geometry.

The source-level observer/IPC tests cover privacy gates, malformed frames,
identity and epoch changes, stale snapshots, bounded Unicode reads and preview
lifecycle. Private socket and actual-addon cases also prove that missed caret
and length changes cannot reach the renderer, even with identical bounded text.
The explicit session-bus lane uses task-created emitters to prove that
unrelated senders and fields cannot deliver typed events, and that invalidation
unsubscribes the authorized field. Its runner creates private D-Bus and XDG
environments and never connects to the normal desktop's accessibility bus.
A 2026-09-07 live Chromium 151 native Wayland trial now verifies the complete
private-socket snapshot and a visibly aligned grey caret pill using fixed
presentation text. Broker origin authorization preceded the text read; the
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
dispatch. The native browser/Codex path is quarantined; matching observation does
not grant an atomic editor operation. Editor-owned correction is separate.
AT-SPI cannot supply the missing operation because Chromium does not expose
`EditableText`; identical caret/selection calls also return without closing the
typing group. The [research findings](../../docs/research/linux-architecture.md)
describe the missing transaction authority and the distinction between renderer
hardening and an end-to-end protocol. Codex and Brave editing remain unverified.

Full desktop installation copies the helper/service, preserves an existing
autostart choice, and waits for an exact-process health reply before restarting
Fcitx. `--broker-only` leaves this helper untouched. Current fixture startup used
`--ozone-platform=wayland --enable-wayland-ime --wayland-text-input-version=3
--force-renderer-accessibility=complete`. These are measured fixture flags, not
proof that all are needed in every Chromium/Electron version. On this workstation
the Codex wrapper reads `~/.config/codex-flags.conf`; an already running process
does not acquire new flags. The system accessibility bus also needs to be
enabled. Enabling its `IsEnabled` property can persist the corresponding GNOME
toolkit-accessibility setting; treat this as a desktop setting with a recorded
prior value, not a process-local toggle.
