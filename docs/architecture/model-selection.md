# Hardware-aware model selection

Status: implemented recommendation foundation; inference remains gated

## Intent

Badi should recommend the smallest local model likely to feel immediate on the
current machine. It should not equate a larger parameter count with a better
typing experience, download gigabytes without consent, or hide an unmeasured
runtime behind an “AI enabled” label.

The design has four separate steps:

1. inspect content-free hardware facts;
2. choose a conservative model tier for a named use case;
3. present a pinned, verifiable download plan; and
4. enable inference only after Badi's quality and latency gates pass.

The first three exist in the default build. An off-by-default
`local-model-eval` feature contains the pinned-candidate verifier, one bounded
native-prefix client, an owned llama.cpp child lifecycle, and the dedicated
development evaluator. It is excluded from the default binary dependency graph,
and the normal broker has no model-activation flag or semantic provider wiring.
The feature-gated production seam requires an opaque qualification value for
which there is deliberately no public constructor. Model advice therefore
reports `runtime_ready: false`.

## Commands

```sh
badictl hardware --json
badictl models writing --json
badictl models code --json
```

These commands are local and do not require a running broker, an XDG runtime
directory, or network access. Their formal JSON contracts
([`badi.hardware.v1`](../../broker/schemas/badi.hardware.v1.schema.json) and
[`badi.model-advice.v2`](../../broker/schemas/badi.model-advice.v2.schema.json))
let a future Omarchy menu, Quickshell surface,
installer, or package script consume the same result without duplicating
selection policy. Model advice v2 supersedes the original candidate-only v1
shape: `tier`, `recommended`, `fit`, and `download` are nullable when
`status: "no_fit"`.

Development-only semantic checks require the explicit feature and evaluator
binary:

```sh
cargo run -p badi-broker --features local-model-eval --bin badi-evaluator -- fixture-self-test
cargo run -p badi-broker --features local-model-eval --bin badi-evaluator -- pinned-development MODEL.gguf LLAMA_SERVER RELEASE_ARCHIVE.tar.gz
```

Both commands emit evaluation evidence to stdout. They neither download a
model nor expose one through the normal broker.

## Observed hardware

The Rust probe reads only machine metadata:

- architecture and logical CPU count;
- AVX2 and AVX-512F availability on x86-64;
- total and currently available memory from `/proc/meminfo`;
- GPU vendor IDs and detected total dedicated VRAM where Linux exposes it
  through DRM sysfs;
- NVIDIA total VRAM through `nvidia-smi` when available, with a two-second
  deadline and a 16 KiB stdout cap; and
- whether a detected battery is currently discharging.

The timeout kills and reaps the directly invoked `nvidia-smi` process. The
standard-library runner does not own a process group: if a future probe starts a
descendant that inherits stdout, that descendant could keep the capture pipe
open after the direct child exits. `nvidia-smi` is not expected to do this; a
different probe with child processes would require explicit process-group
supervision.

Detected GPU total is not usable capacity. `usable_memory_mib` and `backend`
remain null until a validated inference backend can report both; selection
therefore budgets CPU host memory only. Hybrid-vendor detection and missing
power state cap the result below quality. Missing or inconsistent memory,
unsupported architectures, and fewer than four logical CPUs return an explicit
`no_fit` result instead of inventing a recommendation.

This follows the useful part of
[Voxtype's hardware-aware setup](https://github.com/peteonrails/voxtype/blob/dev/docs/USER_MANUAL.md#hardware-aware-recommendations): detection and recommendation are explicit,
inspectable decisions. Badi does not copy Voxtype's engine switching or require
privileged symlink changes.

## Tiers

The selector reserves 2 GiB from both total host capacity and currently
available memory for the OS and desktop, then uses the lower remainder. For
each artifact it adds 768 MiB plus 25% of the exact pinned `download_bytes` as
conservative runtime/KV-cache headroom. A candidate must fit that per-artifact
budget; the tier is only a ceiling.

| Ceiling | Conservative policy cap | Product intent |
| --- | --- | --- |
| Compact | AArch64, x86-64 without AVX2, known battery discharge, or an otherwise supported four-CPU host below the balanced floor | Preserve responsiveness without claiming unbenchmarked CPU equivalence |
| Balanced | At least 8 GiB RAM and six logical CPUs without the known, unambiguous AC and host headroom required for quality | Default for ordinary laptops, desktops with unknown power state, and hybrid graphics |
| Quality | x86-64 with AVX2, at least 24 GiB RAM, 8 GiB currently available, 12 logical CPUs, known non-discharging power, and no hybrid-GPU ambiguity | Expose a larger candidate only when CPU host memory is plainly sufficient |

These are safe starting rules, not performance claims. Artifact fit can lower a
use case independently—for example, writing and code artifacts at the same tier
have different byte counts. Benchmarks may lower a recommendation; they may
raise it only with measured evidence. A `no_fit` response has no tier, artifact,
or download plan and always reports `runtime_ready: false`.

## Candidate catalog

The initial catalog deliberately contains only six Qwen-family GGUF artifacts
distributed under Apache-2.0. Every entry pins the Hugging Face repository
commit, exact filename, byte count, and SHA-256 digest.

| Use case | Tier | Candidate | Quantization | Download |
| --- | --- | --- | --- | ---: |
| Writing | Compact | `Qwen/Qwen3-0.6B-GGUF` | Q8_0 | 639 MB |
| Writing | Balanced | `ggml-org/Qwen3-1.7B-GGUF` | Q4_K_M | 1.28 GB |
| Writing | Quality | `Qwen/Qwen3-4B-GGUF` | Q4_K_M | 2.50 GB |
| Code | Compact | `Qwen/Qwen2.5-Coder-0.5B-Instruct-GGUF` | Q4_K_M | 491 MB |
| Code | Balanced | `Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF` | Q4_K_M | 1.12 GB |
| Code | Quality | `Qwen/Qwen2.5-Coder-7B-Instruct-GGUF` | Q4_K_M | 4.68 GB |

Qwen's model cards describe the 0.6B and 4B GGUF/llama.cpp artifacts at
[0.6B](https://huggingface.co/Qwen/Qwen3-0.6B-GGUF) and
[4B](https://huggingface.co/Qwen/Qwen3-4B-GGUF). The balanced
[1.7B artifact](https://huggingface.co/ggml-org/Qwen3-1.7B-GGUF) is a
`ggml-org` conversion whose card identifies the Qwen base model. The
[Qwen2.5-Coder base model](https://huggingface.co/Qwen/Qwen2.5-Coder-1.5B)
explicitly supports fill-in-the-middle tasks; the instruct GGUF artifacts are
initial code candidates, not a claim that they have passed Badi's completion
benchmark. The quality candidate uses the official
[7B GGUF](https://huggingface.co/Qwen/Qwen2.5-Coder-7B-Instruct-GGUF), avoiding
the 3B release's non-commercial research license.

Keeping the catalog static makes changes reviewable. Discovery feeds and model
popularity never alter a user's recommendation at runtime.

Every candidate records `llama.cpp` b5092 as Badi's minimum reviewed backend
baseline. Badi's writing contract uses llama.cpp native prefix completion over
bounded before-caret text; it does not pay for or rely on a chat template. The
coder contract uses the Qwen2.5 Coder instruct chat template.
Those prompt declarations are compatibility constraints, not evidence that an
instruct GGUF performs fill-in-the-middle completion well. The JSON repeats the
unvalidated prompt, context-size, latency, memory, and quality caveats on each
artifact.

## Download contract

The recommendation contains an argument vector for the official `hf` CLI:

```text
hf download REPOSITORY FILENAME --revision FULL_COMMIT
```

Badi does not execute it automatically. The
[Hugging Face download API](https://huggingface.co/docs/huggingface_hub/guides/download)
uses a version-aware local cache and supports pinned revisions. A future
installer must also verify the catalog SHA-256 after download and remove only
its own incomplete file on failure.

Only data-only GGUF artifacts are eligible in this catalog. Badi does not load
remote Python code or pickle weights; Hugging Face itself warns that untrusted
[pickle deserialization can execute code](https://huggingface.co/docs/hub/security-pickle).
New repositories, formats, licenses, revisions, or runtimes require review and
new benchmark evidence.

## Runtime gate

A candidate is not a production provider until all of the following pass on a
named hardware profile:

- cold start, warm p50, warm p95, cancellation, and peak-memory budgets;
- a single warm end-to-end clock from eligible adapter scheduling after input
  through adapter view visibility, including every debounce, transport hop,
  provider call, validation step, and display operation; p95 must be at most
  500 ms under the hard 600 ms generation ceiling;
- frozen writing or code usefulness corpora;
- output sanitation and eight-word/64-scalar limits;
- stale-result, pause, policy, and shutdown tests;
- no context or suggestion text in logs or receipts; and
- at least `+0.10` useful accepted words per interruption over the deterministic
  lane after interruption cost; this is an absolute difference, not a bounded
  rate, and a tie cannot pass.

The feature-gated evaluator implements only the smaller semantic foundation
needed before those product gates can be scored. It emits a
`badi.semantic-evaluation-bundle.v1` containing a content-free raw run and an
aggregate receipt derived from that run. The receipt binds the raw-run hash,
model and backend provenance, prompt/sampling contract, launch identity,
evaluator/corpus identity, aggregate metrics, and stable semantic check IDs.
Its authority is always `evaluation_only` and `production_ready` is always
false. The older `badi.model-runtime-receipt.v1` schema remains readable as
legacy evaluation metadata, but `runtime_ready`, even when true, is not a
production activation credential.

The current pinned development candidate is the balanced writing artifact,
`Qwen3-1.7B-Q4_K_M.gguf`, at the exact quantizer repository revision recorded
in the catalog. The quantizer model card names `Qwen/Qwen3-1.7B` as the source
but does not disclose the source revision; the evaluator records that revision
as unreported rather than inferring it. Tokenizer provenance is recorded as
embedded in the exact GGUF artifact. The b10726 llama.cpp release archive and
executed loader are both size- and SHA-256-verified. A pinned, deterministic
exact-directory manifest additionally commits to every sibling regular file's
name, size, and SHA-256 plus each safe same-directory symbolic-link target; it
rejects extra, missing, nested, special, or escaping entries. The evaluator
rechecks that manifest immediately before and after spawn and records its digest
in the runtime and backend identities. System DSOs resolved outside the release
directory remain explicit platform dependencies rather than reviewed bundle
members.

The reviewed archive SHA-256 is
`d3c4e406b2911c8c75d2d0858459645960f8f592c1ab372d565cf145b870c901`;
its canonical directory-manifest SHA-256 is
`d1dad3f66d4064b1c2a6d9dc7c824d3d50d2639f3b1d3dd22c7f4355edb99cba`.
The archive contains 60 bundle entries (50 regular files and 10 symbolic
links), and its tar-stream name, size, file-hash, and link-target records match
the installed directory records exactly.

The evaluator owns the runtime it measures. It starts a private IPv4 loopback
child in a new process group with a fresh bearer credential, requires a correct
authenticated `/tokenize` challenge and rejection of a wrong credential, and
terminates and reaps the child. The reviewed b10726 launch contract is CPU-only
and fixes:

- `LLAMA_ARG_CTX_SIZE=512`;
- `LLAMA_ARG_N_PARALLEL=1`;
- `LLAMA_ARG_THREADS=18` and `LLAMA_ARG_THREADS_BATCH=18`;
- `LLAMA_ARG_N_GPU_LAYERS=0`;
- `LLAMA_ARG_UI=0`; and
- `LLAMA_ARG_OFFLINE=1` and `LLAMA_ARG_CACHE_PROMPT=0`.

Writing requests use llama.cpp `/completion`, the raw bounded before-caret
prefix, an eight-token greedy contract, and period/newline stops. The English
scope gate runs before request serialization. The response stream is drained
normally for latency measurement; length-truncated output is rejected, and
script, word-count, and scalar-count checks fail closed before a suggestion is
returned. Cancellation is a separate lifecycle observation rather than a
shortcut in latency samples.

The production Chromium adapter owns the 140 ms user-idle debounce before
context dispatch. The broker's production default adds no second debounce;
nonzero broker debounce remains a test/configuration seam and still counts
against the same absolute generation deadline.

`broker/src/local_model.rs` is now only the disabled production activation
boundary; it contains no second llama.cpp integration. It accepts the same
pinned semantic runtime only after an opaque `QualifiedSemanticActivation`,
re-verifies the pinned bytes before and after launch, and returns the owned
runtime as the existing `CompletionProvider`. No current evaluator receipt can
construct that value. A frozen product corpus, end-to-end adapter measurements,
and an immutable passing scored run still have to earn a future constructor;
the development fixture and pinned-candidate commands cannot qualify a model.

If no candidate passes, Badi keeps the deterministic provider. Silence is a
better fallback than a late or mediocre model.

## Linux and Omarchy fit

The feature uses Linux facts and XDG-compatible tooling without modifying
Omarchy's packaged files. `badictl` is the schema-versioned integration surface;
future Omarchy UI work should branch on the model-advice schema and `status`,
consume only candidate outputs, and live in user/package-owned paths, never
patch `/usr/share/omarchy`.

## Naming boundary

Badi (`بعدی`, Persian for “next”) is the selected product name. An unrelated
[active AI workflow CLI](https://github.com/fatihkan/badi) also uses `badi`, so
this project keeps the distinct `badictl` command and the owned native-messaging
identity `io.github.ahuray.badi`. Public package and trademark clearance remains a
release task; the previous repository codename is retired.
