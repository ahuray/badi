# Neovim inline suggestions

Badi's Neovim plugin shows a grey suggestion at the caret in prose buffers.
Neovim owns the text: the plugin reads the buffer, draws the preview as inline
virtual text and makes the edit itself, so native undo applies. Needs Neovim
0.10 or newer and Node on `PATH`.

| Key (Insert mode) | Action |
| --- | --- |
| Pause typing | A suggestion appears after 120 ms |
| **Tab** | Accept the next word; the rest stays. A correction (drawn `adress → address`) replaces exactly the suffix it names. Without a suggestion, Tab is yours: your mapping, another plugin's, or Neovim's |
| **Ctrl+Right** | Accept it all; for a correction, the same exact replacement |
| Keep typing | Typing its next letters keeps the rest |
| **Escape** | Leaving Insert mode dismisses it |
| **Ctrl+Shift+Space** | Ask for one (`:BadiRequest` from any mapping). On a shown continuation, list up to four other first words where it stood; a digit shows that word's continuation, and pressing again lists the rest. On a correction, it keeps the correction |
| **1–9** | While a numbered list is drawn, show the continuation of the word it numbers; any other digit types normally |

`badi app nvim on` allows Neovim; `badi app nvim off` blocks it. The grant is
Neovim's own (`{kind: linux_app, adapter: neovim, app_id: nvim}`), separate
from Bash and every other app.

## Which buffers

Only a prose filetype in a normal file buffer is read: `markdown`, `text`,
`gitcommit`, `mail`, `rst`, `tex`, `asciidoc`, `org` and `norg` by default.
Nothing is read, and no request is made, when the buffer:

- is a special buffer (`buftype` other than empty or `acwrite`: terminal,
  prompt, `nofile`, help, quickfix);
- is not `modifiable`, is `readonly`, is `binary`, or `'paste'` is set;
- lives under `/dev/shm/` (where `pass edit` writes) or ends in `.gpg`,
  `.asc`, `.pgp` or `.age`;
- sets `b:badi_disable` (or `g:badi_disable` for every buffer).

The completion menu, a running macro, a lost terminal focus and any mode other
than plain Insert also keep Badi quiet. Badi cannot tell a secret from its
words: put `vim.b.badi_disable = true` in an autocommand for anything else
sensitive.

## Selection revision

In an eligible prose buffer, select 1–600 characters on one line with
characterwise Visual mode (`v`) and press Ctrl+Shift+Space. The first request
checks grammar, the next refines wording; for a word or short phrase, later
presses offer a synonym, then another synonym. A `no_change` answer also
allows the next mode; a refusal retires the cycle, so the next chord starts
with grammar. It uses the broker's separate
[revision model and output gate](../../broker/README.md#selection-revision).
Nothing is revised automatically.

The original selection stays intact. A nonfocusable preview near the selection marks removed
text as `[-text]` and added text as `[+text]`, using `BadiRevisionRemoved`
and `BadiRevisionAdded` (linked to `DiffDelete` and `DiffAdd`). Tab replaces
the entire selection in one native undo step. Escape, another key, an edit,
selection or mode change, window/buffer/focus loss, pause, expiry or a failed
connection withdraws it. The complete preview stays inside the requesting
window, moving above the selection when necessary. It must fit within half the window's height;
otherwise `preview_unavailable` refuses acceptance. Existing Visual Tab maps
return when the preview ends.

Linewise, blockwise, multiline, virtual-space and unsupported selections are
refused. Forward/reverse and inclusive/exclusive selections use Neovim's
`getregion()` and `getregionpos()`; a build without those functions refuses
the feature. Native byte ranges and broker Unicode-scalar ranges are both
bound to the exact window, buffer, changedtick, selection endpoints and text.
A broker grant for another range or text changes nothing.

A direct synonym mapping can skip the grammar/refine steps:

```lua
vim.keymap.set('x', '<leader>bs', function() require('badi').revise('synonyms') end)
```

`require('badi').revise('grammar')` and `revise('refine')` work the same way.
Map these functions in Visual mode; an Ex command that leaves Visual mode
does not reuse old selection marks. The bridge gives a revision request
45 seconds (the broker bounds inference at 40), and Neovim allows 47 seconds
for the reply. Shown revisions expire after 60 seconds. Repeated synonyms
keep one session with newer revisions; edits and cancellation discard it.

An unavailable revision shows `run badi revise`; `:BadiStatus` keeps the raw
reason and the same recovery hint. That command distinguishes model setup
from other availability limits; `badi revise install` verifies or repairs
the model. Timeouts, unsupported languages and invalid output retain their
own reasons and never mean that the model needs downloading.

## Configuration

The plugin sets itself up from `vim.g.badi` when Neovim starts. Calling
`require('badi').setup({...})` in `init.lua` replaces that; set
`vim.g.badi_auto_setup = false` to do neither.

```lua
vim.g.badi = {
  filetypes = { 'markdown', 'text', 'gitcommit', 'mail' },
  automatic = true,          -- false: only explicit requests
  delay_ms = 120,            -- typing pause before an automatic request
  language = nil,            -- 'en', 'de' or 'fa'; nil uses $BADI_LANGUAGE, then the locale
  keys = { accept = '<C-Right>', accept_word = '<Tab>', request = '<C-S-Space>' },
}
```

A terminal can send Ctrl+Shift+Space only through an extended keyboard
protocol (foot and Ghostty do, [below](#tested-with)). Where the key does not
arrive, map `:BadiRequest` to one that does. `:BadiStatus` prints a
content-free state, including the policy the bridge last reported
(`automatic`, `manual`, `refused` or `unknown`), and `:BadiReconnect`
restarts the bridge with fresh policy. The preview uses the `BadiSuggestion`
highlight, linked to `Comment`; the word Tab takes uses `BadiSuggestionWord`,
the comment colour in bold. Set either group to restyle it.

## How it works

- **Bridge.** On the first Insert in an eligible buffer, the plugin starts
  [`bridge.mjs`](bridge.mjs) with `vim.fn.jobstart` and speaks one JSON line per
  message over its stdio. The bridge reuses the shared
  [broker client](../shared/broker-client.mjs) (protocol v2, adapter kind
  `neovim`, the 15-second `long_reading_window`, `commit.applied`) through its
  reconnecting wrapper, so the wire, frame, policy and authority handling are
  the same code Obsidian and Bash use. It exits after ten idle minutes; the
  next eligible keystroke starts a new one. A reconnect only refreshes policy
  and never replays text.
- **Policy first.** Every state the bridge reports carries the broker's
  policy for Neovim. The plugin reads no buffer text for a request that
  policy refuses (paused, `badi app nvim off`, or an automatic request under
  a manual grant); until the bridge has reported one, it asks with a
  content-free `status` first. The bridge checks the policy again before it
  sends anything to the broker.
- **Context.** At most 512 Unicode scalars before the caret (up to 64 earlier
  lines, joined with `\n`) and the rest of the caret's line after it, cut on
  UTF-8 boundaries. Text with bytes that are not UTF-8 is refused.
- **Binding.** A request snapshots the window, buffer, `changedtick`, caret row
  and byte column. A result is shown only if all are unchanged and the buffer
  is still eligible; otherwise it is dropped and the broker session is
  cancelled. Typing, caret moves, the completion menu, leaving the window or
  buffer, focus loss and leaving Insert mode withdraw a preview.
- **Display.** One extmark at the caret with `virt_text_pos = 'inline'`; the
  cursor stays before the grey text. A spelling correction is drawn whole as
  `original → corrected`, not as an appendable continuation. It is drawn only
  in the requesting window where Neovim supports per-window namespaces
  (`nvim__ns_set`).
- **Acceptance.** While a preview is visible, Tab and Ctrl+Right are
  buffer-local mappings; a buffer-local mapping you had is restored when the
  preview goes. A key pressed when the preview is no longer current goes to
  your mapping or Neovim instead. Acceptance revalidates the snapshot, asks the
  broker for its one-shot grant, revalidates again, then calls
  `nvim_buf_set_text` at the snapshot caret and moves the cursor after it. For
  a correction, the grant must name the same `replace_before`, and that exact
  byte suffix ending at the caret (including its optional typed space) is the
  only range replaced. The broker proposes corrections only when the request
  has no text after the caret; Neovim sends the rest of the caret line there,
  so live corrections are at a line end (later lines do not block one). It
  reports `applied` only after reading back exactly
  the expected line and caret; anything else is `failed` or `stale`.
- **Undo.** The edit breaks the undo sequence before and after itself
  (`undolevels`), so one `u` removes exactly an accepted insertion or restores
  a replaced suffix; typing before it stays, and typing after it is separate.
- **Terminal focus.** When the terminal reports focus (`FocusGained`), the
  plugin tells the broker it holds that terminal (`editor.focus`), and the
  input method's typed-line path stays out of it until `FocusLost`, Ctrl+Z
  or exit; the bridge then stays connected, and a new connection repeats the
  claim. Ghostty sends focus events by itself; inside tmux, enable
  `focus-events`, or both Badi paths may suggest at once.
  `:lua print(vim.inspect(require('badi').status()))` shows `holds_terminal`.
- **Privacy.** Typed and suggested text goes only to the bridge and the broker.
  The plugin's messages and `:BadiStatus` name reasons, never text; the bridge
  logs nothing, and Neovim discards its stderr. `badi debug on` records
  content-free counts in `debug-neovim.json`.

## Install

```sh
python3 scripts/install-editors.py --neovim
badi app nvim on
```

The plugin goes to Neovim's start package directory,
`stdpath('data')/site/pack/badi/start/badi` (following `XDG_DATA_HOME` and
`NVIM_APPNAME`), and the bridge with its shared modules to
`~/.local/lib/badi/editors/`. When lazy.nvim is detected (its installed
`lazy/lazy.nvim` directory, `lazy-lock.json`, or `lua/plugins/` under
`stdpath('config')`), the installer also writes the new file
`lua/plugins/badi.lua` containing `{ dir = "<installed start package>",
lazy = false }`, because lazy.nvim manages the runtimepath itself and a
start package alone may never load. It never edits an existing user spec:
a foreign `badi.lua` or a symlink at that path is refused, while Badi's
own marked spec is backed up before replacement and receipted so the
editor installation's restore removes it. Restart Neovim. The installer
refuses a symlinked or foreign package directory, skips identical files
and backs up what it replaces ([editor installs](../shared/README.md#install)).
For source testing,
add this directory to `runtimepath` and set `BADI_EDITOR_DIR` to the
repository's `adapters` directory, or set the `bridge` option to a path.

## Check

```sh
npm run editors:check         # plugin.test.mjs with a scripted bridge
npm run editors:integration   # broker.test.mjs: plugin + bridge + real phrase broker
```

Both drive a headless Neovim through its RPC socket with `nvim --server`. The
selection lane covers marked previews, forward/reverse and exclusive Unicode
ranges, native undo, policy-before-acquisition, changed selections, late and
mismatched grants, cancellation, and repeated synonyms against the real
broker's deterministic reviser. Neovim 0.12.6's actual rendered screen grid
also captures the complete preview at the viewport bottom, acceptance, undo,
two synonyms and dismissal (`output/neovim-selection-revision/fixture-grid.json`, local evidence).
This is the source plugin with a private phrase broker. A separate private
real-model RPC trial captured the marked grammar edit, exact selected-range
replacement, one native undo and pending cancellation. Its first synonym
request for `big` was refused as `invalid_output`; the original failed receipt
is retained in `output/neovim-selection-revision/real-model-rpc/`.
The follow-up proved that refusal leaves the selection and document unchanged,
then applied and undid `look after → take care of`. That positive phrase was
selected from a development comparison, not held-out quality evidence. A
repeated request returned `no_change`, so two distinct real-model synonyms are
not proven (`output/neovim-selection-revision/real-model-rpc-follow-up/`).
These trials used Qwen3 1.7B through the real revision runtime and the source
plugin, with keys injected over Neovim RPC. The desktop was locked during
those runs and was left untouched.

After authorized installation and normal unlock, foot 1.28.0 with Neovim
0.12.6 (`-u NORC`, installed start package and bridge, persistent broker)
showed the complete grammar, refine and synonym previews. Ctrl+Shift+Space,
Tab, native `u` and Escape passed through the compositor and terminal using
`wtype`; RPC only prepared disposable text and read state. Grammar and the
development-selected `look after → take care of` each replaced only the
selection and undid once; dismissal and in-flight cancellation left text
unchanged. `big` synonyms abstained (`invalid_output` in the completed run,
`no_change` in the retained earlier attempt). Captures, trace, installed
file hashes and doctor receipt are in
`output/native-trials/2026-11-12-installed-neovim-selection/`.
These are synthesized keys, not physical keyboard presses, and this clean
startup does not test the user's LazyVim configuration. A separate recorded
grammar probe exposed the model's `big **cat**` offer; it was visibly marked
and dismissed without acceptance
(`output/native-trials/2026-11-12-installed-neovim-selection-markup/`).
After the markup guard was installed (broker SHA-256 `03dbe4b200e6…`),
the exact `The |big| house` grammar request returned `invalid_output` with
trace outcome `shape`, no preview and no edit. An ordinary `I has went →
I have gone` preview, Tab replacement and single native undo still passed
through the same installed plugin and `wtype` route; inspected captures and
the new doctor receipt are in
`output/native-trials/2026-11-12-installed-neovim-selection-markup-fixed/`.

The scripted lane covers inline display and cursor placement, Tab (one word) and
Ctrl+Right (all of it), exact spelling replacement (including a typed space,
a mismatched grant and a stale grant),
one-step undo with typing before and after, multibyte (German and Persian)
byte offsets, a result or a grant arriving after an edit, fall-through to
global, buffer-local and completion-menu Tab mappings, a second window on the
same buffer, denied buffers, a refused, manual or unreported policy sending no
text, Escape, expiry and a bridge crash. The real-broker
lane covers the missing grant (no provider call), a compare-and-swap grant,
acceptance counted by the broker, the type-through carry without a model call,
pause, an explicit request, a broker restart without replay, and content-free
debug activity, broker logs and messages. The tests skip where Neovim 0.10 is
missing; the CI job `Neovim plugin / pinned Arch` sets `BADI_REQUIRE_NEOVIM=1`
so they must run there.

## Tested with

The installed recovery trial's [marked preview](../../docs/evidence/neovim-revision-preview.png)
and [field after Tab](../../docs/evidence/neovim-revision-applied.png) are
scoped Hyprland captures of the disposable draft described below.

| Neovim | Where | Provider | Result |
| --- | --- | --- | --- |
| 0.12.6 | headless, both lanes above, including selection revision | phrase fixture | pass |
| 0.12.6 | headless source plugin, private broker, RPC keys | real revision model (Qwen3 1.7B) | marked grammar and development-selected synonym preview, exact replacement, one undo, cancellation and safe `big` refusal; repeated positive synonym returns `no_change`; evidence and limits above |
| 0.12.6 | foot 1.28.0, `-u NORC`, installed start package and bridge, `wtype` keys (2026-11-12, `bab1b04` dirty; broker SHA-256 `7b7d423219da…`) | persistent real revision model (Qwen3 1.7B) | complete grammar/refine/synonym previews inspected; Tab replacement, one native undo, Escape and pending cancellation; exact source hashes, receipts and model limits above |
| 0.12.6 | same installed foot/plugin route after the markup guard (broker SHA-256 `03dbe4b200e6…`) | persistent real revision model (Qwen3 1.7B) | exact `big` grammar request refused with `shape`/`invalid_output`, no preview or edit; ordinary marked grammar preview, Tab and one native undo passed |
| 0.12.6 | installed recovery update, foot 1.28.0, `-u NORC`, `wtype` keys; same broker hash (2026-11-12) | persistent real revision model (Qwen3 1.7B) | two refused `big` requests both stayed grammar; marked `I has went` → `I have gone`, Tab and one native undo passed. Owned-window captures, exact installed hashes and clean shutdown: `output/revision-recovery/installed-live/trial-lBOV2P/`. Unavailable-model notices remain fixture evidence; desktop model files were kept intact |
| 0.12.5 | foot, `nvim -u NONE` with the installed start package | real model | pause preview, Tab word by word, Ctrl+→, Insert-mode exit dismisses, streamed growth in the trace |
| 0.12.5 | foot, this workstation's LazyVim through the installer's lazy.nvim spec | real model | as above, plus Ctrl+Shift+Space as an explicit request; no `output_rejected` in the trace |
| 0.12.6 | Ghostty, the same LazyVim setup | real model | Tab, Ctrl+→ and Ctrl+Shift+Space |
| 0.12.6 | Ghostty, LazyVim (2026-10-24) | real model | a markdown buffer suggested and Tab took a word; while `badi pause` held, typing sent no request and no text to the broker, and suggestions came back after `badi resume` |
| 0.12.6 | foot, `nvim --clean` with this plugin and its bridge | real model | the word Tab takes drawn bold and brighter; Ctrl+Shift+Space on a shown suggestion gave three alternatives with new first words, then the first again; Tab took the shown word |
| 0.12.6 | Ghostty 1.4.0, LazyVim and `nvim -u NORC` with the installed plugin (2026-11-11, installed `3454e2e`) | real model | The five half-typed stems each finished the intended word in a scratch markdown buffer; Tab landed `documents` and the saved file read back exact. Under LazyVim, the Enter between stems left snippet-shaped text (a callout, a date, a UUID) in the saved file after each suggestion had shown, so the `-u NORC` run is the file evidence; with `-u NONE` the plugin did not load and Ghostty's typed line finished the stems instead (`output/native-trials/2026-11-11-partial-words-surfaces/`) |
| 0.12.6 | Ghostty 1.4.0, `nvim -u NORC` with the installed plugin (2026-11-12, installed broker `b6be55f`) | real model | Spelling replacements in a scratch markdown buffer: `adress → address` (dictionary, 0 ms), `recieve → receive` (49 ms) and `teh → the` (64 ms) each showed with their exact suffix, Tab replaced it and the saved file read back exact; typing after the correction and the correction itself undid as separate steps (`output/native-trials/2026-11-12-nvim-spelling/`) |
