# ADR 0009: terminals use the line typed into them

- Status: accepted
- Date: 2026-10-14
- Scope: the Fcitx module's terminal path

## Context

Terminal TUIs, such as a coding agent's prompt in Ghostty, give an input method
no surrounding text and expose no accessible text: Ghostty 1.3's accessibility
tree is empty panels. Bash and Neovim have their own Badi integrations, but
nothing else in a terminal could get a suggestion. The product rule kept Badi
from recording keystrokes.

## Decision

For terminal apps the user allows by exact rule (`badi app
com.mitchellh.ghostty on`), the Fcitx module keeps the line typed into that
input context since its last boundary, in memory, and uses it as the context:

- Typing appends; Backspace, Ctrl+W, Ctrl+U, Ctrl+C and Enter edit or reset
  the line. A key that may put the caret inside the line (arrows, Home,
  Delete, Readline motion such as Ctrl+A or Alt+B) stops tracking until the
  next empty line. Any other key the terminal receives (a paste, Tab
  completion, Escape, End) changes the line invisibly but leaves the caret at
  its end, so tracking restarts there and follows only what is typed next
  (changed 2026-10-17: a pasted image in a coding agent's prompt used to end
  suggestions until Enter). A new focus starts an empty line.
- An automatic request needs at least two words with letters, so a lone
  token, such as a command or a typed secret, never becomes a request.
  Ctrl+Shift+Space asks for any tracked line.
- The suggestion shows in Badi's Fcitx panel at the cursor. Tab types one
  word and Ctrl+→ all of it, each with one `commitString`; the result is
  `dispatched-unverified`. A second Tab pressed before a word acceptance's
  remainder returns is held for it, so it cannot fall through to the
  terminal's own completion. An automatic request that finds nothing stays
  quiet: the no-continuation notice is shown only after an explicit request,
  so a terminal that mostly runs commands is not told about every miss.
- The caret is followed within the tracked line (added 2026-10-17): Left,
  Right, Home, End, Delete and Readline's Ctrl+A/B/E/F/D/K move or edit it
  there, so the text after the caret becomes the request's `after`; motion
  past either end of the line stops tracking, since a multi-line prompt may
  change lines there.
- A spelling correction shows as `teh → the` and is applied only on Tab
  (user decision, 2026-10-17). A terminal ignores the input method's delete
  request, and its Backspace key sends DEL to the program, so the one
  `commitString` leads with one DEL per tracked scalar of the typo and then
  types the correction. Only a suffix that exactly ends the tracked text
  before the caret is erased; nothing is ever deleted that Badi did not see
  typed. A terminal whose erase character is not DEL (`stty erase ^H`) would
  show `^?` instead.
- A field marked password or sensitive is never read. The line is never
  written anywhere except a `badi debug on --text` trace (ADR 0008).
- An editor with its own Badi integration owns the keys typed into it.
  Neovim reports when it holds its terminal's focus (`editor.focus`, added
  2026-10-17), and while it does the broker withholds every desktop-application
  target, so the typed line asks nothing and Tab and Ctrl+Shift+Space reach
  Neovim. The report only withholds, like the desktop signals of ADR 0005, and
  ends with Neovim's focus or connection. Bash keeps both paths: its own
  integration answers only Ctrl-X Tab.

## Consequences

- A terminal cannot mark a password prompt. Typing a passphrase of two or more
  words into an allowed terminal can make it a request's context, sent only to
  the local model. This is why terminals need an exact, opt-in rule.
- A line that already had text when it gained focus is followed only from
  that point, so suggestions there continue just the newly typed part.
- Only Ghostty is listed; another terminal is added after a live trial.
