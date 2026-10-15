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
  the line. Any other key the terminal receives (arrows, Home, Tab, paste,
  Escape) may change the line invisibly, so tracking stops until the next
  empty line. A new focus starts an empty line.
- An automatic request needs at least two words with letters, so a lone
  token, such as a command or a typed secret, never becomes a request.
  Ctrl+Shift+Space asks for any tracked line.
- The suggestion shows in Badi's Fcitx panel at the cursor. Tab, or Ctrl+→
  for a word, types it with one `commitString`; the result is
  `dispatched-unverified`.
- A field marked password or sensitive is never read. The line is never
  written anywhere except a `badi debug on --text` trace (ADR 0008).

## Consequences

- A terminal cannot mark a password prompt. Typing a passphrase of two or more
  words into an allowed terminal can make it a request's context, sent only to
  the local model. This is why terminals need an exact, opt-in rule.
- A line that already had text when it gained focus is followed only from
  that point, so suggestions there continue just the newly typed part.
- Only Ghostty is listed; another terminal is added after a live trial.
