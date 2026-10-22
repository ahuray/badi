# Spelling

Spelling offers one correction for a finished word. You accept it explicitly. Bash, Obsidian, and native exact fields replace the word. Append-only apps do not delete a typo. Writer replaces through its own undo.

## Sub-features

- `spell-show` shows one correction for the finished word.
- `spell-accept` replaces that word and leaves one undo that restores it.
- `spell-ambiguous` does not pretend a single dictionary hit when several words fit.
- `spell-writer` shows Writer's infobar and replaces on a second Alt+F7.

## How to get to it (user POV)

- In Ghostty or foot, a correction looks like `teh → the` and Tab accepts it. The line is not submitted. In Bash the correction is `original → corrected`, and Ctrl-X, then Tab accepts it. The line is not submitted.
- In Obsidian, the misspelled word is replaced and one undo restores it.
- In Writer, finish the word and pause for the infobar, or press Alt+F7, then Alt+F7 again to replace. Ctrl+Z restores the typo.
- `badi spelling` lists languages. English is on by default.

## Driving it with verify-badi

Preconditions:

- Headless: doctor has exited 0 for this run, and the Bash grant is the seeded one. The bridge negotiates replacement.
- The phrase fixture corrects an unambiguous English word. `We need an adress ` becomes `address `, replacing `adress `.
- `teh` is ambiguous for that fixture, so the fixture must not offer a replacement. The real model's `teh → the` needs the installed model and a terminal or an allowed field.

- **Unambiguous Bash correction.** Run `.cursor/skills/verify-badi/verify-badi.sh drive spelling`. `SUGGEST` and `INSERT` each carry `address ` and the replaced suffix `adress `. `commits_applied` increases by one and `sessions` returns to 0. Helper exit code is 0.
- **Ambiguous word stays a correction only for the real model.** Do not drive `teh` against the phrase broker and call a missing replacement a product failure. Report it as the fixture's rule.
- **Writer.** With LibreOffice installed, the extension enabled, and `badi app libreoffice-writer on`, type a disposable typo, press Alt+F7 twice, read the paragraph back, then Ctrl+Z. A VM without Writer cannot run this. `npm run libreoffice:check` is the source contract, not the infobar.

## Gotchas

- Append-only apps (Chromium, Zen, VS Code, Cursor) do not delete the typo. A suggestion there is not a spelling replacement.
- Omawrite with fcitx5-qt 5.1.16 does not get a spelling replacement. Do not claim one from a general Fcitx pass.
- The phrase path proves the replacement payload. It does not prove the grey `original → corrected` label or a native undo stack.
- Writer's grey continuations stay append-only. Only the extension replaces text.
