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
  VS Code and iframe fields get the inline preview.
- **Speed:** a new suggestion is visible 0.35–0.39 s after the last key in
  Chromium. That breaks down as:
  - the 120 ms typing pause;
  - about 25 ms for inspection and policy;
  - about 210 ms for the model and display.
  The KV cache is already reused between keystrokes. Before the timer
  accuracy fix (2026-10-06), Fcitx's 250 ms timer slack made it 0.40–0.62 s.
- **GPU:** Vulkan on the Iris Xe is slower per keystroke (13–20 against 19–30
  tokens/s for generation) and faster only for long cold prompts, so Badi stays
  on the CPU.
- **Quality:** the installed Qwen3-1.7B is weak on independent sets, Persian
  often misses its 550 ms budget, and no ~350M candidate passed the
  English/German/Persian gates.

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
- [ ] An inline preview where caret geometry is missing: Qt (Telegram,
      Omawrite) and LibreOffice report no character extents, and Chromium
      reports none in an iframe editor body (TinyMCE). Anchor ghost text to the
      field's extents instead of the Fcitx panel, at the host's font size.

## Speed and footprint

Exit criteria: warm visible p50 ≤ 250 ms and p95 ≤ 500 ms with misses reported,
resident memory about 1.3 GB, and no quality regression in the Lab.

- [ ] Start inference before the 120 ms pause ends, cancelling on a mismatch.
- [ ] Speculative decoding with a small draft model, or fewer generated tokens.
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
- [ ] Sandbox the broker (`PrivateNetwork=yes`, read-only model directory) so
      the absence of network access is provable; signed updates.
- [ ] Retire the [Fcitx 5.1.22 frontend backport](packaging/fcitx5-wayland-compat/README.md)
      once Omarchy ships Fcitx 5.1.23.
- [ ] An installed-flow matrix with zero wrong-field or stale edits: service
      restart, suspend and resume, undo, pause, composition, stale focus,
      fractional scaling and several monitors.

## Better suggestions

Promoting a model needs a new hypothesis, fresh independent cases, native
Persian review where results are ambiguous and the full Lab qualification.

- [ ] Freeze an independent multilingual confirmation set of at least 100
      prefixes; report usefulness, abstention, errors, useful words, keystroke
      savings and latency misses separately.
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
- [ ] Spelling correction in native fields and Obsidian with exact replacement
      ranges. The 40-case German/Persian run failed (2 and 4 useful, one
      unwanted change each); improve names and nearby words first.
- [ ] Calibrate token confidence against reviewed full additions per language.
- [ ] Measure Cotypist parity with the same tasks on a named Cotypist version on
      a Mac.

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
- [ ] Later: other Linux desktops, a standard tray, emoji shortcodes and
      personal snippets.
