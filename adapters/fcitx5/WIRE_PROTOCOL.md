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
  the connection; a well-formed [refusal](#refusals) of one request does not.
  Comments and duplicate object keys are rejected. A callback that closes the
  connection ends its batch: the remaining frames are never dispatched.

## Negotiation and authority

The first frame is `hello` with `v: 2`, `min_v: 2`, `max_v: 2`, adapter
`{kind:"fcitx",name:"badi-fcitx5",version:"0.1.0"}`, and the exact capabilities
`context`, `suggestion`, `commit.dispatched_unverified`, `text_replacement`,
`control`, `policy`, `long_reading_window`, `selection_revision` and
`alternative_list`. With
`text_replacement`, a native exact field deletes the exact typed suffix before
its one commit; IME-parity apps but LibreOffice take a spelling correction the
same way once the observer has re-confirmed the field, then read the app's
text back ([ADR 0011](../../docs/decisions/0011-checked-corrections-in-append-only-apps.md)).
Every other IME-parity edit stays append-only. `selection_revision` is the
explicit revision below; `hello.ack` must enable exactly these.
`long_reading_window` admits suggestion TTLs up to 15,000 ms. It needs an
updated broker; older clients without it keep the five-second ceiling.
Commit-result deadlines and every acceptance binding remain unchanged.
The adapter waits for `hello.ack` and the initial `authority.changed`, queues an
`authority.ack`, and only then opens sessions. The broker enforces this order:
session, context and suggestion messages from a connection that has not
acknowledged the current epoch fail closed with a stale error. Later authority epochs retire
local candidate/context authority before reopening eligible focused sessions.
A closed connection retires the same authority. A reconnect's initial snapshot
counts as a new authority when its epoch, settings revision or pause state
differs from the last one observed.

## Sessions and targets

An allowed `policy.status` may include `language_override` (`en`, `de` or
`fa`). It takes precedence over the keyboard hint for requests and manual Tab
eligibility. It grants no permission and is cleared with the policy on focus,
field, authority or connection changes. The broker independently applies the
same preference after admitting context.

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

- **Observed field:** the observer's `inspect` target, `browser` with `origin`
  for exact-origin policy, or `desktop_application`, both with the exact
  canonical app id. The addon substitutes only its random field-visit target
  id for broker quieting; observer requests retain the original target.

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
- **Window text** ([ADR 0010](../../docs/decisions/0010-context-finishes-words.md)):
  when the policy reply for the binding carries `context_words: true`, the
  first observer `snapshot` of a field visit asks `window: true`. The reply's
  `window_text`, if well-formed and at most 2,048 scalars, is kept for that
  visit and sent as `window_text` on its observed `context.changed` frames. A
  new field visit asks again; a denial, a retired session or an unobserved
  path sends none. A policy reply with `context_words: true` but suggestions
  denied, or a non-boolean value, closes the connection like any malformed
  policy.

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
`suggestion.clear`, `control.request` (`accept_all`, `accept_word`,
`dismiss` or `alternatives`), `control.result`, `commit.prepare` (`acceptance` `all` or `word`)
and `commit.result` are matched
on session UUID, focus epoch, revision, salted fingerprint, suggestion ID, exact
text, and control ID. `suggestion.clear` accepts exactly the schema's two
payload shapes: fingerprint and reason, or those fields plus a string suggestion
ID. `alternatives` names the shown continuation (never a correction). The
addon negotiates `alternative_list`, so it is answered with a
`suggestion.clear` `superseded` for the shown continuation, then a
`suggestion.list` naming it with up to four other first words; a further
`alternatives` control carrying one listed `word` is answered like an
explicit request with a `suggestion.show` (new ID) of that word's
continuation. A session without the capability instead gets the show
directly: the model's next continuation that starts with a word not shown
yet for that context, or after three (or none), one already shown again.
A missing optional ID is valid; `null`, an extra field, or a malformed ID
closes the connection. A suggestion or commit grant carrying a replacement
field closes it too.

A valid `commit.prepare` is consumed once. The addon calls `commitString(text)`
once and emits:

```json
{"status":"dispatched-unverified"}
```

No success claim stronger than dispatch is available at the Fcitx boundary.

## Refusals

The broker answers a request it refuses but recovers from (stale coordinates,
a suggestion that expired before its acceptance, a pause) with an `error`
frame and keeps the connection:

```json
{"v":2,"id":"<the request's id>","type":"error","mono_ms":0,"payload":{"code":"expired"}}
```

Exactly these keys, an opaque id and a known reason code are accepted; an
`error` without an id, with any other field (the settings-only `committed`,
`settings_revision` or `control_plane_degraded`), or before the handshake
completes closes the connection. A refusal ends only what its id names:

- an acceptance in flight (`fcitx.accept.<epoch>.<revision>`) retires its
  suggestion, so a later grant for it is reported `stale`;
- a revision request or its acceptance ends the revision with a notice;
- the current context's `suggest.request` (`fcitx.suggest.<epoch>.<revision>`)
  says the request failed if the user asked for it, and keeps the context;
- a `policy.query` is owed nothing more, and its session may ask again.

These ids carry a focus epoch and revision, not the session, so two sessions
can share one; every binding holding the id retires it, which only withdraws.

## Revision

An explicit revision of one selection
([ADR 0012](../../docs/decisions/0012-explicit-revision-of-one-selection.md);
`synonyms` by [ADR 0013](../../docs/decisions/0013-synonyms-for-one-short-selection.md)).
`revision.request` is a field revision of its own, with its own salted
fingerprint over the selection: `mode` (`grammar`, `refine` or `synonyms` —
a synonyms selection is at most three words with no digit, address or
sentence mark, answered `invalid_output` otherwise), `before`
(≤512 scalars), `selected` (1–600 scalars on one line), `after` (≤128),
`selection` (Fcitx's anchor and cursor, which span exactly `selected`),
`field`, `activation: "manual"` and `language`. It supersedes the field's
context and any suggestion.

The broker answers once: `revision.show` (`revision_id`, `mode`, `original`,
`text`, `ttl_ms` ≤ 60,000, `provider`), or `revision.clear` with a reason:
`no_change`, `invalid_output`, `unsupported_language`,
`revision_unavailable`, `provider_timeout`, `provider_error`, or a policy
refusal (`paused`, `policy_never`, `field_*`), which is answered rather than
an `error` frame. Both repeat the request's coordinates. `original` must equal
the selection sent, and both texts must be one line with no control or format
character and no whitespace but U+0020, U+00A0 and U+202F. A later
`revision.clear` with `superseded`, `dismissed`, `expired` or
`session_closed` retires a shown revision.

`suggest.cancel` with the request's fingerprint ends one still running.
`control.request` `dismiss` or `accept_all`, with `suggestion_id` set to the
`revision_id`, retires or accepts the shown one; `accept_word` is refused.
The grant is `commit.prepare` with `replace_selection` (the request's
`anchor`, `head` and `unicode_scalar_values`), `acceptance: "all"` and the
revised `text`. A grant naming another selection, or one that also carries
`replace_before`, is refused. The addon checks that Fcitx (and the observer,
for an observed field) still report exactly that selection, calls
`commitString(text)` once over it and reports `dispatched-unverified`.
