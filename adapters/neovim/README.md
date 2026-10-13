# Neovim inline suggestions

Badi's Neovim plugin shows a grey suggestion at the caret in prose buffers.
Neovim owns the text: the plugin reads the buffer, draws the preview as inline
virtual text and makes the edit itself, so native undo applies. Needs Neovim
0.10 or newer and Node on `PATH`.

| Key (Insert mode) | Action |
| --- | --- |
| Pause typing | A suggestion appears after 120 ms |
| **Tab** | Accept it all. Without a suggestion, Tab is yours: your mapping, another plugin's, or Neovim's |
| **Ctrl+Right** | Accept the next word; the rest stays |
| Keep typing | Typing its next letters keeps the rest |
| **Escape** | Leaving Insert mode dismisses it |
| **Ctrl+Shift+Space** | Ask for one (`:BadiRequest` from any mapping) |

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
  keys = { accept = '<Tab>', accept_word = '<C-Right>', request = '<C-S-Space>' },
}
```

A terminal can send Ctrl+Shift+Space only through an extended keyboard
protocol; that has not been tried in a live terminal. Where the key does not
arrive, map `:BadiRequest` to one that does. `:BadiStatus` prints a
content-free state, and `:BadiReconnect` restarts the bridge with fresh
policy. The preview uses the `BadiSuggestion` highlight, linked to `Comment`.

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
- **Context.** At most 512 Unicode scalars before the caret (up to 64 earlier
  lines, joined with `\n`) and the rest of the caret's line after it, cut on
  UTF-8 boundaries. Text with bytes that are not UTF-8 is refused.
- **Binding.** A request snapshots the window, buffer, `changedtick`, caret row
  and byte column. A result is shown only if all are unchanged and the buffer
  is still eligible; otherwise it is dropped and the broker session is
  cancelled. Typing, caret moves, the completion menu, leaving the window or
  buffer, focus loss and leaving Insert mode withdraw a preview.
- **Display.** One extmark at the caret with `virt_text_pos = 'inline'`; the
  cursor stays before the grey text. It is drawn only in the requesting
  window where Neovim supports per-window namespaces (`nvim__ns_set`).
- **Acceptance.** While a preview is visible, Tab and Ctrl+Right are
  buffer-local mappings; a buffer-local mapping you had is restored when the
  preview goes. A key pressed when the preview is no longer current goes to
  your mapping or Neovim instead. Acceptance revalidates the snapshot, asks the
  broker for its one-shot grant, revalidates again, then calls
  `nvim_buf_set_text` at the snapshot caret and moves the cursor after it. It
  reports `applied` only after reading back exactly the expected line and
  caret; anything else is `failed` or `stale`.
- **Undo.** The edit breaks the undo sequence before and after itself
  (`undolevels`), so one `u` removes exactly the accepted text, typing before
  it stays, and typing after it is a separate step.
- **Privacy.** Typed and suggested text goes only to the bridge and the broker.
  The plugin's messages and `:BadiStatus` name reasons, never text; the bridge
  logs nothing, and Neovim discards its stderr. `badi debug on` records
  content-free counts in `debug-neovim.json`.

Not yet: spelling replacement (the plugin does not negotiate
`text_replacement`, so suggestions only append) and a trial with the real
model in an interactive terminal Neovim.

## Install

```sh
python3 scripts/install-editors.py --neovim
badi app nvim on
```

The plugin goes to Neovim's start package directory,
`stdpath('data')/site/pack/badi/start/badi` (following `XDG_DATA_HOME` and
`NVIM_APPNAME`), and the bridge with its shared modules to
`~/.local/lib/badi/editors/`. Restart Neovim. The installer refuses a symlinked
or foreign package directory, skips identical files and backs up what it
replaces ([editor installs](../shared/README.md#install)). For source testing,
add this directory to `runtimepath` and set `BADI_EDITOR_DIR` to the
repository's `adapters` directory, or set the `bridge` option to a path.

## Check

```sh
npm run editors:check         # plugin.test.mjs with a scripted bridge
npm run editors:integration   # broker.test.mjs: plugin + bridge + real phrase broker
```

Both drive a headless Neovim through its RPC socket with `nvim --server`. The
scripted lane covers inline display and cursor placement, Tab and Ctrl+Right,
one-step undo with typing before and after, multibyte (German and Persian)
byte offsets, a result or a grant arriving after an edit, fall-through to
global, buffer-local and completion-menu Tab mappings, a second window on the
same buffer, denied buffers, Escape, expiry and a bridge crash. The real-broker
lane covers the missing grant (no provider call), a compare-and-swap grant,
acceptance counted by the broker, the type-through carry without a model call,
pause, an explicit request, a broker restart without replay, and content-free
debug activity, broker logs and messages. The tests skip where Neovim 0.10 is
missing; the CI job `Neovim plugin / pinned Arch` sets `BADI_REQUIRE_NEOVIM=1`
so they must run there.

Tested headless with Neovim 0.12.5 and the deterministic phrase provider. No
trial has used the real model in an interactive terminal.
