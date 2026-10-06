# Future plans

The only active backlog. Goal: the best local replacement for Cotypist on
Linux, Omarchy first: useful suggestions that appear fast, stay quiet and never
edit the wrong place. Tested coverage is in the [README](README.md); Git
history keeps completed work and earlier experiment logs.

Standing user decisions (2026-09-26): IME-parity for Chromium-based apps and
Zen ([ADR 0003](docs/decisions/0003-ime-parity-append-only.md)), promote a
quality fix only without a per-language harm increase, keep the model resident,
and give explicit requests about 1.2 s.

## Gap to Cotypist (research, 2026-10-06)

Based on Cotypist's macOS documentation and changelog, the Linux and Omarchy
landscape, and Badi's measured state. There has been no side-by-side run on a
Mac yet.

- **Coverage is ahead of every Linux alternative found.** It includes
  Chromium-based apps, Zen, rich editors on any website, Omarchy web apps,
  Telegram, LibreOffice Writer, Obsidian and Bash. All of them keep exact
  binding, and native undo is preserved wherever the app supports it. The
  closest Omarchy alternative, Oma Tab, replaces the input method and ships a
  4.8 GB model.
- **The interaction is behind.**
  - Cotypist keeps a suggestion while you type through it, accepts word by
    word and stays quiet in search boxes and small fields. Where it cannot
    draw ghost text inline, it shows a mirror window.
  - Badi drops the suggestion on every key and requests a new one, although
    [VISION](VISION.md) promises type-through. It accepts word by word only in
    Obsidian.
  - Where caret geometry is missing (Qt, LibreOffice, iframes, VS Code 1.140),
    Badi falls back to the Fcitx panel.
- **Speed and footprint.** A warm suggestion becomes visible in 0.37–0.65 s:
  the model takes 215–280 ms, after a 120 ms typing pause. `llama-server`
  holds about 2.0 GB on the CPU while the Intel GPU sits idle.
- **Quality.** The installed Qwen3-1.7B scores weakly on independent sets, and
  Persian misses its time budget. The language comes from the keyboard layout
  rather than the text.
- **Personal touch.** Cotypist learns from your writing and takes global and
  per-app instructions and languages. Badi has none of these.
- **Installation.** Badi installs through user-local scripts. Omarchy already
  has a pattern for first-class tools, which Voxtype follows: a first-run
  invitation, a menu installer and a removal path. Omarchy's default Chromium
  flags also lack `--enable-wayland-ime`; upstream
  [omarchy#12139](https://github.com/omacom/omarchy/pull/12139) adds it and is
  still open.

## Decisions for the user

1. **Accept keys.** In Cotypist, Tab accepts the next word and a second key
   accepts the rest. Badi's suggestions are at most four words.
   Recommendation:
   - Tab keeps accepting the whole suggestion.
   - A word key (Ctrl+Right, only while a suggestion is shown) accepts the
     next word.
   - The panel can swap the two keys.
2. **Context beyond the field.** Cotypist reads on-screen context, but Badi's
   [context firewall](VISION.md#context-firewall) forbids screen scraping. The
   compatible option is an opt-in, per-app read of the same window's
   accessibility text, such as the message being answered. Either way this
   needs a VISION amendment.
3. **Learning.** Either opt-in style learning or none. Learning would draw only
   on accepted suggestions and samples the user chooses, stored encrypted on
   this machine, with deletion in one step.
4. **Distribution.** An AUR package first, then a proposal to Omarchy. Both
   count as publication and need the naming and licence review.
5. **Discord** (Phase 5): a launcher that re-adds the flag, or the
   desktop-wide AT-SPI switch.

## Phase 1: the interaction

Exit criteria: in Brave, Zen, Telegram, VS Code and Obsidian, typing through a
suggestion keeps it without a model call, and both word and whole acceptance
work. The installed-flow matrix must record zero wrong-field or stale edits.

- [x] Type-through (2026-10-06,
      [ADR 0004](docs/decisions/0004-type-through-carries-text-not-authority.md)).
      - The broker carries the last shown suggestion as text only and answers
        the next admitted request that adds a typed prefix of it with the
        remainder, with no model call.
      - The Fcitx addon re-inspects 30 ms after such a key instead of 120 ms.
      - Verified with engine tests, the real-broker Fcitx smoke and the
        private-bus observed addon lane.
- [x] Word acceptance on Fcitx fields: Ctrl+Right over Badi's own live
      candidate commits `accept_word`, and the rest carries. Tab still accepts
      the whole suggestion: a second Tab before the remainder returns would
      move focus (ADR 0004). This is the recommended answer to decision 1.
- [ ] Live trials of type-through and Ctrl+Right in Brave, Zen, Telegram and
      VS Code. They need the installed broker and addon; the installer run was
      not authorized in the 2026-10-06 session.
- [ ] Quiet where it doesn't help.
      - [x] Search, email and URL inputs, combo boxes and ARIA search boxes
        get no suggestions (search boxes added 2026-10-06).
      - File dialogs and narrow fields.
      - A setting for how long Escape keeps a field quiet. Today it lasts until
        the text changes.
- [ ] A preview wherever caret geometry is missing.
      - Anchor a ghost line to the field's extents instead of falling back to
        the Fcitx panel (Qt, LibreOffice, iframes).
      - VS Code 1.140's hidden textarea is caret-wide, so calibration rejects
        the glyph; a new drawing bound needs recorded-extent tests.
      - Draw the preview text at the host's font size.

## Phase 2: faster and lighter

Exit criteria on this laptop:
- warm visible p50 ≤ 250 ms and p95 ≤ 500 ms, with misses reported;
- resident memory back to about 1.3 GB;
- no quality regression in the Lab.

- [x] Reuse the prompt and KV cache across keystrokes. The runtime already
      runs with `cache_prompt` and context checkpoints.
- [ ] Start inference before the 120 ms pause ends, and cancel it on a
      mismatch.
- [x] Measure GPU offload. Setup: Vulkan on the Iris Xe (Mesa 26.2.2), with the
      official Vulkan build of llama.cpp b10726, against the CPU build (4
      threads, batch 16), 2026-10-06.
      - The GPU is faster only for long cold prompts, and only with a large
        batch that would weaken cancellation: 192-token prompts ran at about
        400 tokens/s on the GPU against 75–107 on the CPU.
      - It is slower on every keystroke's request, which is mostly generation:
        13–20 against 19–30 tokens/s. A 16-token prompt plus 6 generated
        tokens ran at 30 against 33–53 tokens/s.
      - Badi stays on the CPU. Six or eight threads were within noise of four.
- [ ] Generation is the warm bottleneck at about 33–50 ms per token. Evaluate
      speculative decoding with a small draft model, and fewer generated tokens
      per request.
- [ ] Qualify smaller and newer models in the Lab: Qwen3 0.6B, Gemma 4 E2B with
      its multi-token drafter, and Qwen3 4B on the GPU. Defer hybrid-attention
      models until llama.cpp reuses their caches.
- [ ] Battery mode as a setting: a longer pause and a smaller model on battery.
- [ ] Lower-memory hardware: cold start, RAM, sustained latency, power and
      crash recovery.

## Phase 3: one-command install on Omarchy

Exit criteria: on a fresh Omarchy install, one menu action covers install,
suggestion, acceptance, update and uninstall, and uninstall leaves no files,
services or flags behind.

- [ ] A package (PKGBUILD) that installs the addon in `/usr/lib/fcitx5`, plus
      the broker, the observer and the user units. The hardware-selected model
      downloads in a separate step that is pinned and hash-verified.
- [ ] Omarchy integration following Voxtype:
      - a first-run invitation (`omarchy-done ensure` and
        `omarchy-notification-send --exec`);
      - a floating-terminal installer with a hardware probe;
      - menu entries for settings and removal;
      - keybindings;
      - theme colours for the preview.
- [ ] Chromium-family flags: support omarchy#12139, and keep the installer's
      per-browser flags until it lands.
- [ ] Broker sandbox: `PrivateNetwork=yes` and a read-only model directory, so
      that the absence of network access can be proven. Signed updates.
- [ ] Retire the Fcitx 5.1.22 compatibility frontend once Arch ships the
      upstream fix.
- [ ] Installed-flow matrix with zero wrong-field or stale edits, covering:
      - service restart;
      - suspend and resume;
      - native undo and pause;
      - composition and stale focus;
      - fractional scaling and several monitors.
- [ ] Licensing and naming review before any release. No publication or Git
      history changes without the user's request.

## Phase 4: better suggestions

The Qwen3-1.7B baseline is weak on independent sets, and Persian often misses
the 550 ms budget. No candidate of about 350M parameters passed the
English/German/Persian gates (2026-09-10). Promoting a model needs a new
hypothesis, fresh independent cases, native Persian review where results are
ambiguous, and the full Lab qualification.

- [ ] Freeze a new independent multilingual confirmation set of at least 100
      prefixes.
      - Report usefulness, abstention, errors, useful words and keystroke
        savings separately.
      - Report misses against the Phase 2 latency targets.
- [ ] Detect the language from the field's text rather than the keyboard
      layout, with a per-app override.
- [ ] Global and per-app instructions, and a strength setting that trades
      frequency for confidence. Opt-in learning follows decision 3.
- [ ] Content-free statistics in the panel: suggestions shown and accepted,
      and words saved.
- [ ] Reduce the latency of the combined instruction/boundary mode and confirm
      it independently. It scored 18/22 useful in development but had a 2.8 s
      cold median.
- [ ] Keep facts from style examples from overriding the current draft. The
      Lab's `style_fact_conflict` fence still needs a multilingual
      confirmation.
- [ ] Tell an intended partial word from a completed one before automatic
      contextual lookup: `mire` → `Mirella` must complete, but `marin` must not
      become `Marinella`. Dictionary membership is the wrong way to split them.
- [ ] Spelling correction in native fields and Obsidian, with exact replacement
      ranges and explicit acceptance. The 40-case German/Persian run failed: 2
      and 4 useful, with one unwanted change each. Improve the handling of
      names and nearby words on independent cases first.
- [ ] Calibrate token confidence against reviewed full additions, per language
      and boundary category.
- [ ] Measure Cotypist parity: run the same tasks on a named Cotypist version
      on a Mac. Source research cannot establish parity.

## Phase 5: the remaining surfaces

- [ ] Browser rich editors. Chromium already handles multi-block `<p>`/`<div>`
      editors, headings, inline marks, iframes and shadow roots (2026-10-06).
      Remaining:
      - a caret inside a list or a quote;
      - EditContext editors (VS Code's default editor, CodeMirror on recent
        Chromium);
      - multi-paragraph fields in Zen, because Gecko sends only the caret's
        paragraph;
      - Firefox.
- [ ] Discord.
      - Listing `force-renderer-accessibility` in its `settings.json`
        `chromiumSwitches` exposes the web tree for one launch only; the web
        client then resets the list (decision 5).
      - A composer trial needs a private channel, because typing shows an
        indicator.
- [ ] Grok Bot composer trial in the signed-in app, after a relaunch with
      `grok-bot-flags.conf`. A fresh profile shows only its sign-in screen.
- [ ] Native coverage beyond Omawrite, Xournal++, Telegram and LibreOffice
      Writer, through measured toolkit integrations. LibreOffice paragraphs
      expose `EditableText`, a possible route to exact replacement.
- [ ] Fish, Zsh and Neovim through their own buffers; never accept by running
      a generated command.
- [ ] Prompts of terminal agents such as Claude Code and Codex. Terminals send
      no surrounding text, so these need a channel per program or stay
      unsupported.

## Later

- [ ] Other Linux desktops and a standard tray integration.
- [ ] Emoji shortcodes and personal snippets.

Update this file in place; record durable decisions in
[docs/decisions](docs/decisions/).
