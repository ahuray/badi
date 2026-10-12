# Future plans

The only backlog. Goal: the best local replacement for Cotypist on Linux,
Omarchy first, meaning useful suggestions that appear fast, stay quiet and
never edit the wrong place. Delete an item when it is done, and record what is
now true in the [README](README.md) or the affected runbook.

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
- **Speed:** before inspection moved to the keystroke, a new suggestion was
  visible 0.35–0.39 s after the last key in Chromium: about 120 ms of pause,
  25 ms of inspection and policy, and 210 ms of model and display. On
  2026-10-12, Omawrite 0.5.0 with keystroke inspection (build `f657f7c`) showed
  grey text at 266, 329, 347, 350 and 392 ms (median 347 ms). Reusing the
  session for a keystroke in the same field (build `206e782`) showed 260, 267,
  278, 322 and 351 ms (median 278 ms). The same build in a disposable
  Chromium page showed 246, 247, 248, 253 and 361 ms (median 248 ms). These
  are five prefixes each, not a p95. Cold `badictl probe` calls for those same
  prefixes, which do not reuse a keystroke cache, took 312, 405, 458, 461 and
  550 ms (median 458). The live figures are the warm path. The KV cache is
  already reused between keystrokes. Before the timer accuracy fix
  (2026-10-06), Fcitx's 250 ms timer slack made Chromium 0.40–0.62 s.
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
- Tab accepts all and Ctrl+→ one word on Fcitx fields.

## Open decisions for the user

1. **Context beyond the field.** Cotypist reads on-screen context, but the
   [context firewall](VISION.md#context-firewall) forbids screen scraping. The
   compatible option is an opt-in, per-app read of the same window's
   accessibility text, such as the message being answered. Either way it needs
   a VISION amendment.
2. **Learning:** opt-in style learning from accepted suggestions and chosen
   samples, stored encrypted locally with one-step deletion, or none.
3. **Distribution:** an AUR package first, then a proposal to Omarchy. Both are
   publication and need a naming and licence review.
4. **Discord:** a launcher that re-adds its accessibility switch before every
   start, or the desktop-wide AT-SPI `ScreenReaderEnabled` setting.

## Interaction

- [ ] Live type-through trials in Zen and Brave Origin; tab mode needs the
      user's running browser.
- [ ] No automatic suggestions in file dialogs or very narrow fields, and a
      setting for how long Escape keeps a field quiet (today: until the text
      changes).
- [x] Mid-sentence suggestions see only the text before the caret and keep
      what rejoins the next word. Naming the following line in the prompt
      (`following_text`, lab only) does not change that. On 2026-10-12, the
      41 English development cases, warm prefix, production budget: today's
      rejoin showed 2 suggestions and 1 reference match, with no deadline
      misses; the following-line prompt showed 1 suggestion and 1 reference
      match, and missed 550 ms on 34 of 41 because that line is not cached.
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
- [ ] Battery mode: a longer pause and a smaller model on battery.
- [ ] Lower-memory hardware: cold start, RAM, sustained latency, power and
      crash recovery.

## One-command install on Omarchy

Exit criteria: on a fresh Omarchy install, one menu action covers install,
suggestion, acceptance, update and uninstall, leaving nothing behind.

- [ ] A PKGBUILD with the addon in `/usr/lib/fcitx5`, the broker, observer and
      user units; the model downloads in a separate pinned, hash-verified step.
- [ ] Omarchy integration in the style of Voxtype's: a first-run invitation
      (`omarchy-done ensure`, `omarchy-notification-send --exec`), a
      floating-terminal installer, menu entries for settings and removal,
      keybindings and theme colours for the preview.
- [ ] Support [omarchy#12139](https://github.com/omacom/omarchy/pull/12139)
      (`--enable-wayland-ime` by default); keep the per-app flags until it lands.
- [ ] Signed updates. The broker service is already sandboxed: `PrivateNetwork=yes`,
      a Unix-only runtime socket, and a read-only model directory.
- [ ] Retire the [Fcitx 5.1.22 frontend backport](packaging/fcitx5-wayland-compat/README.md)
      once Omarchy ships Fcitx 5.1.23.
- [ ] An installed-flow matrix with zero wrong-field or stale edits: service
      restart, suspend and resume, undo, pause, composition, stale focus,
      fractional scaling and several monitors.

## Better suggestions

Promoting a model needs a new hypothesis, fresh independent cases, native
Persian review where results are ambiguous and the full Lab qualification.

- [ ] Freeze an independent confirmation set you provide or approve, not
      assistant-written text: at least 100 English, 40 German and 40 Persian
      prefixes. Report usefulness, abstention, errors, useful words, keystroke
      savings and latency misses separately before any model or prompt promotion.
- [ ] Detect the language from the field's text, with a per-app override,
      instead of the keyboard layout.
- [ ] Global and per-app instructions, and a strength setting that trades
      frequency for confidence.
- [ ] Content-free statistics in the panel: shown, accepted, words saved.
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

- [ ] The observer spawns `qs ipc` on every inspection
      (`adapters/accessibility/desktop.py`).
- [ ] Browser event cost on large pages (`daemon.py` focus handling and
      `field.py` serialization).
- [x] The grey preview follows a scroll reported on the armed field or a
      scroll-pane ancestor, when the text offset is unchanged. A scroll the
      toolkit reports on neither leaves the overlay where it was.
- [x] Document the limits of sensitive-field detection. A password role or
      password input type is refused. Labels, placeholders, and HTML
      autocomplete hints are not.
- [ ] An ADR retiring protocol v1.
- [ ] Enforce `authority.ack`, or delete it if nothing consumes it.
- [x] The settings panel polls only while its window is open. A closed
      window does not ask the broker. The bar mark updates when pause is
      toggled from the bar, and the next time the window opens.
- [ ] Bash wakes every 100 ms after the first preview (`preview.c`).
- [ ] `badi doctor` in human-readable text as well as JSON.
- [ ] Stop passing the settings document on the `badi` command line
      (`badi-desktop.py`).
- [ ] Split `broker/src/settings.rs`, `broker/src/engine/tests.rs` and
      `adapters/fcitx5/addon.cpp`.

## Remaining surfaces

- [ ] Browser rich editors: a caret inside a list or quote, EditContext editors
      (VS Code's default, CodeMirror on recent Chromium), multi-paragraph fields
      in Zen (Gecko sends only the caret's paragraph) and Firefox.
- [ ] Discord (decision 4); a composer trial needs a private channel, since
      typing shows an indicator.
- [ ] Grok Bot composer trial in the signed-in app after a relaunch with
      `grok-bot-flags.conf`.
- [ ] More native toolkits through measured integrations. LibreOffice
      paragraphs expose `EditableText`, a possible route to exact replacement.
- [ ] Fish, Zsh and Neovim through their own buffers, never by running a
      generated command.
- [ ] Terminal agent prompts (Claude Code, Codex): terminals send no
      surrounding text, so each needs its own channel or stays unsupported.
- [ ] Later: other Linux desktops, a standard tray and personal snippets.
      Emoji stays with Fcitx: QuickPhrase and the Emoji module, not a Badi
      colon shortcode.
