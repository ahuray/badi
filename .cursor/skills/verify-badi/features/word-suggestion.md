# Word suggestion

A word suggestion is the grey continuation Badi shows while you type. Tab takes the next word, Ctrl+Right takes all of it, Escape dismisses it, and Ctrl+Shift+Space asks when nothing is showing.

## Sub-features

- `suggest-show` shows a continuation for the text before the caret.
- `suggest-accept-word` inserts only the next word and keeps the rest available.
- `suggest-accept-all` inserts the whole continuation.
- `suggest-dismiss` removes the continuation without changing the text.
- `suggest-request` asks for a continuation when automatic suggestions are quiet.
- `suggest-literal-tab` types a real Tab with Ctrl+Shift+Tab and does not accept.

## How to get to it (user POV)

- In an allowed field on Hyprland with Fcitx, type and pause, or press Ctrl+Shift+Space.
- In Bash, press Ctrl-X, then Tab. Ctrl-X, then Tab again accepts. Ctrl-X, then Escape dismisses.
- In a Neovim prose buffer, pause while typing, then Tab or Ctrl+Right. Leaving Insert mode dismisses.
- In Obsidian, pause, then Tab or Ctrl+Right (Command+Right on that app's chord).
- In a Xournal++ text cell, Tab asks and Tab again accepts a word.
- Press Ctrl+Shift+Tab while a suggestion is showing. The suggestion goes away and the app receives Tab. Where Badi would not take Tab, that chord stays the app's key.

## Driving it with verify-badi

Preconditions:

- Headless: the Bash grant from the feature index is loaded, and `verify-badi.sh doctor` exited 0 for this run id.
- The phrase fixture only completes `thank you`, `looking forward`, `the next step`, and `please`.
- Desktop entry points need a Hyprland and Fcitx session. They are not driven by the command below.

- **Show and accept on the Bash bridge.** This is the Ctrl-X Tab entry, without a terminal. Run `.cursor/skills/verify-badi/verify-badi.sh drive word-suggestion`. The bridge transcript starts with `READY`, then `SUGGEST` and `INSERT` both decode to ` for your time`, then `DONE`. `status-after-word-suggestion.json` has `sessions` 0 and `commits_applied` one higher than the before file. Exit code of the helper is 0. Evidence is `drive-word-suggestion.txt` and the two status files.
- **Grey preview in Bash.** When `adapters/shell/build/badi-preview.so` exists, export the run's `instance.env` and run `python3 adapters/shell/test.py`. The PTY shows the grey continuation, the buffer becomes `thank you for your time`, undo restores the prefix, and the script prints that no line was submitted. Building the preview needs Bash headers. Without them the journey fails on the missing `.so`; do not skip it and cite the bridge instead.
- **Desktop field.** Focus disposable text in an allowed app, type, and accept with Tab or Ctrl+Right. Capture a screenshot or recording that shows the grey text and the field after accept. `badi debug log --json` is the trace when `badi debug on --text` was turned on for that attempt.

## Gotchas

- The bridge proves the authorized insert. It does not paint grey text and it does not press a key. `suggest-accept-word`, Escape, Ctrl+Shift+Space, and Ctrl+Shift+Tab on a real field stay unverified until those entry points are driven.
- `badi length short` does not shorten this fixture. The phrase provider ignores suggestion length, so ` for your time` is still the whole continuation. A shorter grey suggestion is the local model, in [Suggestion length](./suggestion-length.md).
- Tab with no suggestion stays Tab. Do not treat a focused field that ignored Tab as an accepted suggestion.
- The headless instance grants Bash only. Neovim, Obsidian, and Fcitx apps fail closed against it.
- Fcitx `commitString` shows the text was dispatched. It does not show the app applied it, and undo on an IME-parity app may merge with earlier typing.
- A phrase hit is fixture evidence. The local model is a different run.
