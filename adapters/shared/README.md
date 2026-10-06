# Editor integrations

Obsidian and Bash own their text: Badi reads and edits through the editor
itself, with native undo. Both check exact app policy before reading anything,
bind each acceptance to the document, caret, revision, fingerprint and an
expiring one-shot broker grant, and retain no typed text. Password and
sensitive fields, foreign composition, selections and stale context are
excluded.

| Integration | Keys | Tested with |
| --- | --- | --- |
| Obsidian | Automatic, or Tab to request; Tab accepts one word, Ctrl/Command+Right all, Escape dismisses. Caret at the end of the note. | Obsidian 1.13.7 / Electron 43, real app and model: English, German and Persian display, acceptance, undo, Escape, broker restart |
| Bash | Ctrl-X then Tab requests and accepts, Ctrl-X then Escape dismisses; never submits ([details](../shell/README.md)) | Bash 5.3.15, Readline 8.3, Ghostty 1.3.1 |

Browser fields use the [IME-parity path](../../README.md#how-badi-edits-chromium-based-apps-and-zen)
instead.

## Install

After the desktop model is provisioned:

```sh
python3 scripts/install-editors.py --vault /path/to/vault --bash
badi app obsidian on
badi app bash on
```

The installer keeps existing plugin enablement and Bash configuration, skips
identical files and backs up what it replaces under
`~/.local/state/badi/editor-backups/<UTC time>/` (newest three kept; restore with
`python3 scripts/badi_install.py restore DIR`). It never changes Obsidian's
restricted mode or edits a note.

- **Obsidian.** Reload it normally. The command palette offers **Badi: Request
  or accept words** and **Badi: Reconnect local model**; language commands pick
  the app locale, English, German or Persian. All notes share one broker
  connection, which reconnects with backoff after a broker restart, always
  with fresh policy. Each accepted word is its own undo step.
- **Bash.** Open a new shell (or source
  `~/.local/lib/badi/editors/shell/badi.bash`); Node must be on PATH.
  `BADI_LANGUAGE=de` or `fa` overrides the locale. The Node bridge exits after
  ten idle minutes, and the next Ctrl-X Tab starts a new one.

Language: an Arabic-script letter nearest the caret selects Persian; otherwise
the app's language hint applies. Persian half-spaces (U+200C) are kept only
between the letters in
[the orthographic fixture](../../protocol/orthographic-joiner-fixtures.json).
`badi debug on` records content-free metadata for 15 minutes.

## Check

```sh
npm run editors:check
npm run editors:integration
node adapters/obsidian/live.mjs --headless --local   # real Obsidian, temporary vault and profile
node adapters/shared/evaluate-writing.mjs            # 20-prefix model smoke, not a quality score
```

The live runner uses the installed Obsidian with its own temporary vault,
profile, plugin and broker; headless Ozone exercises the real editor without
the desktop. Results stay under the ignored `output/` directory. These checks
prove editing behavior, not the usefulness of German or Persian prose.
