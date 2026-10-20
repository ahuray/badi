# Changelog

What shipped, newest first, by merged pull request. This is history only:
current behavior lives in the [README](README.md) and the runbooks, open work
in [future plans](<future plans.md>), and lasting decisions in
[docs/decisions](docs/decisions/).

## 2026-10-13: desktop signals, new surfaces and a clean code base

- The spelling replacement shape has one shared fixture
  (`protocol/spelling-replacement-fixtures.json`) that Rust, the v2 schema,
  `text-safety.mjs` and the Writer extension all test against. The schema and
  `text-safety.mjs` now count letters as Rust does (Unicode's Lowercase or
  Uppercase property), and the Writer extension follows suit for symbols with
  case.
- `observed-desktop.py` keeps the session's own output and a `result.json`
  even when the session dies before its logs open. It stops at once when its
  python3 cannot import PyGObject (the 2026-10-20 empty report came from a
  virtualenv's python3). The waits after a settings change get the 8 s bound
  of their neighbours (#81).
- The settings panel's keyboard model (j/k, h/l, Enter, x and search) lives in
  `KeyModel.js` with its own tests. Prediction Lab terminal runs build their
  worker first, so they never run a stale binary, and record every broker
  source. The Lab's Word completion tab runs the broker's vocabulary rule
  instead of its own copy.
- Opt-in context (`badi context on`, or the panel's one-time question)
  finishes a typed word, five letters in, from words in the same window and
  words learned in that app. Learned words are sealed with the keyring key and
  expire after 90 days. Apps and sites can be turned off one at a time, and
  `badi context forget` deletes the learned words. User decision, accepting
  the measured trap cost ([ADR 0010](docs/decisions/0010-context-finishes-words.md)).
- Finishing the typed word from words in view (the window, the writer's
  earlier writing, the draft) is measured in the Lab's `vocabulary` mode: on
  two independent 60-case sets it gave 39 and 23 useful additions against 5
  and 1, but traps became the wrong word (harmful 13 against 3, then 7
  against 4), so the gates fail and production requests carry no vocabulary.
- The Claude desktop app (`com.anthropic.claude`) joins the IME-parity apps:
  its composer accepts through one append-only commit once the app runs with
  renderer accessibility (`--observed-app com.anthropic.claude`).
- The Omarchy panel's accent and urgent colors read `Commons.Color`, as
  Omarchy 4.0.5 does: on Qt 6.12 a bare `Color` names QtQuick's new singleton
  and worked only through Omarchy's temporary Quickshell patch. The pinned
  host, and CI's Arch snapshot, moved to Omarchy 4.0.5, Quickshell 0.3.2 and
  Qt declarative 6.12.0.
- A suggestion now shows only when every token is likely enough (log-probability
  ≥ −2.75, user decision): held-out harmful additions fell from 15 to 10 and
  unhelpful ones from 18 to 10, at 7 of 51 useful ones; at 15 W the preview
  appears at p50 294 against 218 ms. Withheld suggestions count as
  `low_confidence`.
- Token confidence was calibrated on Gemma 3 1B's reviewed suggestions: a
  frozen whole-suggestion rule cut held-out harmful additions from 15 to 10
  but lost 7 of 51 useful ones and needs the full suggestion before display,
  so no gate ships yet.
- The Fcitx 5.1.22 Wayland frontend backport is retired: Arch ships
  Fcitx 5.1.23 with the upstream fix, Chromium 154 kept its surrounding
  text on the stock frontend, and a complete install removes the old
  launcher and drop-in with a backup.
- Gemma 4 E2B stays out of the model catalog by user decision: no overall
  gain over Gemma 3 1B for four times the memory.
- Same-window context on 40 independent English cases: the word bias gained
  nothing, and window text in the prompt helped informative replies but
  drifted in controls, copied a fact the writer was changing and missed
  550 ms, so it stays in the Lab.
- The first qualification on the frozen confirmation set fails Gemma 3 1B
  in every language: 15–25% useful on-time full additions against the 60%
  gate and 1–5 harmful each, so no model is qualified yet.
- Gemma 4 E2B ran the European-language screen beside a paired Gemma 3 1B
  rerun at 28 W: level overall (42 against 40–41% useful in two blind
  rounds), better in German, at 4.6 GB and with 94 of 325 cold requests over
  550 ms; whether the catalog offers it is the user's decision.
- The installed flow held across service restarts, pause, composition,
  stale focus, Ghostty's typed line and fractional scale 1.5 with no
  wrong-field or stale edit (2026-10-19); suspend, VT switch and several
  monitors still need someone present.
- Spelling corrections that need the model's choice between dictionary
  candidates work again with Gemma 3 1B: the prompt ends inside an opened
  quote and the answer is read raw, where the old `Answer:` prompt got only a
  line break from base models. Writer's automatic hint showed
  `grasias → gracias` and `vacanses → vacances` live; the debug trace gains a
  `spelling` line.
- [#102](https://github.com/ahuray/badi/pull/102) The backlog is compact
  and links to its evidence, which now lives in the runbooks; screening
  Gemma 4 E2B comes before qualification.
- [#101](https://github.com/ahuray/badi/pull/101) The larger models were
  timed at Badi's launch: on this laptop's firmware 15 W power limit only
  Gemma 3 fits 550 ms; at 28 W Gemma 4 E2B fits too and is the next Lab
  candidate. Six threads and the Iris Xe were measured and not adopted.
- [#100](https://github.com/ahuray/badi/pull/100) Gemma 3 keeps a full
  sliding-window cache, so it reuses its prompt between keystrokes and after
  a backspace: in Chromium each model call read 1.5 instead of 6.4 new
  tokens and suggestions became visible at p50 241 against 288 ms.
- [#99](https://github.com/ahuray/badi/pull/99) A local LLM bench measures
  bandwidth, model ceilings, thread and backend sweeps, cache reuse after
  edits and a German/English screen; Granite 4.0 Micro, Gemma 4 E2B and
  Qwen3.5-4B write better German than Gemma 3 1B, whose cache survives a
  backspace only with `--swa-full`. Timing awaits unclamped clocks.
- [#98](https://github.com/ahuray/badi/pull/98) Portuguese joins French and
  Spanish as an experimental, opt-in suggestion language that Gemma 3 1B
  heals; a European-language screen of eleven small models keeps Gemma 3 1B
  as the recommendation.
- [#97](https://github.com/ahuray/badi/pull/97) Choose the languages Badi
  suggests in and whether each heals a typed space: `badi suggest LANG
  on|off`, `badi suggest LANG healing on|off|auto` and the panel's
  Suggestions rows. French and Spanish join as experimental, opt-in
  languages; each catalog model names the languages its review heals.
- [#96](https://github.com/ahuray/badi/pull/96) With Gemma 3 1B, German
  after a space is healed like English and Persian: 15 against 7 useful of
  67 German development requests, 3 against 2 harmful, by user decision.
- [#95](https://github.com/ahuray/badi/pull/95) The model can no longer
  write markup, backticks, line breaks or a number the text lacks, which
  used to discard the whole suggestion, and German after a space abstains
  instead of showing fragments; with Gemma 3 1B, 62 against 61 useful of
  215 development requests, harm unchanged. Healing German is an open
  decision.
- [#94](https://github.com/ahuray/badi/pull/94) Badi recommends the writing
  model that fits each device best (Gemma 3 1B on this one, by a blind
  development screen of eight models), and `badi model` and the panel's
  Model row switch models with a pinned, hash-checked download; the
  installer accepts any catalog model.
- [#93](https://github.com/ahuray/badi/pull/93) Lab: window text can bias the
  model's word choice (`context_bias`, English, no longer prompt), and
  `window-ocr.py` reads a window's text from the screen; both measured,
  production unchanged until a window-text source is decided.
- [#92](https://github.com/ahuray/badi/pull/92) In terminals, Tab fixes a
  typo (`teh → the`) instead of appending the correction after it, and the
  typed line follows the caret through arrows, Home, End and Delete.
  Mid-sentence suggestions are on by user choice; a wider prompt window was
  measured and not kept.
- [#91](https://github.com/ahuray/badi/pull/91) The terminal suggestion panel
  wears the Omarchy theme (Omarchy font, the word Tab takes in bold accent)
  and follows theme and font changes; a paste, completion or End no longer
  ends terminal suggestions until Enter.
- [#90](https://github.com/ahuray/badi/pull/90) Neovim in a terminal owns its
  keys: while it holds the terminal's focus (`editor.focus`), the broker
  withholds the input method's typed line, so Tab and Ctrl+Shift+Space reach
  Neovim and only one suggester runs. The 64-scalar context step was measured
  live and needs no cache warming.
- [#89](https://github.com/ahuray/badi/pull/89) The debug trace records
  model timing and cache counts; long run-on sentences keep the runtime's
  prefix cache through 64-scalar context steps; and the Neovim installer
  writes a lazy.nvim spec so the plugin loads under LazyVim.
- [#88](https://github.com/ahuray/badi/pull/88) Follow-up: a provider-rejected
  final also keeps the shown partial, and at most one Tab is held per word
  acceptance; the terminal budget measurement and the Neovim, Writer and
  Omawrite live rows are recorded.
- [#87](https://github.com/ahuray/badi/pull/87) Tab accepts one word and
  Ctrl+→ all of it on every surface; a quick second Tab is held for the
  returning remainder; a rejected or late final no longer clears a shown
  partial; automatic misses (terminals above all) no longer show a notice.
- [#86](https://github.com/ahuray/badi/pull/86) Records only: `AGENTS.md` gives
  agents a numbered start and work loop with live-trial steps; `next steps.md`
  is folded into the backlog, whose step 1 is now Tab-takes-a-word and a
  steadier ghost.
- [#85](https://github.com/ahuray/badi/pull/85) Records only: `next steps.md`
  reviews what to consider after #78–#84; two finished backlog items retired.
- [#84](https://github.com/ahuray/badi/pull/84) The bar mark opens a keyboard-first
  Omarchy popout: mode chip, activity spinner, acceptance meter, settings
  search and rows for battery, spelling languages and Ghostty.
- [#83](https://github.com/ahuray/badi/pull/83) `badi spelling` turns on German,
  French, Spanish, Italian, Dutch and Polish spelling from installed Hunspell
  dictionaries; the model chooses between candidates.
- [#82](https://github.com/ahuray/badi/pull/82) Writer checks each finished word
  on its own and Alt+F7 corrects the word at the caret; the broker serves the
  Writer target corrections only.
- [#81](https://github.com/ahuray/badi/pull/81) Ghostty: suggestions follow the
  line typed into an allowed terminal and Tab types them
  ([ADR 0009](docs/decisions/0009-terminals-use-the-typed-line.md)).
- [#80](https://github.com/ahuray/badi/pull/80) Obsidian and Neovim show the
  whole streamed suggestion and accept the newest; Writer suggests after a
  trailing space at a paragraph end.
- [#79](https://github.com/ahuray/badi/pull/79) `badi debug on [--text]`
  traces each suggestion from model to screen across the broker, Fcitx,
  observer, editors and compositor ([ADR 0008](docs/decisions/0008-debug-trace-may-hold-text-on-request.md));
  Writer's trailing-space sentence lookup no longer refuses the caret.
- [#78](https://github.com/ahuray/badi/pull/78) Type-through keeps the
  carried remainder when a browser field is re-admitted with the same
  context, instead of asking the model again; the observer's overflow chip
  sits under the caret inside the field when nothing follows it. Corrects
  #77: the doubled pill was cotype, a separate autocomplete, not Badi.
- [#77](https://github.com/ahuray/badi/pull/77) Records only: the
  2026-10-13 evening live install/trial results in the plan — session
  helper and binds proven, the observer chip-doubling defect in browsers
  and Writer, and live type-through carry counters. No behavior change.
- [#76](https://github.com/ahuray/badi/pull/76) `badictl status` counts
  type-through carry outcomes by declining gate (`type_through`), and a
  browser-scoped single-space carry is regression-tested; the live
  Brave/Zen gate identification stays open in the plan.
- [#75](https://github.com/ahuray/badi/pull/75)
  [ADR 0007](docs/decisions/0007-retire-protocol-v1.md) retiring protocol
  v1: what keeps it alive and the removal order. Every shipped client
  already negotiates v2 only.
- [#74](https://github.com/ahuray/badi/pull/74) The `badi-session` helper
  (logind sleep and session state, UPower and power-profile state,
  GlobalShortcuts portal, keyring delivery of the style key) and
  [ADR 0005](docs/decisions/0005-desktop-signals-withhold-authority.md);
  battery scheduling (`badi battery`, default `quiet`); the Neovim plugin; the
  LibreOffice Writer spelling extension; Lab-only same-window context; opt-in
  sealed style learning (`badi style`). The same branch removed the dated
  parity snapshot and reports, recorded the overlay decision as
  [ADR 0006](docs/decisions/0006-grey-preview-is-an-overlay.md), turned the
  backlog into an ordered plan and split the largest source files (broker
  settings, engine tests, the Fcitx addon) into focused modules without
  behavior changes.
- [#73](https://github.com/ahuray/badi/pull/73) Installed preview checks and
  the Linux feature research behind #74.
- [#72](https://github.com/ahuray/badi/pull/72) Inline previews with longer
  negotiated reading windows and quiet grey overflow.

## 2026-10-12: parity work, speed and the Lab

- [#70](https://github.com/ahuray/badi/pull/70),
  [#71](https://github.com/ahuray/badi/pull/71) Conservative field-language
  selection, the AUR package candidate, observer lock-state handling and
  runtime verification; dismissal ends when a native field's text changes.
- [#51](https://github.com/ahuray/badi/pull/51)–[#55](https://github.com/ahuray/badi/pull/55),
  [#40](https://github.com/ahuray/badi/pull/40)–[#49](https://github.com/ahuray/badi/pull/49)
  Speed: a known field starts the model on the key, the session is reused
  while typing, each finished word shows while the rest generates; the first
  grey word reached about 220 ms (Omawrite) and 250 ms (Chromium) in the
  recorded samples.
- [#57](https://github.com/ahuray/badi/pull/57),
  [#58](https://github.com/ahuray/badi/pull/58),
  [#59](https://github.com/ahuray/badi/pull/59) Measured and not adopted:
  Q4_0, speculative decoding and a following-line prompt.
- [#61](https://github.com/ahuray/badi/pull/61)–[#66](https://github.com/ahuray/badi/pull/66)
  The preview follows scrolls, the panel polls only while open, Bash's
  preview timer runs only while a suggestion shows, and type-through waits
  for typed characters to land.
- [#37](https://github.com/ahuray/badi/pull/37),
  [#38](https://github.com/ahuray/badi/pull/38) Exact spelling replacement on
  native fields that support it; the grey preview uses the field's font.
- [#35](https://github.com/ahuray/badi/pull/35),
  [#36](https://github.com/ahuray/badi/pull/36) Mid-sentence suggestions stay
  off; Lab frame, field-context and warm-prefix runs.
- [#32](https://github.com/ahuray/badi/pull/32)–[#34](https://github.com/ahuray/badi/pull/34)
  Audit fixes: toolchain, editing contract, sandbox and Lab follow-ups.
- Records and maintenance: [#39](https://github.com/ahuray/badi/pull/39),
  [#56](https://github.com/ahuray/badi/pull/56),
  [#60](https://github.com/ahuray/badi/pull/60),
  [#67](https://github.com/ahuray/badi/pull/67)–[#69](https://github.com/ahuray/badi/pull/69).

## 2026-10-06: type-through and wider coverage

- [#28](https://github.com/ahuray/badi/pull/28),
  [#29](https://github.com/ahuray/badi/pull/29) Type-through, next-word
  acceptance and search-box quieting
  ([ADR 0004](docs/decisions/0004-type-through-carries-text-not-authority.md)).
- [#27](https://github.com/ahuray/badi/pull/27) Rich editors on any website;
  allowlist and blocklist modes for apps and sites.
- [#26](https://github.com/ahuray/badi/pull/26) Omarchy web apps, LibreOffice
  Writer and Grok Bot through IME-parity; a VS Code 1.140 fix.

## 2026-09-27: daily apps

- [#24](https://github.com/ahuray/badi/pull/24) Badi working in daily apps:
  IME-parity for Chromium, Brave, Zen, Codex, VS Code, Cursor and Discord with
  no browser extension ([ADR 0003](docs/decisions/0003-ime-parity-append-only.md));
  the Fcitx 5.1.22 compatibility frontend; a resident model service. The old
  evaluator, browser extension and legacy panel were removed, and the Lab
  worker became its own crate.

## 2026-08-30: foundation

- [#4](https://github.com/ahuray/badi/pull/4) Hardened foundation after an
  adversarial audit ([ADR 0001](docs/decisions/0001-same-uid-trust-boundary.md)).
- [#5](https://github.com/ahuray/badi/pull/5)–[#13](https://github.com/ahuray/badi/pull/13)
  Curated dependency updates and their CI policy.
