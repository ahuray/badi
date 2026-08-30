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
`local-model-eval` feature contains artifact verification, a versioned receipt
contract, deterministic quality thresholds, and a bounded loopback evaluation
client. It is deliberately excluded from production broker wiring and the
default binary dependency graph. Model advice therefore reports
`runtime_ready: false`.

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

The initial catalog deliberately contains only six official Qwen GGUF artifacts
under Apache-2.0. Every entry pins the Hugging Face repository commit, exact
filename, byte count, and SHA-256 digest.

| Use case | Tier | Candidate | Quantization | Download |
| --- | --- | --- | --- | ---: |
| Writing | Compact | `Qwen/Qwen3-0.6B-GGUF` | Q8_0 | 639 MB |
| Writing | Balanced | `Qwen/Qwen3-1.7B-GGUF` | Q8_0 | 1.83 GB |
| Writing | Quality | `Qwen/Qwen3-4B-GGUF` | Q4_K_M | 2.50 GB |
| Code | Compact | `Qwen/Qwen2.5-Coder-0.5B-Instruct-GGUF` | Q4_K_M | 491 MB |
| Code | Balanced | `Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF` | Q4_K_M | 1.12 GB |
| Code | Quality | `Qwen/Qwen2.5-Coder-7B-Instruct-GGUF` | Q4_K_M | 4.68 GB |

The official model cards describe Qwen3's small dense models and GGUF/llama.cpp
usage for [0.6B](https://huggingface.co/Qwen/Qwen3-0.6B-GGUF),
[1.7B](https://huggingface.co/Qwen/Qwen3-1.7B-GGUF), and
[4B](https://huggingface.co/Qwen/Qwen3-4B-GGUF). The
[Qwen2.5-Coder base model](https://huggingface.co/Qwen/Qwen2.5-Coder-1.5B)
explicitly supports fill-in-the-middle tasks; the instruct GGUF artifacts are
initial code candidates, not a claim that they have passed Badi's completion
benchmark. The quality candidate uses the official
[7B GGUF](https://huggingface.co/Qwen/Qwen2.5-Coder-7B-Instruct-GGUF), avoiding
the 3B release's non-commercial research license.

Keeping the catalog static makes changes reviewable. Discovery feeds and model
popularity never alter a user's recommendation at runtime.

Every candidate records `llama.cpp` b5092 as Badi's minimum reviewed backend
baseline. Badi's Qwen3 prompt contract uses the Qwen3 chat template with
thinking disabled; the coder contract uses the Qwen2.5 Coder instruct chat
template.
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

The evaluation-only receipt code derives readiness from those versioned
thresholds and binds the declared artifact, backend, prompt/sampling contract,
launch configuration hash, evaluator hash, corpus hash, hardware profile, and
aggregate metrics. That is a gate scaffold, not current runtime evidence. The
repository does not yet contain the declared launch-manifest producer, corpus,
evaluator implementation, or raw evaluation run.

The production Chromium adapter owns the 140 ms user-idle debounce before
context dispatch. The broker's production default adds no second debounce;
nonzero broker debounce remains a test/configuration seam and still counts
against the same absolute generation deadline.

Plain loopback HTTP also cannot authenticate which local process owns the port,
prove which artifact that process loaded, or close the verify-to-use race on a
replaceable model path. Until Badi owns and supervises the runtime boundary and
attests the loaded artifact, `LlamaCppProvider` remains available only when
explicitly compiling evaluation tooling:

```sh
cargo test -p badi-broker --features local-model-eval local_model
```

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
