# Badi Chromium vertical slice

This is a deliberately narrow unpacked Manifest V3 adapter. It connects only to
the native-messaging host name `io.github.ahuray.badi`. Ordinary build/unit commands
do not register that host, modify a Chromium profile, or install the extension;
the live commands use and remove a fully disposable profile and HOME/XDG tree.

From the repository root:

```sh
npm run typecheck
npm test
npm run build
npm run build:verify --workspace @badi/chromium
```

The unpacked artifact is generated at `adapters/chromium/dist/`. Its
`BUILD_MANIFEST.json` contains stable SHA-256 hashes and no timestamp.

For a controlled page, run:

```sh
npm run fixture --workspace @badi/chromium
```

Then open `http://localhost:4173/chromium.html`. Loading the unpacked extension
into a browser remains a deliberate, manual action.

## Controlled capability boundary

The content controller starts only in the top frame of the exact controlled URL
`http://localhost:4173/chromium.html`. The service worker independently verifies
extension id, origin, URL, top-frame id, active document lifecycle, and tab route
before forwarding a message. Incognito use is disabled in the manifest and an
incognito sender is rejected independently. A nonempty Chromium `documentId` is
mandatory and each opaque session remains frozen to its first trusted
tab/frame/document.
Broader websites and grants belong to a later milestone.

The manifest requires Chromium 132 or later because the sender gate fails
closed unless the Tabs API reports both `discarded === false` and the
Chromium-132 `frozen === false` lifecycle state.

An ambient M1 request requires all of the following:

- a visible, enabled, writable `input[type=text]` or `textarea` in light DOM;
- a unique `id`, unique in-scope `name`, or unique `data-badi-field` marker;
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
with `data-badi="off"`; this boundary is not a general secret detector.
Controlled-framework compatibility and browser-native undo behavior are also not
yet proven. The current vanilla-DOM `setRangeText` plus `input` event path is
reported to the broker as `dispatched-unverified`.

While a controller is alive, it cancels work and clears visible UI on observed
active-field/ancestor policy mutations, identity ambiguity, DOM removal,
document visibility loss, and a native-port disconnect delivered to its frozen
trusted route. M2A proves dynamic mutation, composition, navigation, and native
disconnect on the controlled live document. Active-tab/window changes,
background visibility in the tested headless build, permission revocation,
worker restarts, and routes that were never registered remain full-M2 work.

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
not general page-control APIs: `badictl` request/accept commands exercise the
broker core only and do not route `suggestion.show` or `commit.prepare` into a
Chromium content controller in M1.

## Isolated live proof

The repository-pinned Playwright package drives `/usr/bin/chromium` with a
temporary user-data directory. The runner builds the Rust binaries and
extension, generates an exact-origin native-host manifest below the temporary
tree, starts the broker on a private temporary runtime socket, and verifies
that every tracked process, socket, manifest, and profile is gone afterward.
It never writes the user's normal Chromium profile or native-host directory.

```sh
npm run live:smoke --workspace @badi/chromium
npm run live --workspace @badi/chromium
```

The durable lane uses the real Rust host and broker for handshake/show,
dismiss, accept-word/all, authoritative pause/resume, denied fields, insertion,
latency, geometry, navigation, and disconnect. Synthetic `CompositionEvent`s
exercise composition lifecycle inside the real browser but do not prove a real
IME. A distinctly labeled JavaScript fault host is used only to return canceled
responses late; it cannot be used as evidence for the production native bridge,
broker, privacy gate, insertion, or latency.

Durable evidence requires 1,000 measured interactions after 50 warmups, 100
delayed stale trials, exact cleanup, and both p95 gates. Its raw JSON is checked
against `capabilities/v2/live-run.schema.json` and hash-linked by the V2 receipt.
The smoke deliberately uses smaller counts and cannot produce a live receipt.

This is a static exact-document development proof with only the
`nativeMessaging` API permission. Incognito is declared `not_allowed`, and the
sender boundary independently rejects an incognito tab. The headless browser
did not expose a user permission prompt, so runtime-granted origin consent
remains unproved. The same run reports browser-native undo, tab-background
visibility, and the extension command accelerator as unsupported rather than
inferring them from nearby tests.
