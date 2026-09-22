# Device fit and prediction qualification

Badi separates pinned installation advice from model discovery and empirical
prediction qualification. The normal writing broker already uses verified local
weights. A candidate suggested by `badictl models` is an estimated hardware fit;
its `runtime_ready: false` does not report the live broker's readiness or establish
prediction quality. The existing hardware/advice JSON contracts remain unchanged.

## Shared implementation

The Rust [model-selection module](../../broker/src/model_selection.rs) owns the
pinned catalog and existing hardware detector. Its
[qualification module](../../broker/src/model_selection/qualification.rs) extends
that architecture for dynamic candidates. It is available with `local-model` and
does not start inference, contact the network, install a model, or change editing
policy. The [Lab bridge](../../evaluation/writing/lab/device-qualification.mjs)
uses this engine instead of duplicating its gates in JavaScript.

| API | Contract |
| --- | --- |
| `inspect_device(cache_path)` | Refresh local CPU/topology, RAM, GPUs, disk, power and memory pressure; return `badi.device-inspection.v1`. |
| `evidence_identity(candidate, device, settings)` | Compute identities for the complete candidate metadata, relevant hardware/power and inference/evaluation configuration. |
| `assess(&AssessmentInput)` | Pure assessment for explicit inputs; useful for deterministic tests and estimated compatibility on another device. |
| `assess_current(candidate, settings, evidence, cache_path)` | Refresh this device before evaluating fit and saved evidence. Use this before a local load. |
| `rank(&[Assessment])` | Rank only candidates passing every gate, or return `no_qualified_model`. Refresh assessments before presenting a current recommendation. |

The request/report structs use strict Serde deserialization: unknown fields are
rejected. `CandidateMetadata` records the immutable repository revision, exact
artifact bytes/SHA-256, quantization, architecture, tokenizer identity, context
limit, language claims, access, license and architecture-specific state dimensions.
`QualificationSettings` binds runtime identity/version/CPU architecture and required
instruction features, backend, context, batch,
threads, parallel sequences, cache precision, recurrent snapshots, context
checkpoints, prompt format, a hash of complete generation/guard settings, requested
languages and evaluation version. Language claims in a model card are metadata;
the requested languages each need their own measurements.

The worker exposes the same read-only operations:

```sh
target/release/badi-writing-lab --inspect-device \
  --cache-directory "$HOME/.cache/badi/prediction-lab"
target/release/badi-writing-lab --assess-model \
  --cache-directory "$HOME/.cache/badi/prediction-lab" < assessment-request.json
```

The assessment input is an object containing `candidate`, `settings`, and nullable
`evidence`, using the Rust types above. This CLI supplies a fresh device/time
snapshot itself. An assessment is not permission to execute arbitrary binaries or
repository code. For the actual application, start the Lab through its HTTP server;
see the [Prediction Lab runbook](../../evaluation/writing/README.md#prediction-lab).

## Device observations

The extended inspection reuses `detect_hardware()`. It adds CPU model/features,
physical core/package topology, individual DRM device/vendor/driver identities,
reported total/free GPU memory, cache filesystem capacity, platform power profile,
and `/proc/pressure/memory`. RAM uses Linux `MemTotal` and `MemAvailable`;
filesystem capacity uses a bounded two-second, 4 KiB-output `df` invocation.
The existing NVIDIA probe remains bounded to two seconds and 16 KiB output.

Unknown facts stay null or explicitly unknown. In particular, DRM VRAM labels and
GPU vendor alone do not prove dedicated physical memory: a UMA reservation can
come from the same host RAM. No detected GPU capacity is added to host memory.
The inspection reports GPU execution as unverified; it does not infer successful
inference from GPU detection. A separate owned-runtime Vulkan exercise can prove
that specific runtime/device boundary, but the reusable qualification estimator
currently accepts CPU execution only. It does not turn that probe into measured
GPU prediction quality or apply its timings to the CPU configuration.

Stable device fingerprints include CPU/topology/features, total RAM, GPU identity,
driver and memory type/capacity, battery state and power profile. Volatile free
memory/disk, pressure and timestamps are excluded from that fingerprint and
rechecked separately. A device snapshot older than 60 seconds fails the load-fit
gate. The pure API recomputes hardware identity from the actual fields; it does
not trust a caller-supplied fingerprint string.

## Conservative memory calculation

The estimator charges the complete artifact to RAM even with `mmap`, then adds
model state, workspace and runtime overhead. It preserves room for normal laptop
use. All dimension products are checked; missing, invalid or overflowing state
metadata prevents an estimated fit.

| Component | Estimate |
| --- | --- |
| Weights | Exact artifact bytes. |
| Standard attention state | `layers × padded_context × KV_heads × (key_length + value_length) × cache_element_bytes × sequences`. Context is rounded up to 256 tokens; cache element size is explicitly 2 or 4 bytes. |
| LFM2 state | Attention state for attention layers plus float32 short-convolution state, described below. |
| Workspace allowance | `max(512 MiB, weights / 4) + batch_tokens × 1 MiB`. This is a conservative policy allowance, not an exact model-derived allocation. |
| Runtime overhead | 256 MiB. |
| Host reserve | `max(2 GiB, 20% total RAM)`, subtracted from currently available RAM. |
| Download disk | Artifact bytes plus 64 MiB transfer headroom. The estimate remains conservative even when cached bytes can be reused. |

`required_host_bytes = weights + state + workspace + runtime_overhead` must fit
within `available_RAM - host_reserve`. The direct Lab descriptor supports at most
8 GiB; the compact public discovery lane applies its smaller download limit too.
Unknown/invalid RAM or insufficient/unknown cache disk fails closed.

The attention dimensions follow the official
[GGUF metadata contract](https://github.com/ggml-org/ggml/blob/master/docs/gguf.md).
They cannot be substituted for recurrent state. Other unknown recurrent/hybrid
architectures remain unestimated; parameter count alone does not supply their
state layout.

For exact `lfm2` metadata and reviewed llama.cpp `b10726`, the convolution estimate
is `conv_layers × hidden_size × (conv_cache_length - 1) × 4 × sequences ×
(1 + recurrent_snapshots)`, plus 4 KiB alignment allowance per convolution layer.
The KV and convolution total is multiplied by `1 + context_checkpoints` to allow
retained state copies. The formula follows
[LFM2 recurrent dimensions](https://github.com/ggml-org/llama.cpp/blob/b10726/src/llama-hparams.cpp),
[float32 recurrent row allocation](https://github.com/ggml-org/llama.cpp/blob/b10726/src/llama-memory-recurrent.cpp),
and [hybrid layer filtering](https://github.com/ggml-org/llama.cpp/blob/b10726/src/llama-memory-hybrid.cpp).
A different runtime version needs review before that formula is accepted.
The ordinary non-speculative runtime has zero recurrent snapshots; the source
derives them from enabled speculative modes in
[common parameters](https://github.com/ggml-org/llama.cpp/blob/b10726/common/common.h).
The Lab launch explicitly disables server context checkpoints and RAM prompt
cache copies while retaining its measured within-slot prompt reuse.

These allowances are estimates, not allocation guarantees. Actual peak resident
memory, startup, sustained pressure and cleanup still need measurement. Hardware
inspection alone cannot account for every concurrent desktop allocation or future
runtime change.

## States and hard gates

Every assessment exposes separate stage results and rejection reasons:

1. `discovered`: an identified candidate exists.
2. `estimated_fit`: public GGUF access, license/identity/tokenizer/quantization,
   supported architecture/settings and fresh RAM/disk gates pass.
3. `loaded_and_exercised`: matching local evidence says the verified artifact was
   loaded and exercised on the requested backend, with verified owned cleanup.
4. `meets_performance`: complete-word p95 is at most **550 ms** in every requested
   language, over all assigned requests. Cold startup and preparation are reported
   separately, peak RSS fits normal-use capacity, and 30 minutes of sustained
   exercise plus actual prompt reuse, paced typing and cancellation/recovery pass.
5. `meets_prediction_quality`: every requested language has at least **40**
   independent confirmation cases, all outcomes reviewed, at least **60% useful
   on-time full additions**, and **zero harmful suggestions**. The untouched
   confirmation set and frozen review protocol have SHA-256 identities.
6. `recommended`: current fit, exercise, performance and quality all pass.

The fixed development criteria are versioned as
`badi.prediction-quality.v1`. They are model-experiment criteria, not historical
adapter capability qualification or a claim of Cotypist parity. Smaller screening
sets can reject candidates or inform further experiments; they do not pass the
confirmation gate. Diagnostic requests with longer budgets cannot establish the
550 ms target. The reviewer inspects the complete displayed addition: a generic
function word, an incorrect tail or fact copied from a conflicting style example
must not receive useful credit merely because a reference string overlaps.

For each language, useful/on-time, harmful, generic/incorrect and abstained/failed
counts must sum exactly to the assigned total. Missing responses, errors, timeouts
and abstentions stay in that denominator. Duplicate/unrequested languages or invalid
counts invalidate the supplied evidence. Missing language measurements prevent
performance/quality qualification but do not erase a valid load/exercise result.
These checks validate evidence structure;
they do not independently establish that manually supplied review labels are true.
Retain inspectable raw results and the review protocol through the Lab workflow.

## Evidence identity and ranking

Evidence binds the full candidate metadata hash, recomputed hardware fingerprint
and complete settings hash. Changing model bytes/revision, tokenizer,
quantization, runtime/backend, CPU/GPU/power configuration, prompt/generation
settings, language set or evaluation version invalidates it. Evidence from the
future or older than 30 days is stale. The Node bridge additionally binds the
worker executable and evaluator/HTTP/review source hashes, so compiled or
JavaScript-only contract changes require new evidence. Artifact verification and fresh resource checks remain necessary
before every load even when old measurements match.

Ranking is lexicographic after all hard gates: worst-language useful on-time
yield, overall useful yield, language coverage, worst-language complete-word p95,
then estimated memory. Harmful output fails qualification before ranking, and
speed cannot compensate for lower useful yield. No passing candidate produces
`no_qualified_model` with a reason; the workflow preserves the installed model
and presents experiments without promoting a replacement.

`assess()` can estimate compatibility for another explicit device snapshot, but
only exact matching local evidence advances measured stages. A result measured
here must not be relabeled as measured on another machine. Applications still own
text acquisition, mutation, cancellation, policy, focus/caret binding and native
undo; a model benchmark does not qualify those boundaries.

## Existing pinned advice

`badictl hardware --json`, `badictl models writing --json` and
`badictl models code --json` remain local, non-executing controls. Their schemas
remain [hardware v1](../../broker/schemas/badi.hardware.v1.schema.json) and
[model advice v2](../../broker/schemas/badi.model-advice.v2.schema.json).
The pinned six-artifact catalog still selects a tier using CPU/power and
conservative RAM ceilings. It reserves 2 GiB and adds 768 MiB plus 25% of artifact
bytes as runtime allowance. It can produce a smaller candidate or explicit
`no_fit`; it never treats its tier as measured usefulness.

Installed writing selection prefers a verified pinned artifact that is already
present and still fits current resources, including when power changes advice.
Dynamic discovery, downloads and qualification run through the explicit Lab
workflow; catalog popularity or a new search result cannot silently replace the
installed working model. Historical evaluator receipts remain separate and do
not qualify the current writing path.

## Verification

```sh
cargo test --lib --all-features --locked model_selection
cargo clippy --lib --tests --all-features --locked -- -D warnings
npm run docs:check
```

The focused tests cover no-evidence fallback, normal transformer and LFM2 state,
overflow/unknown-state denial, shared GPU accounting, fresh-resource rejection,
identity invalidation, per-language denominators, harmful output, timing and
lifecycle gates, quality-first ranking, strict JSON and the legacy selector.
Physical model comparisons and the rendered HTTP workflow are reported in the
writing runbook; unit tests do not establish those results.
