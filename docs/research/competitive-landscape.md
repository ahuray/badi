# Badi competitive landscape

Original research snapshot: **2026-08-29**, with the primary-source update below
on **2026-09-07**. Historical project activity counts and model descriptions in
the original sections retain their snapshot date.

## 2026-09-09: a prediction lab before another model switch

The next useful deliverable is a local test workspace that separates the user's
draft, surrounding context, explicit writing examples and expected continuation.
Expected outputs are evaluator labels and must never enter generation prompts,
retrieval indexes or personalization statistics. The workspace should expose
the actual prompt, generated text, display rejection reason and timing, so a
poor model choice can be distinguished from lost context, a token boundary
problem or a deadline. UI polish and application coverage follow that evidence.

This recommendation follows the rejected experiments in the
[writing evaluation record](../../evaluation/writing/README.md). Production
already uses prefix caching, a 160-scalar/current-sentence window and English
partial-word healing. Neither a longer cold prompt, an instruction template,
first-word delivery nor the tested smaller/larger alternatives established a
better default. Repeating those comparisons without a new mechanism would add
little information.

### Primary-source findings

- [Smart Compose's paper, sections 3 and 5](https://arxiv.org/pdf/1906.00080)
  treats useful completion as contextual prediction plus selective triggering.
  It compares predictions at matched coverage and combines its general model
  with a lightweight personal n-gram model. These are precedents for a Badi
  experiment; Google's email corpus, serving hardware and reported gains do not
  transfer to this laptop.
- [LaMP](https://aclanthology.org/2024.acl-long.399/) evaluates retrieval of
  relevant writing from an explicit user profile for personalized generation.
  Its tasks are not next-word typing. It supports testing a few relevant
  examples against no examples and irrelevant examples, rather than assuming
  that a large pasted biography improves completion.
- [The ICML 2025 token-to-character paper](https://arxiv.org/html/2412.03719v2)
  shows that trailing spaces and partial tokens can distort continuation.
  Its full algorithm and token-healing baseline are distinct: backing up one
  token is only a heuristic and can still fail. This identifies a measurable
  boundary hypothesis, not a ready-made Badi quality improvement.
- The pinned [llama.cpp b10726 server contract](https://github.com/ggml-org/llama.cpp/blob/b10726/tools/server/README.md)
  describes common-prefix KV reuse, prompt-only evaluation with `n_predict: 0`,
  explicit slots, token probabilities and per-token timing. The
  [implementation](https://github.com/ggml-org/llama.cpp/blob/b10726/tools/server/server-context.cpp#L1953-L2075)
  distinguishes raw-logit probabilities from sampling-chain probabilities and
  attaches probability data to partial responses. Verify actual response
  fields against this version. A high token probability is not a calibrated
  chance that a human will find the word useful; forced healing echoes must
  not count as newly predicted words.
- Qwen publishes both [pretrained Qwen3-1.7B-Base](https://huggingface.co/Qwen/Qwen3-1.7B-Base)
  and [post-trained Qwen3-1.7B](https://huggingface.co/Qwen/Qwen3-1.7B).
  Model-card chat benchmarks do not decide which completes personal prose
  better. The post-trained model's thinking-mode warning about greedy decoding
  is not evidence that random sampling improves short, non-thinking
  autocomplete. A same-size base comparison is lower priority than fixing the
  observable input and evaluation loop.

### Context reuse: source audit before implementation

The b10726 tag resolves to commit
`85c55223caf0a2ad0d1d88e5a73ab3fe36107867`. Its
[timing serializer](https://github.com/ggml-org/llama.cpp/blob/85c55223caf0a2ad0d1d88e5a73ab3fe36107867/tools/server/server-common.cpp#L67-L79)
reports `timings.cache_n` as reused prompt tokens and `timings.prompt_n` as newly
processed prompt tokens. `tokens_cached` is final slot occupancy and cannot
substitute for either. Experimental Lab streams now collect both optional
terminal counters without enabling prose/debug logging; production-code modes
leave reuse unavailable. Parsing/source verification is separate from measuring
whether a particular priming operation actually reuses context.

The installed binary confirms a contract discrepancy. The
[request schema](https://github.com/ggml-org/llama.cpp/blob/85c55223caf0a2ad0d1d88e5a73ab3fe36107867/tools/server/server-schema.cpp#L44-L48)
advertises zero generated tokens for `n_predict: 0`, while the
[completion path](https://github.com/ggml-org/llama.cpp/blob/85c55223caf0a2ad0d1d88e5a73ab3fe36107867/tools/server/server-context.cpp#L3791-L3869)
appears to sample before its prediction-budget check. The six-request fixed-input
probe (`output/writing/2026-09-09-prefill-probe/`) observed one returned/generated
token in every streaming and non-streaming request, while resolved `n_predict`
remained zero. Both fresh runtime groups shut down and reaped with exit code zero.
This is one-token priming, not prefill-only evaluation; a future warming experiment
should explicitly request one token and discard it.

The tiny cold prompt evaluated 14 new tokens with zero reuse. Its identical
repeat reused 13 and evaluated one; the appended prompt reused 14 and evaluated
one. Request time was 168–176 ms cold and 33–35 ms afterward, with approximately
3.35 seconds of model startup reported separately. This confirms the counter
semantics and reuse mechanism for those disposable prompts. It does not establish
larger-context speed, cancellation recovery or typing latency.

Any warming experiment must use only context already available at each typing
event. Pace explicit development traces, keep one active job and one replaceable
latest snapshot, and start the useful-result deadline at the latest typing event.
Charge cancellation recovery, queueing, token preflight and generation against
that deadline; report priming and first cold-request cost separately. Changed
fields/context and explicit clearing must cancel and reap the owned runtime.
Successful warm timing would demonstrate a Lab mechanism, not safe application
editing or free context acquisition.

The completed paced test now demonstrates that distinction: preparing context
produced three eligible final continuations out of twelve traces, but blinded
full-continuation review found only one useful and two contradicting supplied
facts. Cold inference produced none. Six primed preludes exceeded 1,500 ms,
including all four Persian traces. The frozen usefulness and harm gates both
failed; see the [measured results](../../evaluation/writing/README.md#current-development-findings-2026-09-09).

### Same-family Base checkpoint: a candidate comparison

The next candidate tested was [Qwen3-1.7B-Base](https://huggingface.co/Qwen/Qwen3-1.7B-Base).
Qwen's [technical report](https://arxiv.org/html/2505.09388v1#S4.SS5) describes
distillation used in the smaller post-trained models. This supports testing a
pretrained continuation checkpoint against the installed post-trained artifact;
it does not establish that Base is better at autocomplete. Earlier smaller,
larger, Qwen3.5 and template comparisons do not answer this specific question.

The public [Base Q4_K_M artifact](https://huggingface.co/mradermacher/Qwen3-1.7B-Base-GGUF/tree/7b0494b4ddf6e7aa1ca2da0b2f398be382dd0ea8)
was downloaded into private experiment storage and verified: 1,107,409,024 bytes,
SHA-256 `8b5d946a169e62dd49444be8858c8a7f06067573de990b2bc19380d3a260b5fa`.
The source card, Apache-2.0 license and pinned upstream configurations are saved
under `output/writing/model-research/qwen3-17b-base/`. A real two-trial Lab smoke
loaded this exact artifact with the existing runtime and verified both shutdowns;
its cold trace exhausted a deadline, while the primed trace produced an eligible
final result. This is load/lifecycle evidence, not a quality score.

A read-only header inspection following the [GGUF specification](https://github.com/ggml-org/ggml/blob/master/docs/gguf.md)
found a material artifact difference: Base has 310 tensors and a Q6_K embedding
with no separate output tensor; the installed 311-tensor artifact has Q4_K
embeddings and separate Q6_K output weights. Actual GGUF token and merge arrays
match byte-for-byte. Both GGUF files advertise EOS 151645, although the pinned
Base upstream config specifies 151643. Preserve and test the actual metadata;
do not silently rewrite it to fit the hypothesis. A practical comparison can
choose a better artifact, but cannot attribute any gain solely to training stage.

The matched 32-request development comparison has now completed. Full eligible
continuation review found 8/16 useful Base outputs versus 6/16 for the installed
artifact, with one harmful output each. This misses the frozen +3 gain and
loses the installed artifact's one useful Persian output. All 16 paired actual
payloads and prompts matched, and all owned processes were reaped. The exposed
synthetic set has only two German cases and related boundary pairs; it cannot
establish multilingual quality. See the [evaluation findings](../../evaluation/writing/README.md#current-development-findings-2026-09-09).

The earlier instruction experiment has a separate implementation confound:
instruction wrapping and exact boundary healing were mutually exclusive.
Three of its sixteen outputs were rejected for a duplicated leading space;
recovering those alone cannot explain the ten-point difference in the old
first-word review. The opt-in combined mode subsequently passed a matched
66-request development quality diagnostic: 18/22 useful full continuations
versus 13/22 for healing alone and 6/22 for instructions alone. No language lost
useful output and the candidate had no harmful output in blind agent review.
Its cold median was 2.81 seconds, with no eligible terminal result within
550 ms. Longer ChatML prompts still require a separate latency experiment;
this five-second result does not warrant changing the default.

A paired prepared-instruction experiment subsequently yielded 11/22 useful
terminal results within 550 ms versus none cold, with one neutral and no harmful
candidate results in blind agent review. It still failed the frozen 18-useful
threshold. Median preparation cost was 1.40 seconds, excluding runtime startup;
this was a readiness experiment on exposed synthetic cases, not paced typing.
The improvement supports further latency work without qualifying promotion.

### Multilingual word completion and correction

Source inspection found that the shared proposal guard rejects every German or
Persian letter-to-letter word completion; English uses its compiled lexicon.
The small Lab experiment kept the model's Healed prompt unchanged and
recovered only the first completed suffix when its exact whole word appeared in
the explicitly supplied context or style. Case, diacritics and Persian joiners
must match, with visible source-word boundaries. An unfinished pasted fragment,
expected answer or earlier generated text cannot provide that evidence. This
can reuse names and repeated terms, including mistakes already in the source;
it is not dictionary spelling validation or proof of contextual relevance.
Its 24-request comparison recovered two useful German words, below its frozen
+3 gain requirement, and did not improve Persian. Exact source-boundary checks
passed, including negative controls; two repeated non-Persian legacy suggestions
remained in both arms. The mechanism remains experimental.

Removing inference entirely makes exact word reuse fast but does not resolve
intent. A separate frozen 30-case English/German/Persian lookup test returned all
12 expected completions, with every HTTP response below 36 ms including worker
verification and cleanup. It also wrongly extended three completed words with
other words from context, failing the zero-unwanted-offer gate. The result
supports a manual word-completion workbench, not automatic semantic prediction
or Cotypist parity. Requests, limits and measurements are recorded in the
[writing evaluation](../../evaluation/writing/README.md).

Spelling needs a separate language-aware implementation. Hunspell's
[format contract](https://raw.githubusercontent.com/hunspell/hunspell/master/man/hunspell.5)
uses paired dictionary/affix files with language-specific flags and encodings;
words accepted as correctly spelled may still carry `NOSUGGEST`. The current
English ASCII generator must not be generalized by merely widening its character
filter. LibreOffice's [Persian Lilak notice](https://raw.githubusercontent.com/LibreOffice/dictionaries/master/fa_IR/README_fa_IR.txt)
identifies an Apache-2.0 morphology-based dictionary. Its exact dictionary and
affix behavior should be tested through the real engine before choosing a
production validator or correction policy.
An actual 48-word probe of the pinned pairs found all six authored typo
references in each language's suggestion lists, but Persian ranked only three
first. Unknown names and unfinished prefixes also attracted corrections, while
some accepted words had `NOSUGGEST`. This supports a guarded correction
experiment, with neither rank-one replacement nor spelling acceptance treated
as sufficient authority.
A later real HTTP40 correction test confirmed that limit: six useful reference
matches and two unwanted changes, including an intentional name and a wrong
nearby Persian word after the restricted candidate list excluded another option.
All requests completed, but the frozen quality gate failed. Dictionary checks
are fast enough for this Lab; contextual correctness and coverage remain open.

### Three prioritized, falsifiable experiments

1. **Repair the generation boundary without changing the draft.** Freeze paired
   inputs whose only difference is a trailing ASCII space, plus partial words,
   punctuation and Persian joiner cases. Compare the existing prompt with a
   lab-only mode that removes a bounded trailing space suffix from the model
   prompt, requires generation to reproduce those exact bytes, then strips the
   echo once. Keep the original input for seam validation. Never silently trim
   arbitrary whitespace or repair generated output to match the answer key.
   Compare complete first-word agreement, blinded usefulness, abstentions,
   rejected seams and latency. This is a cheap initial boundary experiment;
   general tokenization marginalization remains a separate research problem.

2. **Preserve useful context across realistic typing traces.** Freeze several
   short documents with explicit contextual cues and a sequence of caret
   prefixes for each. Compare current sentence-only prompting with one bounded,
   structured context/prefix template, first cold and then through append-only
   typing with a stable contextual prefix. Use the same model and decoding.
   Charge all priming time and report the first request separately; prefill is
   not free. Also exercise backspace, a changed context source, field switches
   and cancellation. Record prompt tokens evaluated and reused, first-token and
   first-complete-word time, useful output and suggestion churn. A laboratory
   quality budget may be longer than 550 ms if clearly labeled; report which
   outputs would meet the production budget as a separate measure. Warming is
   worthwhile only if the additional context improves meaning and retains a
   usable latency under these transitions.

3. **Add narrow, explicit personalization and measure its tradeoff.** Start with
   the user's voluntarily supplied writing examples, scoped by language and
   writing task. Compare no profile, a bounded relevant example and an equal-
   size irrelevant example. In a separate candidate-ranking experiment, compare
   the LLM distribution with a small backoff n-gram distribution from those
   examples, tuning the blend on development cases only. Keep facts in current
   context authoritative: frequent personal names must not override a different
   name or negation in the draft. Include those conflicts as negative controls.
   Report same-coverage precision, useful words per request and latency, rather
   than crediting more frequent familiar vocabulary as success. Do not train
   on test answers or automatically collect writing from other applications.

The lab should allow expected alternatives and an explicit “prefer no
suggestion” case. Exact matching is a reproducible diagnostic, not a complete
semantic judgment. Split whole documents and writing examples before evaluation
so adjacent prefixes cannot leak from development into confirmation. Freeze
profiles, prompts and randomized/counterbalanced run order; retain first cold
requests, failed attempts and per-language results. A candidate becomes a
production proposal only after new confirmation cases and actual user writing
show a benefit. None of these research directions establishes Cotypist parity,
screen-context acquisition or editing support in any application.

## 2026-09-07: useful results before broader coverage

The current research supports a focused Linux implementation, but does not
establish that Badi matches Cotypist. Cotypist's public documentation describes
behavior; no reproducible comparative quality or end-to-end latency measurement
was available in the reviewed sources. A parity claim requires the same writing
tasks on a named Cotypist version and Apple Silicon machine, alongside Badi's
named Linux hardware and application versions.

| Primary-source finding | Consequence for Badi |
| --- | --- |
| [Cotypist's tips](https://cotypist.app/help/tips) emphasize automatic updates while typing and the usefulness of accepting only the next word. Its [shortcut guide](https://cotypist.app/help/shortcuts) documents separate full acceptance and temporary quieting after Escape. | Automatic activation on proven fields, next-word acceptance and predictable dismissal belong in the first usable slice. A manual request followed by a separate accept is a different experience. |
| The current [model menu](https://cotypist.app/pricing) includes Qwen3 1.7B and 4B, alongside Gemma choices. | The older Gemma-only/default-size description below is historical. Badi's current Qwen family is a plausible starting point; model choice alone does not explain a missing input path. |
| The [compatibility matrix](https://cotypist.app/compatibility) excludes Ghostty, Kitty, Warp, Thunderbird and several custom editor surfaces. | Evaluate exact fields and application versions. Neither product has demonstrated literal universal application coverage. |
| The [privacy page](https://cotypist.app/help/privacy) distinguishes local inference, optional screen/clipboard context, default-off writing recording, default-on anonymous telemetry, and occasional clipboard-assisted insertion. | Local inference does not imply zero networking or zero context collection. Badi can offer clearer acquisition policy, default-off telemetry and editor-owned insertion. |

Implementation order, subject to actual failure evidence:

1. Prove the installed input-to-insertion path in the user's real writing
   surfaces. Show whether an adapter is connected, has fresh eligible input,
   receives a suggestion, and can apply it with native undo. A ready model is
   not evidence of usable input integration.
2. Make automatic requests, word acceptance, cancellation and type-through
   behavior dependable. Preserve focus, revision, policy, composition and
   sensitive-field boundaries rather than weakening them for coverage.
3. Freeze development and heldout examples before model tuning. Measure
   abstention, errors, reference agreement and independently reviewed semantic
   usefulness separately. Use [the writing evaluator](../../evaluation/writing/README.md)
   for explicit synthetic real-broker requests; it does not measure UI latency
   or actual user acceptance.
4. Optimize the measured bottleneck. The [llama.cpp server contract](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md)
   provides KV-prefix reuse and token probabilities. Verify the installed
   runtime's schema before use; post-sampling certainty is not calibrated model
   confidence. Compare existing prompting and at most a small set of justified
   candidates on identical examples, recording cancellation and latency as well
   as usefulness. Qwen publishes separate [base](https://huggingface.co/Qwen/Qwen3-1.7B-Base)
   and [post-trained](https://huggingface.co/Qwen/Qwen3-1.7B) models; suitability
   for short continuation is an experiment, not a model-card conclusion.
5. Investigate first-word abstention and low-confidence tail trimming using
   development data only. [Pretype's architecture](https://github.com/nikiomori/Pretype/blob/main/docs/ARCHITECTURE.md)
   reports that ungated suggestions were net-negative in its evaluation and
   describes calibrated gating. That is publisher-reported evidence: its linked
   `Eval/BASELINE.md` returned 404 during this review, so its numerical results
   were not reproduced or adopted as Badi thresholds.
6. Expand through cooperative toolkit and editor APIs only after a useful
   vertical slice. [Fcitx's integration documentation](https://fcitx-im.org/wiki/How_does_an_application_talk_to_Fcitx)
   explains why toolkit, compositor and transport change available capabilities.
   Exact browser/editor context is preferable where those applications expose
   it. Unknown terminal/TUI editing authority remains a separate integration
   problem.

The engineering targets remain zero wrong-field or stale edits, native undo,
and warm end-to-end p50 at most 250 ms / p95 at most 500 ms in the selected
application cells. These are Badi targets, not measured Cotypist numbers.
[Google's historical Smart Compose account](https://research.google/blog/smart-compose-using-neural-networks-to-help-write-emails/)
identifies approximately 100 ms as the ideal per-keystroke response budget;
500 ms is therefore a ceiling for this first local implementation rather than
a reason to stop improving responsiveness. A larger suggestion count alone
cannot demonstrate useful writing assistance. Positive typing benefit needs
observed accept, dismiss and correction behavior from real writing trials.

## Executive verdict

There is no mature open-source Linux product that combines all of Cotypist's defining qualities: proactive local-LLM continuation, well-aligned ghost text, word-by-word acceptance, personalization, surrounding-screen context, and reliable per-app/per-domain policy.

The pieces do exist separately:

- **Cotypist, Cotabby, KeyType, and Pretype** demonstrate the desired interaction on macOS.
- **IBus Typing Booster** proves that learned, context-sensitive completion and app-aware policy can live in Linux's input-method layer; it is mature and unusually multilingual.
- **SmartComplete** is the closest Linux/Fcitx5-shaped prototype, but it is tiny and currently predicts from dictionaries, n-grams, rules, and an optional cloud reranker rather than generating Cotypist-quality continuations.
- **WritingTools and LinuxPop** prove demand for system-wide AI rewrite/selection actions, but they are invoked tools, not ambient co-typing.

The opportunity is therefore real, but the moat is not “connect an LLM.” It is a Linux compatibility and trust product: fast cancellation-safe inference, correct caret rendering, coexistence with IMEs and shell completion, and policy that fails closed when the active app or field cannot be identified.

## Exact-name collision: an existing OmaType

An unrelated public repository already uses the exact name [Aayush9029/OmaType](https://github.com/Aayush9029/OmaType). It is an MIT-licensed, Omarchy-focused fork of Voxtype for **local voice dictation**, not predictive ghostwriting: tapping or holding the Home key records speech, local Parakeet/Whisper/SenseVoice models transcribe it, and the result is typed at the cursor. Its current default branch is `hybrid-hotkey`; the repository shows no releases or GitHub stars yet, although its inherited history is substantial.

This does **not** make the product concept redundant. The collision identified
here later prompted the rename to **Badi**. The products still occupy the same
Linux/Omarchy “type anywhere” neighborhood, so positioning and possible voice
interoperability remain relevant. Before a public launch, the project should:

- use an unmistakable ghostwriter/autocomplete subtitle in every README and listing;
- choose distinct technical namespaces for packages, desktop files, D-Bus services, and binaries;
- document the difference and investigate whether interoperability or coordination with the existing maintainer is useful;
- perform normal package-registry, domain, and trademark due diligence rather than assuming a GitHub organization name resolves the collision.

## Cotypist: verified product facts

[Cotypist](https://cotypist.app/) is a proprietary autocomplete utility for Apple Silicon Macs on macOS 14+. It observes the focused text field through Accessibility, generates locally with `llama.cpp` and Gemma-family models, draws a continuation at the caret, and inserts accepted text. Its [press kit](https://cotypist.app/press) says the default model is roughly 3 GB, runtime memory is roughly 1–2.5 GB, and inference is entirely on-device.

The current interaction loop is unusually disciplined:

1. A suggestion appears inside the app where the user is already typing.
2. `Tab` accepts the next word by default; repeated presses progressively accept more. The whole line can also be accepted.
3. Continuing to type implicitly rejects and refreshes the suggestion.
4. Current paid tiers add autocorrect, mid-line completion, word alternatives, configurable length, clipboard context, and stronger personalization. See the current [feature and pricing matrix](https://cotypist.app/pricing).

Its context system has several explicit trust boundaries. According to the [privacy documentation](https://cotypist.app/help/privacy), focused-field access is required; screen OCR and clipboard context are optional; writing-history collection for personalization is off by default; stored writing is encrypted locally; and password fields are filtered. The app does send anonymous feature/performance counts and crash reports by default, but users can disable them, and the vendor says typed text is not included.

Its policy controls are close to the user's stated Badi requirement. Users can exclude an app or browser domain from writing-history collection and attach different instructions by app/domain, as documented under [personalization](https://cotypist.app/help/personalization). The distinction matters, however: excluding collection is not necessarily the same as disabling all suggestions or all context access. Badi should expose those permissions separately and plainly.

“Every app” is positioning, not a literal guarantee. Cotypist's own [compatibility matrix](https://cotypist.app/compatibility) lists unsupported or partial cases, including Thunderbird, OneNote, Anki, Google Slides, several editors, Ghostty, Kitty, and Warp. Google Docs needs accessibility options. Terminals are particularly thoughtful: Terminal.app and iTerm activate automatically at recognized AI-agent prompts, remain quiet for ordinary shell commands, and offer a force-activate shortcut. This is a strong behavior to emulate, not merely a compatibility checkbox.

The vendor's “up to 50% less typing” claim has no public methodology in the sources reviewed, so it should be treated as marketing rather than a benchmark.

## What appears to make Cotypist special

These are evidence-weighted conclusions, not a popularity survey. Public review volume is still small: [Product Hunt shows four reviews](https://www.producthunt.com/products/cotypist), while Reddit threads over-represent enthusiasts and price-sensitive Mac utility buyers.

### 1. It preserves flow instead of creating a second workflow

The core value is the absence of a prompt-copy-paste-edit loop. The suggestion arrives where the thought is already being written, disappears when ignored, and costs one familiar key when useful. [Daring Fireball's hands-on account](https://daringfireball.net/linked/2026/06/18/cotypist) emphasizes the native-looking inline presentation and the simplicity of Tab versus continuing to type. [Tom's Guide](https://www.tomsguide.com/ai/i-installed-a-small-llm-on-my-mac-laptop-heres-why-i-cant-go-back) similarly reports that the small interaction compounds into noticeable writing speed.

**Inference:** the unit of value is not a generated paragraph; it is a low-regret next-word decision. Badi should optimize accepted useful words per interruption, not raw token output.

### 2. Partial acceptance protects authorship

Cotypist's own [usage guidance](https://cotypist.app/help/tips) admits that later words in a suggestion often drift from intent, which is why one-word acceptance is the default. This is unusually honest and explains the design. The user remains the writer, while the model handles predictable connective tissue.

### 3. Local inference creates both trust and immediacy

Privacy is repeatedly praised in [Product Hunt reviews](https://www.producthunt.com/products/cotypist), the [Tom's Guide test](https://www.tomsguide.com/ai/i-installed-a-small-llm-on-my-mac-laptop-heres-why-i-cant-go-back), and [community discussion](https://www.reddit.com/r/macapps/comments/1sugv0u/cotypist_helpful_writing_assistant_or_drunk_typing/). Local inference also removes network jitter. The official pricing FAQ makes the important performance point that a larger model can produce a worse typing experience when it cannot keep pace.

**Inference:** Badi should have a deterministic fast path and expose model latency/quality presets, rather than equating “largest” with “best.”

### 4. It learns a voice across app boundaries

Vocabulary, names, phrasing, custom instructions, and optional writing history move the experience beyond generic autocomplete. This is one of the repeated positive themes in Cotypist's [official product description](https://cotypist.app/) and press testimonials. Screen context can also incorporate names and tone visible around the active field without storing screenshots.

The risk is equally important. In a mixed [Reddit review thread](https://www.reddit.com/r/macapps/comments/1sugv0u/cotypist_helpful_writing_assistant_or_drunk_typing/), one tester felt suggestions changed their voice and did not save time, while another rarely accepted suggestions but found their presence helped maintain momentum. This is directional evidence that ghost text can either unblock or distract depending on timing and confidence.

### 5. “Everywhere” builds muscle memory

A single accept key and behavior across email, notes, web forms, chats, and AI prompts is more valuable than a slightly better model trapped in one editor. Cotypist's compatibility gaps show how hard this promise is, but its consistency where supported is a major differentiator.

### 6. Product polish is part of model quality

The strongest open-source Mac competitors all spend conspicuous effort on baseline alignment, stale-suggestion cancellation, constrained output, app profiles, and insertion safety. [KeyType's repository layout](https://github.com/johnbean393/KeyType) names these as separate subsystems; [Pretype](https://github.com/nikiomori/Pretype) offers both caret-matched ghost text and a floating fallback. This supports an important conclusion: users experience rendering errors and late text as “bad AI,” even when the generated words are good.

## Open-source comparison

“Activity” below is a repository signal, not a durability guarantee. Dates and status are those visible on 2026-08-29.

| Project | Platform and interaction | Local AI / privacy | App policy and terminal behavior | License and activity signal | Fit for Badi |
|---|---|---|---|---|---|
| [Aayush9029/OmaType](https://github.com/Aayush9029/OmaType) | Linux/Omarchy voice typing; tap for accurate batch dictation or hold for live dictation, then type at the cursor | Local Parakeet, Whisper, and SenseVoice models; stores recent transcripts locally without audio | Omarchy hotkey/service integration; not predictive text and no documented per-app completion policy | MIT; fork of Voxtype, no releases and zero stars at snapshot | Adjacent input product and exact-name collision, not a Cotypist alternative; relevant to namespace, discovery, and possible voice-mode interoperability |
| [Cotabby](https://github.com/FuJacob/cotabby) | macOS; near-direct Cotypist alternative with inline ghost text, one-word/whole-suggestion acceptance, autocorrect, screen context, emoji, and macros | Apple Intelligence or local open models; optional user-configured OpenAI-compatible endpoint; no account/telemetry required | Global pause and disabled-app list; current release stays out of terminals, although a terminal PR is open | AGPL-3.0; active beta maintained by two students, with PR activity in Aug 2026 | Best open codebase to study for the complete event loop, context capture, and UX; unusable as a Linux base without replacing the platform layer |
| [KeyType](https://github.com/johnbean393/KeyType) | macOS; focused-field watcher, local continuation, caret overlay, Tab acceptance | Local `llama.cpp` runtime | Repository explicitly separates app/domain policy and insertion strategies | MIT; hundreds of stars; v1.6 released Jun 2026 with extensive cross-app overlay fixes | Best architecture reference for constrained generation, token profiles, insertion safety, and compatibility fixtures; macOS-only Swift code |
| [Pretype](https://github.com/nikiomori/Pretype) | macOS; caret-matched ghost text or floating fallback; word/whole acceptance and inline spelling fixes | Fully local MLX models; model download and optional update check are its stated network uses | Accessibility-based; broad native/Electron/web claims, but no Linux or terminal-specific policy | MIT; young project (59 commits and single-digit stars at snapshot), ad-hoc signed releases | Clean, auditable reference implementation; useful fallback-rendering ideas, but immature and Apple-Silicon-specific |
| [IBus Typing Booster](https://github.com/mike-fabian/ibus-typing-booster) | Linux/FreeBSD via IBus; context-sensitive learned word completion, spellcheck, multilingual input, emoji; candidate list and optional inline completion | Core completion is local; can learn from user text and train from files; newer releases also expose optional local Ollama chat, which is separate from live completion | Mature terminal disable and regex-based autosettings by toolkit/program/window title; temporary re-enable is possible | Repository publishes GPL-3.0 and Apache-2.0 license files; started in 2010, 3,400+ commits, release 2.30.11 in Jul 2026 | Strongest mature Linux-native precedent. It proves input-layer reach and policy, but not generative phrase quality; its own docs say inline styling is poor on Wayland |
| [SmartComplete](https://github.com/ekremx25/smartcomplete) | Linux X11/Wayland via Fcitx5; candidate UI, Tab acceptance, word/phrase/rule prediction, autocorrect, emoji | Offline trie, bigram, phrase and grammar engines; optional OpenAI reranker only reorders candidates | Configurable default terminal/shell blocklist; deliberately avoids terminals | MIT; only 11 commits, no releases visible, and essentially no adoption at snapshot | Closest Linux-shaped prototype, valuable for scaffolding/tests and hybrid fast-path ideas; far too early to treat as a production competitor or quality benchmark |
| [WritingTools](https://github.com/theJayTea/WritingTools) | Windows/macOS/Linux; select text, invoke hotkey, proofread/rewrite/transform; not proactive autocomplete | Local Ollama and other OpenAI-compatible runtimes or cloud providers; invoked only on command | System-wide selection workflow; Linux works well on X11, while its own README calls Linux/Wayland work-in-progress with XWayland caveats | GPL-3.0; 371 commits; latest combined release in May 2026, with active issues in Jul 2026 | Useful secondary “rewrite selected text” mode and provider abstraction; does not solve co-typing or native Wayland reach |
| [LinuxPop](https://github.com/GaimsDevSoftware/linuxpop) | Linux X11 and KDE Plasma 6 Wayland; selection popup with rewrite, translate, Ollama, and plugin actions | Local plugins by default; individual plugins may call services explicitly | Tray pause and configurable popup behavior; no proactive prediction | MIT; active 0.9.x beta, packaged as Flatpak/deb/rpm | Good plugin/action UX reference and KDE Wayland evidence; adjacent rather than a Cotypist replacement |
| [Veya](https://github.com/s3ba-b/veya) | Ubuntu/Linux; D-Bus daemon with overlay, GNOME extension, CLI, screen/clipboard/system context; not a writing predictor | Local-first with visible cloud fallback and an audit log | Per-source default-deny permissions; centralized safety layer | AGPL-3.0-or-later; explicitly pre-alpha, 52 commits, one star, no release | Useful trust/audit and daemon-contract reference, not a competitive writing product |

### Bottom line on alternatives

- **Best direct open-source Cotypist analogue:** Cotabby, with KeyType the stronger architecture-reading companion. Both are Mac-only.
- **Best mature Linux predictive-input foundation:** IBus Typing Booster.
- **Closest Fcitx5 prototype:** SmartComplete, but it should be treated as evidence that the route is possible, not evidence that the product problem is solved.
- **Best Linux system-wide rewrite tool:** WritingTools on X11; its Wayland limitations matter.
- **No project currently satisfies the requested browser + Obsidian + notes + terminal-agent scope with Cotypist-level generative UX.**

## The Linux-wide gap is structural

Fcitx5 officially supports [X11 and Wayland](https://github.com/fcitx/fcitx5), but “Wayland” is not a single uniform integration path. The Fcitx documentation explains that GTK, Qt, Electron/Chromium, XWayland, compositor protocols, and sandboxed apps can expose different input capabilities; see [how applications talk to Fcitx](https://fcitx-im.org/wiki/How_does_an_application_talk_to_Fcitx) and [Fcitx on Wayland](https://fcitx-im.org/wiki/Using_Fcitx_5_on_Wayland/en).

Three constraints directly affect Badi:

1. **Ghost-text rendering:** Wayland clients do not provide arbitrary global positioning. Candidate surfaces can be compositor- or toolkit-mediated. IBus Typing Booster's [inline-completion documentation](https://mike-fabian.github.io/ibus-typing-booster/docs/user/#use-inline-completion) says Wayland cannot style typed and suggested preedit text differently in its current route, making ghost text hard to distinguish.
2. **App identity:** under one Fcitx Wayland protocol, Fcitx sees a single global input context and cannot identify the application unless compositor-specific window-management protocols provide a second signal. This complicates reliable allowlists and denylists.
3. **Surrounding text:** toolkit implementations can be incomplete or buggy. This affects mid-line completion, deletion/replacement, prompt detection, and safe context capture. Sandboxed Flatpaks add another packaging boundary.

Therefore, Badi should not promise literal universal support at launch. It should publish a tested matrix by desktop, display protocol, toolkit, and app, with explicit fallback behavior.

## Product gap and opportunity for Badi

The following are design inferences from the evidence above, not facts about an existing implementation.

### 1. Be the Linux-native co-typing layer, not another AI popup

The initial wedge should be proactive continuation and typo repair in ordinary prose. Selected-text rewrite can be useful later, but it is already served by WritingTools and LinuxPop. The signature loop should remain: appear quietly, accept one word, accept all, or type through it.

### 2. Make policy a first-class safety system

Expose independent controls for:

- suggestions on/off;
- context reading on/off;
- local learning on/off;
- clipboard and screen context on/off;
- cloud inference allowed/forbidden;
- history retention and deletion.

Support **Always allow**, **Ask once**, and **Never** at app level; domain-level rules likely require browser-specific accessibility or extensions and should not be implied before verified. Default-deny password, secret, payment, and unknown-purpose fields. If app identity is unavailable on Wayland, fail closed for collection and cloud use, show that state, and retain a user-invoked one-shot completion.

IBus Typing Booster's [autosettings and terminal controls](https://mike-fabian.github.io/ibus-typing-booster/docs/user/#autosettings) are a strong functional precedent, but its own warning about passwords being visible in preedit/history shows why Badi needs stricter defaults.

### 3. Treat terminal prose as a unique mode

Do not blanket-disable terminals as SmartComplete does. Detect—or let the user explicitly enter—an **agent prompt mode** for Codex, Claude Code, and similar TUIs, while yielding to shell completion in ordinary commands. A force-activate shortcut is the safe MVP. Password/sudo/SSH prompts must disable suggestions and learning. Supporting Ghostty, Kitty, foot, Alacritty, Konsole, and GNOME Terminal would materially exceed Cotypist's present terminal matrix.

### 4. Use a latency ladder

A plausible architecture is:

- immediate local spelling, learned phrase, and n-gram candidates;
- speculative small local LLM generation in parallel;
- optional larger local or explicitly approved remote model only when confidence is low;
- generation IDs, aggressive cancellation, and insertion-safety checks so stale text never lands;
- one-word acceptance and cached continuation so the next Tab is instant.

SmartComplete validates the deterministic fast-path idea; the Mac projects validate constrained generation and cancellation. The actual latency and quality targets still need measurement on representative Linux hardware.

### 5. Turn compatibility into a shipped artifact

Test at minimum GNOME Wayland, KDE Plasma Wayland, one wlroots compositor, and X11; then GTK3/4, Qt5/6, Firefox, Chromium/Electron, Obsidian, LibreOffice, common terminals, and Flatpak variants. Each adapter should declare capabilities such as app identity, field purpose, surrounding text, caret geometry, preedit styling, and safe insertion. Unsupported capabilities should select a visible fallback, not silently guess.

### 6. Win trust with inspectability

Show which context sources are active, which model handled a completion, whether anything left the machine, and why Badi is paused in the current field. Provide a panic key, per-app deletion, short retention defaults, and an audit view without storing content. Veya's permission and cloud-call transparency is an adjacent reference worth borrowing.

## Adversarial check: what could invalidate this opportunity?

1. **“Linux-wide” may fragment the two-day MVP beyond usefulness.** A credible first live result should name one desktop/input framework and a short app matrix, then prove the full event loop there. Cross-desktop architecture can be planned without pretending it is already portable.
2. **The input-method route may conflict with users' existing multilingual IMEs.** Badi must coexist, compose, or provide separate IBus and Fcitx5 engines; asking multilingual users to replace their working IME is a major adoption cost.
3. **Ghost text may be visually impossible through some Wayland paths.** Candidate popup or mirror-window fallback may be necessary. That fallback must be tested for distraction rather than called equivalent.
4. **App blocking cannot be trustworthy if app identity is missing.** Unknown identity must never inherit an “always approved” rule. Policy should be capability-based and fail closed.
5. **Small local models may be fast but mediocre; larger ones may arrive too late.** The Reddit evidence and Cotypist's own model guidance agree on this tradeoff. Badi needs a replayable completion benchmark, not subjective demos alone.
6. **Ambient suggestions can reduce rather than improve flow.** Measure suggestion display rate, acceptance by prefix length, time-to-first-suggestion, stale-display rate, dismissals, and a user-controlled “only when paused” threshold. High generation volume is not success.
7. **Open source does not automatically make model distribution safe.** Code license, model-weight license, tokenizer license, and downloaded training artifacts must be tracked separately.
8. **Privacy claims require more than local inference.** Context capture, logs, crash reports, swap, model downloads, optional remote providers, and history are all part of the threat surface.

## Confidence and unknowns

### High confidence

- Cotypist's current feature, privacy, pricing, platform, and compatibility descriptions, because they come from its live official documentation.
- The basic scope and licenses of the open-source repositories, based on their own READMEs and license metadata.
- The existence of Linux Wayland/toolkit fragmentation, supported by Fcitx and IBus project documentation.
- The market gap: none of the reviewed Linux projects claims and demonstrates the full Cotypist interaction set.

### Medium confidence

- The ranking of features users value. Independent reviews and community threads converge on flow, local privacy, inline placement, and voice preservation, but the sample is small and self-selected.
- Repository activity as a proxy for health. Recent releases and PRs prove motion, not maintainership capacity or code quality.
- The recommendation to use an input-method foundation. It has the broadest native reach in the evidence, but a hands-on prototype is still needed to compare IBus, Fcitx5, AT-SPI overlays, and compositor extensions.

### Low confidence / unresolved

- Real acceptance quality and latency of any alternative on the user's actual machine; no project was installed or benchmarked for this desk study.
- Whether one implementation can deliver correctly styled inline ghost text across GNOME, KDE, wlroots, XWayland, Electron, and sandboxed apps without per-environment fallbacks.
- Reliable browser-domain detection without extensions, and reliable AI-agent versus shell/password detection in all terminal TUIs.
- Cotypist's claimed typing reduction and the relative model quality of competitors; no common corpus or methodology exists.
- Security quality of young repositories such as SmartComplete, Pretype, and Veya; README claims are not audits.

## Source quality note

Primary sources were preferred for product behavior, platform limitations, licensing, and activity. User sentiment uses [Product Hunt](https://www.producthunt.com/products/cotypist), [Daring Fireball](https://daringfireball.net/linked/2026/06/18/cotypist), [Tom's Guide](https://www.tomsguide.com/ai/i-installed-a-small-llm-on-my-mac-laptop-heres-why-i-cant-go-back), and mixed [Reddit discussion](https://www.reddit.com/r/macapps/comments/1sugv0u/cotypist_helpful_writing_assistant_or_drunk_typing/) only as directional evidence. Claims made by project authors about privacy, compatibility, or quality remain claims unless independently tested above.
