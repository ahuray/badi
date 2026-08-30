# Capability evidence

Capability records describe one tested surface; they are not platform-wide
support claims.

- `chromium-dom-foundation.v1.json` records the deterministic jsdom/M1
  foundation. Its `live_browser` and `live_native_host` fields remain false even
  though a separate live record now exists.
- `chromium-native-live.v2.json` records the isolated M2A system-Chromium cell.
  It links the generated extension artifacts and exactly one raw live-run
  document.
- `evidence/chromium-native-live-run.v1.json` is the historical raw document
  linked by the current V2 receipt. A future durable command requires a new ID
  such as `chromium-native-live-run.2026-08-30-review1.v1`; the ID is also its
  filename. Raw documents carry environment versions, scenario counts,
  measurements, isolation outcomes, and opaque artifact digests without field
  or suggestion text.
- `v1/schema.json`, `v2/schema.json`, and `v2/live-run.schema.json` define the
  corresponding machine-readable contracts.

## Validation modes

The ordinary repository gate validates committed evidence as historical
evidence:

```sh
npm run capabilities:check
```

For V2, this mode validates both schemas, the receipt-to-raw hash, internal
receipt/raw agreement, unique checks/scenarios/measurements, mandatory scenario
and command results, linked latency summaries, privacy and cleanup claims, and
safe repository-relative paths. It requires the recorded base commit to exist
locally and reads blobs from that commit with `git cat-file`; it does not check
out the commit. The exact extension/native identity and the recorded hashes for
the fixture, live runner, fault host, and manifest policy are therefore checked
against the recorded source tree rather than the working tree.

The checker never fetches history. A shallow checkout must make the recorded
base commit available (for example, with `fetch-depth: 0` in the evidence-check
CI job) or validation fails closed.

A successful ordinary check does **not** say that today's sources or generated
extension match the historical run. Its stable final output includes
`mode=historical` and `current_links=not-checked` so that distinction remains
visible in CI logs.

V1 has no recorded commit and no linked raw run. In historical mode it receives
schema, uniqueness, and safe-path validation only. It cannot establish either
historical source provenance or current artifact linkage; the checker reports
it as `v1_unanchored`.

When a review specifically requires the current linked files and generated
extension to match a receipt, use the strict gate:

```sh
npm run capabilities:check:current
```

The npm script rebuilds the extension before invoking strict mode. Strict mode
first performs the V2 historical checks, then requires the current manifest
boundary, build-manifest metadata, generated artifact bytes and hashes,
native-host declaration, and every source hash recorded by the raw run to
match. It also requires every current file under `broker/` and `protocol/v1/`,
plus the workspace `Cargo.toml` and `Cargo.lock`, to match the receipt's
recorded clean commit byte for byte. New, removed, or changed Rust-chain inputs
fail closed. V1's current manifest and generated artifacts are checked while
retaining the V1 provenance caveat.

A strict V2 pass therefore establishes current adapter artifacts and full Rust
source/build/test-input identity with the historical receipt. Stable output
reports `v2_full_source_current` separately from V1's
`v1_adapter_current`; it does not flatten those evidence classes. A pass does
not reproduce the recorded Rust binary bytes, assert a clean tree outside
those inputs, rerun Cargo, or constitute a fresh live-browser run.

Changing a linked runner, manifest policy, fault host, fixture, extension,
broker Rust source/test, or Cargo build input should make strict mode fail. That
is expected drift, not permission to rewrite an old receipt or rerun durable
evidence in place.

Hashes make drift visible; they are not code signatures or remote attestation.
The raw report also distinguishes `real-rust-chain` evidence from the narrowly
scoped `live-browser-fault-host` used to inject canceled late responses.

## Immutability and renewal

A committed evidence identity is immutable. A durable rerun, changed
measurement, corrected scenario, changed environment, or changed linked source
must create a new raw-run filename and a new capability-record ID. It must not
rewrite a prior record while retaining that record's `id` or filename.

If an old record is wrong, preserve it, mark it superseded from a new record or
review note, and explain the correction. Do not edit historical measurements
into the old document. A receipt may link unchanged source from an earlier
commit, but must say that it is source-linked historical evidence rather than a
current reproduction.

The smoke command remains disposable and never promotes evidence. Run the
durable command only from an isolated clone or worktree created for that exact
evidence identity; never run it merely to make ordinary repository checks
green.

```sh
npm run live --workspace @badi/chromium -- \
  --evidence-id chromium-native-live-run.2026-08-30-review1.v1
```

The durable runner refuses to start from a dirty tree, records its starting
commit, refuses an existing ID/filename, and rechecks both a clean tree and an
unchanged `HEAD` before it creates the result with exclusive-write semantics.
Smoke mode may run from a dirty tree because its output is diagnostic only; the
raw-run schema records that state, while the promoted V2 receipt schema and
checker require `working_tree_dirty: false`. A smoke result therefore cannot be
relabeled or promoted as durable evidence.
