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
- **The development screen** (2026-10-17, eight models, 135 development
  cases, one blind reviewer per language) chose Gemma 3 1B pt Q4_K_M
  (`gguf-org/gemma-3-1b-pt-gguf`, 0.81 GB, Gemma Terms of Use): useful on 46
  against the then-installed Qwen3-1.7B's 33, harmful on 4 against 3, shown
  p50 247 against 351 ms. A stricter second round on 2026-10-18 found no
  better build of it. Runs: `output/writing/2026-10-17-model-research/`.
- **The European-language screen** (2026-10-19) compared eleven small base
  models with Gemma 3 1B on 325 development and boundary requests each (en
  78, de 67, fr, es and pt 60; Persian set aside by user decision), Lab
  `production_boundary`, every language healed, every suggestion
  blind-reviewed. Useful suggestions, % of requests:

  | Model | All | en | de | fr | es | pt | p50 ms | > 550 ms | Peak |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | Gemma 3 1B | 46 | 29 | 36 | 53 | 60 | 57 | 247 | 11 | 1.1 GB |
  | Granite 4.0 H-Tiny (7B MoE) | 44 | 42 | 46 | 53 | 55 | 23 | 453 | 96 | 7.7 GB |
  | Falcon-H1 1.5B | 44 | 37 | 31 | 58 | 40 | 55 | 396 | 34 | 1.8 GB |
  | Salamandra 2B | 42 | 24 | 31 | 50 | 55 | 53 | 385 | 82 | 2.6 GB |
  | EuroLLM 1.7B | 41 | 23 | 37 | 53 | 42 | 55 | 253 | 1 | 1.9 GB |
  | LFM2.5 1.2B | 39 | 35 | 30 | 53 | 48 | 32 | 214 | 1 | 1.3 GB |
  | Llama 3.2 1B | 38 | 36 | 25 | 45 | 52 | 33 | 346 | 38 | 1.5 GB |
  | Granite 4.0 1B | 37 | 32 | 27 | 45 | 45 | 38 | 349 | 14 | 2.0 GB |
  | Qwen3 1.7B | 36 | 40 | 22 | 40 | 45 | 35 | 374 | 56 | 2.3 GB |
  | Qwen3.5 0.8B | 35 | 36 | 27 | 35 | 43 | 35 | 155 | 0 | 1.0 GB |
  | Qwen3.5 2B | 33 | 36 | 24 | 40 | 33 | 30 | 551 | 234 | 2.2 GB |

  Harmful suggestions were 5 to 12 per model. Gemma 3 1B stays the
  recommendation; English is its weakest language, and Granite H-Tiny writes
  the best English and German but needs a larger machine. Differences of
  about five points are within the noise of 60 to 78 requests.

  Gemma 4 E2B base (i1-Q4_K_M, 3.43 GB) ran the same suites, settings and
  seed paired with a Gemma 3 1B rerun at the 28 W limit, every suggestion
  blind-reviewed twice (useful %, round 1 / round 2):

  | Model | All | en | de | fr | es | pt | p50 ms | > 550 ms | Peak |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | Gemma 3 1B (paired rerun) | 41 / 40 | 29 / 31 | 21 / 22 | 53 / 45 | 57 / 57 | 48 / 50 | 284 | 4 | 1.1 GB |
  | Gemma 4 E2B | 42 / 42 | 28 / 31 | 30 / 30 | 47 / 43 | 58 / 58 | 52 / 53 | 487 | 94 | 4.6 GB |

  It writes better German and is level elsewhere, at four times the memory
  and with a cold prompt over 550 ms in more than a quarter of requests, so
  by user decision (2026-10-20) the catalog leaves it out until a model with
  a real gain reopens the question. Runs:
  `output/writing/2026-10-19-gemma4-screen/`.
- **The larger-model bench** (2026-10-18/19, [llm-bench](../../evaluation/llm-bench/README.md))
  scored 2–4B base models 1–5 on 30 synthetic German and English prompts
  and timed typing requests with Badi's own launch:

  | Model | Quant, file | DE | EN | Typing p90 at 15 W / 28 W | Over 550 ms at 28 W | Peak RSS |
  | --- | --- | --- | --- | --- | --- | --- |
  | Granite 4.0 Micro base | Q4_K_M, 2.10 GB | 4.27 | 4.07 | 902 / 566 ms | 21 of 80 | 2.35 GB |
  | Gemma 4 E2B base | i1-Q4_K_M, 3.43 GB | 4.20 | 4.00 | 905 / 430 ms | 0 of 80 | 3.94 GB |
  | Qwen3.5-4B base | i1-Q4_K_M, 2.71 GB | 4.00 | 4.07 | – / 1029 ms | 80 of 80 | 3.11 GB |
  | Gemma 3 1B pt (production) | Q4_K_M, 0.81 GB | 3.13 | 4.20 | 406 / 238 ms | 0 of 80 | 0.95 GB |

  The larger models write clearly better German; English is level. This
  laptop's firmware caps sustained CPU power at 15 W, where only Gemma 3 fits
  the 550 ms budget; the CPU power limit, not memory bandwidth, bounds
  generation here. Hybrid models (Qwen3.5, LFM2.5, Granite H) usually re-read
  the whole prompt after a backspace; no cached result showed stale context.
  Details and conditions are in the bench's README and results.
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

The first qualification (2026-10-19) ran the installed Gemma 3 1B on the
user-approved confirmation set (UD r2.16 test, 260 prefixes, SHA-256
`72a69d10…`). The model, settings, Lab worker and source hashes and the
review protocol were registered before inference. It ran
`production_boundary` at 550 ms with the catalog's healing at the 28 W
limit, and one blind Sonnet reviewer per language judged each whole
displayed addition. It fails every language:

| Language | Cases | Shown | Useful first word | Useful on-time full additions | Harmful | Abstained | Over 550 ms | Useful words | Useful characters |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| English | 100 | 88 | 22 | 19 (19%) | 5 | 12 | 3 | 281 | 385 |
| German | 40 | 38 | 6 | 6 (15%) | 3 | 2 | 0 | 123 | 147 |
| French | 40 | 32 | 9 | 8 (20%) | 1 | 8 | 1 | 110 | 170 |
| Spanish | 40 | 33 | 12 | 10 (25%) | 4 | 7 | 0 | 95 | 201 |
| Persian | 40 | 33 | 9 | 8 (20%) | 2 | 7 | 3 | 99 | 140 |

There were no errors. Useful words are reviewed consecutive useful words.
Useful characters are those of useful full additions, a potential saving
rather than observed typing; the matching reference graphemes were 249, 83,
113, 92 and 65. The harmful additions invented facts about named people and
places in the encyclopedic and news prefixes: a murder charge, a place of
death, an affiliation, a religious title. Two Persian labels had low
confidence and await native review, though flipping both would not pass.
`no_qualified_model` therefore stands, and the recommendation stays a
screening score. Runs: `output/writing/2026-10-19-qualification-gemma3/`.

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
