# Writing evaluation

## Prediction Lab

The opt-in local workbench accepts your draft, relevant document/screen text,
explicit writing examples and alternative expected **text to append**. Expected
answers are kept out of the model request and used only for scoring. It does not
capture your screen, harvest writing history, edit another application, or alter
the installed broker. Run it from the repository root:

```sh
npm run writing:lab
```

Open the printed `http://127.0.0.1:PORT` address. No browser extension is needed
for this owned test editor. The command builds the opt-in `writing-lab` feature;
the worker verifies the already installed model/runtime and creates an isolated
local model process. Missing artifacts produce a startup error, not mock output.
Ctrl+C closes the server and its model session.

To reuse a known local address, run `npm run writing:lab -- --port 36889`.
The server binds only to `127.0.0.1`; an occupied port reports an error rather
than replacing another service. Opening `lab/public/index.html` directly shows
launch instructions with the workbench hidden. Use the printed HTTP address to
run tests; the raw HTML file cannot connect to the local workers.

### Discover, assess and compare a model

Open **Choose and qualify a local model** in the Prediction tab. This workflow
uses the existing Lab artifact contract and owned runtime; installed Badi keeps
its current model and settings.

1. Choose **Inspect this laptop** to refresh CPU features/topology, available
   RAM, GPU memory classification, cache disk and power information. Enumeration
   does not prove GPU execution. The current qualification path uses the pinned
   x86_64/AVX2 CPU runtime, four threads, 2,048 context tokens and batch 16 or 64.
   A physical Vulkan load/generation/cleanup receipt is separate evidence; it
   does not enable GPU qualification or establish application editing support.
2. Search a model name or exact Hugging Face repository. **Inspect GGUF
   artifacts** shows immutable revision, file bytes/SHA-256, quantization,
   architecture, languages, license/access and tokenizer/template information.
   **Assess this device** explains whether conservative compatibility and
   resource gates permit an experiment.
3. Choose **Download and verify**, then **Select for Lab comparison**. Exact
   cached bytes are rehashed and reused. Selection checks the downloaded GGUF
   and current resources; Rust verifies the complete weights and runtime again
   before loading. Browser requests select server-issued IDs, never paths,
   executables, runtime addresses or arbitrary inference flags. **Use installed
   baseline** restores the Lab's normal baseline selection.
4. Run explicit development cases and review the entire displayed addition,
   including unwanted tails. Base models use unwrapped context/word-boundary
   continuation. **Selected model instructions** (`native_instructed`) asks the
   owned runtime's local `/apply-template` endpoint to format the embedded chat
   template, then prefills the draft. It retains boundary checks and native EOS;
   it does not copy Qwen's token715 mechanism. Legacy Qwen instruction modes are
   refused for other selected artifacts. Inspect the effective prompt and
   timing: a template or a completed load alone does not prove useful output.
5. For confirmation, reserve an unused set and freeze the review protocol in
   **Reserve an untouched confirmation run** before inference. Review each
   assigned outcome, including absent additions, then choose **Assess my
   reviews**. Ordinary development runs cannot become untouched confirmation
   after their outputs are seen. The server rejects previously seen input
   hashes within its lifetime; it cannot verify a person's prior exposure.
6. **Measure runtime behavior** runs local cold/preparation, reuse, paced typing,
   cancellation/recovery and resource diagnostics. Choose 30 seconds, five
   minutes or the 30-minute sustained gate. Short diagnostics do not satisfy
   the sustained requirement. Inspect the qualification stages and rejection
   reasons; a recommendation requires every hard gate to pass. Selecting an
   experimental model never means it is recommended.

The current research shortlist tests distinct approximately 350M families:
[Granite 4.0 350M Base](https://huggingface.co/ibm-granite/granite-4.0-350m-base)
with its [official GGUF](https://huggingface.co/ibm-granite/granite-4.0-350m-base-GGUF),
[LFM2.5 350M](https://huggingface.co/LiquidAI/LFM2.5-350M) with its
[official GGUF](https://huggingface.co/LiquidAI/LFM2.5-350M-GGUF), and
[SmolLM2 360M Base](https://huggingface.co/HuggingFaceTB/SmolLM2-360M) through a
[QuantFactory conversion](https://huggingface.co/QuantFactory/SmolLM2-360M-GGUF).
Q8_0 keeps these candidate files below 390 MB while limiting quantization as an
additional variable. Granite and SmolLM2 test direct prose continuation; LFM2.5
tests a different attention/convolution architecture and instruction format.
The cards do not establish Persian support or autocomplete quality. Liquid's
[LFM Open License](https://huggingface.co/LiquidAI/LFM2.5-350M/blob/9e6c6ccf47cd318696e137d381a7ded8fe4df09f/LICENSE)
is recorded separately from the other candidates' Apache-2.0 declarations.
The [measured development screen](#measured-shortlist-development-2026-09-10)
below rejected every candidate; the reserved confirmation set was not used.
These remain research choices, not qualified recommendations.

### Discovery boundaries and persistence

[Discovery](lab/discovery.mjs) uses the supported
[Hub search/model-info APIs](https://huggingface.co/docs/huggingface_hub/en/package_reference/hf_api)
and [commit-pinned downloads](https://huggingface.co/docs/huggingface_hub/en/guides/download).
Only explicit search terms and public metadata/artifact requests leave the
laptop. It uses no Hub credential, hosted inference endpoint or repository code.
Drafts, context, style, expected answers and reviews are not uploaded to the Hub.

Search is bounded to 12 results per page and three pages; file inspection returns
up to 128 GGUF candidates. Metadata has a 15-second request deadline and 2 MiB
JSON limit; a narrowly parsed converter README is limited to 64 KiB. The in-memory
cache is fresh for five minutes and can report explicitly stale results for up
to 24 hours when the Hub is unavailable. New downloads require fresh pinned
identity/access metadata. Missing data is unknown, not inferred from popularity
or parameter count. A named upstream and its currently pinned config can inform
an estimate; they do not prove the converter's exact upstream revision. The
downloaded GGUF controls the later architecture/state/tokenizer assessment.

Downloads require a public, ungated, single-file GGUF with declared license and
consistent byte count/LFS SHA-256. This discovery lane caps each artifact at
2 GiB, checks remaining bytes plus 256 MiB disk headroom, limits idle transfer
time to 30 seconds and total transfer time to 20 minutes, and validates resumed
HTTP ranges. Linux `flock` prevents simultaneous writers and releases on process
exit. Cancelled partial files can resume; neither partial bytes nor mismatched
hashes can become a selectable artifact.

Artifacts, descriptors and provenance persist under
`${XDG_CACHE_HOME:-$HOME/.cache}/badi/prediction-lab/`, with private directories
and files (0700/0600). Clearing the Lab cancels active work and erases its current
prose/results; it does not delete completed downloads, resumable partial files,
or explicitly saved evidence. In-memory confirmation input hashes remain until
the server exits to prevent treating a repeated set as untouched.

**Save counts and measurements locally (no drafts)** is explicit opt-in.
[Evidence storage](lab/qualification-evidence.mjs) writes content-addressed
receipts under the cache's `evidence/` directory, bounded to 256 files of 64 KiB
each. Receipts contain aggregate counts, measurements and identity/protocol/set
hashes, not drafts, generated additions, case IDs or review prose. To reuse one,
run the selected artifact with the matching configuration/languages and choose
**Recheck saved evidence** with its receipt ID. Changed identity, future dating,
age over 30 days, malformed bytes or unsafe filesystem permissions reject reuse.
A saved receipt alone cannot bypass a fresh resource assessment.
Starting new diagnostics clears older performance evidence. Cancelled diagnostics
retain their cleanup report; unverified cleanup prevents receipt reload for that
run. Repeat the comparison before using it as qualification evidence again.

### Qualification contract and local API

[The Rust qualification engine](../../broker/src/model_selection/qualification.rs)
separates `discovered`, `estimated_fit`, `loaded_and_exercised`,
`meets_performance`, `meets_prediction_quality` and `recommended`. Fit charges
weights, architecture-specific attention/recurrent state, runtime buffers and
backend overhead to host RAM, preserving at least 2 GiB or 20% of total RAM for
normal use. Shared GPU memory is not added to host capacity. Unknown or
contradictory state dimensions block loading. Resource availability and the
selected artifact/configuration are rechecked before each cold runtime launch.
The current estimator also reserves artifact-sized free disk even for cached
weights; this can conservatively reject a load on a nearly full disk.

For every requested language, confirmation requires at least 40 independent
cases, review of every assigned outcome, at least 60% substantive useful full
additions returned within 550 ms, and zero harmful suggestions. Performance also
requires complete-word p95 at most 550 ms, separate cold/preparation measurements,
actual prompt-reuse counters, paced typing, cancellation/recovery, verified
cleanup and at least 1,800 seconds of sustained resource measurement. Missing
results, errors, deadlines and abstentions stay in the denominator. Missing
complete-word delivery is reported as a latency lower bound exceeding the
target, never as an invented successful delivery time. A longer configured
diagnostic budget gives no timing credit: only actual complete additions within
550 ms count as on-time, and diagnostic results remain separately identified.

Evidence binds the revision/weight hash, actual tokenizer metadata, quantization,
device/power identity, worker/runtime/backend and evaluator/HTTP/review source
hashes, exact configuration JSON, requested
languages and evaluation version. After hard gates, ranking prefers worst-language
useful yield, overall useful yield, language coverage, p95 and memory, in that
order. Speed cannot offset harmful output. The valid outcome is
`no_qualified_model` when no candidate passes. Synthetic model trials do not
establish rendering latency, editing authority, native undo or Cotypist parity.

All routes below require the served page's `X-Badi-Lab-Token` and matching local
Host/Origin. POST bodies use `application/json`; selection and review IDs come
from this server session. Device inspection, discovery, qualification, prediction,
spelling and context lookup share an active-operation gate. Disconnect/reset
cancels owned work and waits for cleanup before reuse.

| Method / route | Input or purpose |
| --- | --- |
| `GET /api/device` | Fresh read-only device inspection; no model launch. |
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
| `POST /api/qualification/load` | `{run_id, config_id, evidence_id}`; recheck an explicit saved receipt. |
| `POST /api/reset` | `{}`; clear the active Lab run and await owned cleanup. |

### Writing cases

1. Enter text before the caret and one acceptable addition per line. Preserve
   intended leading spaces. Add several cases to a set or run the editor alone.
2. Optionally supply relevant context and examples of your writing. These are
   explicit experiment inputs. Reusing a phrase found in an example demonstrates
   contextual reuse, not general writing-style learning.
3. Compare current provider logic, an isolated trailing-space fix, full-context
   continuation, instructed context and style, or full-context word-boundary
   healing, or instructions with boundary healing. Inspect the actual prompt, raw
   output, complete-word result, model/runtime hashes and effective configuration.
4. Review useful alternatives. Export only when you want to save drafts and
   results to a JSON download. Import accepts a test set or an exported session;
   imported results are never reused as new measurements.

Drafts/full results stay in page/server memory; no localStorage, prose logs or
automatic prose files are written. Explicit artifact downloads and optional
aggregate evidence persistence are described above. Clearing the session erases the editor/test data and stops its
owned runtime. Each comparison also stops its runtime on completion/cancellation.
An exported file remains wherever you saved it and must be removed separately.
Reset rejects uploads and delayed file imports that began before the clear. Readiness verifies the actual
owned runtime process and executable; an inspection failure stops the run and
reports cleanup uncertainty. Linux parent-death protection also covers a Lab
worker killed before it announces readiness. This containment is specific to the
verified model process, not a general descendant-process sandbox.

The **current Badi logic** mode calls the actual production provider, including
conservative spelling proposals, with its fixed 550 ms budget. The Lab uses a
separate 2,048-token runtime; installed Badi uses 512. This is a provider-code
baseline, not an installed-service latency measurement. Experimental budgets up
to ten seconds diagnose quality/latency tradeoffs and do not change production
deadlines. Token preflight refuses overflow instead of silently discarding cues.

The Lab server and comparison CLI accept `--prefill-batch 16|64`. Omission keeps
16; 64 is an explicit runtime experiment that changes both logical and physical
prompt batches, with the same model, four CPU threads and 2,048-token context.
Readiness verifies both reported batch sizes against the requested launch choice.
The browser cannot supply runtime flags through its request JSON. Changing the
batch does not prepare context automatically or change generation budgets.
The Rust paced worker also accepts the flag; the fixed historical prefill probe
rejects it. Installed production settings remain unchanged. Measure preparation,
terminal target delivery and cancellation separately before interpreting a gain.

The **isolated boundary fix** (`production_boundary`) uses the same production
provider, correction path, clipped sentence, output guards, stopping rules,
decoding settings and 550 ms budget as the baseline. Its only experimental
change is removing trailing ASCII spaces from the inference prompt, requiring
the model to reproduce those exact spaces, then excluding that echo from the
suggestion. The supplied draft is never trimmed. Space healing retains the
normal eight-token budget; existing English partial-word healing retains twelve.
This mode ignores extra context/style and does not change installed defaults.
It separates the boundary hypothesis from the broader `healed` experiment.

The optional **instructions + word boundary** (`instructed_healed`) mode combines
the existing non-thinking ChatML instruction prompt with the exact echo handling
from `healed`. It removes only the same recognized boundary from the model
prefill, requires that echo, strips it once, and validates the continuation
against the original draft. Other modes and UI defaults are unchanged. This
isolates an earlier confound where instruction mode omitted boundary healing;
it does not establish a quality gain or faster context processing. Existing
German/Persian midword proposal denials remain visible. The paced diagnostic
still requires `healed`; it does not silently prime a mismatched ChatML prefix.

The optional **one complete word** (`instructed_word`) mode uses exactly the
`instructed_healed` prompt and boundary echo, with a different decoding grammar.
It requires one Unicode word, an explicit final ASCII space and the runtime's
actual terminal response before returning text. The final space is omitted
from the preview. Token-limit completion is usable only if that separator has
already arrived. Missing terminal/separator, unsupported compounds, invalid
spacing and existing language/seam denials remain abstentions or deadlines.
Persian half-spaces retain their existing guards; this does not authorize
German/Persian partial-word completion or certify spelling and intended meaning.

The grammar changes token choices and may require extra separator/end tokens;
it is not a guarantee of faster or better predictions. A grammar-constrained
end is distinct from an unconstrained model choosing to finish. The pinned
[runtime grammar sampler](https://github.com/ggml-org/llama.cpp/blob/85c55223caf0a2ad0d1d88e5a73ab3fe36107867/src/llama-grammar.cpp)
masks end tokens until the grammar can finish. The Lab still requires the
actual response and exact text boundaries. Existing first-word/four-word timing
observers do not measure this stricter contract, so those fields are explicitly
unavailable in this mode; total request and inference clocks remain measured.
The mode is unchecked by default and does not alter production output.
Completed Lab suggestions that share this observation path now abstain with
`style_fact_conflict` when they introduce a weekday (English, German, or Persian,
including a ZWNJ spelling) or a numeric token that appears in the style examples
and is absent from the current draft and context. ASCII digits and both
Arabic-Indic digit blocks count; a longer number does not match a shorter one.
Ordinary style words still pass. The one-word grammar itself still excludes
numbers. This fence does not change installed prediction and is not a fresh
model confirmation of the Friday/Tuesday failure.

The lexical body is restricted to common Latin letter blocks for English/German
or the existing Arabic base-letter ranges for Persian, with internal apostrophes
or valid Persian half-spaces, followed by optional sentence punctuation. Numbers,
hyphenated compounds and uncommon letter blocks are outside this experiment.
Restricting only ASCII spaces proved insufficient: an archived real-model smoke
joined Latin and Chinese text across punctuation. Output guards rejected it;
the revised grammar also excludes that decoding route. The original smoke and
executed binary are retained under `output/writing/2026-09-09-next-word-grammar-smoke/`.
The corrected grammar passed a headed, unprepared five-second/eight-token smoke
in all three languages: `water,`, `Wasser,` and Persian `که`, with actual terminal
responses and exact separators. Requests took about 1.50–1.52 seconds, excluding
startup. The Persian function word is not evidence of useful contextual content.
All three cases were added without references; the UI marked their scores
unscorable. These are parser/rendering checks, not typing-speed qualification.

The **Word completion** tab performs a separate model-free experiment. Enter a
partial word and explicitly supplied context/style, or repeat a word already
closed by a separator earlier in the draft. The lookup returns only the untyped
suffix when one distinct, longer whole word matches exactly. It preserves case,
diacritics and Persian ZWNJ; it does not join separate input fields or treat a
field-ending fragment as a complete source word. The draft's first word has an
unknown left boundary and cannot serve as either evidence or a partial stem.
The partial stem needs 3–24 characters; ambiguous matches and unsupported
Unicode suffix boundaries abstain. Repeated identical words are deduplicated
with their original source kinds retained.

This path starts a bounded one-shot `badi-writing-lab --context-lookup` process
before any model runtime or dictionary is loaded. The response records a
separate deterministic contract, executed worker hash, cleanup receipt and
lookup/HTTP timings. A result is returned only after the owned worker exits.
Expected suffixes and user judgments remain in the browser. Editing cancels a
pending check and withdraws its preview; clear waits for cleanup and erases only
word-completion data. Export explicitly saves the current draft and up to 50
checks. All three tabs preserve each other's drafts and share the server's
active-test limit. This lookup is exact word reuse, not semantic intent or
spelling validation, next-word generation, or a production provider change.

The frozen 30-case development test completed with stable source/binary/input
hashes and verified exit-0 cleanup for every worker:

| Language | Opportunities | Useful exact completions | Unwanted control offers | HTTP median / maximum |
| --- | ---: | ---: | ---: | ---: |
| English | 10 | 4 | 1 | 23.93 / 28.80 ms |
| German | 10 | 4 | 1 | 26.80 / 29.46 ms |
| Persian | 10 | 4 | 1 | 31.46 / 35.27 ms |

All 12 authored positives completed and 15/18 controls abstained, with zero
errors, rejections, deadlines or unexecuted cases. The 100 ms HTTP gate passed;
the zero-unwanted-offer quality gate failed. The lookup extended completed words
with different intended meanings: a cousin named `Marin` became the teacher
`Marinella`, a wedding `Ring` became an office `Ringordner`, and Persian `سرما`
(cold) became `سرمایه‌گذاری` (investment). Unique contextual spelling does not
establish the writer's intended word or whether that word is already complete.

An unchanged 2026-09-09 rerun reproduced all 12 positive matches and the same
three unwanted offers, with all 30 workers verified and reaped. HTTP medians
were 21.19/20.18/19.53 ms for English/German/Persian. This confirms repeatability
of the failed semantic gate; it adds no independent quality evidence. Its records
are in `output/writing/2026-09-09-context-lookup-run/http30-rerun-2026-09-09/`.

The separate author froze requests and expected outputs before implementation
queries. References were parsed only after all query records were persisted;
these are authored-reference matches and intent controls, not a blinded review
of general predictions. HTTP timing includes executable hashing, observer
verification, process setup, lookup and reaping. There is no isolated startup
clock. Source rehashing before each request is outside HTTP timing and may warm
filesystem caches; its measured cost is retained. Rust lookup medians were
0.0125/0.0191/0.0257 ms respectively, not end-to-end timings.
Source snapshots, decisions and all denominators are in
`output/writing/2026-09-09-context-lookup-run/http30/`; the summary is
`output/writing/2026-09-09-context-lookup-run/http30-review.json`. Real browser
previews and clear/retry receipts are in
`output/writing/2026-09-09-context-lookup-visible-ui.json`. Installed Badi,
Cotypist parity and native application coverage remain unqualified by this run.

The **Spelling** tab checks the last completed German or Persian word through a
separate native Hunspell worker. It needs explicit local artifact configuration:

```sh
cargo build --release --locked --features writing-lab --bin badi-writing-lab
node evaluation/writing/lab/server.mjs \
  --spelling-de /absolute/canonical/path/spelling-manifest.json \
  --spelling-fa /absolute/canonical/path/spelling-manifest.json
```

The same manifest may configure both languages. The machine-local research
manifest is `output/writing/spelling-lab/manifest.json`; it is not a portable
installation artifact. [The manifest validator](../../broker/src/writing_lab/spelling/artifact.rs)
defines its strict `badi.spelling-artifact.v1` schema: pinned native executable,
loaded library and German/Persian dictionary pairs, each with canonical path,
byte count and SHA-256. Paths and engine settings cannot come from the browser.
The Persian CLI pair uses the reviewed `WORDCHARS` ZWNJ addition; the original
upstream pair and its notice remain separate. Moved files require a newly verified manifest. This first native contract also
pins the reviewed Hunspell executable/library hashes in Rust; an engine upgrade
requires reviewing and updating that contract, not just editing its manifest.

Finish one 3–24-character word with exactly one ASCII space, select its language,
and optionally enter an expected corrected word and words to keep unchanged.
Expected answers stay in the browser. A grey preview compares the exact original
suffix with the proposed word and leaves the draft unchanged. Accepted words,
protected originals, incomplete boundaries and ambiguous admissible candidate
lists produce no offer. Candidates preserve the initial character, case pattern
and Persian joiners and need one Unicode edit plus a second native acceptance
check. Returned candidate lists are not exhaustive, and unfamiliar names may
still resemble errors. This workbench does not apply edits to another app.

Spelling never starts the language model. A single dictionary worker persists
until a language switch, cancellation, clear or server shutdown. Each check has
one 550 ms native deadline, including both queries; startup and HTTP/browser
receipt timing are distinct. Reset fences earlier uploads, aborts active checks
and verifies owned-child cleanup before allowing another check. Prediction and
spelling requests share a single active-test limit. Tab changes preserve both
drafts; editing during a spelling check cancels it and rejects late output.
Spelling exports include the current draft and up to 50 reviewed results, only
when explicitly downloaded. There is no automatic prose retention.

An ordinary cancellation during startup now waits for the bounded identity
verification to settle before stopping the owned process; the cancelled request
cannot dispatch a word query. Failed readiness, timeout and EOF retain cleanup
uncertainty. The actual 5 ms startup-cancel/restart trial verified both children
reaped with zero word queries; see
`output/writing/spelling-lab/startup-cancel-verification.json`. Visible German
and Persian previews, protected-name abstention and clear/retry checks are in
`output/writing/spelling-lab/visible-ui-verification.json`. The browser's Cancel
click arrived after the short check completed, so physical in-flight cancellation
is not claimed; deferred-response UI tests cover late-result rejection.

Native readiness verifies the actual executable and loaded-library mapping.
The first HTTP40 run stopped before its first word query because a device-number
comparison did not account for this Btrfs installation: Linux
[process maps](https://github.com/torvalds/linux/blob/master/fs/proc/task_mmu.c)
use the superblock device, while Btrfs
[file metadata](https://github.com/torvalds/linux/blob/master/fs/btrfs/inode.c)
uses the subvolume device. Its one error and 39 unexecuted cases are preserved
in `output/writing/spelling-lab/run-40-http/`, with no lexical-quality conclusion.
The fix prefers direct mapped-file verification. When that access is denied,
it corroborates exact live mapping paths/inodes/devices against the independently
hashed executable, only for library and executable on the same verified stat
filesystem, then rechecks both files. This fallback is mapping metadata plus
disk hashes, not direct mapped-library byte verification under external mount
replacement. Actual German/SIGTERM and Persian/EOF startup trials passed with
matching cleanup receipts and no word queries; see
`output/writing/spelling-lab/startup-diagnostic/fix-summary.json`.



The optional **context word completion** (`healed_attested`) mode uses exactly
the `healed` inference payload. If the existing proposal guard rejects a German
or Persian midword join, it can return only the first completed word's suffix
when the exact reconstructed word occurs in the original context or an individual
style example. Source words need observed lexical separators; EOF, substrings,
different capitalization, diacritics or Persian characters do not match.
Expected answers, the draft itself and generated text cannot attest a word.
Empty healing echoes remain possible when the existing English lexicon recognizes
a German stem; nonempty echoes must match exactly. Other shape, language,
joiner and completion checks remain in force, and no spelling replacement is
introduced. Recovered word-readiness timestamps are explicitly unavailable;
the actual full-result latency is still measured. This is contextual reuse,
which can repeat a source typo, and remains separate from spelling validation.

Comparisons randomize case/trace groups and counterbalance configuration order.
Each independent case/trace/config starts in a new owned runtime. Steps in an
imported trace remain contiguous and can reuse transient context. This is ordered
snapshot replay: `at_ms` records intended event times but does not pace keystrokes
or test in-flight cancellation. Startup is excluded from model-result timing;
worker/startup failures explicitly mark their broader measured interval.

Reference agreement is case-sensitive, with raw and NFC agreement separate.
The first affected complete word is evaluated across the seam (`docum` plus
`entation` forms `documentation`). Raw unfinished tokens cannot earn a completed
word score. Errors, deadlines and abstentions remain in request denominators;
cases without references and spelling replacements are unscored for append
agreement. Matching reference graphemes are potential matching text, never
observed keystrokes saved. Human judgments remain separate from exact agreement.
Every editable/imported Lab case is development data, not a heldout benchmark.
The paced diagnostic also scores two-, three- and four-word reference prefixes.
These are case-sensitive NFC lexical comparisons across the seam; punctuation
is ignored and an unfinished final word earns no completed-word credit. A
reference too short for a particular horizon leaves that horizon unscored.

```sh
npm run writing:lab:check
cargo test --locked --features writing-lab writing_lab
```

For repeatable terminal runs through the same Lab server/worker/scoring path:

```sh
node evaluation/writing/lab/run.mjs \
  --suite evaluation/writing/lab/examples/quality-probes.json \
  --output output/writing/my-boundary-comparison \
  --modes context,healed --budget-ms 550 --max-tokens 8 --seed 42
```

The [16-case development set](lab/examples/quality-probes.json) covers paired
trailing-space boundaries, contrasting contextual facts and supplied spelling
style versus conflicting context. The [six-step trace set](lab/examples/typing-traces.json)
contains two ordered English/Persian typing sequences. Both are assistant-authored
development diagnostics with nonexhaustive expected continuations.

The CLI creates a new private directory under `output/writing/`; it never
overwrites a run. `run.json` freezes the input hash, order and provenance before
inference, `events.ndjson` retains progress/results, and `session.json` can be
imported into the UI. `report.json` records source stability, executed worker
hashes, runtime identities, cleanup and measurements. Progress prints case IDs
and counts without prose. Do not edit or rebuild the recorded sources/binary
during a run; changed provenance invalidates its executed-identity check.

Both comparison CLIs accept an optional `--model-artifact` argument pointing to
an absolute, canonical JSON descriptor. This selects weights only for the Lab;
it does not change installed Badi or another running browser workbench. The
browser's discovery workflow creates the same descriptor contract for its own
verified downloads and can select them explicitly within that Lab session.
The descriptor has exactly these fields:

```json
{
  "schema": "badi.lab-model-artifact.v1",
  "weights_path": "/absolute/canonical/path/model.gguf",
  "sha256": "EXACT_LOWERCASE_SHA256_OF_THE_FILE",
  "bytes": 1107409024,
  "alias": "my-lab-model"
}
```

Supply the actual digest and byte count. Descriptors are bounded to 16 KiB,
weights to 8 GiB, and both paths must identify regular files without symlink
redirection. Rust verifies the weight bytes and the pinned runtime/archive;
the descriptor cannot supply a runtime executable or inference flags. The CLI
saves the exact descriptor privately, checks it before execution and afterward,
and matches readiness against its hash, size, alias and
`model_origin: "explicit_lab_artifact"`. A mismatch still triggers owned cleanup.
The fixed `--prefill-probe` command does not accept artifact overrides. Verifying
or loading a candidate does not qualify its prediction quality.

The internal `target/release/badi-writing-lab --stop-token-probe --model-artifact /absolute/path/descriptor.json`
command tests a fixed termination mechanism through the same owned runtime.
It accepts only the pinned 1.7B Q4_K_M artifact and no arbitrary text, grammar,
request JSON or runtime settings. Each of six fresh runtimes verifies that token
715 decodes to ASCII space/newline and that those exact bytes re-encode to 715
before generation. Three literal words use paired space/EOS and combined-stop
grammars, with one identical primer per arm: at most six primers and six targets.
All targets use five seconds and 16 tokens; only their grammar differs.

The command emits its fixed plan, verified identities, dispatches, synthetic
output, native stop metadata and exact cleanup observations as JSON lines.
Keep returned token IDs and native prediction counters separate. A valid combined
stop must leave the trailing ASCII space and report a native newline stop.
Cancellation or failure stops further dispatches; existing bounded startup and
cleanup remain outside the 120-second dispatch window. A startup failure uses
existing RAII containment without a returned PID receipt and is not a verified
cleanup result. This three-pair mechanism screen does not measure natural word
choice, language quality or general latency improvement. Production and ordinary
Lab prediction modes do not use its model-specific stop token.

Experimental `first_word_ms` and `first_four_words_ms` report the first observed
valid complete prefix, including token preflight and excluding runtime startup.
They do not claim that a user saw that word then: the UI waits for the full
diagnostic result. `slot_tokens_cached` is final slot occupancy, which can include
newly generated tokens. Experimental streams separately expose
`reused_prompt_tokens` from terminal `timings.cache_n`, and
`newly_evaluated_prompt_tokens` from `timings.prompt_n`. Missing/invalid terminal
statistics remain null; zero is a measured zero. Production-code modes do not
yet capture response statistics, so those counters remain unavailable there.
Neither occupancy nor total `tokens_evaluated` may substitute for actual reuse.

For a fixed-input runtime diagnostic, build the worker and run:

```sh
cargo build --release --locked --features writing-lab --bin badi-writing-lab
target/release/badi-writing-lab --prefill-probe
```

This separate CLI command accepts no prompt or generation settings. It uses two
fresh verified runtimes, probes cold/repeated/appended disposable prompts with
streaming and non-streaming, and emits counts, timings, identity and cleanup in
one JSON report. It retains no response prose or token IDs. Requests have a
five-second limit and share a 60-second activation/request budget, followed by
bounded owned-runtime shutdown. Incomplete probes exit nonzero.

The installed b10726 probe generated **one token despite `n_predict: 0`** in all
six requests. Repeated/appended requests reported real prompt reuse and needed
only one newly evaluated prompt token; see the measured
[runtime findings](../../docs/research/competitive-landscape.md#context-reuse-source-audit-before-implementation).
Do not call this prefill-only or extrapolate its tiny-prompt timing to full typing.

### Paced context diagnostic

`node evaluation/writing/lab/paced.mjs --suite PATH --decision PATH --output NEWDIR`
runs the separate, fixed-contract typing experiment. It requires a decision file
that binds the exact suite bytes, two arms (`cold`, `primed`), order seed 71,
and healed generation at 550 ms/eight tokens. Each trace has four append-only
snapshots at 0/100/250/500 ms, each adding one Unicode scalar, with stable supplied
context/style/language. It accepts at most twelve traces. This is a diagnostic
protocol, not a general typing scheduler or a new production setting.

Every trace/arm owns a fresh runtime. Both arms wait the same 1,500 ms after
startup. The primed arm evaluates only supplied context/style, explicitly
generates and discards one token, and verifies terminal counters; the initial
draft is unavailable until event zero. Startup, preflight, priming, the full
prelude and cleanup costs are reported separately. A prelude overrun cannot
delay typing to improve the result.

One active request drains only until its original absolute deadline; one pending
snapshot is replaced by newer typing. Due events invalidate older results before
completion can make them eligible. The 550 ms deadline includes scheduling
lateness, queueing, preflight and inference. Nonterminal responses and hard
timeouts require owned runtime shutdown before another independent trial.
An intermediate Persian trailing half-space remains an observed revision but
receives no model request; it still revokes older suggestions. Normal Lab and
production text guards are unchanged.

The CLI writes private, exclusive `run.json` and `report.json` files plus copies
of the frozen inputs. It verifies actual worker/runtime ownership, executable
hashes, source/input stability and cleanup. Expected measured timeouts retain
all opportunities and permit the next fresh trial after verified cleanup;
protocol, ownership and uncertain cleanup failures halt further inference.
All scheduled events and final revisions remain in scoring denominators.
`delivered_at_ms` measures Rust's terminal eligibility observation, **not visible
UI delivery**. Actual application acceptance, typing savings and the availability
of screen context before typing remain unmeasured.

### Measured shortlist development (2026-09-10)

**No candidate meets the English, German and Persian development gates.**
The frozen screen assigned 24 new synthetic cases per language to each of five
model/configuration arms, retaining all 360 opportunities, including absent and
late results. It required at least 15 useful complete additions within 550 ms
and zero harmful additions in every language before opening the separate
40-case-per-language confirmation set. That set remains unused. No installed
model or prediction default changed.

The approximately 350M shortlist above tests a concrete cost hypothesis:
Granite and SmolLM2 Base use direct continuation, while the official LFM2.5
artifact tests its own instruction template and different architecture. The
[Granite card](https://huggingface.co/ibm-granite/granite-4.0-350m-base) lists
English/German; [SmolLM2](https://huggingface.co/HuggingFaceTB/SmolLM2-360M)
is an English-focused control; neither those cards nor
[LFM2.5's card](https://huggingface.co/LiquidAI/LFM2.5-350M) establishes Persian
autocomplete quality. Exact revisions, conversion provenance, licenses and the
Q8_0 acquisition rationale remain in the
[research receipt](../../output/writing/2026-09-10-discovery/research/shortlist.md).

Agents reviewed each entire displayed addition with model/configuration,
timings and reference answers withheld. Natural open sentences were permitted;
generic function words did not earn useful credit, and a correct first word
could not excuse a harmful tail. Ambiguous Persian readings require native
adjudication and receive no useful credit. These are blinded agent judgments on
synthetic drafts, not human acceptance or a calibrated production benefit rate.

Each cell is **useful within 550 ms / harmful**, out of 24 scheduled cases.
Harm counts include every reviewed displayed addition, irrespective of timing.

| Model and Lab mode | English | German | Persian |
| --- | ---: | ---: | ---: |
| Qwen3-1.7B Q4_K_M, `production_baseline` | Unobserved / 9 | Unobserved / 6 | Unobserved / 10 |
| Qwen3-1.7B Q4_K_M, `healed` | 12 / 2 | 4 / 4 | 0 / 2 |
| Granite 4.0 350M Base Q8_0, `healed` | 12 / 6 | 4 / 13 | 0 / 5 |
| LFM2.5 350M Q8_0, `native_instructed` | 9 / 8 | 5 / 7 | 2 / 3 |
| SmolLM2 360M Base Q8_0, `healed` | 11 / 4 | 3 / 9 | 0 / 1 |

The production baseline does not retain terminal confirmation. Its summary's
zero eligible count therefore means missing completion evidence, **not proven
zero useful output**: its content review found six useful English additions.
Use the matched Qwen `healed` arm for terminal-observed comparisons. Every new
candidate still fails both the useful-yield threshold and the zero-harm rule;
speed and smaller memory footprints cannot offset those failures.

All arms used the same pinned CPU runtime, four threads, 2,048 context tokens,
batch/ubatch 16, eight generated tokens, temperature zero, seed 42 and prompt
caching. Models ran sequentially on the same AC-powered laptop; order was not
randomized and thermal conditions were not laboratory-controlled. Actual
response p50 / p95 below is the larger of provider and request-roundtrip clocks,
rounded up in milliseconds, including abstentions and deadlines. It does not
measure complete-word delivery or visible rendering.

| Model and mode | English response, ms | German response, ms | Persian response, ms |
| --- | ---: | ---: | ---: |
| Qwen `production_baseline` | 293 / 440 | 285 / 445 | 427 / 553 |
| Qwen `healed` | 407 / 553 | 444 / 554 | 520 / 554 |
| Granite `healed` | 137 / 384 | 150 / 511 | 193 / 553 |
| LFM2.5 `native_instructed` | 374 / 553 | 363 / 553 | 373 / 553 |
| SmolLM2 `healed` | 133 / 549 | 167 / 553 | 191 / 553 |

The complete-delivery distribution also retains every scheduled case. Absent
or nonterminal deliveries receive censoring bounds of at least 551 ms rather
than invented successful completion times; affected quantiles are marked `≥`.
For the production baseline these markers reflect unobserved terminal status,
not proof that generation actually completed late. Cold worker startup is
separate from request latency; peak RSS is the largest sampled runtime value.

| Model and mode | English complete p50 / p95, ms | German complete p50 / p95, ms | Persian complete p50 / p95, ms | Cold startup p50 / p95, s | Peak RSS, MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| Qwen `production_baseline` | ≥551 / ≥551 | ≥551 / ≥551 | ≥551 / ≥553 | 4.45 / 4.92 | 2172.8 |
| Qwen `healed` | 411 / ≥553 | ≥551 / ≥554 | ≥551 / ≥554 | 4.44 / 4.88 | 2192.5 |
| Granite `healed` | 149 / ≥551 | 174 / ≥551 | 362 / ≥553 | 1.46 / 1.76 | 500.9 |
| LFM2.5 `native_instructed` | 403 / ≥553 | 467 / ≥553 | ≥551 / ≥553 | 1.49 / 1.76 | 455.3 |
| SmolLM2 `healed` | 156 / ≥551 | 243 / ≥553 | 548 / ≥553 | 1.45 / 1.70 | 515.2 |

The [comparison summary](../../output/writing/2026-09-10-discovery/comparison-summary.json)
retains exact values, model/runtime hashes, all five judgment counts and
per-language ambiguity counts. The
[input freeze](../../output/writing/2026-09-10-discovery/evaluation-inputs/freeze.json),
[rubric](../../output/writing/2026-09-10-discovery/evaluation-inputs/rubric.json),
[granularity clarification](../../output/writing/2026-09-10-discovery/review-clarification.md),
blind reviews and per-arm raw reports remain under
`output/writing/2026-09-10-discovery/`. The retained
[benchmark runner](../../output/writing/2026-09-10-discovery/benchmark.mjs)
records the exact suite/configuration and discovery workflow. Regenerate the
aggregate from those saved reports and reviews with:

```sh
node output/writing/2026-09-10-discovery/summarize.mjs
```

Separate completed runtime diagnostics used the same modes on an i7-12700H
(14 cores/20 threads, AVX2) with 15.3 GiB RAM. The
[device snapshot](../../output/writing/2026-09-10-discovery/development-granite350/workflow.json)
reports `on_battery: null`; the separate sysfs check found mains online and the
battery not charging, so AC power was not inferred from that unknown API field.
The [Vulkan receipt](../../output/writing/2026-09-10-discovery/device/vulkan-proof.json)
proves actual Qwen GPU execution through an increasing i915 render counter,
about 1.27 GiB of system-backed GPU memory, a terminal request in 1.146 s and
verified process cleanup. This separate device proof does not establish GPU
typing performance; the reusable qualification algorithm remains CPU-only.

Diagnostic cold startup and preparation below are separate single-run costs,
rounded up in milliseconds. Preparation includes pre-load checks and supervisor
overhead. Reuse is the sum reported by observed terminal counters in the warm,
repeat and sustained phases, excluding paced requests. Paced cells are final
complete-delivery times from the scheduled event in English/German/Persian;
`miss` means no eligible final delivery within 550 ms, including queueing.

| Diagnostic receipt | Cold / preparation, ms | Peak RSS, MiB | Sustained duration, s | Reused tokens, warm/repeat/sustained | Paced finals EN / DE / FA, ms |
| --- | ---: | ---: | ---: | ---: | --- |
| [Granite `healed`](../../output/writing/2026-09-10-discovery/development-granite350/diagnostics-300-healed.json) | 1082 / 110 | 539.8 | 300.00 | 0 / 39 / 58 | 106 / 109 / 146 |
| [LFM2.5 `native_instructed`](../../output/writing/2026-09-10-discovery/development-lfm350/diagnostics-30-native_instructed.json) | 1880 / 122 | 467.9 | 30.48 | 0 / 0 / 0 | miss / miss / miss |
| [SmolLM2 `healed`](../../output/writing/2026-09-10-discovery/development-smol360/diagnostics-30-healed.json) | 1539 / 97 | 525.8 | 30.07 | 0 / 54 / 13 | 147 / 274 / 154 |
| [Qwen `healed`](../../output/writing/2026-09-10-discovery/development-baseline/diagnostics-30-healed.json) | 4197 / 201 | 2216.3 | 30.24 | 0 / 35 / 8 | miss / miss / miss |

All four runs observed nine scheduled typing events and passed cancellation,
fresh-process recovery and verified cleanup. Granite and SmolLM2 delivered all
three final paced requests on time. LFM2.5 and Qwen did not; LFM2.5 nevertheless
reused **93 prompt tokens in one Persian paced request**, so its zero counters
in the other phases do not mean zero reuse everywhere. These diagnostics assign
no usefulness labels and add no quality credit. Five minutes is still below the
required 30-minute sustained gate. The earlier
[Granite HTTP-client timeout](../../output/writing/2026-09-10-discovery/granite-diagnostic-interrupted.json)
is retained as an incomplete attempt with no qualification credit; the completed
retry extended only the output-script client timeout, leaving product code and
quality inputs unchanged.

These measurements do not establish application rendering, editing authority,
acceptance, native undo or Cotypist parity. The resulting recommendation remains
`no_qualified_model`.

The existing 16-case regression set also completed for all four artifacts, with
both baseline modes: [80 requests](../../output/writing/2026-09-10-discovery/regression-summary.json),
zero worker errors, five deadlines and verified cleanup. These exposed cases
test execution and guards; their reference scores are not fresh quality evidence.
The earlier source-unstable Granite development attempt remains separately under
`interrupted-granite350-source-review/`, with 44 received of 72 assigned requests
and no qualification credit. It was cancelled for an evaluator identity fix
before reviewing quality; its remaining 28 cases were not started.

All completed development and regression runs bind their measured source hashes.
A final cancellation regression fix changed the HTTP evaluator afterward;
[a real saved-receipt reload returned HTTP 409](../../output/writing/2026-09-10-discovery/measured-build-invalidation.json)
because that identity changed. The measurements remain historical evidence for
their exact build; they do not qualify the final build. The fix preserves full
reviews while invalidating superseded performance evidence and preventing old
cleanup claims from surviving a cancelled, unsuccessful lifecycle trial.
The final [rendered UI](../../output/writing/2026-09-10-discovery/qualification-ui-final.jpg)
exercised public search, fit, verified cache reuse, selection, a real prediction,
full-addition review, explicit count-only saving, receipt reload, and cancellation
during sustained work. The [cancelled diagnostic receipt](../../output/writing/2026-09-10-discovery/ui-diagnostic-cancelled-final.json)
retains 98 assigned opportunities, verified cleanup and no qualification credit.
Both visible status messages leave the running state after cancellation. The
Lab selection was restored to the installed baseline afterward.

### Current development findings (2026-09-09)

The paced contextual comparison completed all 24 fresh runtime trials and kept
all 48 scheduled snapshots per arm. Cold inference produced no eligible final
continuation; context priming produced three out of twelve. A new independent,
configuration/reference-blind agent reviewed all three full continuations:

| Final revision measure | Cold | Primed |
| --- | ---: | ---: |
| Scheduled traces | 12 | 12 |
| Eligible terminal suggestions within 550 ms | 0 | 3 |
| Useful full continuations | 0 | 1 |
| Continuations contradicting supplied facts | 0 | 2 |

The +1 useful gain **fails the frozen +3 mechanism gate**, and misleading output
increased. Priming is not promoted. The useful result was German; the English
and other German result introduced scheduling conflicts. There were no eligible
Persian results. All four Persian priming attempts and two German attempts
exceeded the fixed 1,500 ms prelude. Of the other six primed trials, two exhausted
an active deadline and one ended in a guarded seam abstention. Cold inference
exhausted the first active deadline in all twelve traces.

Successful priming took 547–974 ms plus separately measured preflight; the three
eligible final results took 463–510 ms from their scheduled event. Retained
terminal responses report real context-token reuse, but no cold result survived
to provide a matched terminal payload/counter comparison. Missing counters stay
missing. First-word reference agreement credited two results while full review
found two contextual contradictions: partial lexical agreement does not prove a
useful sentence. These synthetic development observations and agent judgments
do not measure screen-context availability, visible delivery or human acceptance.
Frozen inputs, raw reports and the blind review remain under
`output/writing/2026-09-09-paced-context-*`.

The same-family Qwen3-1.7B Base artifact comparison completed 32 requests on the
existing 16-case development set at 550 ms/eight tokens with full-context
healing. Two balanced blocks used post/Base/Base/post order, with a fresh
runtime for every case. All 16 paired actual prompts and payloads matched;
independent metadata inspection confirmed model/source/binary identities and
32 successful worker/runtime cleanups.

| Artifact | Requests | Eligible terminal continuations | Useful full continuations | Harmful to draft |
| --- | ---: | ---: | ---: | ---: |
| Installed post-trained Q4_K_M | 16 | 9 | 6 | 1 |
| Base Q4_K_M | 16 | 10 | 8 | 1 |

Eligibility requires a complete guarded word, terminal response and result
within 550 ms. Partial or late returns remain in all-request denominators.
The +2 useful gain **fails the frozen +3 gate**; Persian useful yield also falls
from 1/4 to 0/4. Base is not promoted. The reviewer saw anonymized exact
input/output pairs without references or timings, but also served as the
metadata auditor; the review discloses that role and no prior output-text or
pair-mapping exposure. One ambiguous Persian output shared by both artifacts
needs native-language adjudication. This already-exposed synthetic set has
10 English, two German and four Persian cases, including related boundary
pairs. It measures neither human acceptance nor writing-style adaptation.
Actual artifact embedding/output representations differ, so this is not a
causal training-stage ablation. Frozen inputs, reports, audit, blind review and
failed decision remain under `output/writing/2026-09-09-base-artifact-*`.

The instruction/boundary ablation completed 66 fresh-runtime requests with
matched five-second/32-token settings on the exposed 16 cases plus six new
partial-word cases. All source, binary and model identities matched, and all
66 worker/runtime cleanups completed.

| Mode | Requests | Eligible continuations | Useful full continuations | Harmful to draft | Cold median |
| --- | ---: | ---: | ---: | ---: | ---: |
| Instructions | 22 | 14 | 6 | 7 | 2,620.1 ms |
| Instructions + boundary | 22 | 18 | 18 | 0 | 2,809.5 ms |
| Boundary healing | 22 | 18 | 13 | 2 | 1,966.9 ms |

The combined mode **passes the frozen development quality gate**: at least
three additional useful continuations against each control, no language loses
useful output and harmful output does not increase. Its useful counts are
12/12 English, 2/4 German and 4/6 Persian; all four abstentions are existing
German/Persian midword guard rejections. No arm returned an eligible terminal
result within 550 ms. The combined mode's median first token was 1,411 ms;
longer instruction prefill and full generation still need separate latency work.

The blind agent review graded 46 deduplicated full input/output pairs without
current mode mappings, references or timing. The reviewer disclosed prior
static implementation review and older anonymous output exposure; this is
development evidence, not a fresh human confirmation. One low-confidence
Persian baseline judgment needs native adjudication; changing that one judgment
does not erase the combined mode's minimum gain. Results and preserved source
and executable bytes are in `output/writing/2026-09-09-instruction-boundary-*`.
The completed run is the unchanged `comparison-retry`: an earlier process ended
by SIGTERM after 65/66 records, without final cleanup metadata or a known cause.
Its incomplete evidence remains separate. Production defaults are unchanged.

The exact-context word recovery experiment completed 24 requests on 12 matched
German/Persian cases at five seconds/32 tokens. All paired prompts, generation
payloads and observed raw outputs were identical; identities and all owned
process cleanups were verified. `healed_attested` recovered the useful German
suffixes `nung` and `ene` where the complete words appeared with exact boundaries
in original context. It gained two useful outputs versus zero for `healed`,
**failing the frozen +3 gate**. Both modes also offered the same two harmful,
repeated non-Persian continuations; the existing Arabic-script guard admitted
them. Other outputs stayed rejected, including all negative recovery controls.
Neither mode had an eligible terminal result within 550 ms; candidate median
was 2,754.5 ms. Recovered-word readiness timestamps remain null, as required.
The four deduplicated pairs received a separate blind agent review with prior
project exposure disclosed. Source, executable, raw results, review and exact
recovery audit remain under `output/writing/2026-09-09-attested-word-*`.

The subsequent Spelling Lab HTTP run completed all 40 frozen opportunities
through the real server, Rust worker and native engine after the startup fix.
Inputs and executed source/binary identities stayed stable. Both language engines
and their workers exited; cleanup receipts matched the observed processes.

| Language | Opportunities | Offers | Useful reference matches | Unwanted control offers | Median / max HTTP receipt |
| --- | ---: | ---: | ---: | ---: | ---: |
| German | 20 | 3 | 2 | 1 | 10.4 / 113.2 ms |
| Persian | 20 | 5 | 4 | 1 | 8.3 / 126.0 ms |

The **frozen quality gate failed**: German missed three useful corrections, and
both languages changed a no-offer control. An intentional German name was changed
to a different name; a Persian typo near a `NOSUGGEST` word received another
unwanted nearby word. The latter is not evidence that native Hunspell offered a
`NOSUGGEST` entry: its restricted returned list still left a misleading unique
candidate. Independent agent review judged both changes unsupported in context,
with prior implementation/review involvement and selection framing disclosed.
That review cannot override the frozen control requirements.

There were 29 abstentions and three input rejections, with no deadlines, execution
errors or unexecuted cases. Ambiguity caused 14 abstentions across the 37 worker
requests. Those requests used 38 native word roundtrips; per-worker `query_ms`
includes artifact verification and can sum two queries. Startup took 55.3/63.7 ms
and cleanup 14.4/12.8 ms for German/Persian. HTTP measurements include observer
checks and are not visible-display latency or a comparison with the LLM.
Synthetic case receipts and the failed first run remain in
`output/writing/spelling-lab/`. The Lab preview is usable for explicit tests;
this result does not qualify automatic correction or native-app integration.

A later prepared-instruction run repeated the same 22-case set with paired cold
and prepared `instructed_healed` requests at 550 ms/eight tokens. Each primer used
only `.` plus the same explicit context/style/language, never the future draft or
references. All 44 target opportunities completed with verified identity/cleanup.
Cold requests yielded no eligible terminal output. Preparation yielded 12 on-time
outputs: blind full-continuation agent review found 11 useful, one neutral and
none harmful. The useful counts were 7/12 English, 1/4 German and 3/6 Persian.
This passes the +5 gain requirement but **fails the frozen 18/22 useful target**.

All 22 primers completed; their median preparation cost was 1,404.9 ms and maximum
2,242.4 ms, separately from roughly 4.4 seconds of fresh-runtime startup. Among
only the 12 eligible prepared outputs, median Node receipt was 508.5 ms and maximum
546.2 ms. Actual terminal prompt-reuse counters were present with positive reuse
in 14 prepared targets; slot occupancy was not used as evidence. This includes
general runtime warming and is not paced typing or a pure KV-cache experiment.
Cases were previously exposed synthetic development inputs; the reviewer authored
the private harness and recognized older outputs, with modes, current timing and
references hidden during grading. Frozen inputs, executed source/binary archives,
all-request receipts and review are under
`output/writing/2026-09-09-prepared-instruction-*`. No default was promoted.

The subsequent matched batch-16/batch-64 experiment retained 22 prepared target
opportunities per profile. Each produced only **one useful eligible continuation**
within 550 ms. Agent review found no eligible harm; batch 64 missed the frozen
18-useful gate and lost German useful yield. Median preparation was 1,970.65 ms
for 16 and 2,020.25 ms for 64, so there was no median preparation improvement.
Most retained text lacked a terminal response by the target deadline. This
contemporaneous comparison does not support batch 64. Its difference from the
earlier 11/22 result is not a controlled regression comparison across host states.

The frozen run dispatched 44 primers and 41 targets; three unavailable primers
suppressed their targets without removing opportunities from the denominators.
All 22 primer payload pairs matched; 19 target pairs matched and three remained
unavailable. Three separate batch-64 cancellation/fresh-recovery trials added six
requests, for 91 actual dispatches against 94 planned maximum. Aborts occurred
80.52–80.66 ms after write; caller rejection took 0.14–0.74 ms, but verified
cleanup took 976.87–1,729.40 ms. Fresh recovery additionally needed
5,089.96–6,967.43 ms startup and 1,544.29–2,725.34 ms terminal response time.
This proves teardown and fresh-process recovery, not rapid same-runtime recovery.
All 50 owned worker/runtime pairs were absent afterward. Executed sources,
binary and blind-review disclosure are preserved under
`output/writing/2026-09-09-prefill-batch-comparison/`; the reviewed result is
`output/writing/2026-09-09-prefill-batch-result.json`.

The fresh `instructed_healed`/`instructed_word` comparison used 24 main cases,
eight each in English, German and Persian, plus six completed-word/name controls.
Both modes used batch 16 and the same prepared context/style prompt. Only the
target grammar differed. Neither mode produced a useful eligible main result
within 550 ms. The constrained mode delivered two English additions: one neutral
and one harmful. The latter copied `Friday` from a style example despite the
current context specifying Tuesday, arriving in 462.25 ms. No German or Persian
addition was eligible. All six controls per mode had no eligible delivery;
deadlines and rejected output are not evidence of effective coverage.

This fails the frozen 18/24 useful target, +4 gain, four useful cases per language
and zero-harm requirements. The 12 anonymous full-output pairs were graded before
mode/timing mappings were revealed; four additional baseline harms occurred only
in retained ineligible output. Labels are synthetic agent judgments, and output
length can reveal a likely mode despite metadata blinding. This is neither human
acceptance evidence nor an installed-service or Cotypist comparison.

All 60 target opportunities remain counted. There were 60 primers and 55 target
dispatches; five unavailable targets were not silently dropped. All 30 primer
payload pairs matched; 27 target comparisons matched except for the declared
grammar intervention and three remained unknown. Median preparation was roughly
2.78/2.88 seconds and startup 5.26/5.40 seconds for baseline/constrained mode,
separate from the target timer. All 60 owned worker/runtime pairs were absent
after cleanup, with source, binary and input hashes stable. Frozen inputs,
execution and review are in `output/writing/2026-09-09-next-word-comparison/`;
the failed decision is `output/writing/2026-09-09-next-word-result.json`.
No prediction default was promoted.

The subsequent fixed stop-token screen completed all six targets and six primers.
Every owned runtime verified token 715 as bytes `20 0a`. All outputs were exactly
the forced literal plus ASCII space; baseline reported EOS/empty stopping word
and the candidate reported a native newline stop. Both native prediction counters
fell by exactly one in every pair:

| Forced literal | Native prediction count, baseline → stop token | Terminal receipt, baseline → stop token |
| --- | --- | --- |
| `copper` | 5 → 4 | 248.08 → 198.92 ms |
| `Garten` | 5 → 4 | 279.11 → 184.61 ms |
| `پنجره` | 12 → 11 | 907.17 → 728.35 ms |

This verifies the narrower termination mechanism. The three elapsed differences
do not establish repeatable performance or natural prediction quality; Persian
still exceeds 550 ms. Returned token arrays had seven/six IDs in the Persian
pair despite native counts of twelve/eleven, so array length cannot stand in for
all generation steps. Prompt counters matched at one newly evaluated and eleven
reused tokens per target. All six runtimes and their worker were absent after
clean exit, with 75 source hashes and six artifact identities stable. The
execution receipt and independent root review are retained under
`output/writing/2026-09-09-stop-token-mechanism/execution/`. Ordinary prediction
modes still use their existing termination path; applying this mechanism to
natural generation needs a separate quality and latency comparison.

A separate native Hunspell 1.7.3 feasibility probe used pinned German igerman98
and Persian Lilak dictionary/affix pairs with their original notices. The 48-word
corpus was frozen before its one-pass spelling/suggestion calls. All eight
ordinary/inflected examples per language were accepted. The six authored German
typo references ranked first; all six Persian references appeared, but only
three ranked first. One Persian form authored as a typo was dictionary-accepted,
illustrating that intended meaning cannot be inferred from spelling alone.
Unknown names and unfinished prefixes also received alternatives. `NOSUGGEST`
words were spelling-accepted but absent from the native suggestion lists.

Dictionary load times were 34.0 ms German and 89.5 ms Persian. Native suggestion
median/maximum times were 64.7/212.6 ms and 1.0/108.2 ms respectively; these are
single-pass operation timings, with uncontrolled OS caching and no application
delivery measurement. All 48 native suggestion lists and both dictionary handles
were freed. The next correction experiment needs completed-word, name, ambiguity,
Unicode-edit and exact-range guards, plus a bounded execution policy. Receipts,
sources and licenses are in `output/writing/model-research/hunspell-language-data/`;
no dictionary or correction change was installed.

The first 64-request diagnostic collected all results and confirmed every model
process was reaped, but its CLI report is incomplete because the client rejected
the server's final cleanup event. That decoder bug is fixed; preserve the failed
terminal report in `output/writing/2026-09-09-lab-quality-diagnostic/`. Its
five-second experimental outputs are development observations, not a successful
CLI qualification or a fair latency comparison with the 550 ms baseline.

The subsequent 48-request comparison at 550 ms / eight experimental tokens
completed with stable executed identities and 48 successful cleanup receipts in
`output/writing/2026-09-09-lab-matched-550/`:

| Mode | Requests | Suggestions | Reference first words | Substantive first words from blinded agent review |
| --- | ---: | ---: | ---: | ---: |
| Current provider logic | 16 | 9 | 2 | 3 |
| Full context | 16 | 5 | 1 | 3 |
| Full-context boundary healing | 16 | 10 | 5 | 10 |

All shown append suggestions were reviewed, with identical input/output tuples
deduplicated. The reviewer did not author the implementation or corpus and saw
no mode identities or expected answers, but had reviewed an earlier anonymous
batch. These are synthetic development cases and agent judgments, not human
acceptance, observed typing savings, or Cotypist parity. The broader healing
mode still differs from production in prompt content and stopping behavior.

The 18-request ordered trace comparison also completed with verified cleanup in
`output/writing/2026-09-09-lab-traces-550/`. English contextual continuations
improved, while all nine Persian requests exhausted their budget without a
suggestion. Cached slot occupancy is not evidence that the failed Persian
prefill was reused. Accurate multilingual context and writing-style adaptation
remain open; a narrow boundary comparison must not be presented as solving them.

The subsequent isolated confirmation used 120 independently authored short
prefixes: 40 each in English, German and Persian, split equally between trailing
ASCII space and no-space controls. Both modes used actual production logic,
550 ms and eight tokens. All 240 results, executed identities and cleanup receipts
validated; all 60 no-space control pairs had identical model payloads and outcome
categories. Three German control pairs differed only in continuation length near
the deadline. The frozen inputs, reports, independent validation and decision
remain under `output/writing/2026-09-09-boundary-confirmation-*`.

| Trailing-space cases | Requests | Suggestions | Substantive first words | Neutral first words | Unhelpful |
| --- | ---: | ---: | ---: | ---: | ---: |
| Current provider logic | 60 | 40 | 2 | 14 | 24 |
| Isolated boundary fix | 60 | 56 | 6 | 45 | 5 |

The blinded agent review withheld configuration identities and expected answers.
The +4 substantive gain **failed the predeclared +8 gate**, so the isolated fix
remains opt-in and is not promoted. No language lost substantive yield, and bad
outputs decreased, but these do not substitute for the failed primary criterion.
Six low-confidence Persian judgments need native-language adjudication. These
short synthetic cases and agent judgments do not measure human acceptance.

Across all 120 requests per mode, baseline p50/p95 was 316/551 ms and candidate
323/551 ms; 13 baseline and eight candidate results exceeded 550 ms. Among shown
outputs the respective p50/p95 was 323/551 and 327/551 ms. Timing excludes model
startup and adapter/display latency. Neither establishes the visible latency
goal or Cotypist parity. The later paced context-preparation experiment above
measures actual reuse counters and cancellation/queue costs; it also failed its
quality gate.

See [research and hypotheses](../../docs/research/competitive-landscape.md) for
why these experiments were chosen. The older installed-broker evaluator below
remains separate, with its original corpus and evidence contracts.

## Installed-broker corpus evaluation

This is a synthetic evaluation of the **running local-model broker**. It does
not mutate a document, accept suggestions, change policy, launch a runtime or
qualify application coverage. It replaces the twenty-prefix smoke's suggestion
count with separate coverage, errors, latency, reference agreement and an
explicit semantic review step. The old smoke remains a diagnostic.

`manifest.json` locks 24 development and 132 heldout examples by SHA-256. There
are 44 heldout examples each for English, German and Persian, including twelve
mixed-language sentences, partial words, punctuation, uncertain claims, notes,
email, planning and technical prose. These provisional languages were selected
before the user's language preference was known. Persian includes normal
zero-width non-joiners: rejecting those inputs must appear as an input error,
not a model abstention. The corpus was authored synthetically by an assistant;
it is neither private user writing nor a representative natural-text dataset.
References are one plausible future, not an exhaustive answer key.

Use development examples for tuning. The heldout set was frozen before
inference. Implementation authors should receive only heldout aggregates until
the implementation is fixed. Any tuning after examining heldout outputs needs
a new independently authored confirmation set. Do not edit the manifest hashes
to make a modified corpus pass. Tests verify hashes, IDs and split separation.

```sh
node --test evaluation/writing/lib.test.mjs
```

Run one inference job at a time. First inspect `badi status` and the exact
selected broker PID/socket. The broker must be idle, unpaused and already
authorized for the Obsidian adapter. The evaluation client uses its normal
context/suggestion capability contract and never negotiates text replacement;
therefore this lane does **not** evaluate spelling replacement precision.

```sh
node evaluation/writing/run.mjs \
  --split development \
  --socket /run/user/1000/badi/broker.sock \
  --badictl /home/ahura/.local/bin/badictl \
  --broker-pid 12345 \
  --label baseline \
  --output output/writing/baseline-development
```

Replace the example PID, UID and executable path with the inspected session.
Use `--split heldout` and a new output directory after tuning is frozen.
`--language en` selects a language stratum; language subsets must be reported
as subsets. `--limit N` is development-only. Existing run directories are never
overwritten. The script verifies that the selected PID owns the chosen Unix
socket, identifies its unique `llama-server` child, hashes executed binaries
and model bytes, and checks that runtime/process and policy identities remained
stable. Source hashes are recorded separately because a changed checkout does
not identify an older installed binary. `/proc` access makes this runner Linux
specific. It records argv and synthetic text only in private ignored run files;
never pass credentials in runtime command-line arguments.

`samples.json` contains synthetic inputs, generated outputs, reasons and timing.
`summary.json` separates errors, abstentions and displayed suggestions by
language and category. It retains both all-request and suggestion-only latency
distributions so fast unsupported-language refusals cannot make generation look
faster. Timing covers the explicit request through the editor client result,
including transport and inference. It excludes input debounce, adapter drawing,
real field acceptance and cold model startup. The first unprimed sample remains
in the measurements; it is not discarded as inconvenient warmup.

Reference next-word agreement is an exact-match diagnostic. A valid alternative
can disagree with the reference. No semantic score is populated until a reviewer
labels the generated `review-template.json`. Reviewers see the prefix and
suggestion without the reference answer or model name. The template explains
the rubric: count consecutive useful words from the start, taking grammar,
language, factual assumptions and the prefix/suffix seam into account. Record
whether the reviewer is a human or an agent and whether they were independent
of implementation. Agent judgments must not be represented as human trials.

Save completed labels under a new filename, then score them:

```sh
node evaluation/writing/score.mjs \
  output/writing/baseline-development \
  output/writing/baseline-development/review.json \
  output/writing/baseline-development/reviewed-summary.json
```

Every label binds to the exact suggestion hash. Duplicate, stale, impossible
or unlabeled judgments fail; partial reviews report their coverage. Reviewed
useful next words and useful words per suggestion measure hypothetical
usefulness. Actual accepted words and observed keystrokes saved remain `null`:
only real writing trials can supply those quantities. Neither synthetic output
nor an independent agent review establishes Cotypist parity, multilingual
product support, or the end-to-end p95 visible-latency gate.

For a paired baseline/candidate comparison, review every shown suggestion in
both runs and use identical ordered samples. The comparison checks those
conditions and reports language strata independently:

```sh
node evaluation/writing/compare.mjs \
  output/writing/baseline-heldout output/writing/candidate-heldout \
  output/writing/baseline-heldout/review.json \
  output/writing/candidate-heldout/review.json \
  output/writing/comparison.json
```

The useful-next-word yield per request includes abstentions and errors in its
denominator and stays unavailable until every shown suggestion is reviewed.
This complements usefulness among shown suggestions: neither can hide a very
low suggestion rate or a flood of irrelevant predictions.

## 2026-09-07 recovery candidate

The frozen 132-prefix comparison used Qwen3-1.7B Q4_K_M on this workstation.
The candidate broker SHA-256 was
`9b199e92e0cf3aaa7882a115545a31bebee9d3c718182d60b6ebae8273fcdd7d`.
Private synthetic records are retained in `output/writing/2026-09-07-baseline-heldout/`,
`output/writing/2026-09-07-candidate-heldout/` and
`output/writing/2026-09-07-comparison-provisional.json`.

| Measure | Previous installed broker | Recovery candidate |
| --- | ---: | ---: |
| Suggestions / 132 requests | 38 | 94 |
| Input/provider errors | 13 | 0 |
| English suggestions / 44 | 38 | 43 |
| German suggestions / 44 | 0 | 39 |
| Persian suggestions / 44 | 0 | 12 |
| Suggestion latency p50 / p95 | 286 / 346 ms | 346 / 553 ms |
| Reviewed useful next words / shown | 24 / 38 | 63 / 94 |
| Reviewed useful words / shown | 62 / 38 | 155 / 94 |

Coverage improved substantially; suggestion precision improved only slightly in
these provisional judgments, and tail latency regressed. Persian remains a low
coverage, slow experimental path. These measurements do not establish Cotypist
parity or production language quality.

The baseline reviewer was an agent independent of model implementation but had
authored the corpus and knew the run identity. The candidate review was completed
by the integrating agent after the support agents reached their usage limit;
it is explicitly **not independent or blinded**. Thirty-six byte-identical
outputs retain the baseline labels. The remaining labels are provisional root
judgments, not a human study. All shown suggestions were reviewed. Actual
accepted-word and keystroke savings remain unmeasured. Raw heldout outputs have
now been inspected; further model tuning requires a new independent confirmation
set, without editing this frozen corpus.

Separately, the real Chromium 151.0.7922.173 integration with the same model
passed 16 synthetic visible samples at p50 348 ms / p95 589 ms on the final
reconnect implementation, including input debounce. This misses both the 250 ms
median and 500 ms tail targets. Nineteen document acceptances
were exercised across restart, next-word/full acceptance and multilingual cases;
native undo, Escape, stale caret, pause and password exclusion passed. One run
during concurrent repository checks timed out waiting for a partial-word sample;
its failed result is retained as
`output/playwright/2026-09-07-browser-load-failure.json`. The subsequent complete
run before the late-bootstrap race fix measured 349/481 ms and is retained in
`output/playwright/2026-09-07-browser-recovery-final/`. The final source run is in
`output/playwright/2026-09-07-browser-recovery-final-02/`. This small
application lane does not prove reliability under sustained load.

## 2026-09-07 development prompt and model experiments

These later experiments used only the 24 development prefixes (eight each in
English, German and Persian), never the exposed heldout outputs. Each comparison
manifest was frozen before generation. They ran private, serial llama.cpp b10726
children on the i7-12700H CPU, with four inference/batch threads, 512 context
tokens and no GPU layers. Each arm started a fresh runtime; the first cold
request remains included. All five children were reaped. No installed model,
normal runtime or service was changed.

The actual Rust `CompletionProvider::propose` ran against a local streaming
proxy that changed only the frozen prompt format. Temperature 0, seed 42, the
8-token budget (12 for existing English word healing), healing grammar, language
and word-seam guards, and 550 ms provider budget remained unchanged. Replacement
was disabled, so spelling-context cases do not evaluate correction. The proxy
also recorded full generation after a provider disconnect; those later words
are hypothetical continuations, not displayed suggestions. Engine/adapter
binding and drawing latency are outside this experiment.

The 1.7B comparison used the installed Q4_K_M artifact, SHA-256
`d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5`.
The two instruction arms used the same short continuation task, either asking
for the missing suffix or prefilling the assistant response with the typed
prefix. The non-thinking template was checked byte-for-byte against
[Qwen's pinned tokenizer template](https://huggingface.co/Qwen/Qwen3-1.7B/raw/70d244cc86ccca08cf5af4e1e306ecf908b1ad5e/tokenizer_config.json).
The 4B comparison used dedicated non-thinking
[Qwen3-4B-Instruct-2507](https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507),
with its correct template, without a think block. Its private Apache-2.0
[Unsloth Q4_K_M artifact](https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/tree/a06e946bb6b655725eafa393f4a9745d460374c9)
was 2,497,281,120 bytes, SHA-256
`3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597`.

| Model / prompt | Proposals EN / DE / FA (each /8) | All-request provider p50/p95 ms EN / DE / FA |
| --- | --- | --- |
| 1.7B raw prefix | 8 / 7 / 6 | 261/334 · 271/347 · 394/501 |
| 1.7B chat suffix | 2 / 0 / 0 | 372/551 · 400/492 · 488/552 |
| 1.7B chat prefill | 7 / 6 / 3 | 390/552 · 450/552 · 552/552 |
| 4B raw prefix | 7 / 6 / 1 | 552/552 · 552/552 · 552/552 |
| 4B chat prefill | 0 / 0 / 0 | 552/552 · 552/552 · 552/552 |

Proposal counts describe delivery through provider guards, not usefulness. The
implementation agent reviewed every shown and full output separately; judgments
are neither independent, blinded nor human. The 1.7B suffix arm often repeats
the input or omits the leading space. Prefill improves some English wording
(a delayed train can be waited for; cups go on a countertop), but invents a
browser-specific cache and remains unreliable in Persian. Raw output can retain
the draft topic yet later invent an unrelated cognitive-dissonance explanation.
German/Persian partial-word completions can be withheld by the existing seam
guard even when the full model output completes the word correctly.

The 4B prefill output sometimes improves meaning, such as expiring a cache when
data becomes stale, but its full-generation p50/p95 was 756/2422 ms in English,
1454/2308 ms in German and 1934/2823 ms in Persian. Its peak process RSS was
4226 MiB. Decode speed decreased from about 13 to 9 tokens/s during the ordered
prefill arm; no root-agent heavy builds or inference were concurrent, but
thermal/background effects were not instrumented. These are single ordered
development passes, not a controlled sustained-load benchmark. **Neither chat
format nor 4B is selected as a new production default.**

Exact manifests, prompts, token events, provider results, source/binary hashes,
qualitative reviews and child cleanup identities are retained under
`output/writing/2026-09-07-prompt-format-development/` and
`output/writing/2026-09-07-four-billion-development/`. The earlier private base
model/context comparison is in
`output/writing/2026-09-07-quality-development-02/`; a relevant English example
initially favored preserving preceding sentences, but the later cold-context
ablation below rejected that production change. It did not justify a base-model
switch.

### Context regression and rollback

A new independently authored 120-prefix confirmation set (40 per language,
matched scenario families) was frozen before the installed baseline and candidate
ran sequentially against identical private copies of the installed CPU model and
runtime. The candidate preserved preceding sentences within the 160-scalar
window. Baseline/candidate emitted 53/8 suggestions, with zero errors and
all-request p50/p95 latency 407/553 versus 553/573 ms. Suggestions by language
were English 27/8, German 11/0 and Persian 15/0. These are availability and timing
measurements, not semantic quality. The implementation agent received aggregates
only; a reviewer received blinded outputs through the integrating
agent. The candidate context-preservation change was rejected.

The blinded agent reviewed 56 randomized, deduplicated prefix/output pairs
representing all 61 shown source outputs. Reference continuations, model/run
identities and timing were withheld. Baseline/candidate had 10/3 useful first
words and 26/4 useful words in total. Useful-first-word yield was therefore
10/120 versus 3/120 requests; baseline's ten comprised six English, three German
and one Persian, while the candidate's three were English. Both remain poor on
this synthetic set. The reviewer did not author the corpus or provider
prompting/decoding, but had contributed English lexicon assets, safety reviews
and integration work. This is **not strict implementation independence**; these
are blinded agent judgments with disclosed prior project involvement, with no
observed human acceptance or Cotypist comparison. Labels and scored
comparison are retained in
`output/writing/2026-09-07-independent-comparison/independent-comparison.json`.
An append-only `review-limitations.json` beside that comparison records the later
independence clarification without rewriting frozen outputs, labels or scores.
The reviewer has now seen the confirmation text, so future confirmation must
use a new independent corpus; it must never become implementation tuning data.

To diagnose without tuning on confirmation text, a separate source copy restored
only the original sentence-clipping loop, retaining the current client,
dictionary corrections and scalar-boundary fix. On the original 24 development
cases, installed baseline/current candidate/restored context emitted 20/21/20
suggestions with zero errors. All 24 restored outputs or abstentions exactly
matched baseline. All-request p50/p95 was 340/455, 352/461 and 354/460 ms. These
mostly short cases did not expose the longer-context regression.

A development stress set was then frozen by prepending neutral same-language
notes to those existing development cases. Repeated headers reused 17–42 KV
tokens, so its warm aggregate hid much of the cold cost. In its first Persian
request, the preserved prompt's first token arrived at 674 ms (58 prompt tokens),
already beyond the 550 ms provider deadline; sentence clipping reduced this to
71 ms (four tokens). A second, separately frozen stress set exercised the existing
160-scalar ceiling, with `cache_prompt: false` in **both private experimental
arms** to isolate cold prefill. Production prefix caching remained enabled.
The actual Rust provider and its token budgets, language/word guards and deadline
were unchanged; a local proxy changed only the prompt clipping and cold-cache
flag and recorded full generation after client disconnect.

| Cold development measure, eight per language | Preserved preceding sentences | Current sentence |
| --- | ---: | ---: |
| English first-token p50 / p95 | 390 / 425 ms | 109 / 126 ms |
| German first-token p50 / p95 | 489 / 537 ms | 139 / 172 ms |
| Persian first-token p50 / p95 | 962 / 1016 ms | 148 / 261 ms |
| Requests whose first token missed 550 ms | 8 / 24 | 0 / 24 |
| Provider-result p50 / p95, all requests | 551 / 552 ms | 301 / 453 ms |
| Suggestions / requests | 12 / 24 | 20 / 24 |
| Provider errors | 0 | 0 |

This identifies cold prefill as a concrete regression mechanism. It supports
restoring the current-sentence context boundary while keeping dictionary
correction; it does not establish usefulness of the resulting predictions or
eliminate the loss of prior-subject context. Runtime/model defaults, correction
authority and full context used for edit binding were unchanged. Decimal/domain
punctuation, Persian question marks, newlines and final punctuation have focused
regression coverage.

Exact private binaries, source copies, frozen development variants, full token
events and cleanup receipts are retained in
`output/writing/2026-09-07-context-ablation/`. The cold development corpus SHA-256
is `e8acca0bdaa2db6fcb6ee2ffbe5c9d26b1ae680f41f1609f8af0cd5cca0d4352`.
The three real-broker arms and four proxy arms ran serially and reaped their
children. An initial private broker startup rejected a symlinked model-directory
path; the failed receipt was retained, then the supervisor used the verified
absolute private asset directory. No normal service was involved. Confirmation
records are in `output/writing/2026-09-07-independent-comparison/`; neither those
prefixes nor output words were inspected for this diagnosis.

### Private Vulkan follow-up

The host already provided Mesa `vulkan-intel` 26.2.1, Vulkan 1.4.354 and a working
Intel Iris Xe ADL GT2 device (`0x46a6`). The installed Badi runtime has CPU/RPC
backends only. The matching official
[b10726 Vulkan runtime](https://github.com/ggml-org/llama.cpp/releases/tag/b10726)
was downloaded into the private experiment directory: archive
`llama-b10726-bin-ubuntu-vulkan-x64.tar.gz`, 33,769,460 bytes, SHA-256
`ec04ea79f6e40d3fa2f602490709ff52a4a93c6f4fa76bc59259be268c308a64`,
source commit `85c55223caf0a2ad0d1d88e5a73ab3fe36107867`, MIT license.
No global package, driver or installed runtime was changed. The project's
[Vulkan instructions](https://github.com/ggml-org/llama.cpp/blob/b10726/docs/build.md#vulkan)
describe full-layer offload and this Intel device family.

Four fresh private runtimes repeated the same frozen development inputs,
prompts and sampling with `Vulkan0` and 99 GPU layers. GPU execution was checked
through 4B process DRM render counters and GPU allocations; a separate 1.7B
diagnostic log confirms 29/29 layers offloaded. That supplementary diagnostic
had an fdinfo enumeration race after generation; its verbose offload log and
successful child cleanup remain recorded. It is not a quality/timing sample.
All four measurement children and the diagnostic child were reaped.

| Vulkan model / prompt | Proposals EN / DE / FA (each /8) | All-request provider p50/p95 ms EN / DE / FA |
| --- | --- | --- |
| 1.7B raw prefix | 8 / 7 / 6 | 496/551 · 323/551 · 487/515 |
| 1.7B chat prefill | 8 / 6 / 6 | 361/552 · 299/528 · 482/518 |
| 4B raw prefix | 5 / 5 / 3 | 551/552 · 551/552 · 552/552 |
| 4B chat prefill | 0 / 0 / 0 | 551/552 · 551/552 · 551/552 |

Vulkan improves some longer-prefill cases but slows this 1.7B raw-prefix pass.
The 4B chat-prefill full-generation p50/p95 remains 875/1751 ms in English,
924/1542 ms in German and 1529/1730 ms in Persian. Its low process RSS is not
total memory consumption: DRM records up to 2891 MiB of GPU-resident shared
system memory. Sampled CPU package temperatures across the GPU passes were
55–70°C; this was not a continuous power or sustained-load measurement.

Thirty-five of 96 full outputs differ from the CPU passes, so CPU semantic
labels cannot be copied indiscriminately. Faster delivery still produces the
unhelpful German rain/lock-the-house phrase and malformed Persian book-location
text. No quality score or runtime promotion follows from the delivery counts.
CPU/GPU passes were sequential and not interleaved or randomized; these results
identify useful follow-up measurements, not a controlled hardware ranking or
physical desktop qualification. The default remains unchanged.

Runtime provenance, device proof, changed-output pairs and summaries are under
`output/writing/2026-09-07-vulkan-development/`. Full records are in
`output/writing/2026-09-07-vulkan-17b-development/` and
`output/writing/2026-09-07-vulkan-4b-development/`.

### Qwen3.5-0.8B base comparison

The ungated Apache-2.0
[Qwen3.5-0.8B-Base Q8_0 artifact](https://huggingface.co/ggml-org/Qwen3.5-0.8B-Base-GGUF)
was downloaded privately: 833,591,872 bytes, SHA-256
`02fbcf13dd07601e5801cdef73ace47426c422742fa68fd0c607a8b88eea0790`,
repository revision `f78a2c518573cedd5585e5e368a6e1844d2ebbda`.
Its license, model card and tokenizer/configuration are preserved in
`output/writing/model-research/qwen35-base-08b/`. No installed asset was replaced.

A 192-request development comparison was frozen before generation: the original
24 prefixes plus the already-frozen 24 long-context stress variants, each with
current-sentence and full bounded-context prompts, for both the installed 1.7B
and new 0.8B model. The actual restored Rust provider retained the 550 ms deadline,
8/12-token budgets, English healing grammar, language/word guards and disabled
replacement capability. Both used private b10726 CPU runtimes, four threads,
512 context tokens and prefix caching. Model order was 1.7B then 0.8B, with a new
runtime per arm; this single ordered pass was not counterbalanced. In particular,
timings slowed during the second 1.7B arm, so model size alone must not be treated
as the established cause of every timing difference.

A blinded agent reviewed all 93 deduplicated prefix/output pairs representing
126 shown source suggestions, without references, timings or arm mapping. The
reviewer disclosed earlier lexicon-asset, safety-review and model-research work;
strict implementation independence is false. These are hypothetical usefulness
judgments, not human acceptance measurements. Each language/length stratum has
eight synthetic cases, with paired scenario families rather than independently
sampled natural writing.

| Short development measure | 1.7B current sentence | 0.8B current sentence | 0.8B bounded context |
| --- | ---: | ---: | ---: |
| English useful first words / 8 requests | 6 | 5 | 6 |
| German useful first words / 8 requests | 5 | 3 | 4 |
| Persian useful first words / 8 requests | 5 | 2 | 2 |
| English provider p50 / p95 | 279 / 340 ms | 280 / 493 ms | 262 / 487 ms |
| German provider p50 / p95 | 312 / 382 ms | 249 / 339 ms | 286 / 366 ms |
| Persian provider p50 / p95 | 418 / 494 ms | 329 / 386 ms | 308 / 473 ms |

The 0.8B bounded-context arm's long-case useful-first-word counts were 6/8 English,
5/8 German and 0/8 Persian. Its Persian long prompts still missed the provider
deadline. Sampled maximum process RSS was approximately 1.05 GiB for 0.8B versus
1.97 GiB for 1.7B, excluding any shared accounting caveats. **The smaller model
is not promoted:** its speed and memory gains did not establish better writing
usefulness. All four generation runtimes exited normally and were reaped.

The paired plan, raw events, timings, labels mapping and reviewed counts are in
`output/writing/2026-09-07-qwen35-base-development/`; the matched 1.7B control is in
`output/writing/2026-09-07-qwen35-control-17b-development/`. The reviewed anonymous
artifact is `output/writing/qwen35-blind-development-reviewed.json`, SHA-256
`d985c7bd84d4065febb6fdf92e800c2a51423e7870c03e8cea6242c7b151091a`.

Both models also completed the same 12-request private repeat/cancel/restart
sequence. Cancellation returned in 6–12 ms, but immediately following requests
could abstain or fail at 550–572 ms; fresh runtime starts restored normal output.
This is an existing runtime integration concern, not proof of a new hybrid-model
bug. Four additional private 1.7B lifecycle arms compared default/batch-16 with
and without `return_progress`. Batch-16 improved some immediate recovery cases;
progress alone did not. All 12 lifecycle runtimes exited normally and were
reaped. A subsequent clean 48-request development comparison found all 24
provider outputs/abstentions byte-identical between default batches and
batch/ubatch 16. Both produced 20 suggestions and zero errors. All-request
p50/p95 were 308/507 ms versus 300/478 ms; per-language p50 was English 271/270,
German 288/294 and Persian 424/405 ms. This ordered, small comparison found no
short-text penalty; it does not establish a general speedup or better wording.
Both owned runtimes exited 0 and were reaped. The preceding run overlapped a
Cargo build and is excluded entirely, including deadline-dependent output
counts. Both runs remain preserved under `output/writing/2026-09-07-batch-development/`
and `output/writing/2026-09-07-batch-development-clean/`.

Writing launches now set both logical and physical prefill batches to 16;
historical evaluation launch defaults remain unchanged. Smaller batches let
the pinned runtime observe cancellation sooner between decode operations.
They do not remove the runtime HTTP disconnect polling delay or guarantee
immediate recovery from every interrupted prompt. `return_progress` remains
disabled. See the [pinned runtime batch parameters](https://github.com/ggml-org/llama.cpp/blob/b10726/tools/server/README.md)
and [decode/cancellation handling](https://github.com/ggml-org/llama.cpp/blob/b10726/tools/server/server-context.cpp).

A matched 24-request follow-up used the actual owned launcher, a persistent
`SemanticClient`, and the production multithread Tokio executor. Default versus
batch-16 immediate post-cancel results were English 551/429 ms with a suggestion,
German 551 ms abstention/475 ms suggestion, and Persian `provider_unavailable`
at 571 ms in both arms. The harness discarded the underlying client error, so
the precise failure cannot be established retrospectively. Fresh runtime starts
had no provider errors. All four runtimes
exited 0 and were reaped; shutdown immediately after the active sequence took
515/509 ms, versus 82 ms after the fresh sequence. This is a bounded recovery
improvement, with the Persian case still unresolved. Writing runtime identities
include the batch and ubatch values; historical semantic identities omit them.

An earlier nine-request owned probe used a single-thread Tokio executor and
reached the two-second shutdown fallback before stopping at its exit-status
assertion. Synchronous shutdown can prevent background HTTP cleanup on that
executor. It is preserved as a harness limitation, separately from the matched
production-executor results, under `output/writing/2026-09-07-writing-batch-owned/`.
The matched plans, source copies, exact wait statuses and results are in
`output/writing/2026-09-07-writing-batch-persistent/`. No connection-pool change
was made or inferred necessary from the single-thread probe.

Read-only follow-up found a separate writing deadline gap: the 550 ms stream
timer began only after HTTP headers, while an outer 570 ms timer converted a
slower header wait into `provider_unavailable`. The pinned runtime can
[wait for the first generated result before sending streaming headers](https://github.com/ggml-org/llama.cpp/blob/b10726/tools/server/server-context.cpp#L4354-L4387).
This strongly fits the observed timings; missing phase/error metadata prevents
proving it caused those two failures. The original results remain unchanged.

Writing requests now apply the same absolute 550 ms budget to header acquisition
and streaming. A spent header budget drops the request and returns no output,
no first-token time and zero received body bytes. It uses the existing
`ModelAbstained` convention for budget exhaustion without validated output;
that label does not imply a deliberate model refusal or establish runtime
health. Cancellation takes priority, actual transport/status/stream errors stay
errors, and an exhausted spelling attempt cannot enqueue a new continuation.
Historical semantic deadlines and dispositions remain unchanged. Socket
regressions cover these boundaries, closure and subsequent requests through the
same client. The source-bound diagnosis is preserved as
`deadline-diagnosis.json` in the matched persistent-client experiment directory.

One completed 12-request follow-up repeated the original plan through an owned
batch-16 runtime and persistent client, preserving immediate request ordering.
Cancellation returned in 10.5/11.4/11.6 ms. The following English/German requests
returned suggestions at 523/551 ms; Persian returned no suggestion and no
provider error at 551 ms. Private instrumentation directly recorded that
Persian header-budget expiry with zero received body bytes. Fresh runtime
requests had no provider errors. Both children exited 0 and were reaped;
shutdown took 529/92 ms after the active/fresh sequences. This verifies bounded
operational behavior, not useful predictions, faster inference, or immediate
upstream cancellation. It does not retroactively prove the old error's cause.
The plan, exact source/binary identities, thermal samples, results and wait
statuses are in `output/writing/2026-09-08-writing-header-owned-immediate/`.
The unchanged 12-request plan SHA-256 is
`255204bccb7056595577dd6fe434aae7e4dc774e68d4d002a1f4b0dd6bb136cc`.

Two instrumentation attempts remain preserved separately: sensor reads after
scheduling cancellation made the first attempt cancel before inference; moving
them before scheduling still added approximately 74 ms between requests in the
second. Neither qualifies as the matched immediate-recovery measurement. The
final probe samples sensors only at runtime boundaries. Full Rust formatting,
Clippy, workspace tests and Rust 1.85 checks passed for the source fix, with
commands/logs in `output/writing/2026-09-08-header-deadline-fix/`.

### Qwen3.5-2B base and tokenizer-bounded context

A further frozen development comparison tested
[Qwen3.5-2B-Base](https://huggingface.co/Qwen/Qwen3.5-2B-Base/blob/b1485b2fa6dfa1287294f269f5fb618e03d52d7c/README.md)
using an ungated Apache-2.0
[Q4_K_M conversion](https://huggingface.co/mradermacher/Qwen3.5-2B-Base-GGUF/tree/8141b666d4a4202269c7b5ead243be45b5bb7491).
The 1,274,396,800-byte artifact's SHA-256 is
`1c42c644e4018f02ddf736e08426aa4597ba76f9de2aa649ef4cd669231743d3`.
The converter does not disclose its exact upstream source revision; independent
conversion reproducibility was not verified. Provenance and license files remain
under `output/writing/model-research/qwen35-base-2b/`.

Four sequential arms compared the existing 1.7B model and 2B Base with either
current-sentence clipping or at most 40 tokenizer tokens of recent context.
The 160-scalar outer bound, exact suffix/word boundaries and existing English
healing remained intact. Calls to the pinned runtime's `/tokenize` endpoint
were inside the actual Rust provider's 550 ms deadline. All arms used default
runtime batches independently of the writing-only batch-16 change. The same
48 pre-frozen development inputs were reused: 24 short inputs and their paired
24 long neutral-note variants, eight scenario families per language. All four
runtime children exited 0 and were reaped. Model order was 1.7B, 2B, 2B, 1.7B;
this does not eliminate thermal, cache or other time-order effects.

A second blinded agent review labeled 93 deduplicated pairs from 121 shown
outputs, with model/context mapping, references and timings withheld. This
reviewer disclosed implementation involvement and used a permissive plausibility
rubric, crediting grammatical open alternatives and single function words.
Consequently these scores must not be compared directly with the earlier
reviewer's labels or treated as human benefit.

| Useful first word per eight requests, EN / DE / FA | 1.7B sentence | 2B sentence | 1.7B 40 tokens | 2B 40 tokens |
| --- | --- | --- | --- | --- |
| Short inputs | 7 / 6 / 5 | 6 / 5 / 4 | 8 / 7 / 5 | 7 / 6 / 5 |
| Paired long inputs | 7 / 6 / 5 | 6 / 5 / 4 | 0 / 4 / 1 | 0 / 0 / 0 |

Short current-sentence provider p50 was 258/278/393 ms for 1.7B and
314/323/369 ms for 2B, by English/German/Persian. Counting tokens cost at most
23 ms; the larger prompt's inference cost still consumed the deadline in the
long-context arms. **Neither the 2B model nor 40-token context was promoted.**
This small development result provides no quality or Cotypist parity claim.
Exact plans, source/binary identities, events, lifecycle results, blinded labels
and mappings are in `output/writing/2026-09-07-qwen35-2b-token-development/`.
The reviewed label file's SHA-256 is
`973c5c973664c950c303f4f6acabeb6bffb63d9dd1f58b3b192ee5844ceb36af`.

### Complete-word delivery and 20-token context

The frozen 2×2 development comparison completed all 240 requests with the
current 1.7B model, batch/ubatch 16, the unchanged 550 ms budget and existing
8/12-token generation limits. Each arm used 24 short inputs, their 24 paired
long neutral-note variants, and twelve prior-sentence cue cases. The original
plan is `output/writing/2026-09-07-word-context-development-plan/plan.json`,
SHA-256 `b80c41457bf39ce792a865f4a9def6f4d31040b5397fb2f2e08da349171e4034`;
the corpus SHA-256 is
`122169e9766cf4a14f37659f648fc0698a84a9ada3189765d1c4613221953997`.
Sources and generation were frozen before the separate header-deadline fix.
The proxy drained every upstream generation before the next request; this is
not a persistent-client typing or cancellation-throughput test. All four
private runtime children exited 0 and were reaped.

A blinded agent labeled all 96 deduplicated prefix/output/language pairs with
arm mapping, references and timings withheld. The rubric required complete
words at the actual seam, appropriate language, grammar and context, and
rejected unsupported specific facts. It separately classified a substantive
first word and a plausible neutral function word. Neutral words may receive
defensible-word credit, so they must not be presented as substantive content.
The reviewer disclosed prior native, lexicon and development-review work;
these are agent judgments, not independent human acceptance or Cotypist
comparison evidence. This rubric also differs from earlier reviews.

Each cell below lists English / German / Persian. Short and paired-long groups
contain eight requests per language; cue groups contain four matched scenario
families per language, not twelve independent natural writing situations.

| Substantive first words | Sentence, four words | Sentence, first word | 20 tokens, four words | 20 tokens, first word |
| --- | --- | --- | --- | --- |
| Short | 4 / 3 / 2 | 4 / 3 / 0 | 5 / 3 / 1 | 5 / 3 / 1 |
| Paired long | 4 / 3 / 2 | 4 / 3 / 0 | 1 / 2 / 1 | 3 / 2 / 2 |
| Prior-sentence cue | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |

| Neutral first words | Sentence, four words | Sentence, first word | 20 tokens, four words | 20 tokens, first word |
| --- | --- | --- | --- | --- |
| Short | 3 / 4 / 2 | 3 / 4 / 1 | 2 / 4 / 1 | 2 / 4 / 1 |
| Paired long | 3 / 4 / 2 | 3 / 4 / 0 | 2 / 3 / 0 | 2 / 3 / 0 |
| Prior-sentence cue | 0 / 0 / 0 | 0 / 0 / 1 | 0 / 0 / 0 | 0 / 0 / 0 |

| Provider p50, ms | Sentence, four words | Sentence, first word | 20 tokens, four words | 20 tokens, first word |
| --- | --- | --- | --- | --- |
| Short | 267 / 309 / 414 | 264 / 344 / 518 | 457 / 551 / 552 | 290 / 346 / 548 |
| Paired long | 262 / 315 / 441 | 279 / 313 / 499 | 551 / 525 / 552 | 538 / 383 / 551 |
| Prior-sentence cue | 552 / 551 / 551 | 375 / 392 / 492 | 551 / 551 / 551 | 544 / 551 / 551 |

The four arms shown in table order produced 42/36/28/29 proposals out of
60 requests each, with zero provider errors. Total defensible words were
95/30/58/28; the one-word arms inherently cap this measure and it is not actual
keystroke savings. First-word credit including neutral words was 36/30/25/28.
Token counting cost at most 34 ms inside the budget. The 20-token window kept
the original prior-sentence cue occurrence in all four English and German
cases, but none of the four Persian cases. No case was rewritten or excluded
after tokenization. Cue retention alone produced no substantive first words.

Fixed arm order and variable runtime performance limit causal latency claims.
For example, raw first-content p50 on short English inputs changed from
101 ms in the sentence/four-word arm to 155 ms in sentence/first-word, before
the delivery choice could shorten subsequent decoding. Raw content may include
healing echoes and is not a validated visible token. Thermal and single-core
frequency samples were retained, but they do not prove a power-policy cause.
No confirmed external heavy CPU overlap occurred. **Neither first-word delivery
nor 20-token context was promoted:** the small English contextual gain did not
establish a multilingual quality/latency improvement, and the cue cases remain
unsuccessful. Preserve the current defaults pending stronger evidence.

Full per-language/group p95 timings, errors, substantive/neutral labels, private
arm mapping, runtime events and cleanup receipts are in
`output/writing/2026-09-08-word-context-development/`. The frozen reviewed file
is `blind-reviewed.json`, SHA-256
`1f60ff7cec28055f54c6313fadd45c95ed31d8a9b3704f871c86277f1252a644`;
`reviewed-comparison.json` validates its identity fields against the unchanged
anonymous artifact before aggregation. The separately frozen rubric is
`output/writing/2026-09-08-firstword-blind-review-rubric.json`, SHA-256
`810b711db1bcfa3582c28626b3a03ac5e9cdaae82a954c0317bbe76b44ee3e63`.

### Next quality strategy and proposed gates

The completed ablations do not support a blanket smaller-model, larger-context
or first-word switch. Next quality work should test **confidence-based selection
of complete words at matched coverage**, separately by language and input
category, before another delivery change. Merely returning a common article or
preposition does not demonstrate useful prediction. Keep dictionary spelling
independent of clause
inference; obtain properly licensed language dictionaries before extending
German/Persian word correction or relaxing their word-seam guard. This direction
is consistent with Google's documented separation of small typing models from
larger proofreading models in
[Gboard's production architecture](https://research.google/blog/synthetic-and-federated-privacy-preserving-domain-adaptation-with-llms-for-mobile-applications/).

Read-only analysis of the 2B comparison's 1.7B control found short current-sentence
p50 first-content times of 117/136/151 ms for English/German/Persian, versus
155/185/284 ms for the first whitespace-delimited span and 258/278/393 ms for
the current provider result. Those span times are estimates from captured token
events, not proposals validated by the Rust word/language guards. In contrast,
all eight long English 40-token requests missed the deadline before the first
content token (p50 621 ms); four German and six Persian requests did too.
Earlier word delivery cannot rescue prefill that already exceeds the deadline.

Collect bounded pre-sampling token probabilities in an isolated experiment and
calibrate sequence/word confidence against independently reviewed usefulness,
separately by language and partial-word category. Probability is a feature, not
a correctness guarantee. Smart Compose's research uses normalized sequence
confidence and compares models at matched coverage; that supports reporting a
precision/coverage curve instead of optimizing suggestion count.
[Smart Compose paper, sections 3.3–4.2](https://arxiv.org/pdf/1906.00080)

The existing `cache_prompt=true` already reuses shared prefix computation.
Measure fresh typing traces with unchanged context prefixes before inventing
another cache. Output reuse across revisions is a separate authority change:
the engine currently retires old derived text on commit. Any future speculative
proposal must acquire fresh exact context, receive a new authority check, and
pass policy, language, focus, suffix, cancellation and expiry gates; no old
commit token or restarted TTL may be inherited. KV reuse does not grant editing
authority. See the pinned
[llama.cpp completion contract](https://github.com/ggml-org/llama.cpp/blob/b10726/tools/server/README.md#post-completion-given-a-prompt-it-returns-the-predicted-completion).

Draft-model speculative decoding is lower priority on this CPU: its benefit
depends on accepted draft tokens and efficient parallel verification, and it
does not repair bad target-model predictions or cold prompt processing. The
published speedups apply to the paper's tested models/hardware, not this host.
[Speculative decoding paper](https://proceedings.mlr.press/v202/leviathan23a.html)

Before promoting a candidate, freeze a new independent confirmation set and
review protocol, including natural writing and synthetic boundary controls.
Proposed experiment gates, not current product claims:

- At least 200 independent prefixes per language, plus dedicated partial-word
  and correct-word controls; keep calibration and confirmation disjoint.
- At matched coverage, improve reviewed useful-word yield per request by at
  least 20% relative to the frozen baseline, with no language regressing. Report
  paired confidence intervals, abstentions, errors and incomplete/generic output.
- Reach warm **visible** p50 ≤250 ms and p95 ≤500 ms including debounce and
  rendering; additionally target ≤100 ms p95 for local dictionary corrections.
  Report first-focus cold latency, typing-trace cache hits/misses, and a 30-minute
  sustained run independently. A lower provider timeout does not count as speed.
- Preserve zero stale or unauthorized mutations in targeted race tests and
  prove accept-word, full acceptance, dismissal, focus changes and native undo
  in each named application. Measure actual accepted words/keystrokes only in
  real writing trials; synthetic reviews cannot establish Cotypist parity.
