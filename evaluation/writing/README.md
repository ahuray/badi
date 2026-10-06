# Prediction Lab

A local workbench for comparing models, prompts and spelling on your own test
drafts. You supply a draft, optional context and writing examples, and the
**text you expect to be appended**. Expected answers stay out of the model
request and serve only scoring. The Lab never captures your screen, harvests
writing history, edits another app or changes the installed broker.

```sh
npm run writing:lab                          # prints http://127.0.0.1:PORT
npm run writing:lab -- --port 36889          # a known local port
npm run writing:lab -- --prefill-batch 64    # runtime experiment (default 16)
```

The command builds the worker crate [`badi-writing-lab`](lab-worker/), which
uses only the broker's public API (the broker never builds or references it).
The worker verifies the installed model and runtime and starts its own model
process; missing files are an error, never mock output. The server binds only
to `127.0.0.1`, and Ctrl+C stops it with its model.

The page has three tabs that keep each other's drafts:
- **Prediction:** model continuations and model qualification;
- **Word completion:** model-free reuse of a word from your context;
- **Spelling:** German and Persian dictionary corrections, when configured.

## Choose and qualify a model

**Choose and qualify a local model** in the Prediction tab follows the
[model fit and qualification contract](../../docs/architecture/model-selection.md):

1. **Inspect this laptop:** CPU, RAM, GPU, cache disk and power. Qualification
   uses the pinned x86_64/AVX2 CPU runtime with four threads and 2,048 context
   tokens.
2. **Search** a model or exact Hugging Face repository, **inspect** its GGUF
   files (revision, bytes, SHA-256, quantization, architecture, license,
   tokenizer) and **assess** the fit.
3. **Download and verify**, then **select** it for comparison. **Use installed
   baseline** returns to the normal model. The browser only ever sends
   server-issued IDs, never paths or runtime flags.
4. Run development cases and review each full displayed addition.
   `native_instructed` formats prompts with the model's own chat template.
5. For confirmation, **reserve an untouched run** and freeze the review
   protocol before inference, review every outcome, then **assess**. A seen
   development run cannot become confirmation.
6. **Measure runtime behavior** for 30 seconds, five minutes or the 30-minute
   sustained gate. Selecting a model never means it is recommended.

**Discovery** sends only your search terms and public metadata requests to the
Hub: no credential, hosted inference or repository code, and never drafts,
context, answers or reviews.
- **Limits.** Results come 12 per page for up to three pages, metadata calls
  time out after 15 s, and cached results stay fresh for five minutes.
- **Downloads** must be a public, ungated, single-file GGUF with a license and
  consistent size and hash, at most 2 GiB. Partial files resume, but never
  become selectable unverified.

**Storage.** Artifacts and descriptors live privately under
`${XDG_CACHE_HOME:-$HOME/.cache}/badi/prediction-lab/`. Clearing the Lab erases
prose and results but keeps downloads. **Save counts and measurements locally
(no drafts)** is an explicit opt-in that writes content-free evidence receipts
(at most 256 files of 64 KiB); a changed identity or an age over 30 days
rejects one.

The worker also has read-only JSON helpers that never download or launch a
model (JSON input up to 128 KiB; `--rank-models` takes at most 32 entries):

```sh
cargo build --release --locked -p badi-writing-lab
target/release/badi-writing-lab --inspect-device
target/release/badi-writing-lab --assess-model < assessment-input.json   # {candidate, settings, evidence}
target/release/badi-writing-lab --rank-models < ranking-input.json
```

## Writing cases and modes

Enter text before the caret and one acceptable addition per line, keeping
intended leading spaces. Optionally add context and examples of your writing,
pick modes and run; inspect the prompt, raw output, complete-word result and
runtime identity. **Export** downloads drafts and results only when you ask.
Drafts and results live in page and server memory only: no localStorage, logs
or automatic files.

| Mode | Tests |
| --- | --- |
| `production_boundary` | The installed provider exactly as production runs it (550 ms, eight tokens, its guards); context and style ignored |
| `context` | Full context and draft in the prompt |
| `context_confidence` | `context` plus an uncalibrated per-token confidence score |
| `instructed` | ChatML instructions with context and style |
| `healed` | Word-boundary healing: the boundary is removed from the prompt, echoed by the model and stripped |
| `instructed_healed` | `instructed` with `healed` boundaries |
| `instructed_word` | `instructed_healed` limited by grammar to one word and a confirmed final space |
| `healed_attested` | `healed`, plus German and Persian mid-word completion attested by an exact word in the context or style |
| `native_instructed` | The model's own chat template with exact word boundaries |

- **Runtime.** The Lab uses a 2,048-token runtime, installed Badi 512; budgets
  from 550 ms to ten seconds are diagnostic only.
- **Style facts.** Every non-production mode abstains with
  `style_fact_conflict` when a suggestion introduces a weekday or number found
  only in the style examples.
- **Scheduling.** Comparisons randomize and counterbalance order, and each
  case and configuration starts a fresh runtime.
- **Scoring.** References are compared case-sensitively and across the seam
  (`docum` + `entation`). Errors, deadlines and abstentions stay in the
  denominators.
- **Status.** Every Lab case is development data, not a held-out benchmark.

## Word completion and spelling

**Word completion** is model-free. For a partial word of 3–24 characters, it
returns the untyped suffix only when exactly one longer whole word in the
supplied context, style or earlier draft matches. Exact reuse is not intent: a
unique match can still turn `Marin` into `Marinella`.

**Spelling** checks the last completed German or Persian word with a native
Hunspell worker and needs a local artifact manifest:

```sh
node evaluation/writing/lab/server.mjs \
  --spelling-de /absolute/path/spelling-manifest.json \
  --spelling-fa /absolute/path/spelling-manifest.json
```

[The manifest validator](lab-worker/src/spelling/artifact.rs) defines
`badi.spelling-artifact.v1`: the pinned executable, library and dictionaries
with paths, sizes and SHA-256. A correction needs one Unicode edit, keeps the
first letter, case and Persian joiners, and passes a second native check within
550 ms. Spelling never starts the language model.

## Terminal runs

```sh
node evaluation/writing/lab/run.mjs \
  --suite evaluation/writing/lab/examples/quality-probes.json \
  --output output/writing/my-comparison \
  --modes context,healed --budget-ms 550 --max-tokens 8 --seed 42
```

Options: `--modes` (default `production_boundary,context`), `--budget-ms`
550–10000, `--max-tokens` 8–64, `--seed`, `--cache-prompt true|false`,
`--prefill-batch 16|64`, `--model-artifact DESCRIPTOR`. Each run writes a new
private directory under the ignored `output/writing/`:
- `run.json` freezes the inputs before inference;
- `events.ndjson` holds progress and results;
- `session.json` imports into the UI;
- `report.json` holds hashes, identities, cleanup and measurements.

Progress prints counts, not prose. The
[development set](lab/examples/quality-probes.json) has 16 boundary, context and
style cases, and the [trace set](lab/examples/typing-traces.json) has six typing
steps.

A model artifact descriptor selects weights for the Lab only (bounded to 16 KiB
and 8 GiB; it cannot supply an executable or flags):

```json
{
  "schema": "badi.lab-model-artifact.v1",
  "weights_path": "/absolute/canonical/path/model.gguf",
  "sha256": "EXACT_LOWERCASE_SHA256_OF_THE_FILE",
  "bytes": 1107409024,
  "alias": "my-lab-model"
}
```

Fixed runtime probes take no prompt or settings:
- **`--prefill-probe`** measures cold, repeated and appended prompts.
- **`--stop-token-probe`** takes `--model-artifact` and checks a termination
  mechanism on the pinned 1.7B model.

`node evaluation/writing/lab/paced.mjs --suite PATH --decision PATH --output NEWDIR`
runs the fixed paced-typing diagnostic. It compares `cold` and `primed` arms
over append-only snapshots at 0/100/250/500 ms with a 550 ms deadline, and
reports eligibility times, not visible UI delivery.

## Local API

Every route needs the served page's `X-Badi-Lab-Token` and a matching local
Host and Origin. Bodies are `application/json`.

| Route | Purpose |
| --- | --- |
| `GET /api/status`, `GET /api/device` | Lab state; read-only device inspection |
| `POST /api/discovery/search`, `/inspect`, `/assess`, `/download`, `/select`, `/reset`; `GET /api/discovery/status` | Model discovery and selection |
| `POST /api/model/baseline` | Return to the installed baseline |
| `POST /api/run` | `{suite, configs, seed, confirmation?}`, up to 300 cases |
| `GET /api/qualification/status`; `POST …/review`, `…/diagnose`, `…/load` | Qualification stages, reviews, runtime measurements and saved evidence |
| `POST /api/spelling`, `/api/context-lookup` (each with `GET …/status`, `POST …/reset`) | One spelling check or word-completion lookup |
| `POST /api/reset` | Clear the active run and await cleanup |

## Checks

```sh
npm run writing:lab:check
cargo test --locked -p badi-writing-lab
```
