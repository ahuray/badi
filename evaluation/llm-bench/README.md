# Local LLM bench

A reproducible bench for choosing Badi's writing model and runtime settings on
one machine: memory bandwidth and per-model generation ceilings, the
`llama-bench` thread and backend sweeps, real `llama-server` latency with
cache behaviour after edits, and a German/English continuation screen. It
uses only public llama.cpp builds and models and the [broker's](../../broker/README.md)
request shape; the broker never references it. Findings live in
[model fit](../../docs/architecture/model-selection.md) and the
[runtime runbook](../../broker/README.md#writing-model-runtime).

## Layout

Source is here; everything large or local (llama.cpp builds, models, the
Python venv, raw results, server logs) goes to the ignored data directory
`output/llm-bench/`, or `$LLM_BENCH_DATA`. `scripts/bench_paths.py` owns
that split; `scripts/models.json` lists every model by label, as a Hugging
Face repository and file or as a path relative to the repository or `~`.

| Path | Purpose |
| --- | --- |
| `corpus/{en,de}.txt` | Disposable sentences that latency documents are built from |
| `quality_set.jsonl` | 30 synthetic prompts, 15 German and 15 English: umlauts and ß, compounds, formal and informal register, names, numbers, mid-sentence |
| `scripts/fetch_model.sh LABEL...` | Download into the data directory; kept only on a Hugging Face LFS sha256 match |
| `scripts/conditions.sh LABEL` | Append power source, profile, governor, load, free memory, package temperature and Badi's request counter to `results/conditions.jsonl` |
| `scripts/membw_read.c`, `scripts/ceilings.py PEAK_GBS` | AVX2 read-bandwidth cross-check; bytes read per token and the ceiling per model (Gemma 4's per-layer embedding lookup excluded) |
| `scripts/step3_threads.sh LABEL` | CPU thread sweep, Badi's shape (48-token prompt, 8 tokens, 16-token batches) and P-core pinning |
| `scripts/step4_backends.sh LABEL THREADS` | Same file on the CPU and Vulkan builds, full and partial offload, no op-offload |
| `scripts/run_latency.sh OUTDIR "ARGS" LABEL[@NAME=EXTRA]...` | `latency.py` per model under a logind idle inhibitor; `LATENCY_BIN` selects another `llama-server`, such as Badi's pinned runtime |
| `scripts/run_quality.sh [LABEL...]` | `quality.py` per model |
| `scripts/summarize_latency.py`, `scripts/cache_table.py`, `scripts/summarize_quality.py` | Tables from raw results; `cache_table.py` lists newly evaluated tokens per latency scenario |
| `scripts/compare_arms.py DIR PREFIX_A PREFIX_B` | Two arms of latency runs side by side: tokens read, TTFT and total percentiles, and same-prompt text agreement |
| `scripts/models_table.py DIR CEILINGS` | One row per model from runs at Badi's launch: decode against the ceiling, TTFT and 8-token totals by scenario, keystrokes over 550 ms, peak RSS, pass/fail |
| `scripts/cold_starts.py --arm NAME="ARGS"...` | Short cold prompts on alternating `llama-server` arms at Badi's launch flags, optionally each after a long prompt |
| `scripts/run_broker_probe.sh ISO BROKER BADICTL LABEL` | Start one broker build in an isolated home (`ISO/{home,run,data}`, settings unpaused) and replay `broker_probe.py`: a disposable typing sequence through `badictl probe`, recording the broker's own latency and outcome |

## Setup

```sh
D=$(python3 evaluation/llm-bench/scripts/bench_paths.py data)
mkdir -p "$D/builds" "$D/results"
for v in x64 vulkan-x64; do
  mkdir -p "$D/builds/$v"
  curl -fL -o "$D/builds/$v.tar.gz" \
    "https://github.com/ggml-org/llama.cpp/releases/download/b11719/llama-b11719-bin-ubuntu-$v.tar.gz"
  tar -xzf "$D/builds/$v.tar.gz" -C "$D/builds/$v" && rm "$D/builds/$v.tar.gz"
done
python3 -m venv "$D/.venv" && "$D/.venv/bin/pip" install llama-roofline==0.2.3
"$D/.venv/bin/llama-roofline" membw --membw-mb 1024 --membw-reps 7 --json "$D/results/membw_roofline.json"
evaluation/llm-bench/scripts/fetch_model.sh granite4-micro-base gemma4-e2b-base qwen3.5-4b-base
```

Compare each archive's SHA-256 with the release's asset digest before
extracting. `llama-roofline` only reaches the network for `inspect` of a URL;
`membw` and `inspect` of a local file are offline.

## Contract

- **Requests** are raw text to `/completion`, temperature 0, seed 42, with
  `cache_prompt` on. Base checkpoints only; an instruct GGUF is not judged on
  raw continuation.
- **Cache claims** come from the server's `timings.cache_n` (reused) and
  `timings.prompt_n` (newly evaluated), never from speed. Each scenario's last
  prompt is repeated with `cache_prompt: false` and top-5 probabilities. A
  different greedy text is a `tie_flip` when the cached token is an uncached
  runner-up within 0.2 probability, otherwise `suspect`.
- **Latency scenarios** at each prompt size: cold, typing (append 1–5 tokens),
  reject (backspace 10–30 and retype), edit at 10%, cancel mid-stream then
  replace, and switch documents and back. TTFT and totals use a monotonic
  clock from just before the request.
- **Safety:** a server is refused unless free memory stays above 2 GiB after
  the model plus 768 MiB, and is killed if it drops below; `--cache-ram`
  defaults to 1024 MiB here because the runtime's default is 8 GiB; runs hold
  a logind idle inhibitor because Omarchy's idle handler switches to
  `power-saver`. Prompts are disposable synthetic text; nothing is uploaded.
- **Conditions:** record `conditions.sh` before and after each run and probe
  clocks under 10 s of sustained load first, and read the sustained power
  limit (`/sys/class/powercap/intel-rapl-mmio:0/constraint_0_power_limit_uw`).
  This laptop's firmware sets it to 15 W, so any timing names the limit it
  ran under. On its USB-C supply the cores were also held at 0.4–2.0 GHz
  (2026-10-18), so time on battery. Cache counts, peak memory and greedy text
  do not depend on clocks.

## Checks

```sh
bash -n evaluation/llm-bench/scripts/*.sh
PYTHONDONTWRITEBYTECODE=1 python3 -m py_compile evaluation/llm-bench/scripts/*.py
```
