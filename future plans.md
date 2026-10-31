# Future plans

The only backlog and the plan for what comes next. Goal: the best local
replacement for Cotypist on Linux, Omarchy first, meaning useful suggestions
that appear fast, stay quiet and never edit the wrong place.

When an item is done, delete it and record what is now true in the
[README](README.md) or the affected runbook; [CHANGELOG.md](CHANGELOG.md)
keeps the history of what shipped. Evidence lives in the runbooks; this file
links to it. A tested app version is not a claim about Linux in general.

## Where Badi stands (2026-10-26)

- **Coverage** is ahead of every Linux alternative found: Chromium-based apps
  and any website, Zen and Zen Twilight, Omarchy web apps in Chromium or Brave, Telegram, Discord,
  LibreOffice Writer
  (spelling as you type, Alt+F7), Obsidian, Bash, Neovim (live under LazyVim
  in foot and Ghostty) and, by an opt-in rule, the typed line in Ghostty and
  foot, Omarchy's default terminal
  ([ADR 0009](docs/decisions/0009-terminals-use-the-typed-line.md)); the
  [app sweep](adapters/fcitx5/README.md#tested-with) of 2026-10-20 passed in
  Brave Origin, a web app, Zen, Writer, Obsidian, Neovim, Ghostty and VS
  Code. Omawrite and other observed Qt fields suggest again despite a
  fcitx5-qt 5.1.16 bug, but cannot replace a typo until it is fixed
  (step 2). German, French, Spanish, Italian, Dutch and Polish spelling are
  opt-in (`badi spelling`). The closest Omarchy alternative, Oma Tab,
  replaces the input method and ships a 4.8 GB model; of the alternatives
  found on 2026-10-24, only ibus-llm-assist (IBus) reaches a desktop Badi
  does not run on yet, GNOME.
- **Interaction:** type-through returns a typed-through remainder in about
  50 ms with no model call, also when a browser re-admits the field
  ([ADR 0004](docs/decisions/0004-type-through-carries-text-not-authority.md));
  Tab takes a word and Ctrl+→ the rest; a half-typed word is finished, also
  in front of the next word; `badi length short` shows one or
  two words instead of up to four; search boxes stay quiet; Ctrl+Shift+Space
  on a selected line checks its grammar or refines it, shown with marked
  changes and applied by Tab
  ([ADR 0012](docs/decisions/0012-explicit-revision-of-one-selection.md));
  typos in Chromium-based apps are corrected after checks
  ([ADR 0011](docs/decisions/0011-checked-corrections-in-append-only-apps.md)); the inline
  preview ([ADR 0006](docs/decisions/0006-grey-preview-is-an-overlay.md))
  reaches VS Code, iframe editors, Qt apps and Writer. Ctrl+Shift+Space on
  a shown suggestion cycles alternatives that start with another word, in
  Fcitx apps, Obsidian and Neovim.
- **Desktop signals:** the installed [session helper](adapters/session/README.md)
  holds grants across sleep and session switches and while the screen is
  shared (a portal screencast in PipeWire), and defers automatic requests on
  battery
  ([ADR 0005](docs/decisions/0005-desktop-signals-withhold-authority.md)).
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
  Revision with Qwen3 1.7B fixed 53 of 75 development errors across the five
  languages (Spanish 7/15, weakest) and left 19 of 20 correct inputs alone
  ([Lab](evaluation/writing/README.md#selection-revision)).
- **Security:** a review on 2026-10-23 found nothing critical or high; the
  broker and runtime run sandboxed (no network, read-only home but Badi's
  two directories, no core dumps), and password-labelled fields are never
  read ([ADR 0001](docs/decisions/0001-same-uid-trust-boundary.md)). The
  pinned llama.cpp runtime is b11840, past the `/completion` double-free fix
  (b11393).
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
- **Distribution:** an Arch package candidate (step 6) and a
  [Debian 13 and Ubuntu 24.04 package](packaging/debian/README.md), which CI
  builds, lints and installs in containers (2026-10-23); neither is
  published, and no GNOME or KDE session has run Badi (step 7). Without an
  installed writing model, the desktop installer lists the models that fit
  the device and lets the user pick one.
- **Code health:** a full review before the 0.2.0 fresh start (2026-10-24)
  fixed every correctness finding it confirmed, with regression tests, and
  removed dormant paths and failed experiments; what it could not check live
  is under [Engineering upkeep](#engineering-upkeep).

Standing user decisions:
- IME-parity for Chromium-based apps, Zen and LibreOffice Writer
  ([ADR 0003](docs/decisions/0003-ime-parity-append-only.md)), with typo
  corrections on by default in all but Writer: checked before and read back
  after, at the accepted risk that a page moving focus splits one
  (2026-10-23, [ADR 0011](docs/decisions/0011-checked-corrections-in-append-only-apps.md));
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
- the panel opens compact and offers European languages only (English,
  German, French, Spanish, Portuguese, Italian, Dutch, Polish), chosen once
  at first run; Persian is off by default and terminal-only (2026-10-23);
- suggestions show one or two words (`short`) or up to four (`normal`, the
  default); up to eight was removed after it doubled harmful additions with
  Gemma 3 1B, and a longer length returns only with a model that writes it
  without more harm (2026-10-21, [comparison](broker/README.md#suggestion-limits));
- distribution starts with an AUR package; publication and an Omarchy proposal
  still need approval after the naming review (the licence review is done);
- Discord gets the renderer flag through a user copy of its launcher
  (`--observed-app discord`), as other Chromium-based apps get it through
  their flags files (2026-10-24).

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

1. **Close the Cotypist gaps that fit Badi.** Cotypist (0.22 researched
   2026-10-20; its 2026.x releases and pricing page rechecked 2026-10-24) is
   still Mac-only, defaults to Gemma 4 E2B, and offers these, which Badi does
   not yet:
   - global and per-app instructions, and personalized word choice (Badi's
     style samples are stored but not yet read, step 5). Instructions wait
     for the larger-model work below: paired Lab probes found neither the
     raw-continuation prompt nor an instructed variant follows them on the
     current models;
   - a numbered list of alternative words in the editor clients too. Fcitx
     apps list up to four other first words on Ctrl+Shift+Space, picked by
     digit or click ([ADR 0014](docs/decisions/0014-numbered-alternative-words.md));
     Obsidian and Neovim still cycle one at a time, as synonyms do
     everywhere ([ADR 0013](docs/decisions/0013-synonyms-for-one-short-selection.md));
   - larger models where the hardware allows (Gemma 4 E2B/E4B, Qwen3
     4B–30B): a llama.cpp Vulkan or CUDA runtime for machines with a GPU,
     measured with the [bench](evaluation/llm-bench/README.md) and the Lab.

   Badi already matches or beats it on:
   - words saved per day and per app or site, as counts only
     ([`badi stats`](README.md#usage-statistics));
   - inline grey text in the field's font with the word Tab takes drawn
     brighter, Tab for a word, a key for all of it, Escape quiet and a
     request key;
   - no repeat of a next word turned down at the same spot, and fewer,
     surer suggestions where they are seldom taken;
   - a length setting for one or two words (`badi length short`) or up to
     four;
   - mid-line completion, and typo correction in browsers and Electron apps
     too ([ADR 0011](docs/decisions/0011-checked-corrections-in-append-only-apps.md));
   - terminal agent prompts, and on-screen words through accessibility
     rather than screen recording;
   - learned words, battery modes, no telemetry, no account, MIT;
   - a snooze for the focused app or site (`badi snooze`, a portal
     shortcut) and Ctrl+Shift+Tab for a real Tab while a suggestion shows;
   - grammar checks and refined wording of a selection, which Cotypist says
     it does not do.

   Left out on purpose: clipboard awareness and screen recording (the
   [context firewall](VISION.md#context-firewall)), sync and telemetry
   (non-goals), colon emoji (Fcitx owns emoji) and suggestions longer than
   four words (standing user decision above). Its app list's gaps here
   (Firefox, VS Code's default EditContext, Google Docs, Slack)
   are under [Remaining surfaces](#remaining-surfaces). *Done when* each
   item ships or is declined in a recorded user decision.
2. **Get the fcitx5-qt surrounding-text bug fixed upstream.** Since
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
   2026-10-20; on 2026-10-24 it had no reply and `master` no fix, so a fix
   would ship in 5.1.17 at the earliest. Debian forky and sid ship 5.1.16
   too; Debian 13 (5.1.9) and Ubuntu 24.04 to 26.10 (5.1.4–5.1.15) predate
   the change. Offering the patch upstream is publication and needs the
   user's go-ahead. *Done when* a fixed fcitx5-qt
   restores the replacement in Omawrite (two native undos), and a Qt
   Widgets app (Telegram) has been checked.
3. **Finish the installed-flow matrix with someone present.** Still open: a
   real suspend and resume, a VT switch and a Tab right after each, and
   several monitors when a second display is at hand. Restarts, undo, pause,
   composition, stale focus, Ghostty's typed line and fractional scaling
   passed on 2026-10-19 ([matrix](adapters/fcitx5/README.md#tested-with)).
   *Done when* those show zero wrong-field or stale edits and the
   [session helper](adapters/session/README.md) runbook records them.
4. **Check context in a real mail client and the Claude desktop app.** Opt-in
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
5. **Compare style learning in the Lab.** Feed an explicit `badi style
   export` into a Lab suite's `style` field and compare against no style on
   the user's own held-out drafts, with the `style_fact_conflict` fence and
   native review ([broker runbook](broker/README.md#style-learning-opt-in)).
   Linux apps cannot learn under the settings schema; lifting that is a
   separate privacy decision. *Done when* the comparison shows a per-language
   gain without more harm, before any prompt reads samples.
6. **Verify the [AUR candidate](packaging/aur/README.md) on a fresh Omarchy.**
   CI builds and namcap-checks the package on every change, and the licence
   review is done: [THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt) ships
   with it and every model download shows its terms first. *Done when*
   install, update and removal on a fresh Omarchy leave nothing behind, the
   panel's model-terms row has been tried live, and publication then waits
   only for the naming review.
7. **Run Badi in real GNOME and KDE Plasma sessions.** The
   [Debian and Ubuntu package](packaging/debian/README.md) and the
   user-local installer pass in Debian 13 and Ubuntu 24.04 containers, but no
   graphical session has run them. Still open:
   - field observer backends outside Hyprland, so Chromium-based apps, Zen,
     VS Code and Writer's grey preview suggest there; the observer unit's
     Hyprland condition goes with them. The observer needs the focused
     window's process and app id (checked 2026-10-24):
     - KWin offers `org_kde_plasma_window_management` (app id, and process
       id since version 8; Fcitx itself uses it) or a KWin script reading
       `workspace.activeWindow`, so Plasma comes first;
     - GNOME's `org.gnome.Shell.Introspect` gives no process id and answers
       only the portal backends, so GNOME needs a Shell extension of Badi's
       own that the user installs;
     - AT-SPI's own process id is a debugging aid, and its application name
       is not the app id;
   - in a VM with GNOME 48 (Debian 13) and Plasma: `im-config -n fcitx5`
     or Plasma's Virtual Keyboard starting Fcitx with the environment.d
     addon directory, the user units, a manual Tab request in a GTK and a Qt
     app, the editors and Writer, and package install, update and purge.
     On GNOME, Fcitx stands in for IBus (Mutter speaks only text-input-v3)
     and needs the Kimpanel extension for its panel; Plasma launches Fcitx
     through input-method-v1. A reported GNOME 50.0–50.3 loss of forwarded
     keys ([fcitx5#1692](https://github.com/fcitx/fcitx5/issues/1692),
     unverified) would affect Ubuntu 26.04.
   *Done when* those pass with zero wrong-field edits and the runbook's
   tested-with matrix records them.

## Later, by area

### Interaction

- [ ] The manual native path has no dialog or width signal, so its file
      dialogs and very narrow fields are not automatic-quiet the way observed
      fields are ([observer runbook](adapters/accessibility/README.md)).
      Escape quiet is a broker setting (`escape_quiet_seconds`), covered in
      the [broker runbook](broker/README.md).
- [ ] Measure overflow frequency per app; the placement and fallback are in
      the [preview contract](adapters/accessibility/README.md#caret-geometry-and-preview).
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
- [ ] Revise a selection in rich editors (contenteditable with several
      blocks) in Chromium-based apps: the observer refuses them now
      (`unsupported_selection`), since their blocks serialize differently for
      Fcitx ([ADR 0012](docs/decisions/0012-explicit-revision-of-one-selection.md),
      [observer](adapters/accessibility/README.md#selection-for-a-revision)).
- [ ] A Revisions row in the settings panel: whether the revision model is
      installed, its size and an install button (`badi revise install`).
- [ ] Pair-run stronger revision models in the Lab's revision lane before
      offering one; each download needs the user's go-ahead. Candidates
      (Apache-2.0, checked 2026-10-24): Qwen3.5-2B (1.28 GB at Q4, about
      Qwen3 1.7B's memory, multilingual MMLU 56.9 against 46.7; thinks by
      default, so thinking must be off), Qwen3.5-4B (2.74 GB, 76.1),
      Qwen3-4B-Instruct-2507 (2.50 GB, no thinking) and Ministral 3 3B
      Instruct (2.15 GB). No published grammar-correction benchmark covers
      Spanish, French or Portuguese, so the lane's 75 errors are the measure.

### Speed and footprint

Exit criteria: warm visible p50 ≤ 250 ms and p95 ≤ 500 ms with misses
reported, resident memory about 1.3 GB, and no quality regression in the Lab.
Measured alternatives that were not adopted are in the
[runtime runbook](broker/README.md#writing-model-runtime).

- [ ] Win back the confidence gate's wait: it holds each suggestion until its
      last token (p50 +76 ms, 4 of 92 shown past 550 ms at 15 W). Fewer
      generated tokens, or showing the first word once its own tokens and
      the rest pass, are candidates; each needs a held-out quality comparison.
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
- [ ] [omarchy#12139](https://github.com/omacom/omarchy/pull/12139)
      (`--enable-wayland-ime` by default) closed unmerged on 2026-10-15:
      Chromium has used text-input-v3 without the flag since 137. Check live
      whether Badi's per-app flags can go for current Chromium and Brave;
      Electron apps on an older Chromium may still need them.
- [ ] Re-pin the panel to Omarchy 4.5 once it is tagged (still `v4.5.0-rc1`
      on 2026-10-24; `quattro` is 47 commits past it, including the
      notification panel's removal, omarchy#16430, which Badi does not use:
      the screen-share hold reads PipeWire). Run the host lane first.
- [ ] Leftovers of the 2026-10-20 Omarchy 4.0.6 audit: the panel's Fcitx
      theme takes its font size only from Ghostty's config (`fcitx-theme.py`),
      each bar instance registers the `badi-writing` IPC target (several
      monitors may duplicate it), and no non-US Fcitx keyboard group has been
      tried.
- [ ] Signed updates. The broker service is already sandboxed: no network,
      a read-only home except its own two directories, only `badi/` in the
      runtime directory, a read-only model directory and no core dumps
      ([ADR 0001](docs/decisions/0001-same-uid-trust-boundary.md)).

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
      would close that gap. The Lab has no separate spelling engine: measure
      through `production_boundary` with the dictionary languages on.

- [ ] Qualify French, Spanish and Portuguese before turning them on by default
      (they are opt-in with `badi suggest LANG on`). Gather independently
      reviewed confirmation prefixes and full-addition reviews for each,
      alongside the existing English, German and Persian confirmation gate.
- [ ] Improve short-fragment language coverage without reducing precision.
      The conservative field selector keeps the adapter hint for short or
      uncertain text; explicit per-app overrides remain available
      ([contract](broker/README.md#writing-model-runtime)).
- [ ] A strength setting that trades frequency for confidence (instructions
      are in step 1).
- [ ] Cut the latency of the combined instruction/boundary mode (18/22 useful
      in development, 2.8 s cold median) and confirm it independently. Its
      template is Qwen3's, so the Lab runs it only on Qwen3 1.7B.
- [ ] Keep facts in style examples from overriding the draft; confirm the Lab's
      `style_fact_conflict` fence across languages.
- [ ] Tell an intended partial word from a completed one before contextual
      lookup: `mire` → `Mirella` must complete, `marin` must not become
      `Marinella`. Dictionary membership is the wrong splitter.
- [ ] Spelling replacement undo in Telegram and Xournal++. Omawrite 0.5.0
      (2026-10-12, disposable `teh ` → `the `) needed two Ctrl+Z steps: the
      first removed the correction and the second restored the typo. Obsidian
      already replaces the suffix in one undo step.
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
- [ ] Screen the base models no Lab run has seen yet (checked 2026-10-24; no
      family released a new 0.5–2B text base model in the four months
      before): EuroMoE-2.6B-A0.6B (Apache-2.0, 0.6B active, 1.62 GB at Q4)
      and LFM2.5-2.6B-Base (LFM Open License, a hybrid that re-reads the
      prompt after a backspace, 1.67 GB); Ministral 3 3B Base only for
      larger machines. Tiny Aya (non-commercial), MiniCPM5 (English and
      Chinese) and Granite SWASH (English) are out.
- [ ] Condition on the text after the caret. Oma Tab feeds it to a
      fill-in-the-middle model; Gemma 3 has no such tokens, and Badi uses the
      after-text only to shape a suggestion. Measure mid-sentence usefulness
      with a model that has them before choosing.
- [ ] Measure Cotypist parity with the same tasks on a named Cotypist version on
      a Mac.

### Remaining surfaces

- [ ] Browser rich editors: a caret inside a list or quote, EditContext editors
      (VS Code's default, CodeMirror on recent Chromium), multi-paragraph fields
      in Zen (Gecko sends only the caret's paragraph) and Firefox.
- [ ] Zen: after Tab accepts a word, the rest shows only after the next key.
      Gecko keeps the surrounding text it gives Fcitx at the pre-commit text
      until the next key, while the observer already reads the committed word,
      so the agreement check refuses (`observer_context_mismatch`, 27 against
      35 characters on 2026-10-24). Accepting an observed text that is exactly
      Fcitx's text plus the committed string would show it, but relaxes the
      agreement rule ([ADR 0003](docs/decisions/0003-ime-parity-append-only.md))
      and needs the user's decision first.
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
- [ ] Terminals other than Ghostty and foot (kitty, Alacritty) on the typed-line
      path ([ADR 0009](docs/decisions/0009-terminals-use-the-typed-line.md)),
      each after a live trial of its app id and `commitString`.
- [ ] Panel rows for per-app language overrides and style learning, which
      are terminal-only today.
- [ ] Later: a standard tray (other desktops are step 7) and personal snippets.
      Emoji stays with Fcitx: QuickPhrase and the Emoji module, not a Badi
      colon shortcode.

### Engineering upkeep

- [ ] Live-check what the 2026-10-24 trial could not reach, each with
      `badi debug on --text` and disposable text: a broker `error` frame
      ending only its request, one language chip on the panel's settings page
      (the trial machine still shows the first-run language question), rows
      and the site draft surviving a poll, and Writer superseding a slow
      spelling request.
- [ ] Broker `error` frames carry no session, and the Fcitx addon's request
      ids repeat across sessions, so the addon retires a refused id in every
      binding holding it (this only withdraws). Echo the session coordinates
      in `error`, or make ids per session, so the match is exact. A refused
      `session.open` also leaves the binding thinking it is open until the
      authority change that follows.
- [ ] Structure left by the review, each only when its file is next touched:
      the Fcitx `Binding` (about 50 fields, one expiry timer for notices,
      suggestions and revisions) as small reset-able parts and one retire
      function for authority change and disconnect; a shared stdio and
      idle-exit module for the Bash and Neovim bridges; and a tested
      `SettingsModel.js` with one status ladder for the panel (new files
      need the installers' and the PKGBUILD's lists, which a test now
      compares).

- [ ] [ADR 0007](docs/decisions/0007-retire-protocol-v1.md) step 4: once
      every known install has started a v2-only broker (0.2.0), delete the
      on-disk v1 settings migration, `badi.settings.v1.schema.json` and the
      panel lifecycle and schema tests that exercise it.
- [ ] Broker structure left by the review: collapse `complete`/`propose`
      once the Lab calls `propose_outcome`, and move the remaining inline
      tests into `<module>/tests.rs` when those files are next touched.

- [ ] The Lab test "process exit rejects the active request and a later
      request starts a fresh process" (`worker.test.mjs`) failed once in CI
      with "Could not confirm cleanup of the owned model process" (#110, a
      docs-only change) and passed on rerun and in three local runs. It
      failed the same way once locally on 2026-10-23 inside a full `npm run
      check` (the screen-share hold, no Lab change) and passed twice alone.
      Find the timing that `cleanupRuntime` races after the fixture's exit.
- [ ] `adapters/neovim/broker.test.mjs` timed out once in CI waiting for
      `paused` after `badi pause` and an explicit request (#118, a change
      outside Neovim), and passed on rerun and in three local runs. If it
      recurs, find whether the plugin's status or the broker's pause
      reaches it late. On #123 (no Neovim change) `plugin.test.mjs:38`
      failed once the same way: after the pause the virtual text was still
      `thank you`, not the suggestion; it passed on rerun and five local runs.
- [ ] `adapters/shell/test-preview.py` failed once in CI in
      `test_the_display_timer_runs_only_while_a_preview_is_held`: the PTY
      never emitted `HOOK:`, only a cleared line (#133, no shell change).
      It passed three local runs. If it recurs, find whether the Readline
      hook prints before the test starts reading.
