# Changelog

What shipped, newest first. This is history only: current behavior lives in
the [README](README.md) and the runbooks, open work in
[future plans](<future plans.md>), and lasting decisions in
[docs/decisions](docs/decisions/). A behavior change adds one line under
Unreleased in the same commit.

## Unreleased

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
  typing changes it, so the model runs during the typing pause.
- Discord gets suggestions: `install-desktop.py --observed-app discord` adds
  the renderer accessibility flag to a user copy of its launcher. The
  `--screen-reader-accessibility` option is gone: it never exposed Discord's
  composer, and at-spi2-core 2.62 no longer has that setting.
- The broker's recorder replies and provider results each go through one
  shared path; behavior is unchanged.

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
