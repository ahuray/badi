# Fcitx wire v2

Broker frames are built and parsed only in `src/transport.{h,cpp}`. The
observer's (`badi.accessibility.v1`) requests and reply checks are in
`src/observation.{h,cpp}`, its socket client in `src/accessibility.{h,cpp}`.

## Transport boundary

- Unix socket: `$XDG_RUNTIME_DIR/badi/broker.sock`.
- The runtime directory must be absolute and normalized. The path is inspected
  with `lstat`; it must be a socket owned by the current UID with mode `0600`.
  Linux `SO_PEERCRED` must report the same UID after connect.
- The socket is `SOCK_NONBLOCK | SOCK_CLOEXEC` and is integrated with Fcitx's
  event loop. Outbound buffering is capped at 32 frames / 1 MiB.
- Frames are a four-byte little-endian length followed by 1..65,536 bytes of
  UTF-8 JSON. Malformed, oversized, unknown, or non-exact inbound shapes close
  the connection. Comments and duplicate object keys are rejected.

## Negotiation and authority

The first frame is `hello` with `v: 2`, `min_v: 2`, `max_v: 2`, adapter
`{kind:"fcitx",name:"badi-fcitx5",version:"0.1.0"}`, and the exact capabilities
`context`, `suggestion`, `commit.dispatched_unverified`, `control`, and `policy`.
The adapter waits for `hello.ack` and the initial `authority.changed`, queues an
`authority.ack`, and only then opens sessions. Later authority epochs retire
local candidate/context authority before reopening eligible focused sessions.
A closed connection retires the same authority. A reconnect's initial snapshot
counts as a new authority when its epoch, settings revision or pause state
differs from the last one observed.

## Sessions and targets

Each binding asks `policy.query` (id `fcitx.policy.<session UUID>`, one
outstanding per session) and, once allowed, sends `session.open` (id
`fcitx.open.<session UUID>`, revision 0, activation `always`, matching the
installed app policy). Both name the same target, a JSON object of at most
4 KiB:

- **Manual path** (native exact apps only), built by
  `desktopApplicationTarget()`:

  ```json
  {
    "kind": "desktop_application",
    "app_id": "<canonical app id>",
    "target_id": "<opaque InputContext UUID>"
  }
  ```

- **Observed field:** the observer's `inspect` target verbatim, `browser` with
  `origin` for exact-origin policy, or `desktop_application` with the same
  canonical app id.

The canonical app id is `InputContext::program()` folded to ASCII lowercase
once at focus-in, accepted only when it matches
`^[A-Za-z][A-Za-z0-9_-]*(\.[A-Za-z][A-Za-z0-9_-]*)*$` within 128 bytes; the
broker's validator stays lowercase-only. Desktop targets omit `origin`; their
settings identity is `{kind:"linux_app",adapter:"fcitx",app_id:<the same id>}`.
The manual target ID is an opaque Fcitx context UUID, not a stable widget
identity: runtime authorization is the exact application ID plus explicit
invocation in an eligible native text context.

## Context

A request is `context.changed` followed by `suggest.request`, with a validated
input-method language and selection unit `unicode_scalar_values`. Before/after
are bounded to 512/128 Unicode scalar values. Sensitive, disabled,
special-purpose, composing, selected, and invalid-language contexts are not
serialized.

- **Manual path:** only the local invocation chord sends context, with
  `activation:"manual"`, `explicit:true`, field purpose `unknown` and
  `identity_known:false`; the broker's explicit-manual policy is the only path
  that may authorize it.
- **Observed field:** `identity_known:true`, purpose `normal`, and activation
  `always` (automatic) or `manual` with `explicit:true`. IME-parity apps (see
  [README](README.md#app-classes)) send only observed frames. When an
  IME-parity field's after-caret text is exactly Chromium's paragraph end
  (`"\n\n"` or `"\n"`) and the snapshot agrees, `context.changed` carries
  `after: ""` with the unchanged selection.

The addon relies on these observer semantics: `snapshot` returns
`before`/`after`/`caret`/`total_chars` for the exact binding; a `preview` reply
with `ok:true` has re-verified caret and length, and `focus.rendered:false` then
selects Badi's own Fcitx panel. An `ok:false` reply or no reply within 500 ms
displays nothing, and a stale snapshot dispatches nothing.

Some toolkits republish surrounding text while modifier chords are formed. The
addon keeps the current revision only when a fresh complete native capture
matches the captured context byte for byte. Changed, sensitive, composing, or
unavailable context revokes local authority before a broker response can act.

## Suggestion and commit

`suggestion.show` (with `accept_word`, the next-word part of `text`),
`suggestion.clear`, `control.request` (`accept_all`, `accept_word` or
`dismiss`), `control.result`, `commit.prepare` (`acceptance` `all` or `word`)
and `commit.result` are matched
on session UUID, focus epoch, revision, salted fingerprint, suggestion ID, exact
text, and control ID. `suggestion.clear` accepts exactly the schema's two
payload shapes: fingerprint and reason, or those fields plus a string suggestion
ID. A missing optional ID is valid; `null`, an extra field, or a malformed ID
closes the connection. A suggestion or commit grant carrying a replacement
field closes it too.

A valid `commit.prepare` is consumed once. The addon calls `commitString(text)`
once and emits:

```json
{"status":"dispatched-unverified"}
```

No success claim stronger than dispatch is available at the Fcitx boundary.
