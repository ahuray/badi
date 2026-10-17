# Future plans

The only backlog and the plan for what comes next. Goal: the best local
replacement for Cotypist on Linux, Omarchy first, meaning useful suggestions
that appear fast, stay quiet and never edit the wrong place.

When an item is done, delete it and record what is now true in the
[README](README.md) or the affected runbook; [CHANGELOG.md](CHANGELOG.md)
keeps the history of what shipped. Evidence here names its kind (fixture,
real broker, real model, live app); a tested app version is not a claim about
Linux in general.

## Where Badi stands (2026-10-17)

- **Coverage** is ahead of every Linux alternative found: Chromium-based apps
  and any website, Zen, Omarchy web apps, Telegram, LibreOffice Writer,
  Obsidian, Bash and, by an opt-in rule, Ghostty's typed line
  ([ADR 0009](docs/decisions/0009-terminals-use-the-typed-line.md)). Neovim
  also passed live under this workstation's own LazyVim on 2026-10-17, in
  foot and in Ghostty, where it takes over from the typed line while it holds
  the terminal's focus.
  Writer checks spelling as you type and Alt+F7
  fixes the word at the caret; German, French, Spanish, Italian, Dutch and
  Polish spelling turn on with `badi spelling` and their Hunspell dictionaries. The closest Omarchy alternative, Oma Tab, replaces the input
  method and ships a 4.8 GB model.
- **Interaction:** type-through, Tab next-word acceptance (a quick second
  Tab is held for the returning remainder) and search-box
  quieting work, and a typed-through remainder returns within about 50 ms with
  no model call ([ADR 0004](docs/decisions/0004-type-through-carries-text-not-authority.md)).
  In browsers it now survives the field being admitted again after the
  keystroke: Chromium, Brave Origin and Zen kept it with no second model call
  on 2026-10-13 (checkout broker, real model; the installed build predates it).
  The inline preview ([ADR 0006](docs/decisions/0006-grey-preview-is-an-overlay.md))
  reaches VS Code, iframe editors, Qt apps (Telegram, Omawrite) and
  LibreOffice Writer.
- **Desktop signals:** the [session helper](adapters/session/README.md) holds
  every grant across sleep and session switches and defers automatic requests
  on battery ([ADR 0005](docs/decisions/0005-desktop-signals-withhold-authority.md)).
  Its fake-bus lane passes, and a short run on this desktop's real buses held
  the sleep inhibitor and registered both shortcuts. It is not installed here
  yet.
- **Speed and footprint:** the latest installed Chromium sample showed and
  accepted 20/20 warm suggestions, p50 228 ms / p95 284 ms, with zero
  wrong-field edits ([live evidence](adapters/fcitx5/README.md#tested-with)).
  The broker and runtime used 1.43 GB RSS (1.33 GiB)
  ([runtime](broker/README.md#writing-model-runtime)). These are bounded
  samples on this machine, not universal latency or language-quality claims.
  Vulkan on the Iris Xe is slower per keystroke (13–20 against 19–30 tokens/s
  for generation), so Badi stays on the CPU. For one long run-on sentence
  typed into Claude Code in Ghostty, moving the broker's 160-scalar prompt
  start in 64-scalar steps changed requests at model inputs of at least
  160 bytes from 8/31 proposals (13 cancelled before a first token) to
  27/27 proposals, with 25 distinct first ghosts within 550 ms
  ([terminal evidence](adapters/fcitx5/README.md#terminals)).
- **Quality:** the installed Qwen3-1.7B is weak on independent sets, and no
  ~350M candidate passed the English/German/Persian gates. Persian keeps the
  8-token automatic budget: on 2026-10-12, 21 tokens missed 550 ms on all 13
  development cases. Mid-sentence is on for this user since 2026-10-17
  (`badi mid-line on`), but it rarely helps: on the 41 mid-line development
  cases at 550 ms, production showed 2 and the Lab's `following_text` prompt 1
  ([why](broker/README.md#suggestion-limits)). Giving the model the whole
  160–224-scalar window instead of the current sentence was tried the same
  day and not kept: cold Lab prompts showed fewer suggestions (10 against 15
  of 19 real-text cases) with no more matches, and a live paragraph in Ghostty
  read no better. Long Tab-only chains drift (a run of Tabs turned a
  sentence into a maths exercise); the model, not the window, is the limit.
- **Diagnosis:** `badi debug on [--text]`, `badi debug watch` and `badi debug
  log` trace every request, suggestion, key and ghost show/hide across the
  broker, Fcitx and observer ([README](README.md#debug-trace),
  [ADR 0008](docs/decisions/0008-debug-trace-may-hold-text-on-request.md)).
  Each `model done` also records time to first token and the runtime's
  newly evaluated, cache-reused and generated token counts when available.
  Use it for every live trial.
- **Context and learning:** same-window context is extracted for the Lab only;
  style learning stores sealed samples behind `badi style on`, and no prompt
  reads them yet.

Standing user decisions:
- IME-parity for Chromium-based apps, Zen and LibreOffice Writer
  ([ADR 0003](docs/decisions/0003-ime-parity-append-only.md));
- promote a quality fix only without a per-language harm increase;
- keep the model resident;
- explicit requests get about 1.2 s;
- Tab accepts one word and Ctrl+→ all of it, on every surface (changed
  2026-10-16);
- Ghostty follows the typed line automatically, by exact opt-in rule
  ([ADR 0009](docs/decisions/0009-terminals-use-the-typed-line.md));
- window context and learning stay on the device, with global and per-app
  opt-outs; language work prioritizes English, German, French and Spanish,
  and Persian keeps its regression gates (confirmed 2026-10-12; the
  [context firewall](VISION.md#context-firewall) owns this contract);
- distribution starts with an AUR package; publication and an Omarchy proposal
  still need approval after naming and licence review;
- Discord uses desktop-wide AT-SPI `ScreenReaderEnabled`, with recorded prior
  settings and unchanged Badi app/site permissions (confirmed 2026-10-13).

Open decisions, for the user:
- **Terminal privacy.** A terminal cannot mark a password prompt, so a
  passphrase of two or more words in an allowed terminal can reach the local
  model. Keep it, make terminals explicit-only, or pause after prompts such as
  `password:` (Badi cannot read terminal output to detect them).
- **Spelling beyond English is unqualified.** It is opt-in; decide whether that
  is enough or each language first needs the held-out measurement under
  [Suggestion quality](#suggestion-quality).
- **The `--text` debug trace** keeps typed text in memory for 15 minutes;
  confirm that, or limit it to suggestions.

## Next steps, in order

Each step ends with its completion test. Steps that install to the desktop or
publish need the user's go-ahead first; the rest can start at once.

1. **Run the installed-flow matrix.** `scripts/install-desktop.py` installs
   the broker, addon, observer and the session helper; `main` runs here
   (`badi doctor` clean, 2026-10-16), and the session helper's Hyprland binds
   (`SUPER+ALT+B` pause-toggle, `SUPER+ALT+PERIOD` open-settings) are in the
   user's `badi.lua`. Still to run, with disposable text: a real suspend and
   resume, a VT switch and a Tab pressed right after resume; a service restart, undo, pause, composition and stale
   focus, including Ghostty's typed line, which must start empty and stay
   quiet after a restart; and fractional scaling / several monitors when a
   second display is at hand. *Done when* the matrix shows zero wrong-field or stale edits and
   the session helper and Fcitx runbooks record it.
2. **Try the new editor surfaces live.** Neovim is done: the real model
   works in an interactive terminal and, through the installer's new
   lazy.nvim spec, under this workstation's own LazyVim in foot
   (2026-10-17: display, Tab, Ctrl+→, streamed growth, Insert-leave
   dismissal and an explicit Ctrl+Shift+Space request). Writer: the grey preview, Tab and
   Alt+F7 with one Ctrl+Z work (2026-10-14, isolated profile); still open are
   a model-backed correction of an ambiguous word and the automatic hint with
   German, French and Spanish dictionaries installed. A dark pill with every
   character doubled is cotype, a separate autocomplete now disabled on this
   desktop, not Badi. *Done when* the
   [Neovim](adapters/neovim/README.md) and
   [Writer](adapters/libreoffice/README.md) runbooks record the live result
   and any defect found is fixed.
3. **Qualify on the frozen confirmation set.** The user-approved public set
   (UD r2.16 test, 100 English and 40 each German, French, Spanish and
   Persian prefixes) is frozen privately at
   `output/writing/confirmation-ud-r2.16-2026-10-12/suite.json`, SHA-256
   `72a69d10d9ec6b261373a6f382b203b512290eb3bff2b71c3b3ee3efd769c1cc`, with
   source hashes, licences and approval. Its runtime-loading equivalence
   ([runtime](broker/README.md#writing-model-runtime)) and field-language
   selector ([Lab](evaluation/writing/README.md)) uses are done; neither is a
   quality qualification. Select and freeze the candidate on development data
   before inference; report usefulness, abstention, errors, useful words,
   keystroke savings and latency misses separately; review full additions.
   Promoting a model needs a new hypothesis, fresh independent cases, native
   Persian review where results are ambiguous and the full
   [qualification](docs/architecture/model-selection.md). The candidate is
   chosen: on 2026-10-17 eight models ran the same 135 development cases
   (20 each en/de/fr/es/fa plus real-text and probe sets; French and Spanish
   in the Lab `healed` mode, since production refuses them) at 550 ms and 8
   tokens, and one blind agent reviewer per language judged all 674 unique
   additions. Gemma 3 1B pt Q4_K_M (`gguf-org/gemma-3-1b-pt-gguf`, 0.81 GB,
   Gemma Terms of Use) was useful on 46 requests against the installed
   Qwen3-1.7B's 33, at least acceptable on 106 against 87, with 4 harmful
   against 3, and faster (shown p50 247 against 351 ms). Llama 3.2 1B was
   next (41 useful, but 10 harmful); Granite-4.0-1B-Base, LFM2.5-1.2B-Base,
   EuroLLM-1.7B and Qwen3.5-2B-Base were near or below the installed model;
   Gemma 4 E2B, at its smallest text-only file (2.19 GB, 2-bit), was useful
   on 19. Development data and agent reviewers, not a qualification; the
   runs are under `output/writing/2026-10-17-model-research/`. The Gemma
   terms need the licence review before any distribution. A second blind
   round on 2026-10-18 (other reviewers, stricter overall) found no better
   build: Google's QAT Q4_0 tied it on useful additions (38 each) but was
   worse on acceptable additions and useful words; an imatrix Q4_K_M and the
   instruct QAT were worse, and so were Llama 3.2 1B's imatrix and instruct
   builds. Since then the catalog recommends by this screen score and this
   desktop runs Gemma 3 1B by the user's choice (`badi model`). *Done when*
   Gemma 3 1B pt passes or fails every gate per language on that set.
4. **Evaluate same-window context independently.** The observer's
   [`window_context()`](adapters/accessibility/README.md#same-window-context-lab-only)
   reads the nearest heading and preceding paragraph of the field's own web
   document, never another field, a password, a hidden node or another frame;
   it is unit-tested on fake trees only. A live check was not run: the
   desktop was locked, and a headless Chromium on a private bus exposed no
   field. On 2026-10-13 the
   [development set](evaluation/writing/lab/examples/window-context-development.json)
   (24 cases written for this, so not independent; `healed`, 8 tokens,
   pinned 1.7B on battery with power-saver) gave the expected next words in 15
   of 16 informative cases with the context and 4 of 16 without, with a 3 s
   diagnostic budget. In 3 of the 8 controls the context pulled the
   continuation onto the page's topic ("I have attached the chapter three"),
   and several additions answered for the user ("November? Yes, it's
   available"). At 550 ms both arms missed the deadline on 23 or 24 cold
   prompts. On 2026-10-17 window words were tried as a logit bias instead of
   prompt text, so no prompt grows and 550 ms holds (`context_bias`,
   [Lab](evaluation/writing/README.md), English only): on the same 24 cases
   first-word matches rose from 8 to 11 and matching characters from 43 to 64
   at bias 2.0, with every suggestion still shown; one control drifted onto
   the page's topic. Across four suites (65 cases) 12 became 15. Bias 4.0
   reached 13 of 24 but wrote loops ("Thursday, Thursday"), and Persian
   degraded at both, so it is excluded. The field's own earlier text as the
   source measured neutral. For terminals, which expose no accessibility
   text, [`window-ocr.py`](evaluation/writing/window-ocr.py) read a
   disposable Ghostty screen with no character errors in about 0.8 s
   (capture 0.2 s, OCR 0.56 s at logical scale), but five replies about that
   screen gained nothing from its words. *Done when* independent cases with
   full-addition review, a fence like `style_fact_conflict` for context facts
   and a measured gain justify production use; only then add the source (the
   observer's accessibility context, and screen reading only after its own
   privacy decision), the global and per-app controls, the wire field and
   cancellation on focus changes.
5. **Compare style learning in the Lab.** `badi style on` keeps the sentence
   written before each accepted suggestion on sites that allow learning,
   sealed with a keyring key; revocation, expiry, key loss, forgetting and the
   locked keyring are tested
   ([broker runbook](broker/README.md#style-learning-opt-in)). Feed an
   explicit `badi style export` into a Lab suite's `style` field and compare
   against no style on the user's own held-out drafts, with the
   `style_fact_conflict` fence and native review. Linux apps cannot learn
   under the settings schema (`linux_app` subjects force `learn: block`);
   lifting that is a separate privacy decision. *Done when* the comparison
   shows a per-language gain without more harm, before any prompt reads
   samples.
6. **Verify the [AUR candidate](packaging/aur/README.md) on a fresh Omarchy.**
   It packages the addon, broker, observer and user units; writing assets are
   a separate pinned, hash-verified download. *Done when* install, update and
   removal leave nothing behind; publication waits for the naming and licence
   review.

## Later, by area

### Interaction

- [ ] The manual native path has no dialog or width signal, so its file
      dialogs and very narrow fields are not automatic-quiet the way observed
      fields are ([observer runbook](adapters/accessibility/README.md)).
      Escape quiet is a broker setting (`escape_quiet_seconds`), covered in
      the [broker runbook](broker/README.md).
- [ ] Measure overflow frequency per app; fractional-scale and multi-monitor
      live trials remain open. The implemented placement and fallback behavior
      is in the [preview contract](adapters/accessibility/README.md#caret-geometry-and-preview).
      Investigate explicit requests in horizontally scrolled, very narrow
      inputs: the installed Chromium trial at 100 CSS px did not establish
      coverage ([tested boundary](adapters/fcitx5/README.md#tested-with)).
- [ ] Zen showed its suggestion in Fcitx's candidate box, not the overlay,
      in a fresh profile started with `GNOME_ACCESSIBILITY=1` (2026-10-13,
      [trial](adapters/fcitx5/README.md#tested-with)). Find which geometry
      gate declined; the user's own Zen profile is untested since.
- [ ] Confirm live that the grey preview uses the host font in Telegram.
      A disposable Chromium textarea (2026-10-12, build `7957f16`) set to
      JetBrainsMono NF at 20px reports `family-name` JetBrainsMono NF and
      `size` 15pt, and the grey continuation is painted on that same line in
      the same monospaced face. Omawrite 0.5.0 reports no font attributes and
      a zero glyph box, so its overlay keeps the caret-size fallback.
- [ ] Browser event cost on large pages (`daemon.py` focus handling and
      `field.py` serialization).
- [ ] Each streamed partial suggestion hides the overlay and draws it again
      about 1 ms later (`preview hidden cause=replaced` in the
      [debug trace](README.md#debug-trace), Chromium, 2026-10-14). Check
      whether that flickers, and move the shown surface instead of hiding it.

### Speed and footprint

Exit criteria: warm visible p50 ≤ 250 ms and p95 ≤ 500 ms with misses
reported, resident memory about 1.3 GB, and no quality regression in the Lab.
Measured alternatives that were not adopted are in the
[runtime runbook](broker/README.md#writing-model-runtime).

- [ ] Fewer generated tokens, only after a held-out quality comparison.
- [ ] Pin the runtime's four threads to performance cores (`taskset`/
      `--cpu-mask`): one idle measurement saw 94/30 against 54/21 prompt/
      generate tokens per second for Qwen3-1.7B, a second under load saw no
      consistent gain. Measure on an idle machine before changing the launch.
- [ ] Battery scheduling: finish the [paced trial](evaluation/power/battery-trial.py).
      On 2026-10-13 (this laptop on battery with power-saver, release build,
      752 disposable keys per mode at 150 ms per key, 450 ms after words and
      1.5 s after sentences) `quiet` made 122 model calls against 586, used
      182 against 464 runtime CPU seconds, showed 313 against 257 suggestions
      with 211 against 190 matching the reference, and had visible p95 latency
      of 550 against 525 ms. The same trial later that day, charging in the
      performance profile (the trial still reports battery, one repeat, 376
      keys per mode), showed the cost on a fast CPU: 45 against 220 calls and
      50 against 148 runtime CPU seconds, but 27 against 31 distinct reference
      words covered and p95 456 against 211 ms. On 2026-10-17 in the
      power-saver profile, a run-on sentence typed at 450 ms per word showed
      15 suggestions from 62 requests under `quiet`, most cancelled by the
      next word after generation began
      ([runtime](broker/README.md#suggestion-limits)). Still open: the balanced
      profile on battery, real typing intervals, energy from a privileged power
      counter rather than runtime CPU time, and a reviewed count of fully
      accepted additions.
- [ ] Use [Linux pressure signals](https://www.kernel.org/doc/html/latest/accounting/psi.html)
      to reduce speculative requests while the machine is contended. PSI files
      are available here; `CPUWeight=1000`, `MemoryHigh=3G` and resident-model
      protection already exist. Measure foreground responsiveness and deadline
      misses before changing those controls; do not add automatic model eviction.
- [ ] Lower-memory hardware: cold start, RAM, sustained latency, power and
      crash recovery.

### One-command install on Omarchy

Exit criteria: on a fresh Omarchy install, one menu action covers install,
suggestion, acceptance, update and uninstall, leaving nothing behind.

- [ ] Omarchy integration in the style of Voxtype's: a first-run invitation
      (`omarchy-done ensure`, `omarchy-notification-send --exec`), a
      floating-terminal installer, menu entries for settings and removal,
      default keys for the session helper's `pause-toggle` and
      `open-settings` portal shortcuts, and theme colours for the preview.
- [ ] Support [omarchy#12139](https://github.com/omacom/omarchy/pull/12139)
      (`--enable-wayland-ime` by default); keep the per-app flags until it lands.
- [ ] Signed updates. The broker service is already sandboxed: `PrivateNetwork=yes`,
      a Unix-only runtime socket, and a read-only model directory.
- [ ] Retire the [Fcitx 5.1.22 frontend backport](packaging/fcitx5-wayland-compat/README.md)
      once Omarchy ships Fcitx 5.1.23.

### Suggestion quality

- [ ] Measure dictionary spelling (`badi spelling`, opt-in beyond English)
      on held-out misspellings per language: correction precision, abstention
      and the model's choice among candidates. That choice is an uncached
      model call of 1-2.5 s here, so it fits Writer's spelling budget but not
      the 550 ms typing budget, where several candidates get no correction;
      a cached spelling prompt slot would close that gap.

- [ ] Qualify French and Spanish before enabling them. Gather independently
      reviewed confirmation prefixes and full-addition reviews for both,
      alongside the existing English, German and Persian confirmation gate.
- [ ] Improve short-fragment language coverage without reducing precision.
      The conservative field selector keeps the adapter hint for short or
      uncertain text; explicit per-app overrides remain available
      ([contract](broker/README.md#writing-model-runtime)).
- [ ] Global and per-app instructions, and a strength setting that trades
      frequency for confidence.
- [ ] Cut the latency of the combined instruction/boundary mode (18/22 useful
      in development, 2.8 s cold median) and confirm it independently.
- [ ] Keep facts in style examples from overriding the draft; confirm the Lab's
      `style_fact_conflict` fence across languages.
- [ ] Tell an intended partial word from a completed one before contextual
      lookup: `mire` → `Mirella` must complete, `marin` must not become
      `Marinella`. Dictionary membership is the wrong splitter.
- [ ] Spelling replacement undo in Telegram and Xournal++. Omawrite 0.5.0
      (2026-10-12, disposable `teh ` → `the `) needed two Ctrl+Z steps: the
      first removed the correction and the second restored the typo. Obsidian
      already replaces the suffix in one undo step. German and Persian stay
      off until the Lab spelling gate passes (the 40-case run was 2 and 4
      useful, with one unwanted change each).
- [ ] Calibrate token confidence against reviewed full additions per language.
- [ ] Spelling for Persian, which has no cased letters and needs its own word
      shape rather than the European one.
- [ ] Measure Cotypist parity with the same tasks on a named Cotypist version on
      a Mac.

### Remaining surfaces

- [ ] Browser rich editors: a caret inside a list or quote, EditContext editors
      (VS Code's default, CodeMirror on recent Chromium), multi-paragraph fields
      in Zen (Gecko sends only the caret's paragraph) and Firefox.
- [ ] Discord with desktop-wide accessibility; a composer trial needs a private
      channel, since typing shows an indicator.
- [ ] Grok Bot composer trial in the signed-in app after a relaunch with
      `grok-bot-flags.conf`.
- [ ] More native toolkits through measured integrations.
- [ ] Neovim spelling replacement through `text_replacement`, with
      stale-replacement and undo tests.
- [ ] Extend shell support using
      [Zsh ZLE buffers and `POSTDISPLAY`](https://zsh.sourceforge.io/Doc/Release/Zsh-Line-Editor.html)
      and [Fish's command-line API](https://fishshell.com/docs/current/cmds/commandline.html).
      Zsh has a documented preview surface; Fish buffer access still needs a
      measured preview integration compatible with its own autosuggestions.
      Neither shell is installed here. Acceptance must only edit the buffer,
      preserve native undo and never execute a generated command.
- [ ] Terminals other than Ghostty (kitty, foot, Alacritty) on the typed-line
      path ([ADR 0009](docs/decisions/0009-terminals-use-the-typed-line.md)),
      each after a live trial of its app id and `commitString`.
- [ ] Panel rows for per-app language overrides and style learning, which
      are terminal-only today.
- [ ] Later: other Linux desktops, a standard tray and personal snippets.
      Emoji stays with Fcitx: QuickPhrase and the Emoji module, not a Badi
      colon shortcode.

### Engineering upkeep

- [ ] `observed-desktop.py` timed out once at "candidate before timed quiet"
      (#81) and passed on rerun. If it recurs, give that wait its neighbours'
      bound or find the timing it races.
- [ ] No test covers `SettingsView.qml`'s keyboard model; a headless `qs`
      test with a fake controller driving `move()` and `activate()` would
      guard j/k, h/l and search.
- [ ] One replacement-shape rule lives in `protocol.rs`, the schema,
      `text-safety.mjs` and `text.py`; a shared fixture file, like
      `orthographic-joiner-fixtures.json`, would stop them drifting.
