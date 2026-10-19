# Future plans

The only backlog and the plan for what comes next. Goal: the best local
replacement for Cotypist on Linux, Omarchy first, meaning useful suggestions
that appear fast, stay quiet and never edit the wrong place.

When an item is done, delete it and record what is now true in the
[README](README.md) or the affected runbook; [CHANGELOG.md](CHANGELOG.md)
keeps the history of what shipped. Evidence lives in the runbooks; this file
links to it. A tested app version is not a claim about Linux in general.

## Where Badi stands (2026-10-19)

- **Coverage** is ahead of every Linux alternative found: Chromium-based apps
  and any website, Zen, Omarchy web apps, Telegram, LibreOffice Writer
  (spelling as you type, Alt+F7), Obsidian, Bash, Neovim (live under LazyVim
  in foot and Ghostty) and, by an opt-in rule, Ghostty's typed line
  ([ADR 0009](docs/decisions/0009-terminals-use-the-typed-line.md)). German,
  French, Spanish, Italian, Dutch and Polish spelling are opt-in
  (`badi spelling`). The closest Omarchy alternative, Oma Tab, replaces the
  input method and ships a 4.8 GB model.
- **Interaction:** type-through returns a typed-through remainder in about
  50 ms with no model call, also when a browser re-admits the field
  ([ADR 0004](docs/decisions/0004-type-through-carries-text-not-authority.md));
  Tab takes a word and Ctrl+→ the rest; search boxes stay quiet; the inline
  preview ([ADR 0006](docs/decisions/0006-grey-preview-is-an-overlay.md))
  reaches VS Code, iframe editors, Qt apps and Writer.
- **Desktop signals:** the installed [session helper](adapters/session/README.md)
  holds grants across sleep and session switches and defers automatic
  requests on battery ([ADR 0005](docs/decisions/0005-desktop-signals-withhold-authority.md)).
- **Speed:** with a full sliding-window cache (2026-10-19), Gemma 3 reads 1.5
  new prompt tokens per keystroke and a Chromium suggestion shows at p50 241
  ms after the key ([runtime](broker/README.md#writing-model-runtime)). This
  laptop's firmware caps sustained CPU power at 15 W; Gemma 3 fits 550 ms
  there and comfortably at 28 W. The Iris Xe and more than four threads were
  measured and not adopted ([runtime](broker/README.md#writing-model-runtime)).
- **Quality:** Gemma 3 1B is the recommended model; its weakest languages are
  English and German. No model qualifies: on the frozen confirmation set
  Gemma 3 1B made 15–25% useful full additions per language against the
  60% gate, with 1–5 harmful each ([qualification](docs/architecture/model-selection.md#stages-and-gates)). In the paired European-language screen at 28 W,
  Gemma 4 E2B was level overall (42 against 40–41% useful) and better only
  in German (30 against 21–22%), at four times the memory
  ([model fit](docs/architecture/model-selection.md)). Mid-sentence
  suggestions rarely help, and longer Tab chains drift; the model, not the
  context window, is the limit ([why](broker/README.md#suggestion-limits)).
- **Diagnosis:** `badi debug on [--text]`, `badi debug watch` and `badi debug
  log` trace every request (with first-token time and new and reused prompt
  tokens), suggestion, key and ghost ([README](README.md#debug-trace),
  [ADR 0008](docs/decisions/0008-debug-trace-may-hold-text-on-request.md)).
  Use it for every live trial.
- **Context and learning:** same-window context exists for the Lab only;
  style learning stores sealed samples behind `badi style on`, and no prompt
  reads them yet.

Standing user decisions:
- IME-parity for Chromium-based apps, Zen and LibreOffice Writer
  ([ADR 0003](docs/decisions/0003-ime-parity-append-only.md));
- promote a quality fix only without a per-language harm increase;
- keep the model resident; explicit requests get about 1.2 s;
- Tab accepts one word and Ctrl+→ all of it, on every surface;
- Ghostty follows the typed line automatically, by exact opt-in rule
  ([ADR 0009](docs/decisions/0009-terminals-use-the-typed-line.md));
- window context and learning stay on the device, with global and per-app
  opt-outs; language work prioritizes English, German, French, Spanish and
  Portuguese, and Persian keeps its regression gates
  ([context firewall](VISION.md#context-firewall));
- distribution starts with an AUR package; publication and an Omarchy proposal
  still need approval after naming and licence review;
- Discord uses desktop-wide AT-SPI `ScreenReaderEnabled`, with recorded prior
  settings and unchanged Badi app/site permissions.

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
- **Gemma 4 E2B in the catalog.** The screen found no overall gain over
  Gemma 3 1B, better German, fewer harmful suggestions and 4.6 GB peak
  memory ([model fit](docs/architecture/model-selection.md)). Leave it out
  (recommended: nothing to gain for most users); offer it through `badi model
  use` only, never recommended; or recommend it above a sustained power
  limit, which needs a device-tier input that reads RAPL PL1 and stays
  unknown on many machines.
- **The 28 W CPU power limit** is set until reboot; installing
  `output/llm-bench/cpu-sustained-power.service` keeps it (see the
  [bench](evaluation/llm-bench/README.md) conditions).

## Next steps, in order

Each step ends with its completion test. Steps that install to the desktop or
publish need the user's go-ahead first; the rest can start at once.

1. **Finish the installed-flow matrix with someone present.** Still open: a
   real suspend and resume, a VT switch and a Tab right after each, and
   several monitors when a second display is at hand. Restarts, undo, pause,
   composition, stale focus, Ghostty's typed line and fractional scaling
   passed on 2026-10-19 ([matrix](adapters/fcitx5/README.md#tested-with)).
   *Done when* those show zero wrong-field or stale edits and the
   [session helper](adapters/session/README.md) runbook records them.
2. **Evaluate same-window context independently.** Development results so far
   are in the [Lab runbook](evaluation/writing/README.md). *Done when*
   independent cases with full-addition review, a fence like
   `style_fact_conflict` for context facts and a measured gain justify
   production use; only then add the source (the observer's accessibility
   context; screen reading only after its own privacy decision), global and
   per-app controls, the wire field and cancellation on focus changes.
3. **Compare style learning in the Lab.** Feed an explicit `badi style
   export` into a Lab suite's `style` field and compare against no style on
   the user's own held-out drafts, with the `style_fact_conflict` fence and
   native review ([broker runbook](broker/README.md#style-learning-opt-in)).
   Linux apps cannot learn under the settings schema; lifting that is a
   separate privacy decision. *Done when* the comparison shows a per-language
   gain without more harm, before any prompt reads samples.
4. **Verify the [AUR candidate](packaging/aur/README.md) on a fresh Omarchy.**
   *Done when* install, update and removal leave nothing behind; publication
   waits for the naming and licence review.

## Later, by area

### Interaction

- [ ] The manual native path has no dialog or width signal, so its file
      dialogs and very narrow fields are not automatic-quiet the way observed
      fields are ([observer runbook](adapters/accessibility/README.md)).
      Escape quiet is a broker setting (`escape_quiet_seconds`), covered in
      the [broker runbook](broker/README.md).
- [ ] Measure overflow frequency per app; the multi-monitor live trial
      remains open (scale 1.5 passed on 2026-10-19). The implemented placement and fallback behavior
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
- [ ] Battery scheduling: finish the [paced trial](evaluation/power/battery-trial.py)
      on the balanced profile, real typing intervals and a privileged energy
      counter ([results so far](adapters/session/README.md#battery)).
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
      and the model's choice among candidates. On 26 disposable development
      cases Gemma 3 1B chose the intended word in 18, a wrong one in 2
      (`Zeitt → Zeigt`, `cassa → casas`) and abstained on 6
      ([broker runbook](broker/README.md#suggestion-limits)). That choice is
      an uncached model call of 0.4–0.6 s at the 28 W limit (1–2.5 s with
      the earlier Qwen3 1.7B at 15 W), so it fits Writer's spelling budget
      but often not the 550 ms typing budget; a cached spelling prompt slot
      would close that gap.

- [ ] Qualify French, Spanish and Portuguese before turning them on by default
      (they are opt-in with `badi suggest LANG on`). Gather independently
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
- [ ] Lift English with Gemma 3 1B, its weakest language (29% useful against
      Qwen3 1.7B's 40% and Granite 4.0 H-Tiny's 42% in the
      [European-language screen](docs/architecture/model-selection.md)):
      measure a second, English-only resident model (about 2.5 GB together)
      against prompt and decoding changes before choosing.
- [ ] Offer Granite 4.0 H-Tiny base on machines that fit it: best English and
      German in that screen, but 7.7 GB peak and 96 of 325 requests over
      550 ms here.
- [ ] Compare Qwen3.5 0.8B base with Qwen3 0.6B as the compact choice (battery,
      no AVX2): it was the fastest screened model (p95 301 ms, no miss) at 35%
      useful against Gemma 3 1B's 46%, but Qwen3 0.6B was not screened.
- [ ] Calibrate token confidence against reviewed full additions per language.
      Gmail Smart Compose shows a suggestion only above a length-normalized
      log-probability threshold set for a target coverage
      ([arXiv 1906.00080](https://arxiv.org/abs/1906.00080)). With Gemma 3 1B
      on 2026-10-18, 62 of 215 English, German and Persian development
      requests were useful, 67 acceptable but generic and 44 unhelpful; such
      a gate, or cutting a suggestion before its first low-confidence word,
      targets the last two.
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
