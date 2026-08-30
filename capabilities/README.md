# Capability evidence

Capability records describe one tested surface; they are not platform-wide
support claims.

- `chromium-dom-foundation.v1.json` records the deterministic jsdom/M1
  foundation. Its `live_browser` and `live_native_host` fields remain false even
  though a separate live record now exists.
- `chromium-native-live.v2.json` records the isolated M2A system-Chromium cell.
  It links the generated extension artifacts and exactly one raw live-run
  document.
- `evidence/chromium-native-live-run.v1.json` is generated only by the durable
  browser command. It carries environment versions, scenario counts,
  measurements, isolation outcomes, and opaque artifact digests without field
  or suggestion text.
- `v1/schema.json`, `v2/schema.json`, and `v2/live-run.schema.json` define the
  corresponding machine-readable contracts.

Validate every committed record and its current repository links with:

```sh
npm run build
npm run capabilities:check
```

The checker rebuilds/reads the referenced extension artifacts, validates raw
evidence against its own schema, requires unique checks/scenarios/measurements,
and cross-checks the receipt timestamp, repository state, exact local
environment, privacy and cleanup claims, all mandatory scenario counts, all
preflight commands, both latency distributions, and linked source hashes. It
also rejects repository escapes and absolute personal paths. Changing a linked
runner, manifest policy, fault host, fixture, or extension invalidates the V2
record until a new durable run is produced.

Hashes make drift visible; they are not code signatures or remote attestation.
The raw report also distinguishes `real-rust-chain` evidence from the narrowly
scoped `live-browser-fault-host` used to inject canceled late responses.
