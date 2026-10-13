# Future plans

The only backlog. Goal: the best local replacement for Cotypist on Linux,
Omarchy first, meaning useful suggestions that appear fast, stay quiet and
never edit the wrong place. Delete an item when it is done, and record what is
now true in the [README](README.md) or the affected runbook. A dated
snapshot of the parity work is in
[docs/parity-status.md](docs/parity-status.md).

## Where Badi stands

- **Coverage** is ahead of every Linux alternative found: Chromium-based apps
  and any website, Zen, Omarchy web apps, Telegram, LibreOffice Writer,
  Obsidian and Bash. The closest Omarchy alternative, Oma Tab, replaces the
  input method and ships a 4.8 GB model.
- **Interaction:** type-through, Ctrl+→ next-word acceptance and search-box
  quieting work, and a typed-through remainder returns within about 50 ms with
  no model call ([ADR 0004](docs/decisions/0004-type-through-carries-text-not-authority.md)).
  The inline preview reaches VS Code, iframe editors, Qt apps (Telegram,
  Omawrite) and LibreOffice Writer.
- **Speed and footprint:** the latest installed Chromium sample showed and
  accepted 20/20 warm suggestions, p50 228 ms / p95 284 ms, with zero
  wrong-field edits ([live evidence](adapters/fcitx5/README.md#tested-with)).
  The broker and runtime used 1.43 GB RSS (1.33 GiB) after an independently
  checked loading change; model and prompt are unchanged
  ([runtime](broker/README.md#writing-model-runtime)). These are bounded
  samples on this machine, not universal latency or language-quality claims.
- **GPU:** Vulkan on the Iris Xe is slower per keystroke (13–20 against 19–30
  tokens/s for generation) and faster only for long cold prompts, so Badi stays
  on the CPU.
- **Quality:** the installed Qwen3-1.7B is weak on independent sets, and no
  ~350M candidate passed the English/German/Persian gates. Persian keeps the
  8-token automatic budget: on 2026-10-12, 21 tokens missed 550 ms on all 13
  development cases. Mid-sentence stays off: the same day's 41-case Lab
  comparison did not justify a rejoin change.

Standing user decisions:
- IME-parity for Chromium-based apps, Zen and LibreOffice Writer
  ([ADR 0003](docs/decisions/0003-ime-parity-append-only.md));
- promote a quality fix only without a per-language harm increase;
- keep the model resident;
- explicit requests get about 1.2 s;
- Tab accepts all and Ctrl+→ one word on Fcitx fields;
- distribution starts with an AUR package; publication and an Omarchy proposal
  still need approval after naming and licence review;
- Discord uses desktop-wide AT-SPI `ScreenReaderEnabled`, with recorded prior
  settings and unchanged Badi app/site permissions (confirmed 2026-10-13).

## Interaction

- [ ] Live type-through trials in Zen and Brave Origin; tab mode needs the
      user's running browser. On 2026-10-12 a disposable Brave Origin page
      showed grey "for the opportunity" after "Thank you". The addon counted
      the following space as type-through and the field gained that space, but
      the broker made another model call. After installing that wait, the same
      trial still made a second model call, and the wait did not run: the
      field already contained the space when the request went out. A fresh Zen
      profile exposed only the window frame, so its page was not typed in.
- [ ] The manual native path has no dialog or width signal, so its file
      dialogs and very narrow fields are not automatic-quiet the way observed
      fields are ([observer runbook](adapters/accessibility/README.md)).
      Escape quiet is a broker setting (`escape_quiet_seconds`), covered in
      the [broker runbook](broker/README.md).
- [x] Mid-sentence suggestions see only the text before the caret and keep
      what rejoins the next word. Naming the following line in the prompt
      (`following_text`, lab only) does not change that. On 2026-10-12, the
      41 English development cases, warm prefix, production budget: today's
      rejoin showed 2 suggestions and 1 reference match, with no deadline
      misses; the following-line prompt showed 1 suggestion and 1 reference
      match, and missed 550 ms on 34 of 41 because that line is not cached.
- [ ] Measure overflow frequency per app; fractional-scale and multi-monitor
      live trials remain open. The implemented placement and fallback behavior
      is in the [preview contract](adapters/accessibility/README.md#caret-geometry-and-preview).
      Investigate explicit requests in horizontally scrolled, very narrow
      inputs: the installed Chromium trial at 100 CSS px did not establish
      coverage ([tested boundary](adapters/fcitx5/README.md#tested-with)).
- [ ] Confirm live that the grey preview uses the host font in Telegram.
      A disposable Chromium textarea (2026-10-12, build `7957f16`) set to
      JetBrainsMono NF at 20px reports `family-name` JetBrainsMono NF and
      `size` 15pt, and the grey continuation is painted on that same line in
      the same monospaced face. An earlier default textarea reported 10pt and
      a 16×35 glyph box, with no weight. Omawrite 0.5.0 reports no font
      attributes and a zero glyph box, so its overlay keeps the caret-size
      fallback.
- [x] The headless Obsidian live lane closes Electron without a process-group
      signal. On 2026-10-12, `node adapters/obsidian/live.mjs --headless`
      passed on Obsidian 1.13.7-2 and left no new core dump. Tab is not
      pressed while a suggestion is absent, because that indents the line.

## Speed and footprint

Exit criteria: warm visible p50 ≤ 250 ms and p95 ≤ 500 ms with misses reported,
resident memory about 1.3 GB, and no quality regression in the Lab.

- [x] Warm first grey word meets the sample thresholds on this machine.
      On build `206e782`
      (2026-10-12, five disposable prefixes) Omawrite showed grey text at
      260–351 ms, median 278 ms. A disposable Chromium page on the same build
      showed 246–361 ms, median 248 ms. Cold probes of those prefixes took
      312–550 ms, median 458 ms, so the live path is already the warm cache.
      On this CPU a private runtime with the production flags (4 threads,
      batch 16, context 512; the live process already loads
      `libggml-cpu-alderlake.so`) reached four words in 206–248 ms on those
      prefixes. Six threads produced the same greedy text and were not faster,
      so the thread cap stays 4. A keystroke in a field already under
      observation re-reads that field without a second inspection. On build
      `7957f16` (2026-10-12, the same five prefixes, predictions left on)
      Omawrite showed grey text at 275–293 ms, median 283 ms. Warm broker
      probes of those prefixes were 190–248 ms, median 192 ms. Skipping the
      extra inspection did not move the median. The gap above 250 ms is still
      the generation after that read. A continuation now shows its first
      finished word and lengthens up to four. On build `2630744`
      (2026-10-12, the same five prefixes) Omawrite's first grey text was
      221–240 ms, median 225 ms. The same build in a disposable Chromium
      textarea, after one warmup, showed 248, 236, 235 and 236 ms, and one
      sample did not appear within 2.8 s. That miss was the dismissed text
      staying declined after the field was cleared; build `5faafcf` ends the
      decline when the text changes. On that build, the same five prefixes
      repeated four times (predictions left on): Omawrite 20/20 shown,
      209–250 ms, middle 220–223, 19th of 20 is 244; Chromium 20/20 shown,
      227–371 ms, middle 246–249, 19th of 20 is 314. No miss in these 40.
      This is one machine and these prefixes.
- [x] A known field starts the model on the key and keeps the suggestion
      hidden until the 120 ms typing pause. The next key drops the held
      suggestion. On the sample above, that pause is inside the generation
      wait rather than added after it.
- [x] Q4_0 does not replace the pinned Q4_K_M. On 2026-10-12, llama.cpp
      b10726 with repack and flash attention left on auto, 4 threads and
      batch 16, compared the installed Qwen3-1.7B-Q4_K_M with Bartowski's
      Q4_0 (`c470091d31c4ada174ee5c2547daa020e930593cbca5ca8ca385ce8ff59a2fdf`,
      file type Q4_0). They are not two quants of one source file. In six
      alternating 8-token decodes, with the broker loaded, Q4_0 was a little
      faster in four pairs and the rate fell from about 26 t/s to about 14
      as the load rose. One paired run had Q4_0 slower (11.5 against 13.2
      t/s). Raw greedy text, conversation mode off, differed on four of the
      five disposable prefixes; Q4_0 continued "Thank" with " youngster".
      Production stays on Q4_K_M.
- [x] Speculative decoding does not bring the first word forward on this
      machine. A private llama-server with the production flags, the healing
      grammar and the five disposable prefixes (2026-10-12) kept the pinned
      1.7B's greedy text under both trials. N-gram speculation was no faster
      than the baseline run beside it. A Qwen3-0.6B Q4_K_M draft
      (`9acfc1e001311f34b4252001b626f2e466d592a42065f66571bff3790d4e1b14`,
      two tokens ahead) accepted between 0.13 and 0.88 of its drafts and
      still took about 0.5–1.8 s to the first word, against about 0.15–0.47 s
      with no draft. Production does not load a draft model.
- [ ] Fewer generated tokens, only after a held-out quality comparison.
- [ ] Qualify Qwen3 0.6B and Gemma 4 E2B (with its multi-token drafter) in the
      Lab. Defer hybrid-attention models until llama.cpp reuses their caches.
- [ ] Battery scheduling is in place ([session helper](adapters/session/README.md#battery)):
      on battery or in the power-saver profile an automatic request waits
      300 ms, so keys typed closer together never start the model. On
      2026-10-13 (this laptop on battery with power-saver, release build,
      [paced trial](evaluation/power/battery-trial.py): 752 disposable keys per
      mode at 150 ms per key, 450 ms after words and 1.5 s after sentences)
      `quiet` made 122 model calls against 586, used 182 against 464 runtime
      CPU seconds, showed 313 against 257 suggestions with 211 against 190
      matching the reference, and had visible p95 latency of 550 against
      525 ms. The same trial later that day with the laptop charging in the
      performance profile (the trial still reports battery, one repeat, 376
      keys per mode) shows the cost on a fast CPU: 45 against 220 calls and 50
      against 148 runtime CPU seconds, but 27 against 31 distinct reference
      words covered and p95 456 against 211 ms. Still open: the balanced
      profile on battery, real typing intervals, energy from a privileged
      power counter rather than runtime CPU time, and a reviewed count of
      fully accepted additions.
- [ ] Use [Linux pressure signals](https://www.kernel.org/doc/html/latest/accounting/psi.html)
      to reduce speculative requests while the machine is contended. PSI files
      are available here; `CPUWeight=1000`, `MemoryHigh=3G` and resident-model
      protection already exist. Measure foreground responsiveness and deadline
      misses before changing those controls; do not add automatic model eviction.
- [ ] Lower-memory hardware: cold start, RAM, sustained latency, power and
      crash recovery.

## One-command install on Omarchy

Exit criteria: on a fresh Omarchy install, one menu action covers install,
suggestion, acceptance, update and uninstall, leaving nothing behind.

- [ ] Verify install/update/removal of the [AUR candidate](packaging/aur/README.md)
      on fresh Omarchy before publication. It
      packages the addon, broker, observer and user units; writing assets are
      a separate pinned, hash-verified download.
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
- [ ] An installed-flow matrix with zero wrong-field or stale edits: service
      restart, suspend and resume, undo, pause, composition, stale focus,
      fractional scaling and several monitors. The
      [session helper](adapters/session/README.md) now holds every grant across
      sleep and session switches; its fake-bus lane covers resume, a restarted
      logind and a broker restart, but a real suspend, VT switch and a Tab
      pressed right after resume have not been tried on the installed desktop.

## Better suggestions

The user confirmed on 2026-10-12: window context and learning stay on the
device, with global and per-app opt-outs. Language work prioritizes English,
German, French and Spanish; existing Persian behavior keeps its regression
gates. The [context firewall](VISION.md#context-firewall) owns this contract.

- [ ] Same-window context is extracted but not used. The observer's
      [`window_context()`](adapters/accessibility/README.md#same-window-context-lab-only)
      reads the nearest heading and preceding paragraph of the field's own web
      document and never another field, a password, a hidden node or another
      frame; it is unit-tested on fake trees only. A live check was not run:
      the desktop was locked, and a headless Chromium on a private bus exposed
      no field. On 2026-10-13 the
      [development set](evaluation/writing/lab/examples/window-context-development.json)
      (24 cases written for this, so not independent; `healed`, 8 tokens,
      pinned 1.7B on battery with power-saver) gave the expected next words
      in 15 of 16 informative cases with the context and 4 of 16 without,
      with a 3 s diagnostic budget. In 3 of the 8 controls the context pulled
      the continuation onto the page's topic ("I have attached the chapter
      three"), and several additions answered for the user ("November? Yes,
      it's available"). At 550 ms both arms missed the deadline on 23 or 24
      cold prompts. Before any production use: independent cases with
      full-addition review, a fence like `style_fact_conflict` for context
      facts, a cache-stable prompt layout that meets 550 ms, and only then
      the global and per-app controls, wire field and cancellation on focus
      changes.
- [ ] Style learning stores but does not use. `badi style on` keeps the
      sentence written before each accepted suggestion on sites that allow
      learning, sealed with a keyring key; revocation, expiry, key loss,
      forgetting and the locked keyring are tested
      ([broker runbook](broker/README.md#style-learning-opt-in)). Next: feed an
      explicit `badi style export` into a Lab suite's `style` field and compare
      against no style on the user's own held-out drafts, with the
      `style_fact_conflict` fence and native review, before any prompt reads
      samples. Linux apps cannot learn under the settings schema
      (`linux_app` subjects force `learn: block`); lifting that is a separate
      privacy decision. Not yet tried with the installed desktop keyring.
- [ ] Qualify French and Spanish before enabling them. Gather independently
      reviewed confirmation prefixes and full-addition reviews for both,
      alongside the existing English, German and Persian confirmation gate.

Promoting a model needs a new hypothesis, fresh independent cases, native
Persian review where results are ambiguous and the full Lab qualification.

- [ ] Complete qualification on the user-approved public confirmation set:
      UD r2.16 test, 100 English and 40 each German, French, Spanish and Persian
      prefixes. It is frozen privately at
      `output/writing/confirmation-ud-r2.16-2026-10-12/suite.json`, SHA-256
      `72a69d10d9ec6b261373a6f382b203b512290eb3bff2b71c3b3ee3efd769c1cc`,
      with source hashes, licenses and approval. Select and freeze the candidate
      on development data before inference. The set's first use is the frozen
      runtime-loading equivalence comparison, followed by the frozen field-language
      selector check. Neither is a full model or language quality qualification. Report
      usefulness, abstention, errors, useful words, keystroke savings and latency
      misses separately; review full additions before model or prompt promotion.
- [ ] Improve short-fragment language coverage without reducing precision.
      The conservative field selector keeps the adapter hint for short or
      uncertain text; explicit per-app overrides remain available
      ([contract](broker/README.md#writing-model-runtime)). French and Spanish
      remain Lab-only until their generation quality passes independently.
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
- [ ] Measure Cotypist parity with the same tasks on a named Cotypist version on
      a Mac.

## Audit follow-ups

From the 2026-10-11 audit. These are not part of the installed behavior.

- [ ] Browser event cost on large pages (`daemon.py` focus handling and
      `field.py` serialization).
- [x] The grey preview follows a scroll reported on the armed field or a
      scroll-pane ancestor, when the text offset is unchanged. A scroll the
      toolkit reports on neither leaves the overlay where it was.
- [x] Document the limits of sensitive-field detection. A password role or
      password input type is refused. Labels, placeholders, and HTML
      autocomplete hints are not.
- [ ] An ADR retiring protocol v1.
- [x] The settings panel polls only while its window is open. A closed
      window does not ask the broker. The bar mark updates when pause is
      toggled from the bar, and the next time the window opens.
- [x] Bash's Readline timer runs only while a grey preview is held
      (`preview.c`). An idle prompt does not wake every 100 ms.
- [ ] Split `broker/src/settings.rs`, `broker/src/engine/tests.rs` and
      `adapters/fcitx5/addon.cpp`.

## Remaining surfaces

- [ ] Browser rich editors: a caret inside a list or quote, EditContext editors
      (VS Code's default, CodeMirror on recent Chromium), multi-paragraph fields
      in Zen (Gecko sends only the caret's paragraph) and Firefox.
- [ ] Discord with desktop-wide accessibility; a composer trial needs a private channel, since
      typing shows an indicator.
- [ ] Grok Bot composer trial in the signed-in app after a relaunch with
      `grok-bot-flags.conf`.
- [ ] More native toolkits through measured integrations.
- [ ] LibreOffice Writer spelling extension: a live trial in the interactive
      Writer window under Wayland with Fcitx beside it (infobar look and focus,
      Alt+F7, one Ctrl+Z) and a model-backed correction of an ambiguous word.
      The [extension](adapters/libreoffice/README.md) passes its headless gates
      (LibreOffice 26.8, phrase fixture); the grey preview stays IME-parity.
      Mid-paragraph corrections need a broker contract beyond end-of-field
      replacement.
- [ ] Neovim: try the real model in an interactive terminal Neovim (display,
      Tab, Ctrl+→, undo, and whether Ctrl+Shift+Space arrives), then add
      spelling replacement through `text_replacement` with stale-replacement
      and undo tests. The plugin and its headless phrase-broker lanes are in
      the [Neovim runbook](adapters/neovim/README.md).
- [ ] Extend shell support using
      [Zsh ZLE buffers and `POSTDISPLAY`](https://zsh.sourceforge.io/Doc/Release/Zsh-Line-Editor.html)
      and [Fish's command-line API](https://fishshell.com/docs/current/cmds/commandline.html).
      Zsh has a documented preview surface; Fish buffer access still needs a
      measured preview integration compatible with its own autosuggestions.
      Neither shell is installed here. Acceptance must only edit the buffer,
      preserve native undo and never execute a generated command.
- [ ] Terminal agent prompts (Claude Code, Codex): terminals send no
      surrounding text, so each needs its own channel or stays unsupported.
- [ ] Later: other Linux desktops, a standard tray and personal snippets.
      Emoji stays with Fcitx: QuickPhrase and the Emoji module, not a Badi
      colon shortcode.
