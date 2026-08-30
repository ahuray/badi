# Badi competitive landscape

Research snapshot: **2026-08-29**. This report asks a narrow question: what makes Cotypist compelling, which open-source projects are genuinely relevant, and what is still missing for a Linux-native ghostwriter that works in browsers, Obsidian, note apps, and terminal AI prompts?

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
