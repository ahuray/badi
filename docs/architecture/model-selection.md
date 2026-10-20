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
- **The development screen** (2026-10-17) ran eight models on the same 135
  development cases (20 each en/de/fr/es/fa plus real-text and probe sets;
  French and Spanish in the Lab `healed` mode) at 550 ms and 8 tokens, with
  one blind agent reviewer per language over all 674 unique additions. Gemma
  3 1B pt Q4_K_M (`gguf-org/gemma-3-1b-pt-gguf`, 0.81 GB, Gemma Terms of Use)
  was useful on 46 against the installed Qwen3-1.7B's 33, at least acceptable
  on 106 against 87, with 4 harmful against 3, and faster (shown p50 247
  against 351 ms). Llama 3.2 1B was next (41 useful, 10 harmful);
  Granite-4.0-1B-Base, LFM2.5-1.2B-Base, EuroLLM-1.7B and Qwen3.5-2B-Base were
  near or below the installed model; Gemma 4 E2B at its smallest text-only
  file (2.19 GB, 2-bit) was useful on 19. A second blind round on 2026-10-18
  (other reviewers, stricter) found no better build: Google's QAT Q4_0 tied
  on useful additions (38 each) but was worse on acceptable ones and useful
  words; an imatrix Q4_K_M, the instruct QAT and Llama 3.2 1B's imatrix and
  instruct builds were worse. Runs: `output/writing/2026-10-17-model-research/`.
  Development data and agent reviewers, not a qualification; the Gemma terms
  need the licence review before any distribution.
  Earlier, the then-installed Qwen3-1.7B was weak on independent sets, and no
  ~350M candidate passed the English, German and Persian gates.
- **The European-language screen** (2026-10-19) compared eleven small base
  models with Gemma 3 1B on 325 development and boundary requests per model
  (en 78, de 67, fr, es and pt 60 each; Persian set aside by user decision),
  in Lab `production_boundary` with every language healed
  (`BADI_LAB_HEALING=all`), run one model at a time, every suggestion
  blind-reviewed by Sonnet. Useful suggestions, as % of requests:

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

  Harmful suggestions were 5 to 12 per model (Qwen3.5 2B, which mostly ran
  out of time, 0). Gemma 3 1B stays the recommendation; English is its
  weakest language. Granite H-Tiny writes the best English and German but
  needs a larger machine. A 35-request pre-screen on 2026-10-18 dropped
  Gemma 3 270M and Falcon-H1 0.5B on quality and LFM2.5 8B-A1B on its 9.2 GB
  peak and licence. Differences of about five points are within the noise
  of 60 to 78 requests.

  Gemma 4 E2B base (i1-Q4_K_M, 3.43 GB) ran the same screen on 2026-10-19
  with Gemma 3 1B rerun as its paired baseline: the same seven suites,
  settings and seed, one model at a time at the 28 W limit in the
  performance profile, on mains (four busy threads held 2.2–2.4 GHz at a
  60–65 °C package). Both models' 539 unique suggestions were blind-reviewed
  together by Sonnet, twice with different reviewers; useful %, round 1 /
  round 2:

  | Model | All | en | de | fr | es | pt | p50 ms | > 550 ms | Peak |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | Gemma 3 1B (paired rerun) | 41 / 40 | 29 / 31 | 21 / 22 | 53 / 45 | 57 / 57 | 48 / 50 | 284 | 4 | 1.1 GB |
  | Gemma 4 E2B | 42 / 42 | 28 / 31 | 30 / 30 | 47 / 43 | 58 / 58 | 52 / 53 | 487 | 94 | 4.6 GB |

  Harmful suggestions were 7 / 5 for Gemma 4 E2B and 14 / 10 for Gemma 3 1B;
  consecutive useful words 774 / 772 against 862 / 854, because the 550 ms
  budget cut Gemma 4's cold prompts short. These reviewers were stricter than
  the table's (315 of Gemma 3's 325 suggestions were identical to its screen
  run, which scored 46), so Gemma 4 is compared with its paired baseline
  only. It writes better German (20 against 14–15 useful of 67) and is level
  elsewhere, but with four times the memory and a cold prompt over 550 ms in
  more than a quarter of requests. The bench's typing timing reuses the
  prompt cache, which the Lab's cold requests do not. No 15 W run was made:
  lowering the limit needs root. Runs: `output/writing/2026-10-19-gemma4-screen/`.
  By user decision (2026-10-20) the catalog leaves Gemma 4 E2B out: it
  gains nothing overall for four times the memory, so model advice needs no
  sustained-power input yet. A later model with a real gain reopens that
  question.
- **The larger-model bench** (2026-10-18, [llm-bench](../../evaluation/llm-bench/README.md),
  llama.cpp `b11719`) screened eight 2–4B base models against Gemma 3 1B
  and Qwen3 1.7B on 30 synthetic prompts (15 German, 15 English), greedy, 16
  tokens, one reviewer scoring 1–5 what Badi could show:

  | Model | Quant, file | Arch | DE | EN | Ceiling¹ | Typing p90 at 15 W / 28 W² | Over 550 ms at 28 W | Peak RSS² |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | Granite 4.0 Micro base | Q4_K_M, 2.10 GB | dense | 4.27 | 4.07 | 30 tok/s | 902 / 566 ms | 21 of 80 | 2.35 GB |
  | Gemma 4 E2B base | i1-Q4_K_M, 3.43 GB | dense, sliding window | 4.20 | 4.00 | 42 tok/s | 905 / 430 ms | 0 of 80 | 3.94 GB |
  | Qwen3.5-4B base | i1-Q4_K_M, 2.71 GB | hybrid | 4.00 | 4.07 | 23 tok/s | – / 1029 ms | 80 of 80 | 3.11 GB |
  | Gemma 3 1B pt (production) | Q4_K_M, 0.81 GB | dense, sliding window | 3.13 | 4.20 | 79 tok/s | 406 / 238 ms | 0 of 80 | 0.95 GB |
  | Qwen3 1.7B | Q4_K_M, 1.28 GB | dense | 3.07 | 3.87 | 57 tok/s | not timed | | |

  SmolLM3-3B, Granite 4.0 H-Micro, LFM2.5-2.6B, MiniCPM5-2B and Qwen3.5-2B
  scored 3.40–3.87 in German and 3.93–4.20 in English and were not kept. No
  model switched language or wrote chat text. The top three write clearly
  better German than production; English is level, and differences under
  about 0.3 are noise at 15 prompts. After edits, every hybrid (Qwen3.5,
  LFM2.5, Granite H) reused its cache while typing (about 4 extra tokens per
  keystroke) but usually re-read the whole prompt after a 10–30-token
  backspace, and an early edit re-read everything; checkpoint spacing
  (`-cms 32`) changed nothing. Full-attention models re-read 1–11 tokens after
  the same backspace. No cached result showed stale context.

  Timing (2026-10-19) used Badi's own launch: the pinned `b10726` runtime,
  `-c 512`, 16-token batches, four threads, the full sliding-window cache and
  8 generated tokens, 80 typing requests per model and power limit over 48-
  and 128-token English and German prompts, on battery in the performance
  profile. This
  laptop's firmware caps sustained CPU power (RAPL PL1) at 15 W, which holds
  four busy cores near 1.6 GHz; raised to 28 W at runtime they held about
  2.6 GHz at a 70 °C package. At 15 W only Gemma 3 fits the 550 ms automatic
  budget. At 28 W Gemma 4 E2B fits too (first token p50 70 ms, decode 24.6
  tok/s), Granite Micro misses a quarter of keystrokes, and Qwen3.5-4B misses
  all of them and needs p90 5.0 s after a backspace. Generation ran at
  55–62% of the bandwidth ceiling at 28 W and 33–44% at 15 W, so on this
  laptop the CPU power limit, not memory, bounds it.

  ¹ At the 62.9–64.2 GB/s read bandwidth measured here (LPDDR5-4800,
  128-bit); Gemma 4's per-layer embedding lookup is not counted as streamed.
  ² 8-token typing requests, 80 per power limit and model in the paced
  comparison; `llama-server` peak RSS at Badi's launch. The 15 W Qwen3.5-4B
  run was stopped when conditions changed and is not reported.
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
