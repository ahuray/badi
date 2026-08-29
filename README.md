# Omatype

Omatype is a capability-aware local co-writing layer for Linux. It offers one
short, revision-bound continuation only where an adapter can read, display,
and edit through a tested target API. Unsupported is a valid result; raw global
input capture and synthetic typing are not part of the architecture.

> **Codename warning:** an unrelated
> [OmaType dictation project](https://github.com/Aayush9029/OmaType) already
> occupies the same Linux/Omarchy category. `Omatype` is an internal repository
> codename. A distinct public name and technical namespace are release gates.
> The project license is also deliberately unresolved pending a user decision.

## Current status

The repository contains research plus a working M1 foundation:

- a strict JSON Schema protocol with positive/negative examples, 64 KiB
  little-endian frames, explicit UTF-16 browser offsets, surrogate-safe bounded
  context, relative TTLs, and a shared multilingual accept-word fixture;
- a Rust 2024 broker and `omatypectl` with fail-closed policy, secure local
  Unix socket, peer-UID checks, deterministic suggestions, latest-wins
  cancellation, addressed commits, global pause, and content-free metrics;
- a strict TypeScript Manifest V3 adapter limited to the localhost fixture,
  with pre-acquisition field denial, adapter-owned ghost UI, exact
  type-through, broker-authorized word/all acceptance, dismissal, and pause;
  and
- deterministic Rust, schema, TypeScript, jsdom race, and extension-build
  checks in CI.

This is a simulated DOM-adapter foundation. No extension or native host is
installed, no browser profile or desktop configuration is changed, and the
current evidence does **not** prove live Chromium lifecycle, layout, framework
state, native undo, arbitrary-site compatibility, semantic-model quality,
Obsidian support, or terminal support.

## Start here

- [Vision V2](VISION-V2.md) — the current product and trust contract.
- [V2 landscape](docs/research/vision-v2-landscape.md) — primary-source review
  of Cotypist and sixteen direct or adjacent products/projects.
- [V2 implementation plan](docs/plan/vision-v2-implementation.md) — exact
  milestones, capability cells, agent workflow, tests, rollout, and stop
  conditions.
- [Chromium foundation receipt](capabilities/chromium-dom-foundation.v1.json) —
  machine-readable evidence class, supported surface, exclusions, versions,
  and checks.

The original [V1 vision](VISION.md),
[Linux architecture research](docs/research/linux-architecture.md),
[adversarial review](docs/research/adversarial-review.md), and
[two-day plan](docs/plan/two-day-delivery.md) remain useful decision history.
Their conflicting three-target/48-hour gates are superseded by V2.

## Architecture

```text
supported field -> Chromium controller -> MV3/native boundary -> Rust broker
      ^                    |                         |
      |                    v                         v
target-API edit      adapter ghost UI       policy/provider/metrics
```

The adapter alone may read and mutate its target. The broker validates policy
and state, prepares a commit, and never drives a keyboard. Every non-global
action carries session, focus epoch, revision, fingerprint, and suggestion ID.
Only pause is global. See the V2 plan for the full trust boundary.

## Verify from a clean checkout

CI tests Node.js 22.23.2 and 24.20.0, Rust 1.98.0, and the declared Rust 1.85
minimum. The project default is pinned to Node 24.20.0 in `.nvmrc`; the
capability receipt records the separate local validation environment. From the
repository root:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm ci
npm run check
git diff --check
```

The Chromium build is generated at `adapters/chromium/dist/` and is ignored by
Git. Its timestamp-free `BUILD_MANIFEST.json` records stable SHA-256 hashes;
the root check compares two clean builds byte for byte and validates both local
documentation links and the machine-readable capability receipt.

To inspect the controlled page only:

```sh
npm run fixture --workspace @omatype/chromium
```

Open `http://localhost:4173/chromium.html`. Suggestions still require the M2
native-host bridge, which is intentionally not installed by this foundation.

## Product order

1. Finish a live temporary-profile Chromium/native-host proof.
2. Prove the same signature loop through supported Obsidian/CodeMirror APIs.
3. Evaluate a small local semantic suffix provider against the deterministic
   baseline.
4. Run the manual Fcitx5/Ghostty/Codex feasibility cell independently and
   report failure honestly if the native path cannot meet the contract.

There is no fallback to `evdev`, `wtype`, clipboard insertion, `xdotool`, or a
shell-only demonstration.
