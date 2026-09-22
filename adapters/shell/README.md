# Bash inline suggestions

Badi displays a grey suggestion at the end of the current Readline command.
Press **Ctrl-X, then Tab** to request it, and again to accept the visible text.
**Ctrl-X, then Escape** dismisses it. Ordinary **Tab** remains Bash completion;
**Ctrl-_** undoes an accepted edit. Suggestions never submit the command.
Spelling corrections are shown explicitly as `original → corrected` and replace
only the exact suffix authorized by the broker.

The display uses neutral mid-grey (`#808080`) when `COLORTERM` advertises
`truecolor` or `24bit`, independent of the theme's often-dark bright-black color.
Terminals advertising `256color` use grey index 244; basic terminals retain the
configured foreground and request italic text. Badi does not query terminal
input or change the theme. Styling and the cursor are restored to the real
command. The preview disappears on editing, caret movement,
search/completion, dismissal, or after four seconds. Returning to an old caret
position requests a fresh suggestion. A preview must fit beside the command on
one terminal row; wrapped commands, embedded control characters and insufficient
space receive a notice instead. Multiline and colored prompts are supported when
their last line and command fit. Terminal focus detection is not supplied by this
explicit Readline path; text is acquired only on its shortcut after fresh policy.

## Build and install

The display-only loadable builtin is compiled against the system's Bash and
Readline headers. It uses public Readline display hooks; it never adds preview
text to `rl_line_buffer`, intercepts arbitrary keys, reads terminal output, or
invokes a shell command. The existing Bash hook remains the only editor mutation
path, through `READLINE_LINE`, with native undo. The bridge negotiates exact
suffix replacement separately; a mismatched replacement grant cannot edit.

Build prerequisites are Python 3, a C compiler, Bash development headers and
Readline development headers. Arch packages are `gcc`, `bash`, `readline` and
`python`; Ubuntu packages include `build-essential`, `bash-builtins`,
`libreadline-dev` and `python3`. Runtime requires Bash, Node and a VT-compatible
terminal with clear-to-end-of-line support. `TERM=dumb` fails closed. Build for
the target Bash ABI; rebuild after incompatible Bash/Readline upgrades.

```sh
python3 adapters/shell/build-preview.py
npm run shell:check
python3 scripts/install-editors.py --bash
badi app bash on
```

The installer completes the build before changing installed files. It installs
`badi-preview.so`, the hook and bridge under
`~/.local/lib/badi/editors/shell/`, preserves existing Bash configuration, and
records original files in its rollback map. Open a new shell afterward. It does
not modify Ghostty configuration. For source testing, set `BADI_EDITOR_DIR` to
the repository's `adapters` directory before sourcing `badi.bash`; the hook finds
the built module in `shell/build/`. `CC` and `BADI_BASH_INCLUDE_DIR` can override
the build toolchain and Bash header directory.

## Verification boundary

The 2026-09-07 checks used Bash 5.3.15 and Readline 8.3 in actual disposable PTYs:

- Grey inline placement without an extra preview line, colored multiline prompt
  preservation, edit/caret invalidation, expiry, resize and control rejection.
- Truecolor, indexed-grey and basic-terminal styling, with foreground/italic
  reset and cursor restoration; previews never depend on ANSI bright-black.
- Exact spelling suffix replacement, trailing-space preservation, mismatched
  grant refusal, native undo, ordinary Tab completion and builtin unload.
- Persian text acquisition and caret placement, including Bash 5.3's character
  offsets; older Bash uses byte offsets in the hook.
- Literal accepted shell syntax remains editable text; no test line is submitted.
- Ctrl-C restores the normal prompt and SIGHUP exits without being swallowed by
  the timed display hook; its callback preserves Bash's deferred signal checks.

`npm run shell:check` builds the native module and runs these PTY tests with a
controlled bridge. `npm run editors:integration` additionally exercises the hook
with a disposable Rust phrase broker, policy pause/resume and reconnects. These
are editing and rendering checks, not local-model quality scores.

A separate physical Ghostty 1.3.1-arch2 / Bash 5.3.15 / Readline 8.3 trial on
2026-09-07 used the installed builtin and normal Qwen3-1.7B broker. The grey
preview left the command buffer unchanged, explicit acceptance inserted the
continuation, and native undo restored the exact prefix. A second trial replaced
`adress ` with `address ` and undid that exact correction. No Enter was submitted.
Screenshots and synthetic buffer receipts are under
`output/extensionless/installed-terminal-02/`; this proves the named shell flow,
not arbitrary terminal editors or multilingual model quality.

The implementation follows GNU's
[Readline redisplay API](https://tiswww.case.edu/php/chet/readline/readline.html#Redisplay)
and [Bash bind documentation](https://www.gnu.org/software/bash/manual/html_node/Bash-Builtins.html).
Ghostty's [shell integration](https://ghostty.org/docs/features/shell-integration)
provides prompt marking and terminal behaviors; the inline prediction belongs to
Bash's editor rather than a terminal capture layer.
