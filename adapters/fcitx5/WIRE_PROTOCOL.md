# Fcitx wire v2 assumptions

All protocol names and JSON construction/parsing are isolated in
`src/transport.{h,cpp}` so the evolving v2 contract can change without leaking
wire DTOs into Fcitx lifecycle code.

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

## Desktop session and context

`session.open` uses activation `always`, matching the installed application
policy. Badi still requests no content automatically: each user invocation
sends a fresh context with activation `manual` and `explicit:true`. The open
frame contains:

```json
{
  "target": {
    "kind": "desktop_application",
    "app_id": "<canonical app id>",
    "target_id": "<opaque InputContext UUID>"
  }
}
```

The canonical app id is `InputContext::program()` folded to ASCII lowercase
once at focus-in, accepted only when it matches
`^[A-Za-z][A-Za-z0-9_-]*(\.[A-Za-z][A-Za-z0-9_-]*)*$` within 128 bytes; the
broker's validator stays lowercase-only. Desktop targets omit `origin`. Their
settings identity is assumed to be
`{kind:"linux_app",adapter:"fcitx",app_id:<the same canonical app_id>}`.
The target ID is an opaque Fcitx context UUID, not a stable widget identity.
Compatibility is proven only for named editor cells; runtime authorization is
the exact application ID plus explicit invocation in an eligible native text
context. Accordingly, Fcitx context frames declare field purpose `unknown` and
`identity_known:false`; the broker's explicit-manual policy is the only path
that may authorize them.

Only the local invocation chord produces `context.changed` followed by
`suggest.request`. The context uses `activation:"manual"`, `explicit:true`, a
validated input-method language, and selection unit `unicode_scalar_values`.
Before/after are UTF-8 bounded to 512/128 Unicode scalar values. Sensitive,
disabled, special-purpose, composing, selected, and invalid-language contexts
are not serialized.

The manual path above is limited to native exact apps. An observed field uses
the observer's `inspect` target verbatim for `policy.query` and `session.open`
(`browser` with `origin` for exact-origin policy, or `desktop_application` with
the same canonical app id) and sends `identity_known:true`, purpose `normal`,
activation `always` (automatic) or `manual` with `explicit:true`. IME-parity
apps (see [README](README.md#app-classes-and-ime-parity)) send only observed
frames. The addon relies on these observer (`badi.accessibility.v1`) semantics:
`snapshot` returns `before`/`after`/`caret`/`total_chars` for the exact binding;
a `preview` reply with `ok:true` has re-verified caret and length, and
`focus.rendered:false` then selects Badi's owned Fcitx panel. An `ok:false`
reply or no reply displays nothing and a stale snapshot dispatches nothing.
When an observed IME-parity field's after-caret text is exactly Chromium's
paragraph end `"\n\n"` and the snapshot's `after` agrees, `context.changed`
carries `after: ""` with the unchanged selection
([rich editors](README.md#app-classes-and-ime-parity)).

## Suggestion and commit

`suggestion.show`, `suggestion.clear`, `control.request` (`accept_all` or
`dismiss`),
`commit.prepare`, and `commit.result` retain their v1 payload names under a v2
envelope. Every suggestion and commit is matched on session UUID, focus epoch,
revision, salted fingerprint, suggestion ID, exact text, and control ID.
`suggestion.clear` accepts exactly the schema's two payload shapes: fingerprint
and reason, or those fields plus a string suggestion ID. A missing optional ID
is valid; `null`, an extra field, or a malformed ID closes the connection.

Some toolkits republish surrounding text while modifier chords are formed. The
addon preserves the current revision only when a fresh complete native capture
matches the captured context byte-for-byte. Changed, sensitive, composing, or
unavailable context revokes local authority before a broker response can act.

A valid `commit.prepare` is consumed once. The addon calls `commitString(text)`
once and emits:

```json
{"status":"dispatched-unverified"}
```

No success claim stronger than dispatch is available at the Fcitx boundary.
