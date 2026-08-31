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
npm run build:product --workspace @badi/chromium
npm run build:verify --workspace @badi/chromium
```

The two unpacked artifacts are intentionally separate:

- `adapters/chromium/dist/` is the historical localhost fixture build.
- `adapters/chromium/dist-product/` is the opt-in Dillinger product slice.

Each `BUILD_MANIFEST.json` contains stable SHA-256 hashes and no timestamp.
Both directories are generated and ignored by Git.

For a controlled page, run:

```sh
npm run fixture --workspace @badi/chromium
```

Then open `http://localhost:4173/chromium.html`. Loading the unpacked extension
into a browser remains a deliberate, manual action.

## Exact Dillinger product slice

The product manifest requests only `nativeMessaging` and `scripting` up front.
It declares one optional Chrome host match,
`https://dillinger.io:443/*`; the shipped runtime, content registration, MAIN-
world bridge, and sender checks still accept only the exact top-level document
`https://dillinger.io/` with no path, query, fragment, subframe, incognito tab,
background tab, stale `documentId`, hidden document, or unfocused window.
Chrome's permission UI grants the declared host match, which is necessarily
broader than that exact runtime URL gate.

After the user approves the optional host prompt, the worker registers one
non-persistent isolated content script. The worker alone invokes the frozen
Dillinger Monaco bridge in the MAIN world. The bridge requires one focused
Markdown editor/model, a collapsed visible caret, an exact model URI/version/
length/context snapshot, and a current visible/focused document. Acceptance is
one one-shot broker-authorized Monaco transaction. Revocation, pause, route
retirement, a new request, expiry, or worker loss invalidates its service-worker
commit epoch before the synchronous `chrome.scripting.executeScript` boundary.
The preview is suggestion text only—no card or shortcut hint—and is a fixed,
caret-relative overlay with viewport and five-point stacking checks. It is not
a Monaco inline decoration, so this slice does not close the product-showing/M4
gate.

The default acceptance command is `Ctrl+Shift+Y`. Chromium can leave a command
unassigned when there is a collision; users can inspect or remap it at
`chrome://extensions/shortcuts`. On the tested Omarchy device there was no exact
`Ctrl+Shift+Y` compositor binding, and an unextended Dillinger editor preserved
value, caret, focus, and scroll for that key.

### Disposable real-device commands

This focused, non-release probe loads the product worker/popup/bundles in an
isolated profile and compiles the exact repository MAIN-world/view source for a
real `https://dillinger.io/` Monaco insertion, undo, redo, viewport, occlusion,
focus, caret, scroll, restoration, and cleanup check. It deliberately does not
grant host access or exercise native messaging/content routing:

```sh
npm run live:product:probe --workspace @badi/chromium -- \
  --chromium-executable /usr/bin/chromium
```

For the full disposable chain, including a generated native-host manifest,
private broker socket, exact HTTPS policy, optional-permission click, dynamic
registration, `phrase_v1`, command acceptance, revoke, undo/redo, and verified
process/profile cleanup:

```sh
npm run live:product --workspace @badi/chromium -- \
  --chromium-executable /usr/bin/chromium
```

Approve Chromium's exact Dillinger prompt when the runner asks, then click the
Dillinger editor when it asks for focus. The automated mode types the fixed
English case `thank you`, waits for the ` for your time` preview, presses
`Ctrl+Shift+Y`, and checks the transaction. To use the disposable product by
hand instead, add `--interactive`; type `thank you` yourself, accept it, then
return to the terminal and press Enter. `Ctrl-C` also closes the browser,
broker, native host, socket, manifest tree, and disposable profile.

The headed end-to-end runner remains a manual proof boundary: browser-native
permission confirmation and OS window focus cannot be truthfully synthesized
by Playwright. In the current Codex-controlled Hyprland session, the exact host
grant and registration succeeded, but Ghostty retained compositor focus, so the
strict product gate stopped before the broker-to-insertion chain. The focused
probe above passed on real Chromium, but it is not a substitute for completing
that headed gesture. Neither command writes a capability receipt or establishes
release readiness.

This slice does not claim generic Monaco support, arbitrary Dillinger URLs,
multiple editors/models, non-English language inference, browser-store
packaging, other sites/apps, or production compatibility. Its English `en`
request language is a frozen product-cell contract, not an inference from
Monaco's `markdown` model id.

## Localhost fixture capability boundary

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
npm run live --workspace @badi/chromium -- \
  --evidence-id chromium-native-live-run.2026-08-30-review1.v1
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
against `capabilities/v2/live-run.schema.json` and hash-linked by a new V2
receipt. The explicit ID becomes both the raw document ID and filename; the
runner refuses an existing target and never overwrites a prior run. The smoke
deliberately uses smaller counts and cannot produce a live receipt.

This is a static exact-document development proof with only the
`nativeMessaging` API permission. Incognito is declared `not_allowed`, and the
sender boundary independently rejects an incognito tab. The headless browser
did not expose a user permission prompt, so runtime-granted origin consent
remains unproved. The same run reports browser-native undo, tab-background
visibility, and the extension command accelerator as unsupported rather than
inferring them from nearby tests.
