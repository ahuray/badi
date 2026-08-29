# Omatype Chromium vertical slice

This is a deliberately narrow unpacked Manifest V3 foundation. It connects only
to the native-messaging host name `io.omatype.broker`. Building and testing do
not register that host, modify a Chromium profile, or install the extension.

From the repository root:

```sh
npm run typecheck
npm test
npm run build
npm run build:verify --workspace @omatype/chromium
```

The unpacked artifact is generated at `adapters/chromium/dist/`. Its
`BUILD_MANIFEST.json` contains stable SHA-256 hashes and no timestamp.

For a controlled page, run:

```sh
npm run fixture --workspace @omatype/chromium
```

Then open `http://localhost:4173/chromium.html`. Loading the unpacked extension
into a browser remains a deliberate, manual action.

## M1 capability boundary

The content controller starts only in the top frame of the exact controlled URL
`http://localhost:4173/chromium.html`. The service worker independently verifies
extension id, origin, URL, top-frame id, active document lifecycle, and tab route
before forwarding a message. Broader websites and grants belong to a later
milestone.

An ambient M1 request requires all of the following:

- a visible, enabled, writable `input[type=text]` or `textarea` in light DOM;
- a unique `id`, unique in-scope `name`, or unique `data-omatype-field` marker;
- a collapsed selection, stable focus/revision, and no active composition;
- no sensitive autocomplete purpose, opt-out ancestor, or failing field
  constraint.

M1 excludes `contenteditable`, search/URL/telephone/email and other input types,
iframes, shadow roots, anonymous or duplicate-identity fields, non-collapsed
selection, and hidden/disabled/read-only/composing fields. Password, hidden,
credential, OTP, identity, address, phone, birthday, and payment autocomplete
purposes are hard-denied before value access.

A plain text field can still be secret-like without declaring a semantic signal;
the adapter cannot infer that safely. Sites and fixtures must mark such regions
with `data-omatype="off"`; this boundary is not a general secret detector.
Controlled-framework compatibility and browser-native undo behavior are also not
yet proven. The current vanilla-DOM `setRangeText` plus `input` event path is
reported to the broker as `dispatched-unverified`.

While a controller is alive, M1 cancels work and clears visible UI on observed
active-field/ancestor policy mutations, identity ambiguity, DOM removal,
document visibility loss, and a native-port disconnect delivered to its frozen
trusted route. This is deterministic unit-level coverage, not a claim of live
MV3 lifecycle proof. Active-tab changes, window changes, browser navigation,
worker restarts, and routes that were never registered remain M2 work.

Keyboard behavior while a suggestion is visible:

- `Tab`: accept all
- `Ctrl/Command + Right Arrow`: accept the next Unicode-aware word chunk
- `Escape`: dismiss (it is not intercepted while no suggestion is visible)
- `Alt + Shift + P`: ask the broker to toggle globally, await its authoritative
  state, then broadcast exact pause/resume only to deduplicated content routes
  previously established by trusted fixture messages

That extension-shortcut broadcast stops and clears controllers that are
currently reachable. M1 does not yet have the cross-connection pause epoch/ack
barrier needed to synchronize pause state. A pause issued by an external CLI
may therefore leave already-open controllers, as well as newly created or
MV3-restarted controllers, locally unpaused. They can emit repeated requests;
the paused broker rejects each before provider execution and does not issue new
commit authorization. Rejected extension shortcut controls do not toggle or
broadcast local state.

The extension-owned ghost consumes exact typed Unicode scalars from its prefix
in place. That locally continued suffix is display-only and cannot be accepted
until a fresh provider response binds it to the new revision and fingerprint.
A contradictory edit, field replacement, selection change, or focus change
clears the suffix. Receiver-local TTL expiry and matching broker clears also
hide it without requiring another page event.

Acceptance never mutates first: it sends one session-addressed control request,
waits for the broker's matching `commit.prepare`, revalidates the complete DOM
snapshot and constraints, and then performs one insertion. Denial, expiry,
revocation, or any intervening edit produces no insertion.

After an accepted word, M1 clears any local remainder and requests a freshly
addressed suggestion before another acceptance. Unsolicited broker frames are
not general page-control APIs: `omatypectl` request/accept commands exercise the
broker core only and do not route `suggestion.show` or `commit.prepare` into a
Chromium content controller in M1.
