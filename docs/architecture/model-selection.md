# Model fit and qualification

How Badi decides whether a model fits this machine and whether it is good
enough to recommend. Advice, discovery and qualification are separate from the
running broker, which keeps its installed, verified model until a person
promotes another.

## Who owns what

- **The broker** ([model_selection.rs](../../broker/src/model_selection.rs))
  owns the pinned catalog and hardware detector behind `badictl hardware` and
  `badictl models`. Their schemas are
  [hardware v1](../../broker/schemas/badi.hardware.v1.schema.json) and
  [model advice v3](../../broker/schemas/badi.model-advice.v3.schema.json). The
  pinned advice reserves 2 GiB and adds 768 MiB plus 25% of the artifact as
  runtime allowance; it may answer `no_fit`. Among the writing models that
  fit, within the tier the device allows (compact on battery or without
  AVX2), it recommends the highest `screen_useful`: the blind-reviewed
  useful suggestions of 135 development requests in the 2026-10-17 Lab
  screen (Gemma 3 1B 46, Llama 3.2 1B 41, Qwen3 1.7B 33; Qwen3 0.6B was not
  screened and stays the compact choice). That is a screening score, not the
  qualification below. The advice lists the whole catalog, each entry marked
  whether it fits, for `badi model` and the panel's Model row, and each
  entry's `healed_languages`, the languages its own review heals.
- **Lesser-known small models** were screened on 2026-10-18 against Gemma 3
  1B: 335 development and boundary requests in all five languages, every
  language healed (`BADI_LAB_HEALING=all`), blind Sonnet review. Useful:
  Granite 4.0 H-Tiny base (7B MoE, 1B active) 122, Gemma 3 1B 120, Falcon-H1
  1.5B base 100, Qwen3.5 0.8B base 96. Granite missed 550 ms on 112 requests,
  peaked at 7.6 GB and wrote almost no Persian (2 useful of 70); Falcon wrote
  none. Qwen3.5 0.8B was the fastest (p95 301 ms, no miss) with the most
  harmful (8). A 35-request pre-screen dropped Gemma 3 270M and Falcon-H1
  0.5B on quality and LFM2.5 8B-A1B on its 9.2 GB peak and licence. None
  replaces Gemma 3 1B on this laptop.
- **The Prediction Lab** ([qualification.rs](../../evaluation/writing/lab-worker/src/qualification.rs))
  owns discovery and qualification; the shipped broker does not contain it.
  The [Lab bridge](../../evaluation/writing/lab/device-qualification.mjs) calls
  this Rust engine instead of duplicating its gates.

| API | Contract |
| --- | --- |
| `inspect_device(cache_path)` | Fresh CPU, RAM, GPU, disk, power and memory-pressure facts (`badi.device-inspection.v1`) |
| `evidence_identity(candidate, device, settings)` | Identities over the full candidate metadata, hardware and configuration |
| `assess(&AssessmentInput)` | Pure assessment for explicit inputs, including another device |
| `assess_current(candidate, settings, evidence, cache_path)` | Refresh this device, then assess; use before any local load |
| `rank(&[Assessment])` | Rank candidates that pass every gate, or `no_qualified_model` |

Inputs are strict Serde types that reject unknown fields. A candidate records
its revision, exact bytes and SHA-256, quantization, architecture, tokenizer,
context limit, languages, access, license and state dimensions. Settings bind
the runtime version, backend, context, batch, threads, cache precision,
checkpoints, prompt format, generation settings, languages and evaluation
version.

## Device facts

Inspection reuses the broker's detector and adds CPU features and topology, DRM
devices, GPU memory, cache disk, power profile and `/proc/pressure/memory`.
Unknown facts stay unknown. GPU memory is never added to host memory, and GPU
detection proves no GPU inference; qualification currently accepts CPU
execution only. Volatile facts (free memory and disk, pressure) are rechecked
separately from the stable device fingerprint, and a device snapshot older than
60 seconds fails the fit gate.

## Memory estimate

The estimate charges the whole artifact to RAM (even with `mmap`) and fails
closed on missing, invalid or overflowing dimensions:

| Component | Estimate |
| --- | --- |
| Weights | Exact artifact bytes |
| Attention state | `layers × context (rounded up to 256) × KV_heads × (key_length + value_length) × cache_element_bytes × sequences` |
| LFM2 state | Attention layers as above, plus float32 short-convolution state reviewed against llama.cpp `b10726` |
| Workspace | `max(512 MiB, weights / 4) + batch_tokens × 1 MiB` |
| Runtime overhead | 256 MiB |
| Host reserve | `max(2 GiB, 20% of total RAM)`, subtracted from available RAM |
| Download disk | Artifact bytes plus 64 MiB |

The sum must fit within available RAM minus the host reserve, and the Lab loads
at most 8 GiB. Other recurrent or hybrid architectures stay unestimated. These
are estimates; real peak memory, startup and sustained pressure still need
measurement.

## Stages and gates

1. `discovered`: an identified candidate exists.
2. `estimated_fit`: public GGUF, license, identity, tokenizer, supported
   settings and fresh RAM and disk gates pass.
3. `loaded_and_exercised`: matching local evidence shows the verified artifact
   loaded and ran on this backend with verified cleanup.
4. `meets_performance`: complete-word p95 ≤ **550 ms** in every requested
   language, plus measured cold start, peak memory, prompt reuse, paced typing,
   cancellation and 30 minutes of sustained exercise.
5. `meets_prediction_quality`: per language, at least **40** independent
   confirmation cases, every outcome reviewed, at least **60% useful on-time
   full additions** and **zero harmful suggestions**. The confirmation set and
   frozen review protocol are hashed.
6. `recommended`: all of the above, currently.

Errors, timeouts and abstentions stay in each denominator, and the counts must
sum exactly. Smaller screening sets can reject a candidate but never pass the
confirmation gate, and a longer diagnostic budget earns no timing credit.
Reviewers judge the whole displayed addition: a generic word, a wrong tail or a
fact copied from a conflicting example is not useful.

## Evidence and ranking

Evidence binds the candidate metadata, hardware fingerprint and complete
settings; the Lab also binds its worker and evaluator source hashes. Any change
invalidates it, as does an age over 30 days. Artifacts and resources are
rechecked before every load anyway.

After the hard gates, ranking is lexicographic:
1. worst-language useful on-time yield;
2. overall useful yield;
3. language coverage;
4. worst-language p95;
5. estimated memory.

Harmful output fails before ranking, and speed never compensates for
usefulness. `no_qualified_model` keeps the installed model. A result measured
here is never relabeled as measured elsewhere, and a model benchmark qualifies
no adapter's editing behavior.

## Checks

```sh
cargo test --lib --locked -p badi-broker model_selection
cargo test --lib --locked -p badi-writing-lab qualification
```
