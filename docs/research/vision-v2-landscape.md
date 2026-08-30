# Omatype Vision V2: product landscape and decisions

Research snapshot: **2026-08-30**

> **Working-name warning:** `Omatype` is already the public name of an
> Omarchy-oriented local dictation project. In this document it means only this
> repository's codename. A distinct public name is a release gate, not optional
> polish.

## Executive conclusion

The research still supports building the product, but it sharpens what the
product is.

Omatype should **not** be positioned as a generic AI writing assistant or as a
literal system-wide clone of Cotypist. Browser extensions already combine
autocomplete, generation, and rewriting; Linux already has mature input-method
completion, system-wide rewrite utilities, and powerful text expanders. What is
still missing is one product that combines:

1. a quiet, partial-acceptance prose loop;
2. local inference that is good enough and fast enough on ordinary Linux
   hardware;
3. target-owned context, rendering, revalidation, and insertion where possible;
4. policy that independently controls activation, context, inference, learning,
   and retention; and
5. an honest, inspectable compatibility contract for Linux rather than an
   “everywhere” badge.

The differentiator is therefore **trustworthy continuity of interaction across
heterogeneous Linux apps**, not access to an LLM. The model can be replaced.
Correct focus, unobtrusive timing, safe insertion, visible policy, and a stable
adapter contract are the durable product.

The strongest V2 direction is:

> **A quiet, local next-word layer for Linux that appears only when the current
> target can be proved safe, explains what it used, and yields to the tools that
> already belong there.**

## Evidence method and confidence

Primary sources were used wherever available: current product documentation,
official help centers, public repositories, release pages, code architecture
documents, and original research papers. Repository activity was checked on the
snapshot date. No product was installed, security-audited, or benchmarked on the
target workstation for this report.

The labels below mean:

- **Verified** — present in a current first-party document, repository, or
  release record. This verifies that the publisher documents or implements the
  behavior; it is not an independent compatibility or privacy audit.
- **Inference** — a product decision drawn from the verified evidence.
- **Evidence A** — official documentation plus inspectable code or original
  research; **A-** — detailed first-party documentation or code without hands-on
  validation; **B** — publisher/store claims with limited architecture detail.

Stars, commits, and releases are activity signals only. They do not prove
quality, security, or long-term maintenance.

## Market map

| Category | Strongest current signal | Unfilled part relevant to Omatype |
|---|---|---|
| Cotypist-style completion | Cotypist has the most coherent commercial interaction; Cotabby, KeyType, Pretype, and GhostType expose increasingly strong open implementations | All are macOS-only; none solves Linux capability, policy, and insertion fragmentation |
| Linux predictive input | IBus Typing Booster is mature and multilingual; SmartComplete is a small Fcitx5-shaped prototype | Neither combines short semantic continuation, target-owned or extension-owned ghost text, strict policy, and verified insertion |
| Rewrite/selection tools | WritingTools and LinuxPop make explicit transformations available across Linux apps | They are invoked workflows, not low-friction co-typing; their reach does not prove safe ambient context |
| Text expansion | Espanso and Text Blaze make deterministic, user-authored knowledge extremely useful | They do not infer the user's next thought, but their high-confidence path should complement generation |
| High-quality inline UX | Gmail Smart Compose, GitHub Copilot/VS Code, and JetBrains Full Line establish the strongest timing, acceptance, filtering, and metrics patterns | They own one app/editor surface and cannot be generalized to arbitrary Linux fields without adapters |
| Trust architecture and adjacent input | Veya demonstrates permission/audit patterns; the existing OmaType demonstrates local voice input on Omarchy | Neither is prose completion; both clarify scope, trust, interoperability, and naming decisions |

## Direct Cotypist-style products

### 1. Cotypist

**Verified — surface and interaction.** [Cotypist](https://cotypist.app/) is a
proprietary Apple-Silicon/macOS product. It places a short continuation at the
active caret, accepts the next word by default, supports whole-suggestion
acceptance, updates as the user types through a suggestion, and now offers
mid-line completion, autocorrect, word alternatives, and configurable length
across its tiers. Its own [usage guide](https://cotypist.app/help/tips) says the
first one or two suggested words are often the useful part and explains why
word-by-word acceptance is the default.

**Verified — data, controls, personalization, and architecture.** Inference is
on-device; Accessibility supplies focused-field access, while screen OCR and
clipboard context are optional. Writing-history collection is off by default,
stored locally in an encrypted database, deletable globally or by app/domain,
and can be limited to sessions in which a suggestion was accepted. Users can
set global and per-app/domain instructions and tune personalization strength.
Anonymous usage counts and crash reports are on by default but can be disabled.
See the current [privacy](https://cotypist.app/help/privacy),
[personalization](https://cotypist.app/help/personalization), and
[pricing/feature](https://cotypist.app/pricing) documentation. The published
[compatibility matrix](https://cotypist.app/compatibility) is notably honest:
Ghostty, Kitty, Warp, Thunderbird, OneNote, and several editor surfaces are not
supported; Terminal.app and iTerm use special agent-prompt behavior. **Evidence
A-** (detailed official behavior and limitations; closed implementation).

**Inference — lesson / do not copy.** Preserve its small-decision loop, partial
acceptance, optional accepted-session learning, and explicit compatibility
matrix. Do not copy the blurred distinction between “collection disabled” and
“all context disabled,” do not enable product telemetry by default, and do not
repeat “every app” where a capability tuple is the truthful unit.

### 2. Cotabby

**Verified — surface and interaction.** [Cotabby](https://github.com/FuJacob/cotabby)
is a macOS 14+ menu-bar app with inline ghost text, next-word or whole-phrase
acceptance, type-through dismissal, emoji completion, macros, and autocorrect.
It supports Apple Intelligence, in-process open GGUF base models, and an
explicitly configured OpenAI-compatible endpoint.

**Verified — data, controls, personalization, architecture, and activity.** A
normal local install writes no typed text to disk and sends no telemetry;
screen capture is optional, endpoint credentials live in Keychain, and secure
fields block generation, presentation, and insertion. The repository's unusually
useful [architecture document](https://github.com/FuJacob/cotabby/blob/main/ARCHITECTURE.md)
shows immutable request snapshots, monotonically increasing work IDs,
latest-wins streaming, type-through reconciliation, bounded context, and
separate generation engines. It also discloses privacy debt: a bounded secure
field context can be acquired before the field is marked blocked, and visual
capture can warm even though the excerpt cannot reach prediction; explicit
debug mode may persist captures. The product conditions local base models on
name, style, language, and optional context, but does not learn from a raw
cross-app typing log by default. AGPL-3.0; latest release
[v0.6.2-beta](https://github.com/FuJacob/cotabby/releases/tag/v0.6.2-beta) in
July 2026, with repository activity in August. **Evidence A** (code and candid
architecture documentation; runtime claims not independently tested).

**Inference — lesson / do not copy.** Adopt immutable snapshots, work identity,
monotonic partials, seam normalization, and the idea that a partial stream may
become accept-ready. Put the hard sensitive gate **before acquisition**, not
merely before generation, and never let a debug facility weaken that invariant.
Do not copy AGPL code until Omatype's own license has been chosen.

### 3. KeyType

**Verified — surface and interaction.** [KeyType](https://github.com/johnbean393/KeyType)
is an MIT-licensed macOS app that observes the focused field, generates a short
local continuation, renders ghost text, and accepts it with Tab.

**Verified — data, controls, personalization, architecture, and activity.** The
repository separates `AutocompleteCore`, Accessibility capture, budgeted
prompting, a `llama.cpp` runtime, constrained generation, token profiles,
overlay UI, insertion strategies, and per-app/per-domain compatibility policy.
That decomposition is stronger evidence than a single “system-wide” module.
The public README documents on-device inference but does not currently document
a learned personal-writing store or a full set of independent context/learning
permissions; those should be treated as unknown rather than assumed. MIT;
[v1.8.0](https://github.com/johnbean393/KeyType/releases/tag/v1.8.0) was released
on 2026-08-29. **Evidence A-** (inspectable modular code and current release;
limited user-policy documentation).

**Inference — lesson / do not copy.** Mirror the separation between context,
prompting, constrained output, presentation, insertion, and compatibility.
Do not treat “on-device” as a complete privacy model, and do not omit partial
acceptance or policy receipts merely because generation is local.

### 4. Pretype

**Verified — surface and interaction.** [Pretype](https://github.com/nikiomori/Pretype)
offers macOS ghost text or a floating fallback, next-word acceptance with Tab,
whole-tail acceptance with Shift-Tab, type-through rejection, typo repair,
explicit selection rewriting, and optional hold-to-talk dictation. It publishes
per-model multilingual evaluations and self-measured warm completion latency.

**Verified — data, controls, personalization, architecture, and activity.** The
default MLX and Apple Intelligence engines are local. Optional focused-window
OCR and microphone access are off; terminals, password managers, secure input,
and active IME composition are blocked. Users can blacklist apps, define a
persona and per-app style, and keep or delete a local suggestion/context journal
capped at 50 MB. The [privacy document](https://github.com/nikiomori/Pretype/blob/main/docs/privacy.md)
identifies exactly what is stored and how to remove it. Its
[architecture](https://github.com/nikiomori/Pretype/blob/main/docs/ARCHITECTURE.md)
uses KV-cache reuse, debounced cancellation, confidence-tail trimming, an
optional calibrated first-word confidence gate, adaptive quieting in low-value
apps, and idle model unloading. It reports that ungated autocomplete was
net-negative on its held-out evaluation—an important warning even though the
result is publisher-measured. MIT; [v2.4.0](https://github.com/nikiomori/Pretype/releases/tag/v2.4.0)
was released in July 2026. **Evidence A** (source, privacy map, eval design, and
current release; performance not independently reproduced).

**Inference — lesson / do not copy.** Confidence **abstention** and tail trimming
belong in the product, not just the model benchmark. Publish the evaluation
corpus and per-language uncertainty. Learn quietness from accept/dismiss/erase
signals, but do not start with a raw surrounding-text journal or synthetic event
insertion as a Linux-wide default. Dictation and rewriting should remain
optional sibling modes after completion earns trust.

### 5. GhostType

**Verified — surface and interaction.** [GhostType](https://github.com/mk668a/GhostType)
is a Mac app with pause-triggered ghost text, whole-suggestion Tab acceptance,
Escape dismissal, manual activation, and app modes that are automatic, manual,
or disabled. It yields during non-ASCII IME composition and disables terminals
and IDEs by default.

**Verified — data, controls, personalization, architecture, and activity.** It
runs a bundled `llama-server` or reuses an existing loopback-compatible server;
the project documents no cloud endpoint, account, telemetry, input log, or
personalization store. Its best architecture signal is task-specific inference:
`/infill` supplies text on both sides of the caret for mid-line edits, while a
GBNF grammar makes newlines, code fences, and other unwanted forms unreachable
instead of cleaning them up later. MIT; [v1.0.0](https://github.com/mk668a/GhostType/releases/tag/v1.0.0)
was released in August 2026. **Evidence A-** (clear code and product contract;
young project and no independent test).

**Inference — lesson / do not copy.** Add fill-in-the-middle and constrained
decoding once the suffix path is stable. Keep a loopback-provider option for
users who already have a model resident. Do not make whole-tail Tab acceptance
the only granular action, and do not generalize “every field” from one OS
accessibility API.

## Linux input-method and completion projects

### 6. IBus Typing Booster

**Verified — surface and interaction.** [IBus Typing Booster](https://github.com/mike-fabian/ibus-typing-booster)
has provided context-sensitive completion through IBus since 2010. It supports
learned word completion, spellchecking, emoji, transliteration, multiple
simultaneous languages, candidate lists, and an optional inline completion
style. Tab can select the inline candidate or reveal alternatives.

**Verified — data, controls, personalization, architecture, and activity.** Its
core is local, learns from accepted/user input into a local database, and can be
trained from supplied files. “Off the record” prevents recording. The detailed
[user documentation](https://mike-fabian.github.io/ibus-typing-booster/docs/user/)
supports terminal disablement and regex autosettings keyed by toolkit, program,
and window title. It also warns that terminal passwords can appear in preedit
and be stored if users fail to disable the engine, and that distinctly styled
inline completion is difficult on Wayland. The input-method architecture gives
broad toolkit reach and multilingual composition, but app identity and styling
remain environment-dependent. The repository carries GPL-3.0 and Apache-2.0
license files; [2.31.0](https://github.com/mike-fabian/ibus-typing-booster/releases/tag/2.31.0)
was released on 2026-08-26. **Evidence A** (mature code, releases, and candid
platform documentation).

**Inference — lesson / do not copy.** Treat multilingual IME coexistence and
“off the record” as core requirements, and study its app autosettings and
long-lived user dictionary. Do not rely on titles as a security identity, allow
secret text into preedit/history, or call an indistinguishable Wayland preedit
“ghost text.”

### 7. SmartComplete

**Verified — surface and interaction.** [SmartComplete](https://github.com/ekremx25/smartcomplete)
is an Fcitx5 addon with a candidate list and Tab acceptance. It combines trie,
bigram, phrase, grammar-rule, typo, and emoji strategies; it deliberately passes
through known terminals and shells.

**Verified — data, controls, personalization, architecture, and activity.** The
six local strategies run offline. User selection frequency is persisted and
heavily boosts future ranking. An optional OpenAI reranker is confidence-gated,
cached, and timeout-bounded; it only reorders candidates generated locally.
Program blocklists are configurable, while fine-grained browser-origin,
context-source, learning, and retention policies are not documented. The C++17
architecture cleanly separates Fcitx, prediction, and external data, with test
suites for ranking and rules. MIT; no release is published, and the small
repository's last push was in April 2026. **Evidence A- for implementation,
low maturity confidence**.

**Inference — lesson / do not copy.** A deterministic fast lane can deliver
high-confidence phrases, spelling, and user vocabulary before an LLM finishes.
Keep it independently testable. Do not claim semantic Cotypist-quality prose,
“all Wayland apps,” or production safety from a small unvalidated Fcitx
prototype; an OpenAI reranker also needs an explicit remote-context receipt.

## Deliberate writing and rewrite assistants

### 8. WritingTools

**Verified — surface and interaction.** [WritingTools](https://github.com/theJayTea/WritingTools)
is a Windows/macOS/Linux selection tool: select text, invoke a hotkey, then
proofread, rewrite, change tone, summarize, or run a custom instruction. It can
also open a temporary chat when no text is selected. Linux support is described
as good on X11 and work-in-progress on native Wayland, with XWayland caveats.

**Verified — data, controls, personalization, architecture, and activity.** It
supports local Ollama and other OpenAI-compatible runtimes or cloud providers.
Invocation is manual, API keys/config stay local, and the project claims no
logging, tracking, or ads; cloud privacy is necessarily the selected provider's
privacy. Custom buttons and per-command prompts are durable user-authored
personalization, but it does not learn the user's voice from ambient typing.
The architecture is command/selection-first rather than a continuous field
session. GPL-3.0; its combined Windows/macOS v9/v6.1 release was published in
May 2026, with repository activity in August. **Evidence A-** (open source and
clear scope; Linux Wayland behavior not independently tested).

**Inference — lesson / do not copy.** An explicit selected-text transform is a
valuable later mode and a natural place to let users choose a larger local or
remote model. Keep it a separately invoked contract with one undoable
replacement. Do not let rewrite/chat/summarization feature breadth delay the
next-word loop or imply that X11-style selection reach solves Wayland.

### 9. LinuxPop

**Verified — surface and interaction.** [LinuxPop](https://github.com/GaimsDevSoftware/linuxpop)
shows context-aware actions over selected text on X11 and KDE Plasma 6 Wayland.
Actions include copy, search, translation, formatting, local Ollama, and plugins;
the bar can appear automatically on selection or from a global shortcut.

**Verified — data, controls, personalization, architecture, and activity.** Text
stays local unless the chosen plugin visibly sends it elsewhere. A tray toggle
pauses the product, and plugins are small local Python files. Its architecture
separates selection/editable detection, popup presentation, classification, and
plugins, with optional AT-SPI anchoring. There is no ambient completion model or
learned writing profile. MIT; [v0.9.7](https://github.com/GaimsDevSoftware/linuxpop/releases/tag/v0.9.7)
was released in June 2026 and remains labeled beta. **Evidence A-**.

**Inference — lesson / do not copy.** A compact action surface and manifest-like
plugins could become a later explicit-tools layer. Network behavior must be
declared per action. Do not conflate selection geometry with caret authority,
allow arbitrary plugins into the always-on completion process, or add “run as a
command” anywhere near suggestion acceptance.

## Text expansion and user-authored personalization

### 10. Espanso

**Verified — surface and interaction.** [Espanso](https://espanso.org/) is a
cross-platform Rust text expander. Typed triggers expand into deterministic
text, dates, emoji, forms, script output, or community packages; a search bar
finds snippets when the trigger is unknown. It supports app-specific
configuration.

**Verified — data, controls, personalization, architecture, and activity.** The
project is local, file-configured, and describes itself as no-tracking. Its
personalization is explicit and inspectable: users author YAML snippets and
install packages rather than surrender a writing stream. The engine observes
triggers and injects expansion text; that broad compatibility path is distinct
from verified field-native insertion. GPL-3.0. [v2.4.0](https://github.com/espanso/espanso/releases/tag/v2.4.0)
shipped in July 2026 with a wlroots `WaylandAppInfoProvider`, Hyprland-family
work, opt-in usage statistics, and Linux fixes. **Evidence A** (mature open
project and current release; per-app injection still needs tuple testing).

**Inference — lesson / do not copy.** Put deterministic, user-authored phrases
ahead of generation when a unique trigger or strong prefix matches. Offer an
import/export format and explain exactly which entry fired. Do not inherit
global injection as the semantic completion path, run arbitrary package scripts
inside the broker, or make community packages part of the first trust boundary.

### 11. Text Blaze

**Verified — surface and interaction.** [Text Blaze](https://blaze.today/) is a
proprietary Chrome text expander with Windows and macOS apps, but no native Linux
desktop app. Short triggers insert reusable snippets; dynamic commands, formulas,
and [forms](https://blaze.today/guides/forms/) collect structured values before
insertion. Shared folders synchronize team snippets.

**Verified — data, controls, personalization, architecture.** The extension
keeps a short recent-typing buffer locally and clears it after inactivity. The
vendor says it does not log keystrokes, website content, or form values, and
lets users restrict or block sites; snippets and folders themselves are account
data stored in Google Cloud. See [data collected](https://blaze.today/datacollected.html)
and the [FAQ](https://blaze.today/faq/). Personalization is authored templates,
folders, forms, and page-aware commands rather than learned prose. The browser
extension owns the page surface, which enables site-level policy and structured
DOM context. **Evidence B+** (detailed first-party docs, proprietary runtime,
some privacy pages are old).

**Inference — lesson / do not copy.** User-authored names, phrases, tone rules,
and templates are safer and more predictable than recording everything typed.
A bounded ephemeral trigger buffer is a useful precedent. Do not require an
account/cloud sync for the core, silently let templates read arbitrary page
content, or turn a low-friction completion into a form-building workflow.

## High-quality inline-completion UX

### 12. Gmail Smart Compose

**Verified — surface and interaction.** Gmail's
[Smart Compose](https://support.google.com/mail/answer/9116836?hl=en-gb) shows a
single inline continuation as an email is typed and accepts it with Tab. It can
be disabled at account level; personalized suggestions can be disabled
separately, reverting to generic suggestions. Google explicitly warns that it
is not designed to answer questions and may be factually wrong.

**Verified — data, controls, personalization, architecture.** It is a
Gmail-owned, server-served machine-learning feature. The original
[architecture account](https://research.google/blog/smart-compose-using-neural-networks-to-help-write-emails/)
conditions generation on the compose prefix, subject, and prior email body,
then uses a hybrid bag-of-words plus RNN language model and TPU inference. The
team treated per-keystroke latency below 100 ms as the ideal and reduced average
serving latency from hundreds to tens of milliseconds. Personalized style is
on by default for eligible accounts but separately switchable. The
[KDD paper](https://research.google/pubs/gmail-smart-compose-real-time-assisted-writing/)
documents model selection, serving, and evaluation challenges. **Evidence A**
(official production docs and original paper; architecture is historical, not
a description of every current backend detail).

**Inference — lesson / do not copy.** The owning app can supply a small,
semantically labeled context schema instead of scraping a whole screen. Latency
and abstention are product quality. Measure acceptance and keystrokes saved, not
generation volume. Do not copy cloud dependence, personalized-by-default
behavior, Gmail-only assumptions, or whole-suggestion-only acceptance.

### 13. Compose AI

**Verified — surface and interaction.** The
[Compose AI Chrome extension](https://chromewebstore.google.com/detail/compose-ai-ai-powered-wri/ddlbpiadoechcolndfeaonajmngmhblj)
combines inline sentence autocomplete accepted with Tab, `//` generation,
email replies, and selection rewriting across websites. The store listed about
300,000 users but shows its last update as January 2025.

**Verified — data, controls, personalization, architecture.** It is a cloud,
proprietary browser extension. The store disclosure says it handles personal
communications, web history, user activity, and website content. Its current
[privacy policy](https://content.composeai.io/privacy/) permits collection of
account and usage data and use of information to personalize and improve
services and AI models. Context-aware personalization is marketed, but its
mechanism, retention boundary, granular site policy, and inference architecture
are not publicly explained in enough detail to treat them as verified. **Evidence
B** (official store/privacy disclosures; architecture opaque and activity stale).

**Inference — lesson / do not copy.** It validates browser demand for Tab
completion and shows how quickly one product accumulates autocomplete,
generation, reply, and rewrite modes. Do not copy the feature sprawl, broad data
surface, or vague personalization. Omatype's browser adapter should start with
runtime-granted origins and suffix completion only.

### 14. GitHub Copilot inline suggestions in VS Code

**Verified — surface and interaction.** [VS Code's current inline-suggestion
UX](https://code.visualstudio.com/docs/editing/ai-powered-suggestions) renders
dim ghost text, accepts all with Tab, accepts the next word or line with
`Ctrl+Right`, cycles alternatives, and supports a timed Snooze. Next Edit
Suggestions can predict both the location and content of a subsequent edit.

**Verified — data, controls, personalization, architecture.** Copilot sends
bounded editor context—including current/open files and edit state—to a cloud
model. It adapts contextually to code and style rather than documenting a local
personal prose store. Users can disable suggestions globally or per language;
Business/Enterprise administrators can apply
[content exclusions](https://docs.github.com/en/copilot/concepts/context/content-exclusion),
although GitHub documents limitations for indirect semantic information,
symlinks, remote filesystems, and some modes. GitHub's
[responsible-use description](https://docs.github.com/en/copilot/responsible-use/inline-suggestions)
emphasizes scoped prompts, output filters, visual distinction, and explicit
acceptance. Its [usage metrics](https://docs.github.com/en/copilot/reference/copilot-usage-metrics/copilot-usage-metrics)
separate suggestions shown, suggestions accepted, acceptance rate, and accepted
output. **Evidence A** (official UX, policy, and metrics documentation;
proprietary models/service).

**Inference — lesson / do not copy.** Word/line/all granularity, snooze,
alternatives on demand, context scopes, and separate shown/accepted/output
metrics are excellent patterns. Do not add next-edit or multi-location prose
rewrites before suffix insertion is reliable, and do not assume enterprise path
exclusion equals a fail-closed field policy.

### 15. JetBrains Full Line Code Completion

**Verified — surface and interaction.** JetBrains
[Full Line completion](https://www.jetbrains.com/help/idea/full-line-code-completion.html)
shows inline local-model suggestions, accepts all with Tab, accepts a word with
`Ctrl+Right`, and accepts a line with End. It formats output, adds brackets and
quotes, and performs language-aware checks.

**Verified — data, controls, personalization, architecture.** The bundled
per-language models run locally and send no code over the internet; model
downloads/updates can be automatic, manual, or confirmation-gated. Users enable
the feature and languages independently. Smart filtering suppresses patterns
that users frequently cancel or delete immediately after acceptance, providing
behavioral adaptation without a documented raw personal-writing profile. An
official [architecture overview](https://blog.jetbrains.com/blog/2024/04/04/full-line-code-completion-in-jetbrains-ides-all-you-need-to-know/)
described a roughly 100M-parameter local model, 1,536-token context, and the
review-cost reason for initially preferring one line over longer output. The
plugin is proprietary/bundled with qualifying JetBrains products. **Evidence A**
(current docs plus first-party architecture; code-domain results do not transfer
directly to prose).

**Inference — lesson / do not copy.** A small task- and language-specialized
model with deterministic validation can beat a bigger general model in the
interactive path. “Canceled or erased soon after acceptance” is a valuable
quietness signal. Do not import code-specific auto-edit semantics or optimize
for longest completion; review cost is part of latency.

## Trust architecture and adjacent input

### 16. Veya

**Verified — surface and interaction.** [Veya](https://github.com/s3ba-b/veya)
is a pre-alpha Ubuntu/Linux system assistant, not a typing predictor. A GNOME
extension, overlay, CLI, and other clients talk to a user daemon over D-Bus; an
MCP server performs permission-gated system actions.

**Verified — data, controls, personalization, architecture, and activity.** It
routes local Ollama first with visible cloud fallback, grants context by source
(clipboard, files, notifications, screen), centralizes command safety, and logs
tool/cloud metadata without content. It also has a personal-context index. The
daemon/frontend/tool separation and ADRs make permissions inspectable. AGPL-3.0
or later; the repository labels itself pre-alpha, has no release, and last
showed activity in July 2026. **Evidence A-** (inspectable design and code;
minimal adoption and no independent security review).

**Inference — lesson / do not copy.** Borrow source-specific grants, visible
local/cloud routing, metadata-only audit, versioned desktop IPC, and recorded
architecture decisions. Do not put MCP tools, arbitrary system actions, or an
agent into the always-on typing trust boundary; a co-writer must remain much
narrower than a desktop assistant.

### 17. the existing OmaType dictation project

**Verified — surface and interaction.** [Aayush9029/OmaType](https://github.com/Aayush9029/OmaType)
is an unrelated Omarchy-native voice-typing project: tap Home to record and
transcribe accurately, hold for live dictation, then type the transcript at the
cursor. It supports local Parakeet, Whisper, and SenseVoice models.

**Verified — data, controls, personalization, architecture, and activity.** The
active model stays warm; the newest 200 transcripts are stored locally without
audio. The product has an explicit record gesture and visible capture state but
does not document per-app field policy or predictive completion. It is an
MIT-licensed fork of Voxtype, has no GitHub release, and its `hybrid-hotkey`
branch was active in August 2026. **Evidence A-** (small inspectable project;
behavior not tested).

**Inference — lesson / do not copy.** The exact public-name collision is severe:
same spelling, same Linux/Omarchy neighborhood, same “input anywhere” framing.
Keep this repository codename private and choose distinct package, binary,
desktop, D-Bus, domain, and product names before distribution. Voice could later
arrive through an explicit adapter, but do not merge microphone permission,
transcript retention, or synthetic dictation into the completion core.

## Cross-product decisions for Vision V2

### What the evidence consistently rewards

1. **One suggestion, one small decision.** Cotypist, Gmail, Copilot, and
   JetBrains all keep the primary path inline and accept-driven. Partial
   acceptance is the strongest authorship-preserving pattern.
2. **Abstention is a feature.** Pretype's published evaluation, JetBrains'
   rejection filtering, Cotypist's guidance, and IBus's “enable by key” option
   all show that a plausible but distracting suggestion can have negative
   value.
3. **A fast deterministic path belongs beside the model.** SmartComplete,
   IBus, Espanso, and Cotabby's native corrections cover names, phrases,
   spelling, emoji, and macros with lower latency and higher confidence than
   unconstrained generation.
4. **Small/specialized beats large/late.** Gmail engineered for tens of
   milliseconds; JetBrains uses a narrow local model; Cotypist warns that a
   larger model can worsen the experience; Pretype relies on cache reuse and
   per-language evaluation.
5. **The owning surface is the best integration.** Gmail and IDE completions
   work well because the application supplies exact context, caret state, and
   edit authority. Linux input methods provide reach, but not equivalent
   semantics in every toolkit.
6. **Personalization should begin with authored or approved evidence.** Custom
   instructions, app profiles, snippets, imported text, and accepted-session
   learning are safer than an ambient cross-app writing archive.
7. **Policy needs more dimensions than on/off.** Products often conflate
   suggestion display, context access, learning, remote inference, and storage.
   Omatype can make this separation a visible product advantage.
8. **Compatibility is a shipped artifact.** Cotypist's exceptions, IBus's
   Wayland caveats, and the young Fcitx projects show why exact tuples and
   failure reasons are more credible than platform logos.

### Differentiation thesis

Omatype's wedge is not “open-source Cotypist” and not “AI everywhere.” It is:

> **The capability-aware Linux co-writing runtime:** one consistent
> partial-acceptance loop across verified adapters, local by default, with a
> receipt for context/model/policy and a safe unsupported state everywhere
> else.

This produces four defensible differences:

- **Linux-native coexistence:** yield to IMEs, shell completion, editor
  completion, password surfaces, and app shortcuts instead of replacing them.
- **Observable trust:** show which adapter read what bounded context, which
  provider handled it, whether learning occurred, and why the feature is
  ambient/manual/blocked.
- **Quietness as quality:** optimize retained accepted text and net keystrokes,
  with abstention, cancellation, and per-app quieting as first-class behavior.
- **Adapter leverage:** a versioned capability contract and compatibility suite
  make each additional target cheaper and safer without pretending Linux has
  one universal text API.

### Product principles

1. **The user is the author.** Complete forward; never silently rewrite intent.
2. **No suggestion is better than a low-confidence interruption.** Generation
   volume is not a success metric.
3. **Deny before acquisition.** A hard-denied field produces zero context bytes,
   not merely zero provider calls.
4. **The target owns the edit.** Context, current revision, rendering, and final
   insertion stay in the adapter whenever the app exposes them.
5. **Capabilities, not platform claims.** Support is a tested app/toolkit/
   compositor/sandbox/version tuple with a reasoned fallback.
6. **Local by default, explicit when remote.** “Local” does not excuse broad
   capture, logging, weak socket boundaries, or hidden retention.
7. **Learn from intent, not surveillance.** Start with authored vocabulary,
   imported text, and accepted completions; raw ambient history is opt-in and
   unnecessary for V2.
8. **One reversible action.** Acceptance is one native transaction where the
   app can verify it; no retry through a second insertion mechanism.
9. **Coexist before expanding.** IME composition, editor suggestions, shell
   completion, undo, accessibility, and normal Tab behavior outrank Omatype.
10. **Explain failure.** Unsupported and manual states should be calm, visible,
    and actionable rather than silently degraded.
11. **Separate modes by risk.** Ambient suffix completion, manual rewrite,
    snippets, voice, screen context, and remote providers are separate grants
    and milestones.
12. **Measure retained value.** Count useful text that survives, not tokens
    generated or suggestions displayed.

## Chromium integration research delta

The competitor survey identifies the interaction to protect; primary Chromium
contracts determine how narrowly the first browser proof can make that
interaction real. These findings refine the implementation without widening
the product claim:

- Chrome native messaging is a framed JSON process boundary, not an extension
  socket API. Chrome writes a platform-native-endian 32-bit length followed by
  UTF-8 JSON; messages from Chrome may be at most 64 MiB and messages returned
  to Chrome at most 1 MiB. Omatype deliberately applies its much smaller
  65,536-byte protocol limit in both directions before allocation. The native
  host remains a bounded relay to the private Unix socket; it does not become a
  second policy or prediction implementation. [Chrome native messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging)
- A Linux user-level native-host manifest names an absolute executable and an
  exact `allowed_origins` extension origin. Chromium also defines a
  profile-scoped `NativeMessagingHosts` lookup path, which enables a disposable
  development proof without modifying the user's real configuration. A
  print-only manifest generator is therefore safer than an installer for this
  milestone. [Chrome native messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging),
  [Chromium path definitions](https://chromium.googlesource.com/chromium/src/+/master/chrome/common/chrome_paths.cc)
- An unpacked extension needs a stable identity before the native manifest can
  use an exact origin. Chromium documents the manifest `key` mechanism, and its
  source derives the 32-letter extension identifier from the public key. The
  development key is public identity material, not a signing secret; changing
  the production identity must require an explicit host rebuild and manifest
  update. [Manifest key](https://developer.chrome.com/docs/extensions/reference/manifest/key),
  [Chromium ID implementation](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/components/crx_file/id_util.cc)
- The first automated proof uses one static exact-document content-script match
  and only the `nativeMessaging` API permission. Chromium's runtime permission
  request is a user gesture and browser-prompt contract; the headless test path
  cannot honestly prove that consent UX. Runtime-granted origin access remains
  a later headed product gate, while `<all_urls>`, file URLs, incognito, and
  arbitrary frames remain excluded. [Permissions API](https://developer.chrome.com/docs/extensions/reference/api/permissions),
  [match patterns](https://developer.chrome.com/docs/extensions/develop/concepts/match-patterns),
  [Playwright extension testing](https://playwright.dev/docs/chrome-extensions)
- Incognito exclusion must be declarative, not merely absent from tests.
  Chromium's default mode is `spanning`, and a user can otherwise enable the
  extension in incognito. The development manifest therefore pins
  `"incognito": "not_allowed"`, while the sender gate independently rejects an
  incognito tab. [Incognito manifest key](https://developer.chrome.com/docs/extensions/reference/manifest/incognito)
- The sender gate fails closed unless Chromium reports the tab as explicitly
  active, non-discarded, and non-frozen. Because `tabs.Tab.frozen` first exists
  in Chrome 132, the development manifest and live receipt set 132 as the
  minimum supported Chromium version rather than treating a missing lifecycle
  signal as safe. [Chrome Tabs API](https://developer.chrome.com/docs/extensions/reference/api/tabs#type-Tab)
- Browser automation proves the browser, extension, native host, broker, and
  controlled DOM path on the named build. It does not prove a focused Wayland
  window, compositor geometry, framework-controlled inputs, browser-native
  undo grouping, or arbitrary-site behavior. Those stay separate capability
  cells instead of being inferred from a headless success.

## Prioritized V2 feature ladder

Each rung depends on the prior rung's exit gate. Later features should not be
pulled forward merely because a competitor has them.

| Priority | Rung | Build | Exit gate before advancing |
|---:|---|---|---|
| P0 | Trust substrate | Versioned broker/adapter protocol; focus epoch + revision + fingerprint; hard field deny before transport; Always/Manual/Never activation; separate context/inference/learning/retention axes; metadata-only diagnostics; capability receipt; global pause | Sensitive fixtures produce zero bytes/calls/log text; race suite produces zero stale display/insertion; every request has one policy receipt |
| P1 | Signature loop | Chromium ordinary fields and Obsidian Markdown adapter; deterministic provider; short local suffix provider; one-word and whole-suffix acceptance, dismiss, type-through reconciliation, configurable non-stealing shortcuts; adapter-owned ghost text; warm/cold state | Both target-API cells pass exact insertion, undo, composition, focus, and latency gates; blinded 12-prompt test finds at least 8 useful next words |
| P2 | Quiet intelligence | Confidence tail trim and abstention; adaptive debounce; late-result TTL; per-app quieting; deterministic phrase/spell/emoji lane; accepted-only vocabulary and user-authored app instructions; local import/export/delete | Net keystroke savings is positive over a real week; retained acceptance and interruption targets below pass; personalization beats base without raw-history collection |
| P3 | Linux breadth proof | Manual Fcitx terminal experiment; then conditional GTK/Qt/Fcitx tuples and a separate IBus lane; Firefox adapter; machine-readable compatibility report and reason codes | Each advertised tuple passes repeated position/commit/coexistence tests; failures remain explicitly unsupported; no synthetic fallback is relabeled as native support |
| P4 | Deliberate assist tools | Explicit selected-text rewrite as one undoable transaction; snippet/phrase library; alternatives on demand; optional local-provider chooser | Invoked mode never runs ambiently, provider/source is visible, and exact replacement/undo passes in each target |
| P5 | Optional richer context | Per-source grants for clipboard or focused-window OCR; explicit remote provider; encrypted sync only if demanded; mid-line fill-in-the-middle; voice adapter | Independent consent, revocation, deletion, audit, and redaction tests pass for every source/provider; no grant implies another |
| P6 | Advanced prediction | Next-edit suggestions, richer app adapters, shared adapter SDK, organization policy | Only after suffix completion demonstrates durable weekly value and adapter maintenance cost is acceptable |

### What remains out of the core

- a chatbot or general desktop agent;
- autonomous system actions or MCP tools in the typing process;
- ambient terminal completion at ordinary shell or secret prompts;
- raw `evdev`, screenshots, clipboard scraping, or synthetic typing as the
  universal context/insertion route;
- raw cross-app prose retention by default;
- accounts, subscriptions, sync, plugin marketplace, or team administration
  before local value is proven;
- voice, long-form generation, and next-edit prediction in the first proof.

## Measurable UX and trust metrics

The following are **provisional closed-alpha gates**, not industry benchmarks.
They should be revised after baseline data from the target machine, but every
metric has a precise numerator and denominator before implementation starts.

### Safety and correctness

| Metric | Definition | Initial gate |
|---|---|---:|
| Hard-denied context escape | Bytes of field text sent across adapter IPC, to a provider, or to normal logs from password/PIN/OTP/payment/secure fixtures | **0** |
| Stale display / insertion | Suggestions shown or inserted after focus epoch, revision, selection, composition, target, or policy changes | **0** in deterministic race suite and live focus tests |
| Exact target-API insert | Successful insertions whose resulting text and caret equal the prepared transaction | **100%** in 100 repeated browser and 100 Obsidian trials |
| Terminal safety | Accepted terminal text that changes bytes, targets another context, appends a control/newline, or submits/executes | **0** in 20 positioning/commit trials |
| IME interference | Foreign preedit/candidate events swallowed, reordered, or cleared by Omatype | **0** in supported tuples |

### Responsiveness

| Metric | Definition | Initial gate on the named i7-12700H/16 GB machine |
|---|---|---:|
| Warm first usable suffix | Last eligible edit to first visible, revision-valid suffix, including debounce | p50 **≤250 ms**, p95 **≤500 ms** |
| Deterministic result | Provider work after debounce | p95 **≤15 ms**; visible path **≤50 ms** |
| Accept-to-insert | Acceptance action to verified browser/Obsidian mutation | p95 **≤30 ms** |
| Clear latency | Invalidating event to invisible suggestion | p95 **≤32 ms** |
| Late result | Result older than the configured 600 ms generation TTL that becomes visible | **0** |

### Utility and quietness

| Metric | Definition | Initial gate |
|---|---|---:|
| Display acceptance | Displayed suggestions for which at least one word is accepted | Record by app, language, ambient/manual, and provider; do not use alone as a launch gate |
| Retained acceptance | Accepted graphemes not undone or erased within 10 seconds / accepted graphemes | **≥90%** in eligible prose sessions |
| Net keystroke savings | `(retained accepted graphemes - correction graphemes caused by acceptance) / graphemes in final text` | **≥10%** median over a one-week dogfood period |
| Bad appearance rate | Suggestions immediately escaped or contradicted by typing within two seconds / suggestions shown; exact type-through matches are excluded | **≤15%** after per-app tuning |
| Blind next-word usefulness | Prompts where an evaluator would willingly accept at least the first word | **≥8/12** on the frozen mixed prose/Markdown/agent-prompt set |
| Quietness lift | Change in net savings and bad-appearance rate with confidence gating versus ungated baseline | Gate must improve bad appearances without making net savings non-positive |
| Personalization lift | Difference in blind/retained acceptance for accepted-only vocabulary + explicit instructions versus base | **≥5 percentage points** before enabled by default |

Track accepted words and graphemes separately. A high acceptance rate can be
manufactured by showing only trivial completions; high keystroke savings can be
manufactured with long risky text. The product needs both retained value and a
low interruption cost.

### Trust and comprehensibility

| Metric | Definition | Initial gate |
|---|---|---:|
| Receipt completeness | Eligible requests with adapter, context sources, policy mode, provider locality, learning, and retention fields | **100%** |
| Remote visibility | Remote requests preceded by the necessary app/origin + provider grants and accompanied by a visible remote state | **100%**; remote remains absent from first proof |
| Deletion verification | Personalization records remaining after “delete all” plus restart | **0** |
| Capability truth | Advertised cells that fail the published compatibility test | **0** |
| User comprehension | Test participants who can identify whether the current suggestion is local/remote and why the target is Always/Manual/Never from the receipt | **5/5** before public beta |

## Unresolved naming and market risks

1. **Exact name collision — release blocker.** The existing
   [OmaType](https://github.com/Aayush9029/OmaType) is recent, Omarchy-native,
   local, and also types into the focused target. Different modality does not
   prevent package, search, service, support, or trademark confusion.
2. **Derivative positioning risk.** “Cotypist for Linux” explains the prototype
   but makes the product sound like a port with a replaceable model. Public
   positioning must lead with capability-aware trust and Linux coexistence.
3. **Crowded category language.** “AI writing assistant,” “copilot,” “type
   faster,” and “write everywhere” are saturated and hard to substantiate.
   Naming and copy should emphasize quiet co-writing, local control, and tested
   reach without promising omnipresence.
4. **Permission adoption.** Persistent input-method/accessibility software is
   held to a higher trust bar than an invoked rewrite tool. Open source helps
   inspectability but does not replace a clear permission map, packaging, and
   security review.
5. **Linux fragmentation can erase the market benefit.** A maintenance-heavy
   set of fragile app adapters is not automatically a moat. Track adapter effort
   per retained accepted word and stop widening if compatibility cost outpaces
   user value.
6. **Complement conflict.** Users already have IMEs, Espanso, browser grammar
   tools, editor Copilot, and shell completion. Omatype must yield cleanly and
   integrate selectively; “one assistant to replace them” is the wrong promise.
7. **Local model expectations.** Competitors on Apple Silicon benefit from a
   uniform accelerator stack. Linux CPU/iGPU performance and model licensing
   vary. “Local” must include measured hardware profiles and a graceful
   deterministic/manual fallback.
8. **Open-source license decision.** Relevant references span MIT, GPL-3.0, and
   AGPL-3.0. Choose Omatype's license before accepting or adapting code, and
   track model, tokenizer, dataset, and generated-artifact licenses separately.
9. **Weak demand evidence.** Product docs and repository activity establish a
   gap, not willingness to grant permissions or pay. The first prototype is a
   discovery instrument; retained use over a week matters more than launch
   enthusiasm.

## Explicit changes recommended to `VISION.md`

These are recommendations only; this research file does not edit the existing
vision.

| Existing area | V2 change | Why |
|---|---|---|
| Title / name warning | State that `omatype` is a private repository codename and a distinct public name is a hard distribution gate; prohibit reserving package, binary, desktop, D-Bus, or domain names under the collision | The adjacent OmaType is not hypothetical and occupies the same Omarchy input category |
| “The promise” | Replace “broad coverage” emphasis with “one consistent loop across verified adapters, and an explained unsupported state elsewhere” | Compatibility truth is a differentiator; “everywhere” is not a credible Linux capability |
| “Native to Linux's shape” | Name target-specific API adapters as the preferred tier, input methods as conditional breadth, AT-SPI as optional observation, and unsupported as a valid terminal state | Gmail/IDE quality comes from owned surfaces; Fcitx/IBus do not provide equivalent semantics everywhere |
| “Quiet enough to trust” | Add confidence tail trimming, abstention, per-app adaptive quieting, late-result TTL, and erase-after-accept as explicit product behavior | Competitor evidence shows ungated completion can be net-negative |
| “Context firewall” | Replace the three-state list as the whole policy with five independent axes: activation, context source, inference destination, learning, and retention. Keep Always/Manual/Never as activation only | Existing products commonly conflate these controls; separation is a real product advantage |
| “Local first” | Add “deny before acquisition,” metadata-only logs, exact local-process/network boundaries, model-license visibility, and a cold/pressure fallback | Local inference alone is not a privacy or reliability guarantee |
| “The interaction” | Make next-word acceptance the primary action; retain whole suffix and optionally one line. Add exact type-through reconciliation, no Tab theft without a visible current candidate, and confidence-based silence | Partial acceptance protects voice; typed-through text should refine rather than punish |
| New “authorship contract” section | State that ambient V2 only appends a short suffix. Rewrites, dictation, snippets, and next-edit changes are explicit separate modes and grants | Prevents feature sprawl and makes the user's mental model stable |
| “Product ideas worth protecting” | Add a deterministic authored-phrase lane, confidence receipt, accepted-only personalization, import/export/delete, and a compatibility artifact. Keep screen/clipboard context out of the default | Espanso/SmartComplete and the Mac projects show that high-confidence deterministic value complements the model |
| “Terminal-aware” | Preserve manual/local/ephemeral terminal behavior, but describe Ghostty/Codex as a falsifiable experiment rather than promised first-class support | The current architecture review rates this path low confidence; failure must not sink or mislabel the two target-API cells |
| “First proof” | Split success into a required two-adapter target-API proof (Chromium + Obsidian) and an independent terminal feasibility gate. Add retained acceptance, blind usefulness, net savings, bad appearance, and zero-byte safety metrics | A model demo or dispatched Fcitx commit is not proof of user value or verified editing |
| “Non-goals” | Add accounts/sync, raw-history personalization, remote inference, screen/clipboard defaults, rewrite suite, voice, next-edit, arbitrary plugins, and desktop-agent actions | Competitors demonstrate these are separate products and permission boundaries |
| North-star sentence | Suggested V2 wording: **“Help me write my own next words across the Linux apps that can support it—quietly, locally, and with proof of what the system saw and did.”** | Keeps authorship, reach, trust, and honest capability in one testable sentence |

## Final recommendation

Proceed with implementation, but protect the narrow signature loop. Build the
trust substrate and two target-specific adapters first, evaluate one deterministic
and one local semantic provider, and let the terminal experiment fail honestly
if Fcitx/Ghostty cannot meet the contract. Add quieting and accepted-only
personalization before rewrite, voice, remote providers, or broader desktop
reach.

The product earns a V2 only if it does three things together: inserts exactly
where the user expects, stays silent when it should, and makes its trust decision
legible. Anything less is either a model demo, a text expander, or another AI
popup—useful work, but not the vision.
