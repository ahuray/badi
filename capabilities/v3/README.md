# V3 product-cell evidence

V3 represents one exact product-showing cell. It does not broaden the
historical V1/V2 localhost claim.

- `run.schema.json` covers one semantic, headed Chromium, or headed Omarchy raw
  run whose producer is required to omit document and suggestion text. The
  validator constrains the record shape; it does not content-scan arbitrary
  human-readable detail fields.
- `product-cell.schema.json` covers one top-level receipt that hash-links
  exactly one passing run of each kind to one clean Git commit and exact
  artifact/hardware/compatibility identities.
- The receipt computes one canonical qualification digest over the fixed
  `policy.mjs` artifact plus the named corpus, prompt, evaluator, and sampling
  identities and their complete artifact records. Every raw run and approval
  must repeat that digest. A `candidate` requires an owner-role pre-run
  approval; a `live` record additionally requires owner, Omarchy-reviewer, and
  GrillMe-reviewer approvals recorded after all three linked runs, plus a clean
  rollback result. Signatures and review authority remain out of band; the
  repository validator checks role, chronology, and exact digest binding, not
  the human behind a role string.
- Passing checks require at least one observation. Product gates bind exact
  raw checks, measurements, or manual attestations. Numeric measurements carry
  their content-free observations; values and nearest-rank percentiles are
  recomputed, rates bind one binary numerator/denominator, and units,
  statistics, sample floors, warmups, and thresholds are frozen. A `0/0`
  check, contradictory rate, negative physical value, or weaker
  caller-selected threshold cannot pass. The sole signed domain is a bounded
  `[-1, 1]` ratio used by the frozen mean-difference gate.
- Broker, adapter, evaluator, plugin, corpus, prompt, sampling, and versioned
  evidence-policy artifacts are non-empty regular files at role-scoped
  repository paths. The policy role is fixed to `capabilities/v3/policy.mjs`;
  another file under that directory cannot impersonate the policy. Model and
  backend artifacts instead carry their source repository, source artifact,
  revision, license, size, and hash.
- Historical records remain immutable. A rerun or changed source creates new
  IDs and files.

Ordinary validation checks every committed receipt historically. Strict-current
validation may select exactly one top-level receipt with:

```sh
npm run capabilities:check:current -- --receipt-id <id>
```

The selected V3 validator recursively checks its linked runs, product/desktop/
model bindings, and exact implementation artifacts. The receipt names a clean
implementation commit; later commits may add only new, append-only V3 receipt
and evidence files. This avoids a self-referential commit hash while rejecting
post-proof source changes. It never reinterprets a V1/V2 receipt with V3
manifest policy.

V3 compatibility, gate, approval, and cross-record semantics live in the
versioned `policy.mjs` surface. The sibling `validator.mjs` owns V3 document
classification, exact location/version/schema identity, fixed-schema loading,
semantic dispatch, and append-only interpretation. The immutability gate
protects both modules and both V3 schemas; changing that protected V3 contract
therefore requires a new version.

The generic capability checker remains mutable orchestration, but delegates
every V3-shaped document to that protected validator and retains only the V1/V2
schema maps. Focused regressions guard that delegation boundary. As with any
in-repository check, a coordinated edit that removes the checker or its CI gate
must still be caught in code review; immutability does not cryptographically
protect mutable workflow wiring.

Every JSON file under `capabilities/evidence/` is schema- and
semantics-validated even before receipt traversal, and each must be hash-linked
by exactly one top-level receipt. Pre-receipt experiments belong outside that
immutable directory. Record location and version select one exact schema
identity before validation; a document cannot self-select a weaker schema.

CI additionally compares new V3 product-cell receipts with the PR base or
pre-push commit and runs strict-current validation for each addition. The
package-level local gate defaults that comparison to `HEAD^`; pass
`CAPABILITY_BASE_SHA=<ref>` when checking a wider local commit range.

A release check is intentionally separate and accepts only a `live` V3 receipt
with all required role-attestation records and rollback evidence. A passing
command is not, by itself, proof that a named human approved the run or that
free-text detail contains no sensitive content:

```sh
npm run capabilities:check:release -- <receipt-id>
```
