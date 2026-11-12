# Editor integrations

Obsidian, Bash and Neovim own their text: Badi reads and edits through the
editor itself, with native undo. Each checks exact app policy before reading
anything, inserts a grant only when it is the text on screen, binds each
acceptance to the document, caret, revision, fingerprint and an expiring
one-shot broker grant, and retains no typed text. Password and
sensitive fields, foreign composition and stale context are excluded.
Continuations require a collapsed selection; Neovim additionally negotiates
explicit [selection revision](../neovim/README.md#selection-revision), as does
[Obsidian](#obsidian-selection-revision).

Obsidian and Neovim negotiate a 15-second reading window with the updated broker.
Expiry still clears the preview and prevents acceptance; it is never renewed
just because the preview remains visible. Bash keeps its existing lease and
four-second local display timer. Update the broker before the editor plugin.
Selection revisions have their own 60-second reading window, exact range
grant and 45-second client request bound for a cold model start. Repeated
requests retain the same session with newer coordinates so the broker can
avoid already-shown synonyms. A changed selection, dismissal or reconnect
ends that session; no revision text is replayed or carried through typing.

| Integration | Keys | Tested with |
| --- | --- | --- |
| Obsidian | Automatic, or Ctrl+Shift+Space to request; on a shown suggestion it lists up to four other first words where the suggestion stood, and a digit shows that word's continuation. Tab accepts one word only while a suggestion is visible; otherwise it stays Tab. Ctrl/Command+Right accepts all, Escape dismisses. Mid-note, only what fits before the next word. | Obsidian 1.14.4 / Electron 43, real app and model for the continuation checks: English and German display, the word Tab takes in its own colour, Tab, Ctrl+Right, undo, Escape, Ctrl+Shift+Space for another first word, broker restart; headed on Wayland with a temporary vault (2026-11-11, `3454e2e` code), the five half-typed stems each finished the intended word and Tab landed `documents` |
| Bash | Ctrl-X then Tab requests and accepts, Ctrl-X then Escape dismisses; never submits ([details](../shell/README.md)) | Bash 5.3.15, Readline 8.3, Ghostty 1.3.1 |
| Neovim | Prose buffers, automatic after a 120 ms pause, or Ctrl+Shift+Space, which on a shown suggestion lists up to four other first words where the suggestion stood; a digit shows that word's continuation. Tab accepts one word and Ctrl+Right all of it, only while a suggestion is visible; a correction replaces its exact suffix. Leaving Insert mode dismisses ([details](../neovim/README.md)) | Neovim 0.12.5 headless, scripted bridge and real phrase broker; live with the real model in foot (0.12.5) and Ghostty (0.12.6) ([matrix](../neovim/README.md#tested-with)) |
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
  or accept words**, grammar, refinement and synonym commands for selected text,
  and **Badi: Reconnect local model**; language commands pick
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

## Obsidian selection revision

Select 1–600 Unicode scalar values on one line, then press Ctrl+Shift+Space:
grammar, refinement, then repeated synonyms for a one-to-three-word phrase.
The command palette also offers **Check selected grammar**, **Refine selected
wording** and **Find a synonym**. Requests are explicit; selecting text alone
does nothing. Multiple selections, multiline text, composition, active editor
completion, read-only notes and denied app policy refuse revision.

A nonfocusable tooltip marks removed text as `[-removed]` and added text as
`[+added]`. Tab applies the whole revision; Escape or another key dismisses.
The tooltip may move above or below the selection to fit. Acceptance checks
that its full contents and selection anchor remain inside the visible editor pane, both before the
grant and before editing. A clipped preview cannot be accepted. CodeMirror's
UTF-16 range and selection direction are retained locally; Unicode scalar
offsets bind the broker request without splitting surrogate pairs.

The document, exact selection, focus, app policy and grant remain checked
through acceptance. An edit, selection or focus change, expiry, reconnect or
broker revocation retires the request. Applying uses one isolated CodeMirror
history transaction and reads back the whole document and caret before reporting
`applied`; one native undo restores the original text. Revisions never carry
through typing or become word-sized continuation grants.

An unavailable revision names `badi revise` in the status bar for setup and
verification; `badi revise install` verifies or repairs its model. Other
refusals retain their own reasons. A transport failure asks for a retry;
neither failures nor refusals advance the grammar/refinement/synonym cycle.

Tested with Obsidian 1.14.4-2 / Electron 43.8.3: the actual application in
headless Ozone, a temporary vault and private phrase broker passed all 16
live-runner checks. They include top- and bottom-pane preview layout, reverse
Unicode selection, whole replacement, one native undo, repeated synonyms,
selection changes and clipped-preview refusal. The inspected captures and
result are in `output/playwright/obsidian-PxjY04/`.

The same source plugin with private real Gemma 3 1B and Qwen3 1.7B runtimes
passed continuation, grammar replacement and undo, layout and refusal checks
(`output/playwright/obsidian-g0jdkx/`, including the complete synthetic trace).
With mid-line suggestions explicitly enabled in the private profile, the
unresolved `adress` typo incremented `typo_suspected` and left the field quiet
and unchanged. Synonyms have a model limitation: `big` returned rejected
output, and the development-selected `look after → take care of` applied and
undid correctly but repetition returned `no_change`. No second distinct
real-model synonym is proven. The first failing model receipt is retained at
`output/playwright/obsidian-ZwAjlC/`; the paired prompt investigation is in the
[Lab results](../../evaluation/writing/README.md#synonym-prompt-diagnostic).
These are editor/model behavior checks with synthetic text, not held-out
quality scores or installed Wayland plugin verification.

A visible Wayland run of the same temporary-vault plugin and private
real-model broker (SHA-256 `03dbe4b200e6…`) passed all 17 checks on Obsidian
1.14.4-2 / Electron 43.8.3 / Hyprland 0.56.2. The complete top- and bottom-pane
marked previews were inspected in compositor captures scoped to that app
window. Grammar and the development-selected synonym applied and undid;
`big` was refused and the synonym repeat returned `no_change`. Evidence,
source hashes, trace and captures are in `output/playwright/obsidian-93Efig/`.
CDP drove keys and editor setup, so compositor shortcut routing and physical
keys remain untested. No personal vault or installed Obsidian plugin was
changed. This adds visible rendering evidence, not a quality score.

## Check

```sh
npm run editors:check
npm run editors:integration
node adapters/obsidian/live.mjs --headless --local   # real Obsidian, temporary vault and profile
node adapters/obsidian/live.mjs --headless           # deterministic phrase/revision fixture
node adapters/obsidian/live.mjs --local              # visible private Wayland vault; unlocked, awake desktop
```

The live runner uses the installed Obsidian with its own temporary vault,
profile, plugin and broker; headless Ozone exercises the real editor without
the desktop. The visible lane verifies the launched process and active editor
window before keys and captures, and saves both CDP images and scoped
`*-wayland.png` compositor captures. It ends Obsidian by signalling only the app process, never its
process group, so Electron's crash helper does not fire. It never presses Tab
without a suggestion, because Tab then indents the line. Results stay under
the ignored `output/` directory. These checks prove editing behavior, not the
usefulness of German or Persian prose.
