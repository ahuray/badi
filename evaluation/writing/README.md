# Writing evaluation

## Prediction Lab

The opt-in local workbench accepts your draft, relevant document/screen text,
explicit writing examples and alternative expected **text to append**. Expected
answers are kept out of the model request and used only for scoring. It does not
capture your screen, harvest writing history, edit another application, or alter
the installed broker. Run it from the repository root:

```sh
npm run writing:lab
npm run writing:lab -- --port 36889          # reuse a known local address
npm run writing:lab -- --prefill-batch 64    # explicit runtime experiment (default 16)
```

Open the printed `http://127.0.0.1:PORT` address. The command builds the Lab
worker, `badi-writing-lab`: its own workspace crate in [lab-worker](lab-worker/)
that uses only the broker's public API. The broker never depends on it, and
installs, `cargo build` and `-p badi-broker` builds never compile it. The worker
verifies the already installed model/runtime and creates an isolated local model
process. Missing artifacts produce a startup error, not mock output. The server
binds only to `127.0.0.1`, and an occupied port reports an error rather than
replacing another service. Opening `lab/public/index.html` directly shows only
launch instructions. Ctrl+C closes the server and its model session.

The page has three tabs that preserve each other's drafts and share one
active-test limit: **Prediction** (model continuations and model qualification),
**Word completion** (model-free reuse of a word from supplied context) and
**Spelling** (German/Persian dictionary corrections, when configured).

### Discover, assess and compare a model

Open **Choose and qualify a local model** in the Prediction tab. This workflow
uses the Lab artifact contract and owned runtime; installed Badi keeps its
current model and settings.

1. **Inspect this laptop** refreshes CPU features/topology, available RAM, GPU
   memory classification, cache disk and power information. Enumeration does not
   prove GPU execution: qualification uses the pinned x86_64/AVX2 CPU runtime,
   four threads, 2,048 context tokens and batch 16 or 64.
2. Search a model name or exact Hugging Face repository. **Inspect GGUF
   artifacts** shows immutable revision, file bytes/SHA-256, quantization,
   architecture, languages, license/access and tokenizer/template information.
   **Assess this device** explains whether conservative compatibility and
   resource gates permit an experiment.
3. **Download and verify**, then **Select for Lab comparison**. Cached bytes are
   rehashed and reused. Selection checks the GGUF and current resources; Rust
   verifies the complete weights and runtime again before loading. Browser
   requests select server-issued IDs, never paths, executables, runtime
   addresses or inference flags. **Use installed baseline** restores the normal
   baseline selection.
4. Run explicit development cases and review the entire displayed addition,
   including unwanted tails. Base models use unwrapped context/word-boundary
   continuation. **Selected model instructions** (`native_instructed`) asks the
   owned runtime's local `/apply-template` endpoint to format the embedded chat
   template, then prefills the draft, keeping boundary checks and native EOS.
   Base models without a template refuse it. A template or a completed load does
   not prove useful output.
5. For confirmation, reserve an unused set and freeze the review protocol in
   **Reserve an untouched confirmation run** before inference. Review each
   assigned outcome, including absent additions, then choose **Assess my
   reviews**. A development run cannot become untouched confirmation after its
   outputs are seen. The server rejects previously seen input hashes within its
   lifetime; it cannot verify a person's prior exposure.
6. **Measure runtime behavior** runs cold/preparation, reuse, paced typing,
   cancellation/recovery and resource diagnostics for 30 seconds, five minutes
   or the 30-minute sustained gate. Short diagnostics do not satisfy the
   sustained requirement. A recommendation requires every hard gate to pass;
   selecting an experimental model never means it is recommended.

### Discovery boundaries and persistence

[Discovery](lab/discovery.mjs) uses the
[Hub search/model-info APIs](https://huggingface.co/docs/huggingface_hub/en/package_reference/hf_api)
and [commit-pinned downloads](https://huggingface.co/docs/huggingface_hub/en/guides/download).
Only explicit search terms and public metadata/artifact requests leave the
laptop. It uses no Hub credential, hosted inference endpoint or repository code.
Drafts, context, style, expected answers and reviews are never uploaded.

Search returns 12 results per page for up to three pages; file inspection
returns up to 128 GGUF candidates. Metadata requests have a 15-second deadline
and a 2 MiB JSON limit (64 KiB for a parsed converter README). The in-memory
cache is fresh for five minutes and may serve explicitly stale results for up to
24 hours when the Hub is unavailable; new downloads always require fresh pinned
identity/access metadata. Missing data is unknown, not inferred from popularity
or parameter count. The downloaded GGUF controls the later architecture, state
and tokenizer assessment.

Downloads require a public, ungated, single-file GGUF with a declared license and
consistent byte count/LFS SHA-256, capped at 2 GiB with 256 MiB disk headroom,
a 30-second idle and 20-minute total transfer limit, and validated resumed HTTP
ranges. Linux `flock` prevents simultaneous writers. Partial files can resume;
neither partial bytes nor mismatched hashes become selectable.

Artifacts, descriptors and provenance persist privately (0700/0600) under
`${XDG_CACHE_HOME:-$HOME/.cache}/badi/prediction-lab/`. Clearing the Lab cancels
active work and erases its prose/results; it keeps completed downloads,
resumable partial files and explicitly saved evidence. Confirmation input hashes
stay in memory until the server exits.

**Save counts and measurements locally (no drafts)** is explicit opt-in.
[Evidence storage](lab/qualification-evidence.mjs) writes content-addressed
receipts under the cache's `evidence/` directory (at most 256 files of 64 KiB).
Receipts contain aggregate counts, measurements and identity/protocol/set
hashes, not drafts, generated additions, case IDs or review prose. **Recheck
saved evidence** reuses one for the matching artifact, configuration and
languages; changed identity, future dating, age over 30 days, malformed bytes or
unsafe permissions reject it, and it never bypasses a fresh resource assessment.
Starting new diagnostics clears older performance evidence; unverified cleanup
prevents receipt reload for that run.

The worker also offers read-only JSON helpers. They inspect resources or assess
supplied metadata; they neither download nor launch an inference model:

```sh
cargo build --release --locked -p badi-writing-lab
target/release/badi-writing-lab --inspect-device
target/release/badi-writing-lab --assess-model < /absolute/path/assessment-input.json
target/release/badi-writing-lab --rank-models < /absolute/path/ranking-input.json
```

`--assess-model` takes `{candidate, settings, evidence}`; `--rank-models` takes
an array of at most 32 such objects. JSON input is limited to 128 KiB, and the
qualification engine below defines their strict types and stages. Optional
`--cache-directory /absolute/path` selects the filesystem whose free capacity is
inspected. The browser sends server-issued IDs instead of these low-level
metadata/settings objects, arbitrary paths or runtime flags.

### Qualification contract and local API

[The Rust qualification engine](lab-worker/src/qualification.rs)
separates `discovered`, `estimated_fit`, `loaded_and_exercised`,
`meets_performance`, `meets_prediction_quality` and `recommended`. Fit charges
weights, architecture-specific attention/recurrent state, runtime buffers and
backend overhead to host RAM, preserving at least 2 GiB or 20% of total RAM.
Shared GPU memory is not added to host capacity, and unknown or contradictory
state dimensions block loading. Resources and the selected
artifact/configuration are rechecked before each cold launch. The estimator also
reserves artifact-sized free disk even for cached weights.

For every requested language, confirmation requires at least 40 independent
cases, review of every assigned outcome, at least 60% substantive useful full
additions returned within 550 ms, and zero harmful suggestions. Performance
requires complete-word p95 at most 550 ms, separate cold/preparation
measurements, actual prompt-reuse counters, paced typing, cancellation/recovery,
verified cleanup and at least 1,800 seconds of sustained resource measurement.
Errors, deadlines and abstentions stay in the denominator; missing complete-word
delivery is reported as a lower bound exceeding the target. A longer diagnostic
budget gives no timing credit.

Evidence binds the revision/weight hash, tokenizer metadata, quantization,
device/power identity, worker/runtime/backend and evaluator/HTTP/review source
hashes, exact configuration JSON, languages and evaluation version. After hard
gates, ranking prefers worst-language useful yield, overall useful yield,
language coverage, p95 and memory, in that order. Speed cannot offset harmful
output, and `no_qualified_model` is a valid outcome. Synthetic trials do not
establish rendering latency, editing authority or native undo.

All routes require the served page's `X-Badi-Lab-Token` and matching local
Host/Origin. POST bodies use `application/json`; selection and review IDs come
from this server session. Disconnect/reset cancels owned work and waits for
cleanup before reuse.

| Method / route | Input or purpose |
| --- | --- |
| `GET /api/status`, `GET /api/device` | Lab state; fresh read-only device inspection (no model launch). |
| `POST /api/discovery/search` | `{query, cursor?}`; opaque bounded pagination. |
| `POST /api/discovery/inspect` | `{model_id}`; pinned metadata and candidate IDs. |
| `POST /api/discovery/assess` or `/download` | `{candidate_id}`; fit or verified acquisition. |
| `POST /api/discovery/select` | `{artifact_id}`; select an experiment. |
| `GET /api/discovery/status` | Operation/download progress and experimental selection. |
| `POST /api/discovery/reset` | `{}`; cancel discovery, retain reusable artifact bytes. |
| `POST /api/model/baseline` | `{}`; restore the normal Lab baseline. |
| `POST /api/run` | `{suite, configs, seed, confirmation?}`; up to 300 cases. |
| `GET /api/qualification/status` | Stages, review targets, measurements and recommendation. |
| `POST /api/qualification/review` | `{run_id, config_id, reviews, save?}`; bound full-addition judgments. |
| `POST /api/qualification/diagnose` | `{run_id, config_id, duration_seconds}`; 30, 300 or 1800 seconds. |
| `POST /api/qualification/load` | `{run_id, config_id, evidence_id}`; recheck a saved receipt. |
| `POST /api/spelling`, `/api/context-lookup` | One spelling check or word-completion lookup; each has `GET …/status` and `POST …/reset`. |
| `POST /api/reset` | `{}`; clear the active Lab run and await owned cleanup. |

### Writing cases

1. Enter text before the caret and one acceptable addition per line. Preserve
   intended leading spaces. Add several cases to a set or run the editor alone.
2. Optionally supply relevant context and examples of your writing. Reusing a
   phrase from an example demonstrates contextual reuse, not style learning.
3. Select comparison modes (below). Inspect the actual prompt, raw output,
   complete-word result, model/runtime hashes and effective configuration.
4. Review useful alternatives. **Export** saves drafts and results to a JSON
   download only when you ask; import accepts a test set or an exported session,
   and imported results are never reused as new measurements.

Drafts and results stay in page/server memory: no localStorage, prose logs or
automatic prose files. Clearing the session erases editor/test data and stops
its owned runtime; each comparison also stops its runtime on completion or
cancellation. Reset rejects uploads and file imports that began before it.
Readiness verifies the actual owned runtime process and executable, and Linux
parent-death protection covers a worker killed before readiness. This
containment is specific to the verified model process, not a general sandbox.

| Mode | What it tests |
| --- | --- |
| `production_boundary` | "Current Badi logic": the installed provider exactly as production runs it, with its correction path, guards, trailing-space healing for English and Persian, and fixed 550 ms / eight-token budget. Context and style are ignored. |
| `context` | Full supplied context and complete draft in the prompt. |
| `context_confidence` | `context` plus an uncalibrated per-token log-probability score (below). |
| `instructed` | Non-thinking ChatML instructions with context and style. |
| `healed` | Exact word-boundary (prefix) healing: the recognized boundary is removed from the prefill, the model must echo it, and the echo is stripped once. |
| `instructed_healed` | `instructed` with `healed` boundary handling. |
| `instructed_word` | `instructed_healed` constrained to one Unicode word and a confirmed final space (below). |
| `healed_attested` | `healed`, plus German/Persian midword completion attested by an exact word in the supplied context or style (below). |
| `native_instructed` | The selected model's own chat template with exact word boundaries. |

The Lab uses a separate 2,048-token runtime; installed Badi uses 512. Production
modes are a provider-code baseline, not an installed-service latency measurement.
Experimental budgets from 550 ms to ten seconds diagnose quality/latency
tradeoffs without changing production deadlines. Token preflight refuses
overflow instead of silently discarding cues. `--prefill-batch 64` changes both
logical and physical prompt batches; readiness verifies the reported sizes, and
the browser cannot supply runtime flags.

`context_confidence` reports a mean score only when the returned token bytes
exactly cover the complete displayed addition at token boundaries; absent
probabilities, clipped words and ambiguous alignment stay unscored. The score is
not a probability that the writing is correct. **Experimental confidence and
coverage** shows thresholds against reviewed usefulness per language; no
threshold is applied to production. The pinned runtime's
[`n_probs` contract](https://github.com/ggml-org/llama.cpp/blob/b10726/tools/server/README.md#post-completion-given-a-prompt-it-returns-the-predicted-completion)
supplies the probabilities.

`instructed_word` requires the runtime's actual terminal response and an
explicit final ASCII space (omitted from the preview); token-limit completion is
usable only if that separator already arrived. Its lexical body allows common
Latin letters (English/German) or Arabic base letters (Persian) with internal
apostrophes or valid Persian half-spaces and optional sentence punctuation;
numbers and hyphenated compounds are outside it. The grammar can change the
chosen word and is no guarantee of speed or quality. First-word/four-word
timing fields are unavailable in this mode.

Every experimental (non-production) mode abstains with `style_fact_conflict`
when a completed suggestion introduces a weekday or number found in the style
examples but absent from the draft and context (digit scripts are normalized; a
longer number does not match a shorter one). Ordinary style words still pass.

`healed_attested` can return only the first completed word's suffix when the
existing guard rejects a German or Persian midword join and the exact
reconstructed word occurs, with observed lexical separators, in the context or
an individual style example. Expected answers, the draft and generated text
cannot attest a word. This is contextual reuse, which can repeat a source typo,
not spelling validation. The paced diagnostic requires `healed`.

Comparisons randomize case/trace groups and counterbalance configuration order.
Each independent case/trace/config starts a new owned runtime; steps of an
imported trace stay contiguous and may reuse transient context. This is ordered
snapshot replay: `at_ms` records intended times but does not pace keystrokes.
Startup is excluded from model-result timing.

Reference agreement is case-sensitive, with raw and NFC agreement separate. The
first affected complete word is evaluated across the seam (`docum` + `entation`
forms `documentation`); raw unfinished tokens earn no completed-word score.
Errors, deadlines and abstentions stay in request denominators; cases without
references and spelling replacements are unscored. Matching graphemes are
potential matching text, never observed keystrokes saved, and human judgments
stay separate from exact agreement. Every Lab case is development data, not a
held-out benchmark.

`first_word_ms` and `first_four_words_ms` report the first observed valid
complete prefix, including preflight and excluding startup; the UI still waits
for the full result. `slot_tokens_cached` is final slot occupancy.
`reused_prompt_tokens` (`timings.cache_n`) and `newly_evaluated_prompt_tokens`
(`timings.prompt_n`) come from terminal statistics; missing values stay null and
production modes do not capture them.

### Word completion

The **Word completion** tab is model-free. Enter a partial word and explicitly
supplied context/style, or repeat a word already closed by a separator earlier
in the draft. The lookup returns only the untyped suffix when exactly one
distinct, longer whole word matches. It preserves case, diacritics and Persian
ZWNJ and never joins separate fields. The draft's first word has an unknown left
boundary and cannot serve as evidence or stem. The stem needs 3–24 characters;
ambiguous matches and unsupported Unicode suffix boundaries abstain.

Each check runs a bounded one-shot `badi-writing-lab --context-lookup` process
and returns only after the owned worker exits, with its contract, executed
worker hash, cleanup receipt and timings. Expected suffixes and judgments stay
in the browser. Editing cancels a pending check; clear waits for cleanup; export
saves the current draft and up to 50 checks. Exact reuse is not intent: a unique
match can extend a complete word into a different one (`Marin` → `Marinella`).

### Spelling

The **Spelling** tab checks the last completed German or Persian word through a
separate native Hunspell worker and needs explicit local artifact configuration:

```sh
cargo build --release --locked -p badi-writing-lab
node evaluation/writing/lab/server.mjs \
  --spelling-de /absolute/canonical/path/spelling-manifest.json \
  --spelling-fa /absolute/canonical/path/spelling-manifest.json
```

One manifest may configure both languages. [The manifest validator](lab-worker/src/spelling/artifact.rs)
defines the strict `badi.spelling-artifact.v1` schema: pinned native executable,
loaded library and German/Persian dictionary pairs, each with canonical path,
byte count and SHA-256. Paths and engine settings cannot come from the browser.
The Persian pair uses the reviewed `WORDCHARS` ZWNJ addition. Moved files need
a newly verified manifest, and an engine upgrade needs a reviewed update of the
Hunspell hashes pinned in Rust. A machine-local manifest such as
`output/writing/spelling-lab/manifest.json` is not a portable artifact.

Finish one 3–24-character word with exactly one ASCII space, select its
language, and optionally enter an expected correction and words to keep. A grey
preview compares the original suffix with the proposed word and leaves the
draft unchanged. Accepted words, protected originals, incomplete boundaries and
ambiguous candidate lists produce no offer. Candidates preserve the initial
character, case pattern and Persian joiners, and need one Unicode edit plus a
second native acceptance check. Unfamiliar names may still resemble errors.

Spelling never starts the language model. One dictionary worker persists until a
language switch, cancellation, clear or shutdown. Each check has one 550 ms
native deadline covering both queries. Readiness verifies the actual executable
and loaded-library mapping; where direct mapped-file access is denied, it
corroborates exact live mapping paths/inodes/devices against the hashed files on
the same filesystem, then rechecks them. Cancellation during startup waits for
that verification before stopping the process, so no word query is dispatched.
Exports include the current draft and up to 50 results, only when downloaded.

### Terminal runs and model artifacts

For repeatable runs through the same server/worker/scoring path:

```sh
node evaluation/writing/lab/run.mjs \
  --suite evaluation/writing/lab/examples/quality-probes.json \
  --output output/writing/my-boundary-comparison \
  --modes context,healed --budget-ms 550 --max-tokens 8 --seed 42
```

Options: `--modes` (default `production_boundary,context`), `--budget-ms`
550–10000, `--max-tokens` 8–64, `--seed`, `--cache-prompt true|false`,
`--prefill-batch 16|64` and `--model-artifact`. Production modes always use 550
ms and eight tokens. Use `--modes context,context_confidence` for paired score
capture, then review each suggestion in the HTTP Lab; a CLI run without reviews
cannot report reviewed precision.

The [16-case development set](lab/examples/quality-probes.json) covers paired
trailing-space boundaries, contrasting contextual facts and supplied spelling
style versus conflicting context. The [trace set](lab/examples/typing-traces.json)
holds six ordered English/Persian typing steps. Both are development diagnostics
with nonexhaustive expected continuations.

The CLI creates a new private directory under the ignored `output/writing/` and
never overwrites a run. `run.json` freezes the input hash, order and provenance
before inference; `events.ndjson` retains progress/results; `session.json` can
be imported into the UI; `report.json` records source stability, executed
worker hashes, runtime identities, cleanup and measurements. Progress prints
case IDs and counts without prose. Do not edit or rebuild recorded sources or
the binary during a run.

`--model-artifact` (also accepted by `paced.mjs`) points to an absolute,
canonical descriptor that selects weights for the Lab only; the browser's
discovery workflow writes the same contract:

```json
{
  "schema": "badi.lab-model-artifact.v1",
  "weights_path": "/absolute/canonical/path/model.gguf",
  "sha256": "EXACT_LOWERCASE_SHA256_OF_THE_FILE",
  "bytes": 1107409024,
  "alias": "my-lab-model"
}
```

Descriptors are bounded to 16 KiB and weights to 8 GiB; both paths must be
regular files without symlink redirection. Rust verifies the weights and the
pinned runtime/archive; the descriptor cannot supply an executable or inference
flags. The CLI saves the descriptor privately, rechecks it after execution, and
matches readiness against its hash, size, alias and
`model_origin: "explicit_lab_artifact"`. Loading a candidate does not qualify it.

Two fixed runtime probes accept no prompt, grammar or generation settings:

```sh
cargo build --release --locked -p badi-writing-lab
target/release/badi-writing-lab --prefill-probe
target/release/badi-writing-lab --stop-token-probe --model-artifact /absolute/path/descriptor.json
```

`--prefill-probe` uses two fresh verified runtimes to probe cold, repeated and
appended disposable prompts, streaming and non-streaming, and emits counts,
timings, identity and cleanup as one JSON report without response prose or
token IDs. Requests have a five-second limit within a 60-second budget;
incomplete probes exit nonzero, and it accepts no artifact override.
`--stop-token-probe` accepts only the pinned 1.7B Q4_K_M artifact and checks, in
six fresh runtimes, that token 715 round-trips as ASCII space/newline before
comparing paired space/EOS and combined-stop grammars on three literal words. It
emits JSON lines and measures a termination mechanism, not word choice or
latency; production and ordinary Lab modes do not use that stop token.

### Paced context diagnostic

`node evaluation/writing/lab/paced.mjs --suite PATH --decision PATH --output NEWDIR`
runs the separate, fixed-contract typing experiment. A
`badi.paced-context.decision.v1` file binds the exact suite bytes, two arms
(`cold`, `primed`), order seed 71, and `healed` generation at 550 ms/eight
tokens. Each trace (at most twelve) has four append-only snapshots at
0/100/250/500 ms, each adding one Unicode scalar, with stable supplied
context/style/language. It is a diagnostic protocol, not a typing scheduler or
a production setting.

Every trace/arm owns a fresh runtime, and both arms wait the same 1,500 ms after
startup. The primed arm evaluates only supplied context/style, generates and
discards one token, and verifies terminal counters; the draft is unavailable
until event zero. Startup, preflight, priming, prelude and cleanup costs are
reported separately, and a prelude overrun cannot delay typing.

One active request drains only until its original absolute deadline; one pending
snapshot is replaced by newer typing, and due events invalidate older results.
The 550 ms deadline includes scheduling lateness, queueing, preflight and
inference. Nonterminal responses and hard timeouts require owned runtime
shutdown before another trial. An intermediate Persian trailing half-space is an
observed revision that gets no model request but still revokes older
suggestions.

The CLI writes private, exclusive `run.json` and `report.json` plus copies of
the frozen inputs, verifying worker/runtime ownership, executable hashes,
source/input stability and cleanup. Measured timeouts keep all opportunities;
protocol, ownership and uncertain-cleanup failures halt further inference. The
report also scores two-, three- and four-word reference prefixes (NFC, across
the seam, punctuation ignored). `delivered_at_ms` is Rust's terminal eligibility
observation, **not visible UI delivery**; acceptance, typing savings and screen
context availability remain unmeasured.

### Checks

```sh
npm run writing:lab:check
cargo test --locked -p badi-writing-lab
```
