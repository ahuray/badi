# Bash inline suggestions

Badi shows a grey suggestion after the current Readline command.

- **Ctrl-X, then Tab** requests it, and again accepts the visible text.
- **Ctrl-X, then Escape** dismisses it.
- **Ctrl-_** undoes an accepted edit.
- Ordinary **Tab** stays Bash completion, and nothing ever submits the command.

Spelling corrections appear as `original → corrected` and replace only the
exact suffix the broker authorized.

The preview is mid-grey: `#808080` on truecolor terminals, index 244 on
256-color ones, and italic in the normal colour on basic terminals. Badi neither
queries the terminal nor changes its theme. The preview disappears on editing,
caret movement, search or completion, dismissal, or after four seconds. It must
fit beside the command on one row; wrapped commands, control characters and
missing space get a notice instead. `TERM=dumb` fails closed.

## How it works

A display-only loadable builtin, `badi-preview.so`, is compiled against this
system's Bash and Readline headers. It draws through Readline's public display
hooks and never adds text to `rl_line_buffer`, reads terminal output, intercepts
other keys or runs a command. The Bash hook is the only mutation path, through
`READLINE_LINE`, so native undo works. Persian caret positions are counted in
characters (Bash 5.0+). Each shell starts its Node bridge (about 60 MB) on the
first Ctrl-X Tab, and the bridge exits after ten idle minutes.

## Build and install

Needs Python 3, a C compiler and the Bash and Readline development headers (on
Arch: `gcc`, `bash`, `readline`, `python`). Runtime needs Bash, Node and a
VT-compatible terminal. Rebuild after an incompatible Bash or Readline upgrade.

```sh
python3 adapters/shell/build-preview.py
npm run shell:check
python3 scripts/install-editors.py --bash
badi app bash on
```

The installer builds first, then installs the builtin, hook and bridge under
`~/.local/lib/badi/editors/shell/` with a rollback map. For source testing, set
`BADI_EDITOR_DIR` to this repository's `adapters` directory before sourcing
`badi.bash`. `CC` and `BADI_BASH_INCLUDE_DIR` override the toolchain and header
directory.

## Verification

`npm run shell:check` builds the module and runs PTY tests with a controlled
bridge, covering:
- placement, invalidation, expiry and resize;
- terminal styles;
- exact spelling replacement and grant refusal;
- native undo and ordinary Tab completion;
- Persian caret positions;
- Ctrl-C and SIGHUP handling.

`npm run editors:integration` adds a real phrase broker, pause and reconnects.

A physical trial in Ghostty 1.3.1 with Bash 5.3.15, Readline 8.3 and the real
model showed the preview, acceptance, an `adress ` → `address ` correction and
exact undo, without submitting anything.
