# Prediction Lab

A local workbench for comparing models and prompts on your own test drafts. You supply a draft, optional context and writing examples, and the
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

The page has two tabs that keep each other's drafts:
- **Prediction:** model continuations and model qualification;
- **Word completion:** model-free reuse of a word from your context.

## Choose and qualify a model

**Choose and qualify a local model** in the Prediction tab follows the
[model fit and qualification contract](../../docs/architecture/model-selection.md):

1. **Inspect this laptop:** CPU, RAM, GPU, cache disk and power. Qualification
   uses the pinned x86_64/AVX2 CPU runtime with automatic thread selection
   and 2,048 context tokens.
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
Spanish, Persian and Portuguese.
`production_boundary` and the healed modes
measure all six, each healed as production heals the loaded model
(`healed_languages` in its catalog entry, the fallback English and Persian for
weights outside the catalog), whatever the person's settings say;
`BADI_LAB_HEALING=all node evaluation/writing/lab/run.mjs …` heals all six
production writing languages instead, so that models outside the catalog are screened on equal
terms. French, Spanish and Portuguese are opt-in in production and
unqualified; their output uses the Latin script and seam checks, which do not
prove correct language or quality. Full-addition review remains necessary.
The legacy context modes' style-fact fence knows English, German, French,
Spanish and Persian weekday names; Portuguese shares only `sábado` and
`domingo` there. The three style arms also recognize Portuguese weekday
roots, including hyphenated `sexta-feira`. Word completion
keeps its own coverage.

| Mode | Tests |
| --- | --- |
| `production_boundary` | The installed provider exactly as production runs it (550 ms, eight tokens, its guards); context and style ignored |
| `style_gentle`, `style_balanced`, `style_strong` | Production logic with respectively the newest one, two or four explicitly supplied writing examples before the draft; the newest stays closest. Tokenization and inference share 550 ms, and the prompt plus generation and one spare token must fit 512 tokens. Window context is ignored |
| `context` | Full context and draft in the prompt |
| `context_confidence` | `context` plus an uncalibrated per-token confidence score |
| `instructed` | Qwen3 ChatML instructions with context and style. Only on the pinned Qwen3 1.7B weights the template is written for; the worker refuses any other model (`instructed_template_model_mismatch`) |
| `healed` | Word-boundary healing: the boundary is removed from the prompt, echoed by the model and stripped |
| `instructed_healed` | `instructed` with `healed` boundaries, on the same model only |
| `native_instructed` | The model's own chat template with exact word boundaries; asks the template to disable thinking before the draft prefill |
| `vocabulary` | The production provider with context on: the case's context gives the window's words and its style examples the writer's earlier words. When the caret ends a partial word that exactly one longer word there, or earlier in the draft, starts with, its rest is the suggestion and no model runs ([rule](../../broker/README.md#vocabulary-completion)); otherwise it behaves as `production_boundary` |

Modes that failed their gate are deleted; [Retired experiments](#retired-experiments)
keeps one result line each.

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

### Style comparison

The three style arms use the same provider as `production_boundary`, including
spelling, healing, confidence, seam and unsupported-number checks. They change
only continuation prompts, never spelling prompts or the text being edited.
Examples are separated by blank lines, oldest first, at most eight examples
of 240 Unicode characters each (the sealed store's sample bound). Each case's
examples must come from its own subject and declared language. The Lab does
not infer their authorship or language, query the desktop store, or carry an
example from one case into another. Private exports and drafts stay outside
the checkout; the bundled [style suite](lab/examples/style-development.json)
is synthetic development data.

The draft keeps production's existing sentence window; style never removes
additional draft text to fit. Overflow abstains with `style_context_overflow`.
The result's `style_budget` reports supplied/selected sample counts and, for
each attempted continuation prompt, tokenizer counts with and without examples,
generation reserve and the 512-token limit. Tokenization consumes the same
deadline as generation; no prefix means the unchanged baseline path. The Lab
runtime still has 2,048 tokens, recorded in its identity: this is a 512-token
prompt admission test, not an installed-runtime performance claim.

`style_fact_conflict` withholds copied weekdays, numbers (also across digit
scripts and the caret seam), and capitalized words present only in selected
examples. Bare names and possessives with ASCII or curly apostrophes share
the same key in examples, the draft and additions. The capitalized-word check
is conservative: sentence starts and German nouns can be withheld too. It is
not entity recognition or a semantic fact guarantee; review full additions
for factual drift, language and harm.
Ignored window context cannot authorize a fact. Spelling corrections remain
unchanged. No production prompt reads samples and no strength setting is
enabled by running this comparison.

```sh
node evaluation/writing/lab/run.mjs \
  --suite evaluation/writing/lab/examples/style-development.json \
  --output output/writing/my-style-comparison \
  --modes production_boundary,style_gentle,style_balanced,style_strong --seed 42
```

These modes always use production's decoding settings, even if diagnostic
CLI budget/token options were supplied. Synthetic runs exercise the feature;
they cannot satisfy the private held-out drafts and native-review promotion
gate in [future plans](../../future%20plans.md).

### Instruction adherence

The [synthetic instruction suite](lab/examples/instruction-adherence.json)
has eight probes, two each for terminology, language, tone and formality.
The [runner](lab/instruction-adherence.mjs) turns each probe into an
instruction/control pair with identical draft, factual context, declared
language and requested settings. Only the instruction variant appends the rule to
explicit context. Scoring checks stay outside model input; expected answers
are empty. Opposing terminology and formality rules expose default-answer
bias instead of counting an already compliant control as an instruction gain.

```sh
node evaluation/writing/lab/instruction-adherence.mjs \
  --suite evaluation/writing/lab/examples/instruction-adherence.json \
  --output output/writing/my-instruction-comparison \
  --modes production_boundary,healed --budget-ms 3000 --max-tokens 16 --seed 42 \
  --threads 4
```

Prediction CLI runs may select `--threads 1`, `2`, `3` or `4` to keep cold
launches matched. Omission preserves automatic hardware selection. The
requested count is frozen in provenance and worker receipts; every ready
and result identity must agree, and mismatch cleanup still owns the exact
runtime. The revision lane rejects this option, and the browser cannot set
it. CPU affinity alone does not establish matching launch settings.

For the pinned Qwen3 1.7B, add `instructed_healed` to the modes and pass its
explicit `--model-artifact` descriptor. Other instruction models can use
`native_instructed`, which calls the local formatter with the nested boolean
`chat_template_kwargs.enable_thinking=false`. On pinned b11840 and Qwen3
0.6B this inserts a closed thinking block before the draft; a top-level flag
is ignored and a string value is rejected. Inspect the actual prompt when
screening another template: the flag alone is not proof that it honored it.
Gemma 3 1B PT has no instruction
template: it uses the raw `healed` arm, and the Qwen template guard remains
in force. These experiments retain the existing production healing, language
and seam helpers and the Lab fact fence. They do **not** run the complete
production provider's confidence, spelling and 512-token path: their window
is 2,048 tokens and their selected budget is diagnostic. The separate
`production_boundary` control keeps its own 550 ms / eight-token settings
and ignores all supplied context, including instructions.

Each run keeps the ordinary identity and cleanup records, the frozen
`instruction-suite.json`, and `instruction-adherence.json`. The latter
scores complete, valid shown additions using whole lexical words, optional
case sensitivity and explicit exclamation-mark counts. Every error,
abstention, incomplete word and missing result stays in the denominator.
Passes within 550 ms are reported separately from diagnostic passes, and
pair results distinguish instruction-only gains, control-only losses, both
and neither. Actual runtime identities must also match: different thread
counts or missing identities mark the pair `unpaired`, with no gain/loss
credit and both outcomes retained. Language rules agree with the declared language; Qwen's system
prompt already specifies it, so those probes measure any additional benefit.

These are lexical compliance checks, not general language or tone judgments,
and not usefulness or safety scores. Every full addition needs separate
review for factual drift, contradictory tails and usefulness; late output
earns no production-usefulness credit. Nothing enables custom instructions
in desktop settings or changes permissions or editing authority.

## Word completion

**Word completion** runs the broker's vocabulary rule
([broker runbook](../../broker/README.md#vocabulary-completion)), the one
production uses with context on. The case's context stands for the window's
words and its style examples for the writer's learned words. With five or
more typed letters, it returns the untyped rest of the one longer whole word
there, or earlier in the draft, that starts with them. When several fit, it
returns one only if that word begins all the others. Otherwise it gives the
broker's reason: `no_stem`, `known_word`, `no_match`, `ambiguous` or
`unsafe_suffix`. Exact reuse is not intent: a unique match can still turn
`Marin` into `Marinella`.

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
`--model-artifact DESCRIPTOR`, `--lengths short,normal` (default `normal`:
each [suggestion length](../../broker/README.md#suggestion-limits) becomes
its own `production_boundary` configuration, `production_boundary-short`,
while normal keeps the plain id and request). The command first builds the worker as
`npm run writing:lab` does, so a run never executes a binary older than its
sources. The report hashes every file of the Lab and of the worker crate and
every broker source before and after. The memory-loading experiment changes only how
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

## Selection revision

The revision lane measures the grammar check and refinement of
[ADR 0012](../../docs/decisions/0012-explicit-revision-of-one-selection.md)
and the synonyms of
[ADR 0013](../../docs/decisions/0013-synonyms-for-one-short-selection.md)
from the terminal; the page has no revision tab. Each selection goes through
the broker's own revision request (`revision::runtime::complete`) to the
installed revision model in the broker's revision launch, and the engine's
decision follows: a truncated answer is never shown, any other goes through
`revision::accept` with the text before and after the selection.

```sh
node evaluation/writing/lab/run.mjs \
  --suite evaluation/writing/lab/examples/revision-development.json \
  --modes revision_grammar,revision_refine --seed 42 \
  --output output/writing/my-revision-run
```

- **Modes.** `revision_grammar`, `revision_refine` and `revision_synonyms`
  run apart from the
  prediction modes and take no budget, token, cache or launch option. One
  runtime serves the run, as the broker keeps one; modes, then languages,
  then cases run in seeded order, so a language's prompt prefix stays cached
  after its first case. Startup is excluded from latency, and the run files
  are those of any terminal run.
- **Suites** use `badi.revision-suite.v1`: `id`, `language` (`en`, `de`,
  `fr`, `es`, `pt`), `mode`, a one-line `selected` of at most 600 characters
  and optional `before` (512) and `after` (128). A `grammar` case is
  `kind: "error"` with its `expected` corrections, or `kind: "correct"`,
  which should come back unchanged. A `refine` case lists `facts` found in
  its selection. A `synonyms` case selects a word or short phrase (the
  broker's bound applies) with its `expected` synonyms. Expected text and
  facts never reach the worker.
- **Records** keep the outcome (`shown`, the gate's class such as
  `unchanged` or `edits`, `truncated`, `timeout`, `failed`), the model's
  answer, the shown text, changed-word share, latency, generated tokens, the
  answer's confidently detected language and the broker's field-language
  choice. **Fixed** means shown and equal to an expected correction after
  whitespace normalization; **facts kept** means every declared fact appears
  in the shown text; **matched** (synonyms) means shown and equal to an
  expected synonym the same way. The development set below has no synonym
  cases yet, so no synonym score is claimed.

The [development set](lab/examples/revision-development.json) has 27 cases
per language: 15 grammar errors (three mid-sentence), 4 correct inputs (one
mid-sentence) and 8 refinements carrying a number, name, address or phone
number. It was written for this lane, so it is development data.

| Language | Grammar fixed | Shown, not expected | Errors returned unchanged | Correct inputs unchanged | Refinements shown, all keeping their facts | Grammar / refine p50 · p90 |
| --- | --- | --- | --- | --- | --- | --- |
| English | 12/15 | 1 | 2 | 4/4 | 5/8 | 1.0 · 1.5 s / 1.9 · 3.7 s |
| German | 11/15 | 1 | 3 | 4/4 | 6/8 | 1.3 · 2.1 s / 2.0 · 4.7 s |
| French | 11/15 | 3 | 1 | 3/4 | 4/8 | 1.4 · 1.6 s / 2.2 · 3.3 s |
| Spanish | 7/15 | 4 | 4 | 4/4 | 4/8 | 1.2 · 1.5 s / 2.2 · 3.5 s |
| Portuguese | 9/15 | 5 | 1 | 4/4 | 4/8 | 1.2 · 1.7 s / 2.3 · 4.9 s |

- Runs with seed 42 are deterministic: three runs of the first prompt gave
  identical answers and outcomes for all 135 cases. The gate's only
  abstention is `unchanged`, no answer came back in another language, and
  every expected correction passes the gate verbatim.
- Paired with the run before the change (same suite, modes and seed), giving
  a mid-sentence selection its sentence's start and dropping an added period
  whenever anything follows on the line fixed 50 of 75 errors instead of 46
  and showed 14 wrong answers instead of 18. Only the eight mid-sentence
  cases changed: the period before a following comma is gone, and a German
  clause after "dass" keeps its verb last, so a correct input is no longer
  reordered (19 of 20 come back unchanged instead of 18). One German
  refinement that reordered such a clause now comes back unchanged instead.
  A changed sentence start (`context`) never occurred.
- Of the 14 shown answers that differ from the expected text, several are
  acceptable alternatives ("bezahlt wurde", "pour que nous puissions"); the
  rest are wrong or partial words the model invents within the edit bound
  ("Nós vão", "vena", "mon clé", "puedamos").
- Against the earlier generic prompt (no language name or example; same
  launch, request and gate), the language-specific prompt fixed 46 of 75
  errors instead of 25. The generic prompt mostly returned errors unchanged
  (40 of 75), and neither came back in another language.

Tested with Qwen3-1.7B-Q4_K_M (launch `badi-revision-qwen3-v1`: 4 threads,
1,024 context, batch 64, copied weights, 1.5 GiB peak RSS, 3.0 s startup)
on llama.cpp b10726, i7-12700H at the 15 W PL1 on AC with the performance
profile (busy cores 1.4–1.7 GHz), 2026-10-23, from the paired run with no
other build running. Runs: `output/writing/2026-10-23-revision/`
(`shipped-3` before, `after-lead` after, `generic/`).

## Results

Every result here is development data unless it says independent; evidence
stays under the ignored `output/` paths named below.

### Synonym prompt diagnostic

The new editor trials exposed Qwen3 1.7B copying its worked example for
`The big`; the existing output gate correctly refused it. A 16-case,
five-language synthetic comparison on the pinned b11840 runtime tested the
original prompt, a consistently labelled example and no example, with the
same runtime, sampling and unchanged output checks. The original showed
7/16 answers, the candidates 5/16 and 4/16; neither fixed `big`, and shown
answers could still drift in meaning (for example `grande → cómoda`). Neither
candidate was adopted. Exact prompts, raw outputs, runtime identity, cleanup
and review are retained in
`output/neovim-selection-revision/synonym-prompt-comparison/`.
This diagnostic is not held-out or native-reviewed qualification. The
editor follow-ups selected `look after → take care of` from it to prove
preview, apply and undo; that smoke case earns no independent quality credit.

### Style arms: tested with

Gemma 3 1B Q4_K_M on the pinned b11840 CPU runtime (four threads, batch 16,
2,048 runtime tokens, 512-token style admission), 2026-11-12. The 24-case
[synthetic suite](lab/examples/style-development.json) was partitioned by
language: four cases per language, all four modes, seed 42, identical inputs
and settings within each paired comparison. These are cold independent
requests, not the desktop's warm typing latency or a private held-out score.
Power state was not controlled, so these timings are diagnostic only.

| Language | Shown / 4: off, gentle, balanced, strong | Style-fence abstentions / 4 per strength | Results after 550 ms |
| --- | --- | --- | --- |
| English | 3, 3, 3, 3 | 1 | 0 |
| German | 3, 1, 1, 1 | 3 | 0 |
| French | 4, 4, 4, 4 | 0 | 0 |
| Spanish | 3, 3, 3, 3 | 0 | 0 |
| Portuguese | 3, 3, 3, 3 | 1 | 0 |
| Persian | 1, 2, 2, 2 | 1 | 2, both strong |

All 96 requests completed, zero errors; each language's report verifies
stable source hashes for that recorded revision, executed identity, and
server/group/worker cleanup.
The six empty-style controls returned the same text or abstention in every
mode. Full additions, timing, abstentions and the unblinded agent review are
in `output/writing/2026-11-12-style/` (`en/`, `de/`, `fr/`, `es/`, `pt/`,
`fa/`, and `review.json`). No harmful tail was identified in that synthetic
review; it is not native review and earns no production qualification.
German's capitalization fence withheld useful ordinary nouns, and the
strong Persian arm missed the target twice. Private drafts and native review
remain necessary before choosing a strength or enabling desktop use.

These model runs and the UI capture below predate the possessive-fence fix;
their original source hashes and measurements remain unchanged. The final
fence was replayed offline against all 48 saved visible style additions,
with no new conflicts. The 24 baseline records were unchanged; the 24 style
abstentions lack retained candidates and cannot be replayed. Source and input
hashes, harness source and per-record outcomes are in
`output/writing/2026-11-12-style/possessive-offline-replay.json`. This replay
adds no inference or timing evidence. Regression tests separately cover bare
and possessive names in samples, additions, caret seams and existing drafts.

The rendered Chromium 155.0.8059.84 Lab compared all four modes, exported the session,
and cleared it with no console errors. Selecting 1.5 s / 16 diagnostic tokens
still sent 550 ms production configurations for these arms. The example
`Please send me the` changed from ` link to the article.` to
` updated draft.`; prompt counts were 12, 20 and 36 tokens for one, two and
four examples, against four without examples. Evidence:
`output/writing/2026-11-12-style/ui-session.json`,
`output/playwright/2026-11-12-style-controls.png`, and
`output/playwright/2026-11-12-style-results.png`.

### Instruction adherence: tested with

Gemma 3 1B PT Q4_K_M and Qwen3 1.7B Q4_K_M, pinned b11840 CPU runtime,
2,048 context tokens, batch 16, seed 42, 2026-11-12. Both used the same
eight synthetic probes: 3 s / 16 tokens for experimental arms and separate
550 ms / eight-token production controls. Explicit `--threads 4` kept one
actual runtime identity per model. All 32 Gemma and 48 Qwen requests
completed, every arm had eight matched pairs and zero unpaired results,
sources stayed unchanged, and all 160 owned worker/runtime processes exited.

| Model / arm | Control passes / 8 | Instruction passes / 8 | Matched gains / losses | Instruction passes within 550 ms / 8 |
| --- | --- | --- | --- | --- |
| Gemma / production | 0 | 0 | 0 / 0 | 0 |
| Gemma / healed | 1 | 2 | 1 / 0 | 2 |
| Qwen / production | 1 | 1 | 0 / 0 | 1 |
| Qwen / healed | 1 | 1 | 0 / 0 | 0 |
| Qwen / instructed-healed | 2 | 2 | 0 / 0 | 0 |

Gemma's one lexical gain used the requested `folder`; the opposing
`directory` instruction still produced `folder`. Its courtesy marker passed
in the incomplete addition `please send it to`, which was unhelpful on full
review. Qwen showed no instruction-only lexical gains. Its template passed
the same Spanish and courtesy probes as the controls, after 550 ms. Wording
can change without satisfying a lexical marker: formal German `Ihre` is a
possible register effect despite missing the exact `Sie` marker.

An unblinded agent reviewed every full addition and outcome: Gemma had 10
useful, 13 unhelpful and nine abstentions across both control and instruction
requests; Qwen had 26 late, 10 unhelpful and 12 abstentions. No harmful factual
tail was identified in this small synthetic set. This is not native review,
a safety guarantee or qualification; neither model earns desktop instruction
settings. The observed host profile was balanced on AC. Explicit thread
selection fixes that launch parameter, not CPU frequency, thermal state or
scheduler interference; timing remains diagnostic.

Evidence: `output/writing/2026-11-12-instructions-pinned/`, including each
model's input, run, session and adherence records, `review-protocol.json`,
`review.json` and `verification.json`. Earlier attempts remain separately at
`output/writing/2026-11-12-instructions/` and
`output/writing/2026-11-12-instructions-matched/`: automatic selection mixed
two and four threads, even with CPU affinity. Their scorers correctly deny
unpaired gain credit; the second attempt's verification explicitly corrects
its preregistered assumption about affinity. They are not the fully paired
result above.

The catalog-pinned Qwen3 0.6B Q8_0 then ran the same eight probes before and
after disabling thinking in its native template: 32 requests per run, four
threads, identical suite, settings and order seed. The actual formatter
confirmed the closed thinking prefix, and that prefix was the only change
to all 16 native prompts. Every pair matched runtime identity, both runs
kept their sources stable, and all 128 owned processes exited.

Native instruction lexical passes rose from 2/8 to 4/8 (matched gains from
one to three; no losses), with controls unchanged at 1/8. The added `folder`
and informal `dich` passes were late; no native instruction pass arrived
within 550 ms. The full unblinded review gave no instructed addition timely
usefulness credit. Across all requests, before had zero useful, 12 unhelpful,
13 late and seven abstentions; after had one useful control, 15 unhelpful,
12 late and four abstentions. The sole useful control was generic, with an
explicit review caveat. No unsupported factual tail was identified; this
small synthetic review proves neither safety nor language quality.

Production texts stayed identical in all 16 cases. Both runs started on AC
with power-saver; the after run ended balanced. Power was uncontrolled, so
these are observed diagnostic latencies, not a causal speed comparison.
The native-template correction is retained; the model earns no production
instruction setting. Evidence, full prompts, frozen protocol, every reviewed
addition and cleanup verification: `output/writing/2026-11-12-qwen06-instructions/`.
A stronger instruction candidate needs separate development and held-out
review before promotion.

### Window context

Cases written for it: as prompt text (`healed`, 8 tokens, pinned Qwen3 1.7B,
3 s diagnostic budget) the window's heading and paragraph gave the expected
next words in 15 of 16 informative cases against 4 of 16 without, but 3 of 8
controls drifted onto the page's topic, several additions answered for the
user, and at 550 ms both arms missed the deadline on 23 or 24 cold prompts.
For terminals, which expose no accessibility text, reading the screen with OCR
cost about 0.8 s and gained nothing; screen reading stays out (the
[context firewall](../../VISION.md#context-firewall)). No live check of the
observer's Lab context has run.

Independent cases: an agent with no access to the development set or its
results wrote 40 English cases. In 22 the window names what comes next, in 6
copying the window's fact would be wrong (a declined day, a corrected total),
and 12 are controls. Frozen by hash before inference; installed Gemma 3 1B at
28 W; one blind reviewer saw the window text and judged each whole addition:

| Arm | Useful full additions (all / informative / controls / no-copy) | Harmful | Wrong context fact | Over 550 ms |
| --- | --- | --- | --- | --- |
| `production_boundary`, no window text | 9 / 1 / 8 / 0 | 15 | 0 | 1 |
| Logit bias toward the window's words | 11 / 2 / 8 / 1 | 14 | 0 | 1 |
| `context`, window text in the prompt, 1.2 s diagnostic budget | 10 / 7 / 3 / 0 | 16 | 4 | 28 |

Neither form justifies production use. The bias changed the shown text in
only 4 of 40 cases. Prompt text found the informative words (an earbud, a
permission slip, the oat milk), but 9 of 12 controls drifted into invented
numbers ("60th birthday!"), it copied a fact the writer was changing (the
postponed event's original "3 February"), and it missed 550 ms cold. A
context-fact fence and a prompt that keeps controls quiet would come first.
Runs: `output/writing/2026-10-19-window-context-independent/`.

### Vocabulary completion

`vocabulary` finishes the word at the caret with an exact word in view
instead of putting context in the prompt. Two independent sets of 60 mid-word
cases (40 English, 8 German, 4 each Spanish, French and Portuguese) were each
written by a separate agent with no access to earlier cases or the code and
frozen by hash before inference: 20 where the window names the word, 11 where
only the writer's earlier writing does, 6 where the draft does, 13 traps
where a different word in view starts with the same letters, and 10 controls.
Installed Gemma 3 1B, 15 W PL1 on battery with the performance profile; two
blind reviewers judged each whole addition and a third settled disagreements.

| Set (rule) | Arm | Shown | Useful full additions | Harmful |
| --- | --- | --- | --- | --- |
| First (three letters) | `production_boundary` | 8 | 5 | 3 |
| | `vocabulary` | 52 | 39 | 13 |
| Second (five letters) | `production_boundary` | 7 | 1 | 4 |
| | `vocabulary` | 31 | 23 | 7 |

- On the first set every word in view was finished correctly (37 of 37), and
  every trap was finished into the other word ("user stor" became
  "storage"). The five-letter minimum was chosen on that seen set, where
  three or four letters gave 12 right and 13 wrong finishes and five or more
  gave 25 right.
- On the second set 23 of 26 finishes were right. The three wrong ones were
  traps with five or more letters ("migra" to "migration" for "migrate",
  "Verla" to "Verlag" for "Verlauf", "Domini" to "Dominic" for "Dominican");
  the other four harmful additions are the model's own, the same in both arms.
- A finish takes 0.1–0.3 ms at the median, the model 320–430 ms.
- Both runs fail the registered gates: harmful additions rise (on the second
  set English 4 to 6, German 0 to 1). The model cannot referee a trap without
  the window: under the healed echo, "on Fri" became "Friends" at
  log-probability −19 once the grammar ruled out the digits it preferred, and
  a window's names are as unlikely to it as wrong words.

Runs: `output/writing/2026-10-20-vocabulary-independent/` and `-2/`.

### Retired experiments

- **Compact native prompt** (Qwen3 0.6B, 2026-11-12): two controlled-run preflights stopped when the power profile changed, before any inference. No comparison or quality claim is available; the prototype and its experimental admission limit were removed. Frozen inputs, patches, guard failures and cleanup: `output/writing/2026-11-12-native-compact/`.

- **Italian diagnostic admission and space healing** (Gemma 3 1B, 2026-11-12): 32 fully matched requests gave 2/16 coherent, timely full additions without healing and 1/16 with it; both arms invented facts, and healing added a new harmful result where the control abstained. More on-time first words did not improve full additions. The prototype was removed; Italian stays spelling-only. Frozen inputs, all 32 reviews, identities, cleanup and removal patch: `output/writing/2026-11-12-italian-diagnostic/`.

- **Bounded heading/paragraph context** (Gemma 3 1B, 2026-11-12): 48 fully matched requests at four threads, 550 ms and eight tokens gave 4/16 useful additions with matching context against 6/16 without (three gains, five losses); wrong context caused one harmful topic override. The prototype was removed. Full additions, frozen review, identities and cleanup: `output/writing/2026-11-12-bounded-context/`.

Each of these failed its gate, and its code is deleted.

- **`instructed_word`** (next-word study, 2026-09-09): no useful result in 30
  opportunities and one harmful one.
- **`healed_attested`**: added harmful suggestions on top of `healed`.
- **`framed`, `probability_rejoin`, `following_text`** (41 English mid-line
  development cases, Qwen3 1.7B, 2026-10-12): none improved production's
  rejoin, which showed two suggestions, one matching the reference. The
  everyday-English frame missed 550 ms on every cold case; forcing the next
  words and keeping a likely prefix showed only the punctuation completion;
  naming the line after the caret showed one suggestion and missed 550 ms on
  34 of 41, since that line changes per case and defeats the prompt cache.
- **`field_context`** (2026-10-17): the field's own earlier text as context,
  cut on a stable grid for the prefix cache, measured neutral.
- **`context_bias`** (logit bias 2.0 on the first token of the window's
  distinctive English words, 2026-10-17): first-word matches rose from 12 to
  15 across four development suites (65 cases), but the independent set above
  barely changed. Bias 4.0 wrote loops ("Thursday, Thursday"), and Persian
  degraded at both.
- **Paced typing diagnostic** (12 traces of four snapshots at 0, 100, 250 and
  500 ms, Qwen3 1.7B, 2026-09-09): no final snapshot of the cold arm became
  eligible within 550 ms. Priming the runtime with the supplied context made
  3 of 12 eligible, and a blind review found 1 useful and 2 that contradicted
  the context.
- **Prefill probe** (Qwen3 1.7B, 2026-09-09): a 14-token prompt took 168–176
  ms cold and 35 ms when repeated or appended, with 13 or 14 tokens from the
  prefix cache that production's `cache_prompt` relies on.
- **Stop-token probe** (three fixed words in one AB/BA/AB screen, Qwen3 1.7B,
  2026-09-09): ending a word on the newline token instead of a space and end
  of sequence saved one generated token and 49–179 ms per word; the Persian
  word still missed 550 ms, and production keeps its stop strings.
- **Spelling tab**: it checked German and Persian with its own Hunspell
  helper, while production corrects English with its lexicon and other
  languages with installed dictionaries and a model choice among candidates,
  without Persian ([broker runbook](../../broker/README.md#suggestion-limits)).
  The broker offers that path only through its provider, so the Lab measures
  production spelling with `production_boundary`, where English corrections
  appear as `production_correction`; dictionary languages stay off there, as
  they are by default.

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
| `POST /api/context-lookup` (with `GET …/status`, `POST …/reset`) | One word-completion lookup |
| `POST /api/reset` | Clear the active run and await cleanup |

## Checks

```sh
npm run writing:lab:check
cargo test --locked -p badi-writing-lab
```

After retiring the failed prototypes, the retained changes passed the full
Rust gate (including MSRV 1.85) and all nine `npm run check` lanes on
2026-11-12. Commands, exit codes, logs and source hashes are in
`output/writing/2026-11-12-cotypist-checks/retained-scope/verification.json`.
The parent directory records the earlier editor and Fcitx real-broker
integration passes. These checks are separate from the real-model and
visible-app evidence above.
