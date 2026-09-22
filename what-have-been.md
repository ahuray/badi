# What has been built

Badi is a pre-release local writing assistant for Linux/Omarchy.

The 2026-09-10 [model discovery development screen](evaluation/writing/README.md#measured-shortlist-development-2026-09-10)
retained 360 requests across the production Qwen baseline, its terminal-observed
boundary mode, and three approximately 350M candidates. Granite, LFM2.5 and
SmolLM2 yielded 12/4/0, 9/5/2 and 11/3/0 useful on-time English/German/Persian
additions per 24 opportunities, with harmful counts of 6/13/5, 8/7/3 and 4/9/1.
Candidate peak RSS was 455–515 MiB and cold startup medians were 1.45–1.49 s;
actual response clocks and censored complete-delivery bounds are documented
separately. No candidate passed the language quality gates, so the sealed
confirmation set remains unused. The production arm's missing terminal evidence
does not prove zero useful output. Reviews were blinded agent judgments on
synthetic drafts, not human acceptance or native integration evidence. No
installed model or prediction default changed.
The reusable HTTP Lab now connects bounded public Hub discovery, conservative
Rust fit gates, pinned/resumable verified downloads, full-addition review and
private identity-bound evidence. Eighty existing regression requests also
completed without worker errors. A five-minute Granite diagnostic and shorter
checks for all artifacts verified cancellation/recovery and cleanup; only Granite
and Smol delivered all final paced requests within 550 ms. Actual Vulkan execution
was separately proven on the Intel GPU. The final browser check exercised review,
receipt save/reload and cancellation; a lifecycle regression fix invalidates old
performance receipts after cancelled cleanup failures. Measurements are bound to
their exact source build, and changed evaluator code rejects historical receipts.

The 2026-09-09 rerun found host drift after Omarchy upgraded to 4.0.3. The
isolated test harness now handles the combined panel/bar placement and its
separate desktop bridge. The reviewed host passed 100 copied-shell open/close
cycles, forced-cleanup recovery, strict QML/hash checks and a clean pinned-source
check; installed desktop configuration was not changed. The Lab also supports a
fixed loopback port and explains how to start its server when raw HTML is opened.
The unchanged word-lookup rerun reproduced its three unwanted completions. A
new prepared batch-16/batch-64 comparison returned only one useful terminal
suggestion within 550 ms per 22 opportunities in either arm, with no preparation
speed gain for 64. The original quality gate failed. Three separate cancellation
and fresh-process recovery trials passed with all owned processes reaped; cleanup
took roughly one to two seconds and fresh startup cost remained separate.
The Lab now adds an explicit one-word mode with a bounded language grammar,
actual terminal/separator checks and unchanged editing guards. Headed English,
German and Persian smoke requests finished correctly after the first grammar's
mixed-script output was rejected and the lexical constraint repaired. Answer-free
case creation/import also works, with stale-import and reset regressions.
Its fresh 24-main/six-control comparison failed the typing-speed quality gate:
both modes yielded zero useful main results within 550 ms. The constrained mode
delivered one neutral addition and one wrong date copied from a style example;
no German or Persian output qualified. All 60 opportunities remain counted and
all owned workers were reaped. The mode remains an unchecked Lab experiment.
A fixed internal native-stop probe then verified exact space/newline token
handling and one fewer generation step in each of three literal-word pairs.
Their terminal times fell by 49/95/179 ms, but the Persian case remained above
550 ms. All six runtimes were reaped and source/artifact identities stayed
stable. This supports a specific termination experiment, not a prediction
quality gain. The final Rust checks passed 371 tests with three intentional
ignores; the final Lab suite passed 135 tests and the npm aggregate passed.

The 2026-09-09 opt-in [Prediction Lab](evaluation/writing/README.md#prediction-lab)
adds an editable draft/context/style and expected-output workbench, a reproducible
CLI, prompt/output inspection and explicit import/export. Real local-model browser
checks exercise cancellation, clearing, recovery, keyboard import and visible
predictions; default session storage stays in memory. The matched development
comparison identifies exact word-boundary handling as a promising narrow change,
while Persian context deadlines and writing-style quality remain unresolved.
The isolated boundary mode shares actual production logic and is opt-in; these
experiments do not change installed prediction defaults or application coverage.
Its 240-request independent synthetic confirmation reduced unhelpful outputs but
failed the predeclared substantive first-word gain, so it remains experimental.
The Lab's final lifecycle fixes fence reset/upload races and verify owned runtime
identity; process tests and a real startup-kill trial cover cleanup before ready.
The paced context diagnostic retains every typed revision and verifies terminal
completion, deadlines and owned cleanup. Its 24-trial comparison failed the
quality gate: priming yielded one useful final continuation and two factual
conflicts out of twelve traces, versus no eligible cold output. It remains a
diagnostic; no prediction default was promoted.
An explicit, verified Lab-only model descriptor enabled a same-family Base
comparison without changing the installed model. Its 32-request development
screen produced 8/16 useful eligible continuations versus 6/16, but failed the
required gain and regressed in Persian. This candidate also remains experimental.
Combining instructions with exact boundary healing passed a later 66-request
development diagnostic: 18/22 useful full continuations versus 13/22 for healing
alone and 6/22 for instructions alone, with no candidate harm in agent review.
Its 2.81-second cold median remains too slow; no installed default changed.
A paired preparation experiment raised on-time useful yield from 0/22 to 11/22,
but missed its 18-result gate; preparation cost remains separate from the 550 ms
target timing. The Lab also adds a separate native German/Persian Spelling tab
with exact suffix previews, local expected-answer scoring and cancellation.
Its real HTTP40 run completed after a Btrfs startup-verification fix, with all
owned processes reaped. Six corrections matched expected words, but two unwanted
control changes failed the quality gate; automatic correction remains unqualified.
Ordinary startup cancellation now waits for bounded ownership verification before
shutdown, with actual zero-query cancel/restart proof and no late word dispatch.

The separate model-free Word completion tab reuses one exact matching source
word with grey suffix previews. Its 30-case development run supplied all 12
intended completions, but wrongly extended three completed words, failing the
quality gate. All responses arrived within 35.27 ms over HTTP and all 30 workers
were reaped. Real browser checks passed multilingual previews, ambiguous-match
withdrawal, keyboard tab navigation, clear/retry and preservation of other drafts.
The full npm check and affected Rust/CLI checks pass; no production default changed.

The separate exact-context word recovery gained two useful German completions
in 24 requests, below its +3 gate, with no false recovery authorization. German
and Persian dictionary pairs are pinned with notices; a 48-word native Hunspell
probe supports a separate conservative correction experiment, not auto-correction.

- Rust broker: private IPC/settings, per-app policy, cancellation, focus/revision
  binding, expiring suggestions, one-shot acceptance, content-free metrics.
- Hardware-based model selection and verified local llama.cpp startup. This
  workstation uses Qwen3-1.7B Q4_K_M for English continuations up to four words.
- Cooperative Fcitx addon: tested Omawrite and Xournal++ text cells, manual
  Tab request/accept and Escape dismissal. Explicit grants now support
  further canonical app identities where the toolkit supplies suitable context.
- Chromium: separate localhost fixture and opt-in Dillinger/Monaco extension;
  guarded append and narrow spelling replacement. A separate ordinary-web build
  adds input/textarea predictions and separate undo, including normal Brave setup.
- Obsidian CodeMirror and Bash Readline integrations: real local-model prediction,
  explicit acceptance, dismissal, revision/focus checks and native undo.
- Persistent desktop model service, terminal controls, native launchers, and
  an installed Omarchy bar/settings panel. Loading a model does not prove input.
- Opt-in expiring activity diagnostics, distinct Badi mark, and settings controls
  that remain enabled during background status polling. Physical panel tests
  verified pause/resume and 45 stable captures across polling cycles.
- Physical Omarchy typing checks showed real Qwen words in Omawrite, Xournal++,
  Obsidian and Bash; browser/Obsidian/Bash also exercise acceptance and undo.
- Historical compatibility receipts remain in `capabilities/`; they are
  immutable observations of earlier code, not certification of today's tree.

The repeated dated plans and delivery reports were consolidated on 2026-09-05.
Their recovery archive is stored outside the checkout under
`~/.local/state/badi/document-archives/`. Existing Git history also remains.
The only active backlog is [future plans](<future plans.md>).

On 2026-09-07, the no-suggestions investigation found failed model startup after
hardware advice selected an absent model. The installed broker now selects
verified installed models within resource limits, supervises its owned runtime,
and reports actionable startup/input diagnostics. Browser and Obsidian reconnect
without reopening the document; browser native-host EOF and cold-restart backoff
have process and real-application regressions. Obsidian's editor-local Escape
observer fixes dismissal when the host consumes CodeMirror's key event.

The update installed the broker, Fcitx addon, Obsidian/Bash/browser files and
Omarchy panel with backups. English partial-word filtering uses a pinned,
reproducible 77,928-word lexicon with complete notices. German and Persian
continuations and contextual Persian half-spaces have explicit, experimental
contracts. The 132-prefix evaluation and its non-independent candidate review
are documented in [writing evaluation](evaluation/writing/README.md); parity and
human keystroke savings remain unproved. Current Rust/MSRV and aggregate source
checks passed, as did real-broker Fcitx/editor integrations, actual headless
Obsidian/Chromium flows and installed native D-Bus routing. A current Omawrite
desktop trial saved a real prediction and undid exactly that insertion.
Xournal++ 1.3.7 likewise saved the changed text object and restored the original
prefix with document undo after leaving its text editor.

The installed Bash preview now uses a real Readline redisplay hook and a readable
neutral grey. Physical Ghostty 1.3.1 / Bash 5.3.15 / Readline 8.3 trials verified
real-model continuation, explicit correction of an exact English suffix, and
native undo; previews did not alter the command and no test command was submitted.
The installed observer and opt-in pinned Fcitx frontend produced a grey real-model
preview in extension-free Chromium. Sandbox-enabled physical tests then failed
separate undo and exact editing authority: page callbacks redirected both
replacement and single native insertion to another field, or changed its caret.
Native replacement is withdrawn and the native browser/Codex path is quarantined;
editor-owned correction remains separate. The active backlog tracks the missing
editor transaction authority and the Codex trial.
The normal desktop returned to the stock Fcitx frontend and original browser
startup configuration. The installed quarantine preserves Tab navigation and
text with zero model context or native commit in the sandbox-enabled trial.
The writing client's shared 550 ms deadline now includes waiting for HTTP
headers; expiry closes the request and prevents another inference after an
exhausted spelling attempt. Real errors and cancellation remain distinct from
budget exhaustion. Rust, MSRV, desktop and aggregate source checks pass.
