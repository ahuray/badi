# Changelog

What shipped, newest first. This is history only: current behavior lives in
the [README](README.md) and the runbooks, open work in
[future plans](<future plans.md>), and lasting decisions in
[docs/decisions](docs/decisions/). A behavior change adds one line under
Unreleased in the same commit.

## Unreleased

- A quality and portability pass from a full audit, each fix with a
  regression test. The screen-share hold no longer lifts when `pw-dump` is
  missing (only a `pw-dump` that ran and could not reach PipeWire settles
  the screen as unshared), a stalled `pw-dump` is restarted, compact
  one-line dumps are read, and the Debian package now depends on
  `pipewire-bin`. Obsidian accepts a spelling correction mid-note, with
  text after the caret. A missing preview extent or a client that vanishes
  before its credentials are read can no longer crash the field observer.
- The broker sends a commit grant only to an adapter that can report a
  landed commit, an invalid landed status consumes its one-shot grant and
  retires the pre-commit context, and a desktop hold is reported to the
  session helper even when the outcome fence fails. Failed connections
  and writers leave a content-free journal line.
- Other desktops, named sooner: the installers say Badi needs a Wayland
  session (not X11) and warn when `graphical-session.target` is inactive,
  an interrupted first install no longer leaves the broker disabled at
  login, a generic Hyprland session is told the grey preview's layer rule
  in both config syntaxes, `badi doctor` names a systemd-skipped broker
  and a pre-5.1.23 Fcitx on Hyprland or Sway, the Fcitx build floor is the
  tested 5.1.7, and the AUR candidate no longer forces Hyprland and
  Quickshell on every Arch user (they are optdepends now).
- The Omarchy panel updater enables the plugin whenever Omarchy reports
  it disabled — a run that once stopped after writing files finishes the
  job — and writes its receipt only after the restarted panel answers.
- CI now runs the LibreOffice Writer integration lane and the keyring
  style-key delivery test (which fails instead of skipping without a
  keyring), builds the Fcitx addon with warnings as errors on Arch,
  Debian and Ubuntu, and checks out full history for the agent-rules gate,
  which now counts only a changelog line under the newest heading.
- Rich editors on websites (mail composers, ProseMirror, Lexical, Quill)
  suggest after a space again: the observer reads a contenteditable's
  no-break spaces as Chromium sends them, and a block-scoped snapshot agrees
  with the paragraphs below the caret.
- A suggestion survives a window title change, such as an editor's autosave
  or a terminal's running command, instead of ending as it appears.
- The grey preview ends instead of staying behind when its field scrolls in
  a pane that reports no scroll, and a hidden preview is never drawn again.
- Zen Twilight (`zen-twilight`) gets suggestions and corrections like Zen.
- Bash draws an accepted suggestion once instead of repeating the prompt
  mid-row.
- Suggestions in observed apps appear sooner after you stop typing: a field
  is inspected on the keystroke itself, and again at once when your own
  typing changes it, so the model runs during the typing pause. A field
  that reports your edits is not re-read first, which only started a model
  run its report then cancelled.
- Discord gets suggestions: `install-desktop.py --observed-app discord` adds
  the renderer accessibility flag to a user copy of its launcher. The
  `--screen-reader-accessibility` option is gone: it never exposed Discord's
  composer, and at-spi2-core 2.62 no longer has that setting.
- The broker's recorder replies and provider results, and the observer's
  snapshot and preview drawing, are each split into shared or named steps;
  behavior is unchanged.
- The session helper looks up the keyring's style key asynchronously, so a
  slow Secret Service can no longer delay a sleep or screen-share report.
- In Zen, a field the observer first declines because a click reached Badi
  before Zen moved its focus gets one more look on the next key, so its first
  suggestions are not held until typing pauses.
- A next word you turned down, with Escape or by typing other letters twice,
  is not offered again after the same words in that app or site, until you
  ask for it there and take it. Leaving Neovim's Insert mode turns nothing
  down.
- Apps and sites where you seldom take a suggestion get fewer, surer ones,
  from the usage counts Badi already keeps; it never gets bolder there.
- The grey preview draws the word Tab takes a step brighter than the rest,
  in observed apps, Obsidian and Neovim, as Fcitx's panel already did.
- In Fcitx apps, Ctrl+Shift+Space on a shown suggestion shows another one
  that starts with a different word, from the model's own next choices;
  after three, presses cycle through them.
- Obsidian and Neovim show alternatives the same way: Ctrl+Shift+Space on a
  shown suggestion replaces it with one that starts with another word.
- Alternatives right after a finished word (`attached the`) start the next
  word; before, they could only end that word another way, so presses
  showed the same suggestion again.
- The desktop installer lists every writing model that fits the device,
  the recommended one first, with its size, screen result and licence, so
  someone who declines a model's terms can pick another (Qwen3 is
  Apache-2.0). Before, declining the recommended model's terms ended the
  install with nothing to choose. A model download now prints a line even
  without a terminal, and one when its hash matches.
- The panel's Writing model row names the licence with the model in use
  (`Qwen3 0.6B, Apache-2.0 · Recommended: Gemma 3 1B`) instead of after the
  recommended model's name.

## 0.2.0 (2026-10-24)

The first release on a fresh history: the development before it, pull
requests #1 to #146, is squashed into this one commit. Its last step was a
full code review, whose fixes are part of this release:

- the Fcitx addon no longer reads a freed input context when a window
  closes during a revision, keeps Ctrl+Shift+Tab as the app's key where
  Badi would not take Tab, and ends only the refused request on a broker
  `error` frame instead of the whole connection;
- the grey preview follows a scroll instead of ending the suggestion, and
  window words keep text with emoji or soft hyphens;
- the settings panel accepts `screen_share` (it used to disable every write
  after `badi screen-share suggest`), and a language chip changes only its
  own language;
- a suggestion never waits on a disk sync for learned words, and authority
  changes reach clients in order;
- the session helper reports sleep before it asks the keyring for the style
  key, and Neovim reads no buffer text while Badi is paused or off there;
- protocol v1, the dormant partial-suggestion path, the context-word logit
  bias and the Lab's failed experiments and separate spelling engine are
  removed, and the runbooks keep the current contract and tested-with
  results instead of dated narrative.

What 0.2.0 contains:

- **Suggestions.** Grey continuations from a local model the broker picks
  for the device (Gemma 3 1B here), shown only when every token is likely
  enough, in English and German by default and French, Spanish and
  Portuguese on request. Tab takes a word, Ctrl+Right the rest, Escape
  dismisses, Ctrl+Shift+Space asks and Ctrl+Shift+Tab types a real Tab;
  type-through keeps a suggestion while you type it
  ([README](README.md#keys), [ADR 0004](docs/decisions/0004-type-through-carries-text-not-authority.md)).
- **Where.** A cooperative Fcitx 5 addon for native fields; the
  accessibility observer plus Fcitx for Chromium-based apps, Zen, VS Code,
  Cursor and Writer ([ADR 0003](docs/decisions/0003-ime-parity-append-only.md),
  [ADR 0006](docs/decisions/0006-grey-preview-is-an-overlay.md));
  Obsidian, Bash Readline and Neovim through their own editors; the typed
  line in Ghostty and foot by exact opt-in rule
  ([ADR 0009](docs/decisions/0009-terminals-use-the-typed-line.md))
  ([where it works](README.md#where-it-works)).
- **Spelling.** One explicit correction per finished word, replaced in the
  editor or, in append-only apps, deleted and committed with a read-back
  ([ADR 0011](docs/decisions/0011-checked-corrections-in-append-only-apps.md));
  Writer corrects through its own extension.
- **Selection revision.** Ctrl+Shift+Space on one selected line asks a local
  instruction model (Qwen3 1.7B, started on demand) for a grammar fix, and
  again for a refined wording; Tab applies it as one edit
  ([ADR 0012](docs/decisions/0012-explicit-revision-of-one-selection.md)).
- **Opt-ins.** Context words from the window and words learned per app
  ([ADR 0010](docs/decisions/0010-context-finishes-words.md)), style samples
  sealed with the keyring's key, and content-free usage statistics.
- **Desktop signals.** Sleep, session switches, screen sharing and battery
  hold or quiet suggestions and never grant
  ([ADR 0005](docs/decisions/0005-desktop-signals-withhold-authority.md));
  pause, snooze and portal shortcuts.
- **Control.** The `badi` command and `badi doctor`, the explicit, expiring
  debug trace ([ADR 0008](docs/decisions/0008-debug-trace-may-hold-text-on-request.md)),
  and the Omarchy bar mark and settings panel.
- **Packaging.** User-local installers that back up what they replace, an
  Arch package candidate and Debian 13 and Ubuntu 24.04 packages, all
  unpublished. The broker and its pinned llama.cpp b11840 runtime run
  sandboxed on protocol v2 over a same-user Unix socket
  ([ADR 0001](docs/decisions/0001-same-uid-trust-boundary.md)).
- **Evaluation.** The Prediction Lab for paired model and prompt runs, the
  local LLM bench and the paced battery trial.
