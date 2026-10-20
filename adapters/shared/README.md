# Editor integrations

Obsidian, Bash and Neovim own their text: Badi reads and edits through the
editor itself, with native undo. Both check exact app policy before reading anything,
bind each acceptance to the document, caret, revision, fingerprint and an
expiring one-shot broker grant, and retain no typed text. Password and
sensitive fields, foreign composition, selections and stale context are
excluded.

Obsidian and Neovim negotiate a 15-second reading window with the updated broker.
Expiry still clears the preview and prevents acceptance; it is never renewed
just because the preview remains visible. Bash keeps its existing lease and
four-second local display timer. Update the broker before the editor plugin.

| Integration | Keys | Tested with |
| --- | --- | --- |
| Obsidian | Automatic, or Ctrl+Shift+Space to request. Tab accepts one word only while a suggestion is visible; otherwise it stays Tab. Ctrl/Command+Right accepts all, Escape dismisses. Mid-note, only what fits before the next word. | Obsidian 1.14.4 / Electron 43, real app and model (2026-10-20, all eight live-runner checks): English, German and Persian display, Tab, Ctrl+Right, undo, Escape, broker restart |
| Bash | Ctrl-X then Tab requests and accepts, Ctrl-X then Escape dismisses; never submits ([details](../shell/README.md)) | Bash 5.3.15, Readline 8.3, Ghostty 1.3.1 |
| Neovim | Prose buffers, automatic after a 120 ms pause, or Ctrl+Shift+Space. Tab accepts one word and Ctrl+Right all of it, only while a suggestion is visible; leaving Insert mode dismisses ([details](../neovim/README.md)) | Neovim 0.12.5 headless, scripted bridge and real phrase broker; live with the real model under this workstation's LazyVim through the installer's lazy.nvim spec, in foot (0.12.5, 2026-10-17) and Ghostty (0.12.6, 2026-10-20), including Tab, Ctrl+Right and Ctrl+Shift+Space |
| LibreOffice Writer (spelling) | A finished misspelled word shows its correction; Alt+F7 shows one for the word at the caret; Alt+F7 again replaces it in one undo step, Escape dismisses ([details](../libreoffice/README.md)) | LibreOffice 26.8 headless, phrase fixture; the interactive Wayland window live (Alt+F7, replace, one Ctrl+Z) |

Browser fields use the [IME-parity path](../../README.md#how-badi-edits-chromium-based-apps-and-zen)
instead.

## Install

After the desktop model is provisioned:

```sh
python3 scripts/install-editors.py --vault /path/to/vault --bash --neovim
badi app obsidian on
badi app bash on
badi app nvim on
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
- **Neovim.** Restart it. The plugin is a start package under
  `stdpath('data')/site/pack/badi`; when lazy.nvim is detected the installer
  also writes its own new `lua/plugins/badi.lua` spec for that package.
  Node must be on PATH, and its bridge exits after ten idle minutes like
  Bash's ([details](../neovim/README.md)).

A suggestion appears whole, once the broker's confidence gate has judged
it ([suggestion limits](../../broker/README.md#suggestion-limits)), and
acceptance binds to the version shown.

Language: an Arabic-script letter nearest the caret selects Persian; otherwise
the app's language hint applies. Persian half-spaces (U+200C) are kept only
between the letters in
[the orthographic fixture](../../protocol/orthographic-joiner-fixtures.json).
`badi debug on` records content-free metadata for 15 minutes and appends each
event to the shared [debug trace](../../README.md#debug-trace).

## Check

```sh
npm run editors:check
npm run editors:integration
node adapters/obsidian/live.mjs --headless --local   # real Obsidian, temporary vault and profile
```

The live runner uses the installed Obsidian with its own temporary vault,
profile, plugin and broker; headless Ozone exercises the real editor without
the desktop. It ends Obsidian by signalling only the app process, never its
process group, so Electron's crash helper does not fire; on 2026-10-20 it
passed all eight checks on Obsidian 1.14.4-2. It never presses Tab
without a suggestion, because Tab then indents the line. Results stay under
the ignored `output/` directory. These checks prove editing behavior, not the
usefulness of German or Persian prose.
