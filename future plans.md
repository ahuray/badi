# Future plans

The only backlog and the plan for what comes next. Goal: the best local
replacement for Cotypist on Linux, Omarchy first, meaning useful suggestions
that appear fast, stay quiet and never edit the wrong place.

When an item is done, delete it and record what is now true in the
[README](README.md) or the affected runbook; [CHANGELOG.md](CHANGELOG.md)
keeps the history of what shipped. Evidence lives in the runbooks; this file
links to it. A tested app version is not a claim about Linux in general.

## Where Badi stands (2026-10-20)

- **Coverage** is ahead of every Linux alternative found: Chromium-based apps
  and any website, Zen, Omarchy web apps in Brave (Chromium's are step 1),
  Telegram, LibreOffice Writer
  (spelling as you type, Alt+F7), Obsidian, Bash, Neovim (live under LazyVim
  in foot and Ghostty) and, by an opt-in rule, Ghostty's typed line
  ([ADR 0009](docs/decisions/0009-terminals-use-the-typed-line.md)); the
  [app sweep](adapters/fcitx5/README.md#tested-with) of 2026-10-20 passed in
  Brave Origin, a web app, Zen, Writer, Obsidian, Neovim, Ghostty and VS
  Code. Omawrite and other observed Qt fields suggest again despite a
  fcitx5-qt 5.1.16 bug, but cannot replace a typo until it is fixed
  (step 4). German,
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
  new prompt tokens per keystroke and a Chromium suggestion showed at p50 241
  ms after the key ([runtime](broker/README.md#writing-model-runtime)). The
  confidence gate (2026-10-20) judges each suggestion whole before showing
  it: at 15 W the preview now appears at p50 294 against 218 ms, and a
  sixth fewer word pauses show one
  ([gate](broker/README.md#suggestion-limits)). This
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
- **Context and learning:** `badi context on` (or the panel's one-time
  question) lets Badi finish a typed word from words in the same window and
  words learned in that app ([ADR 0010](docs/decisions/0010-context-finishes-words.md));
  prompt-text window context stays Lab-only. Style learning stores sealed
  samples behind `badi style on`, and no prompt reads them yet.

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
- opt-in context finishes words from the window and learned words although
  a trap word in view can show wrongly: harmful additions rose from 4 to 7
  on the second independent set, for 23 useful ones against 1 (2026-10-20,
  [ADR 0010](docs/decisions/0010-context-finishes-words.md));
- observed Qt fields keep reading the text fcitx5-qt 5.1.16 withdraws right
  after sending it, while the manual path stays closed and spelling
  replacement is withheld there (2026-10-20,
  [Fcitx contract](adapters/fcitx5/README.md#contract));
- Gemma 4 E2B stays out of the catalog: no overall gain for four times the
  memory ([model fit](docs/architecture/model-selection.md));
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
- **The 28 W CPU power limit** is set until reboot; installing
  `output/llm-bench/cpu-sustained-power.service` keeps it (see the
  [bench](evaluation/llm-bench/README.md) conditions).

## Next steps, in order

Each step ends with its completion test. Steps that install to the desktop or
publish need the user's go-ahead first; the rest can start at once.

1. **Make a fresh Omarchy 4.0.6 work out of the box.** A read-only audit
   on 2026-10-20 found gaps that only this workstation's setup hides:
   - Omarchy's web apps open in Chromium by default, as `chrome-<site>`
     windows, which Badi denies; only Brave web apps are mapped.
   - foot, Omarchy's default terminal, has no typed-line support
     ([ADR 0009](docs/decisions/0009-terminals-use-the-typed-line.md)).
   - `omarchy-refresh-hyprland`, `omarchy-refresh-chromium` and
     `omarchy-install-browser` replace `hyprland.lua` and the browser flag
     files, silently dropping Badi's `require` and accessibility flag.
   - The installer never checks for Fcitx 5.1.23, reads the Fcitx profile
     without checking that it exists, and the suggested `SUPER + ALT +
     COMMA` is Omarchy's "invoke last notification".
   - The observer's fast lock check is pinned to Quickshell 0.3.1 (0.3.2 is
     installed), and the panel plugin is pinned to Omarchy 4.0.5-1.
   - The panel runs `~/.local/bin/badi-desktop`, which a package install
     does not have.
   *Done when* each is fixed or refused with a clear message, live where it
   touches an app, and `badi doctor` reports a dropped line with its repair.
2. **Finish licence and packaging for publication.** Badi is MIT and no
   shipped dependency is GPL or unknown (audit, 2026-10-20), but:
   - a binary package needs a third-party notices file (BSD-3-Clause
     `subtle`, MPL-2.0 `spellbook`'s source location, Zlib, MIT and Apache
     texts, nlohmann-json) and a wider `license=` array;
   - Gemma, the recommended model, is downloaded with its terms shown only
     by `badi model use`: the installer's first-model hint and the panel
     skip them, and no acceptance is recorded;
   - `.SRCINFO` is stale (`fcitx5>=5.1.22`, no `libsecret`), the desktop
     launcher, `hypr/badi.lua` and the theme hook are not packaged, and the
     addon builds with `-Werror` under makepkg's flags;
   - CI never builds the package, and the plugin manifests carry no licence.
   *Done when* CI builds a namcap-clean package that installs the notices,
   no model downloads before its terms are shown and accepted, and the
   [AUR README](packaging/aur/README.md) lists exactly what is packaged.
   Publishing still waits for the naming review.
3. **Close the Cotypist gaps that fit Badi.** Cotypist 0.22 (researched
   2026-10-20 from its site, help pages and reviews) offers these, and Badi
   does not yet:
   - a suggestion length setting, from 1–2 to 10–15 words (Badi shows up
     to four), each length measured in the Lab before it ships;
   - a shortcut that turns the current app off for a few minutes, and a way
     to send a literal Tab while a suggestion shows;
   - words completed per day and per app (Badi counts since model start,
     content-free, and should keep it that way);
   - global and per-app instructions, and personalized word choice (Badi's
     style samples are stored but not yet read, step 7);
   - larger models where the hardware allows (Gemma 4 E2B/E4B, Qwen3
     4B–30B): a llama.cpp Vulkan or CUDA runtime for machines with a GPU,
     measured with the [bench](evaluation/llm-bench/README.md) and the Lab.

   Badi already matches or beats it on:
   - inline grey text in the field's font, Tab for a word, a key for all
     of it, Escape quiet and a request key;
   - mid-line completion and typo correction where the editor allows it;
   - terminal agent prompts, and on-screen words through accessibility
     rather than screen recording;
   - learned words, battery modes, no telemetry, no account, MIT.

   Left out on purpose: clipboard awareness and screen recording (the
   [context firewall](VISION.md#context-firewall)), sync and telemetry
   (non-goals) and colon emoji (Fcitx owns emoji). Its app list's gaps here
   (Firefox, VS Code's default EditContext, Google Docs, Slack, Discord)
   are under [Remaining surfaces](#remaining-surfaces). *Done when* each
   item ships or is declined in a recorded user decision.
4. **Get the fcitx5-qt surrounding-text bug fixed upstream.** Since
   fcitx5-qt 5.1.16
   ([upstream change](https://github.com/fcitx/fcitx5-qt/commit/bac684ce07d9df78ee37dab1cbdcebfa71bf24ec),
   unchanged on its `master` branch), an input-method update that does not ask
   for the surrounding text clears fcitx5-qt's copy and withdraws
   `SurroundingText`. The addon now keeps observed Qt fields suggesting
   through that echo, but Qt drops Fcitx's delete request against the
   cleared copy, so Omawrite gets no spelling replacement
   ([contract](adapters/fcitx5/README.md#contract)). Removing the text only
   when the query asked for it would fix it upstream; reported as
   [fcitx/fcitx5-qt#91](https://github.com/fcitx/fcitx5-qt/issues/91) on
   2026-10-20. *Done when* a fixed fcitx5-qt
   restores the replacement in Omawrite (two native undos), and a Qt
   Widgets app (Telegram) has been checked.
5. **Finish the installed-flow matrix with someone present.** Still open: a
   real suspend and resume, a VT switch and a Tab right after each, and
   several monitors when a second display is at hand. Restarts, undo, pause,
   composition, stale focus, Ghostty's typed line and fractional scaling
   passed on 2026-10-19 ([matrix](adapters/fcitx5/README.md#tested-with)).
   *Done when* those show zero wrong-field or stale edits and the
   [session helper](adapters/session/README.md) runbook records them.
6. **Check context in a real mail client and the Claude desktop app.** Opt-in
   context shipped ([ADR 0010](docs/decisions/0010-context-finishes-words.md)).
   A disposable mail-style page in Chromium 154 passed every check live on
   2026-10-20 ([observer runbook](adapters/accessibility/README.md#window-words-context-on)),
   and window words reach Zen and Chromium-based apps since `f180653` (VS
   Code live). Still unchecked:
   - the user's own web mail, where only they can open a draft;
   - the Claude desktop app's composer, after a relaunch picks up
     `--force-renderer-accessibility=complete`.
   *Done when* both show a window or learned word finished, and
   `window_chars` appears on their `context.changed` in a `badi debug on`
   trace.
7. **Compare style learning in the Lab.** Feed an explicit `badi style
   export` into a Lab suite's `style` field and compare against no style on
   the user's own held-out drafts, with the `style_fact_conflict` fence and
   native review ([broker runbook](broker/README.md#style-learning-opt-in)).
   Linux apps cannot learn under the settings schema; lifting that is a
   separate privacy decision. *Done when* the comparison shows a per-language
   gain without more harm, before any prompt reads samples.
8. **Verify the [AUR candidate](packaging/aur/README.md) on a fresh Omarchy.**
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
- [ ] Win back the confidence gate's wait: it holds each suggestion until its
      last token (p50 +76 ms, 4 of 92 shown past 550 ms at 15 W). Fewer
      generated tokens, or showing the first word once its own tokens and
      the rest pass, are candidates; each needs the held-out comparison.
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

- [ ] The Lab test "process exit rejects the active request and a later
      request starts a fresh process" (`worker.test.mjs`) failed once in CI
      with "Could not confirm cleanup of the owned model process" (#110, a
      docs-only change) and passed on rerun and in three local runs. If it
      recurs, find the timing that `cleanupRuntime` races after the
      fixture's exit.
- [ ] `adapters/neovim/broker.test.mjs` timed out once in CI waiting for
      `paused` after `badi pause` and an explicit request (#118, a change
      outside Neovim), and passed on rerun and in three local runs. If it
      recurs, find whether the plugin's status or the broker's pause
      reaches it late.
