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

Only the first three exist today. The output reports `runtime_ready: false`.

## Commands

```sh
badictl hardware --json
badictl models writing --json
badictl models code --json
```

These commands are local and do not require a running broker, an XDG runtime
directory, or network access. Their versioned JSON contracts (`badi.hardware.v1`
and `badi.model-advice.v1`) let a future Omarchy menu, Quickshell surface,
installer, or package script consume the same result without duplicating
selection policy.

## Observed hardware

The Rust probe reads only machine metadata:

- architecture and logical CPU count;
- AVX2 and AVX-512F availability on x86-64;
- total and currently available memory from `/proc/meminfo`;
- GPU vendor IDs and dedicated VRAM where Linux exposes it through DRM sysfs;
- NVIDIA VRAM through `nvidia-smi` when available; and
- whether a detected battery is currently discharging.

Missing data stays unknown. It is never invented from a GPU name or marketing
label. Unknown or constrained hardware selects the compact tier.

This follows the useful part of
[Voxtype's hardware-aware setup](https://github.com/peteonrails/voxtype/blob/dev/docs/USER_MANUAL.md#hardware-aware-recommendations): detection and recommendation are explicit,
inspectable decisions. Badi does not copy Voxtype's engine switching or require
privileged symlink changes.

## Tiers

| Tier | Conservative selection rule | Product intent |
| --- | --- | --- |
| Compact | Less than 6 GiB RAM, less than 1.5 GiB currently available, fewer than four logical CPUs, or x86-64 without AVX2 | Preserve responsiveness and leave room for the desktop |
| Balanced | At least 6 GiB RAM and four logical CPUs, without enough headroom for quality | Default for ordinary laptops and integrated graphics |
| Quality | At least 16 GiB-class RAM with 6 GiB available and eight logical CPUs, or at least 6 GiB dedicated VRAM with host headroom | Prefer usefulness when the machine can sustain it |

Battery discharge caps the result below `quality`. These are safe starting
rules, not performance claims. Benchmarks may lower a recommendation; they may
raise it only with measured evidence.

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

A candidate is not a provider until all of the following pass on a named
hardware profile:

- cold start, warm p50, warm p95, cancellation, and peak-memory budgets;
- frozen writing or code usefulness corpora;
- output sanitation and eight-word/64-scalar limits;
- stale-result, pause, policy, and shutdown tests;
- no context or suggestion text in logs or receipts; and
- a clear win over the deterministic lane after interruption cost.

If no candidate passes, Badi keeps the deterministic provider. Silence is a
better fallback than a late or mediocre model.

## Linux and Omarchy fit

The feature uses Linux facts and XDG-compatible tooling without modifying
Omarchy's packaged files. `badictl` is the stable integration surface; future
Omarchy UI work should consume its JSON and live in user/package-owned paths,
never patch `/usr/share/omarchy`.

## Naming boundary

Badi (`بعدی`, Persian for “next”) is the selected product name. An unrelated
[active AI workflow CLI](https://github.com/fatihkan/badi) also uses `badi`, so
this project keeps the distinct `badictl` command and the owned native-messaging
identity `io.github.ahuray.badi`. Public package and trademark clearance remains a
release task; the previous repository codename is retired.
