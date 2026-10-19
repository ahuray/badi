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
intended leading spaces. An optional `after` field is the text after the caret,
at most 128 characters; `production_boundary` passes it to the provider.
Optionally add context and examples of your writing,
pick modes and run; inspect the prompt, raw output, complete-word result and
runtime identity. **Export** downloads drafts and results only when you ask.
Drafts and results live in page and server memory only: no localStorage, logs
or automatic files.

Prediction experiments and qualification accept English, German, French,
Spanish, Persian and Portuguese. `production_boundary` measures all six, each healed as
the model's catalog review decided (`healed_languages`), whatever the
person's settings say; `BADI_LAB_HEALING=all node evaluation/writing/lab/run.mjs …`
heals every language instead, so that models outside the catalog are screened
on equal terms. French, Spanish and Portuguese are opt-in in production and unqualified;
their output uses the Latin script and seam checks, which do not prove correct
language or quality.
Full-addition review remains necessary. The style-fact fence covers weekday
names in all five languages. Spelling and word lookup retain their own coverage.

| Mode | Tests |
| --- | --- |
| `production_boundary` | The installed provider exactly as production runs it (550 ms, eight tokens, its guards); context and style ignored |
| `context` | Full context and draft in the prompt |
| `context_confidence` | `context` plus an uncalibrated per-token confidence score |
| `instructed` | ChatML instructions with context and style |
| `healed` | Word-boundary healing: the boundary is removed from the prompt, echoed by the model and stripped |
| `instructed_healed` | `instructed` with `healed` boundaries |
| `native_instructed` | The model's own chat template with exact word boundaries |
| `framed` | Lab only: version `everyday-english-2026-10-12.v1`, production healing, and the constant everyday-English frame |
| `field_context` | Lab only: earlier text of the same field, kept to about 200 tokens and moved only in jumps of about 64 so the prefix cache can hold |
| `probability_rejoin` | Lab only: force the next words and keep a prefix only when they are not unlikely. The cut is uncalibrated and is not installed |
| `following_text` | Lab only: name the line after the caret, then continue from the text before it. Production prompts do not include that line |
| `context_bias` | The production provider, with the case's context as window text: its distinctive English words raise the first token of each by a logit bias of 2.0 (`CONTEXT_BIAS`); the prompt is unchanged. Production has no window text yet, so it behaves as `production_boundary` there |

`instructed_word` and `healed_attested` are not selectable. The 2026-09-09 next-word study had no useful `instructed_word` result in 30 opportunities and one harmful result. `healed_attested` added harmful suggestions on top of `healed`.

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

## Window text from the screen

[`window-ocr.py`](window-ocr.py) captures the focused window (Hyprland
geometry, `grim`) and recognises its text in memory with Tesseract, keeping
the last 800 characters, nearest a terminal prompt or a chat composer. It
prints timings and sizes only, or with `--expect FILE` a character error rate
against text known to be shown, and writes the text only to an `--output`
file for disposable trials. Nothing installs or calls it; reading the screen
needs its own privacy decision. On 2026-10-17 a disposable Ghostty window
(1416×850 logical, JetBrainsMono 9) read with no errors at native scale in
about 1.25 s and with one character wrong in 547 at `--scale 1` in about
0.8 s.

## Terminal runs

```sh
node evaluation/writing/lab/run.mjs \
  --suite evaluation/writing/lab/examples/quality-probes.json \
  --output output/writing/my-comparison \
  --modes context,healed --budget-ms 550 --max-tokens 8 --seed 42
```

Options: `--modes` (default `production_boundary,context`), `--budget-ms`
550–10000, `--max-tokens` 8–64, `--seed`, `--cache-prompt true|false`,
`--warm-prefix true` (one runtime per configuration, so a repeated prompt
prefix can stay cached; not a counterbalanced cold comparison),
`--prefill-batch 16|64`, `--memory-map true|false` (default true),
`--model-artifact DESCRIPTOR`. The memory-loading experiment changes only how
the runtime loads weights, and records that choice in its verified identity
and qualification identity. It does not change installed broker settings.
The installed broker now uses copied weights; the Lab retains an explicit
mapped default so existing experiment commands keep their loading mode.
Each run writes a new
private directory under the ignored `output/writing/`:
- `run.json` freezes the inputs before inference;
- `events.ndjson` holds progress and results;
- `session.json` imports into the UI;
- `report.json` holds hashes, identities, cleanup and measurements.

Progress prints counts, not prose. The
[development set](lab/examples/quality-probes.json) has 16 boundary, context and
style cases, the [mid-sentence set](lab/examples/mid-line-development.json) has
40 English cases with text after the caret, the [longer prose set](lab/examples/real-text-development.json)
has 19 email and note prefixes, the [trace set](lab/examples/typing-traces.json)
has six typing steps, and the [window-context set](lab/examples/window-context-development.json)
has 24 reply prefixes whose `context` is the nearby heading and paragraph the
observer would read (16 where it names the next words, 8 controls).
`python3 evaluation/writing/window-context.py APP_ID --delay 3` prints that
context for a disposable page's focused field
([observer rules](../../adapters/accessibility/README.md#same-window-context-lab-only)).

Same-window context so far (development cases written for it, so not
independent). As prompt text (2026-10-13, `healed`, 8 tokens, pinned 1.7B on
battery with power-saver, 3 s diagnostic budget) the expected next words came
in 15 of 16 informative cases with the context and 4 of 16 without, but 3 of
the 8 controls drifted onto the page's topic ("I have attached the chapter
three"), several additions answered for the user ("November? Yes, it's
available"), and at 550 ms both arms missed the deadline on 23 or 24 cold
prompts. As a logit bias instead (`context_bias`, 2026-10-17, English only;
no prompt grows, so 550 ms holds) first-word matches rose from 8 to 11 and
matching characters from 43 to 64 of 24 cases at bias 2.0 with every
suggestion still shown, one control drifting; across four suites (65 cases)
12 became 15. Bias 4.0 reached 13 of 24 but wrote loops ("Thursday,
Thursday"), and Persian degraded at both, so it is excluded. The field's own
earlier text as the source measured neutral. For terminals, which expose no
accessibility text, `window-ocr.py` read a disposable Ghostty screen with no
character errors in about 0.8 s (capture 0.2 s, OCR 0.56 s), but five replies
about that screen gained nothing from its words. No live check of the
observer's context has run: the desktop was locked, and a headless Chromium
on a private bus exposed no field.

Independent cases (2026-10-19). An agent with no access to the development
set or its results wrote 40 English cases. In 22, the window names what
comes next; in 6, copying the window's fact would be wrong (a declined day,
a corrected total); 12 are controls. The suite was frozen by hash before
inference and run on the installed Gemma 3 1B at the 28 W limit. One blind
reviewer saw the window text and judged each whole addition:

| Arm | Useful full additions (all / informative / controls / no-copy) | Harmful | Wrong context fact | Over 550 ms |
| --- | --- | --- | --- | --- |
| `production_boundary`, no window text | 9 / 1 / 8 / 0 | 15 | 0 | 1 |
| `context_bias` | 11 / 2 / 8 / 1 | 14 | 0 | 1 |
| `context`, window text in the prompt, 1.2 s diagnostic budget | 10 / 7 / 3 / 0 | 16 | 4 | 28 |

The word bias changed the shown text in only 4 of 40 cases, far below its
development gain. Prompt text found the informative words (an earbud, a
permission slip, the oat milk), but 9 of 12 controls drifted into invented
numbers ("60th birthday!", "1000 USD?"). It also copied a fact the writer
was changing (the postponed event's original "3 February"), and it missed
550 ms cold. Neither arm justifies production use; a context-fact fence and a
prompt that keeps controls quiet come first, and these 40 cases now count as
development data. Runs: `output/writing/2026-10-19-window-context-independent/`.

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

The field-language diagnostic starts no model:

```sh
printf '%s\n' '{"id":"sample","before":"Vielen Dank für Ihre freundliche Nachricht."}' \
  | target/release/badi-writing-lab --detect-language
```

It reads bounded JSON lines and emits only the sample id, selected language
(or `null`) and elapsed microseconds. It calls the broker's exact selector.
After development on 5,700 UD r2.16 dev prefixes, a frozen confirmation rule
(32–512 Unicode scalars, confidence ≥0.9, only EN/DE/FA eligible) selected
30/100 English, 23/40 German and 22/40 Persian test prefixes correctly, with
no selections on the 40 French or 40 Spanish cases. All 75 selections were
correct and the broker diagnostic matched the frozen candidate. These are
language-selection results, not suggestion usefulness or new-language
qualification. Short and uncertain input retains its adapter hint.

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
