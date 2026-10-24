# LibreOffice Writer extension

An editor-owned channel for **spelling replacement** in Writer, in the
languages `badi spelling` turns on. It
corrects the word before the caret through Writer's own UNO text API, in one
native undo step, and reads the paragraph back before it reports `applied`.
Continuations and the grey preview in Writer stay on the
[IME-parity path](../../docs/decisions/0003-ime-parity-append-only.md), which
remains append-only; this extension never appends.

## Use

| Action | Key |
| --- | --- |
| See a correction as you type | Finish a misspelled word with a space and pause: the infobar shows it |
| Show a correction for the word at or before the caret | **Alt+F7** (or Tools ▸ Add-Ons ▸ Badi: Correct Last Word) |
| Replace it | **Alt+F7** again, or the infobar's **Replace** button |
| Dismiss | **Escape**, the infobar's close button, or any edit or caret move |

Alt+F7 sits beside F7 (spelling dialog) and Shift+F7 (online spelling); Writer
and Omarchy leave it free, Fcitx keeps Ctrl+Shift+Space and Ctrl+Alt+F7 stays a
VT switch. Writer's *Tools ▸ Customize ▸ Keyboard* can move it.

The proposal (`Badi: adress → address`) appears in an infobar and the document
is untouched until you accept it. After a replacement, one Ctrl+Z restores the
typo exactly and Ctrl+Y redoes the correction. The infobar also names why
nothing was offered: no correction, Badi paused or not allowed, the model
offline, or an unsupported place.

```sh
python3 scripts/install-editors.py --libreoffice   # LibreOffice must be closed
badi app libreoffice-writer on                     # its own grant
unopkg remove io.github.ahuray.badi.libreoffice    # removal
```

The installer builds `build/badi-libreoffice.oxt` reproducibly from
[extension/](extension/), keeps a copy at
`~/.local/lib/badi/editors/libreoffice/badi-libreoffice.oxt` with the usual
backup and receipt, and registers it with `unopkg add --force` in the default
LibreOffice profile. Restoring an editor backup restores that file but does not
unregister the extension; its backup holds a note with the `unopkg remove`
command.

## Contract

- **Identity and grant.** The client is adapter kind `libreoffice` with the
  single target `{kind:"libreoffice", app_id:"libreoffice-writer"}`, settings
  subject `linux_app/libreoffice/libreoffice-writer`. The Fcitx grant
  `badi app libreoffice` covers the grey preview only and does not authorize
  this extension; the app blocklist mode does, as for Obsidian and Bash. The
  broker serves this target corrections only: it calls no model unless the
  last word is a correction target, and withholds any continuation.
- **Automatic checks.** A job on each new or opened document (`Jobs.xcu`)
  watches its edits. After a 0.5 s pause following a word and one space at
  the end of a paragraph, one automatic request checks that word once; only a
  correction is shown, and nothing else appears. An Alt+F7 request replaces a
  check in flight and closes its broker connection at once.
- **Where it works.** The collapsed caret in a body-text paragraph of an
  editable, plain Writer document, with no selection or multi-selection. The
  corrected word holds the caret, ends at it (with its own space), or sits
  before closing punctuation; `correction_span()` finds it. Tables, frames,
  headers, footnotes, fields, protected sections, read-only and Web/master
  documents, change tracking (`RecordChanges`) and paragraphs with objects
  (U+FFFC) are refused before anything is sent.
- **Context.** The paragraph up to that word's end (last 512 characters) and
  its character locale (`en-US`); the broker sees no text after it and the
  rest of the paragraph must stay unchanged. Text is held in memory only for
  the attempt and never logged.
- **Binding.** The frame, controller, document (`RuntimeUID`), the paragraph
  up to the word, the rest of it and its language are captured at the request and
  compared again: before display, every 250 ms and on every edit, selection
  change or Escape while the proposal is visible, before the grant is asked
  for, and once more right before the edit. A Replace button from another
  window never touches the proposal; a new request anywhere retires it.
- **Broker.** One private v2 connection per attempt (`$XDG_RUNTIME_DIR/badi/broker.sock`,
  owner and `0600` checked, `SO_PEERCRED` UID checked) negotiating
  `context`, `suggestion`, `commit.applied`, `text_replacement`, `control`,
  `policy` and `long_reading_window` (a 15 s reading window). It acknowledges
  every authority epoch; any later epoch, a `suggestion.clear`, a malformed
  frame or a closed socket retires the attempt. A proposal must replace exactly
  the bound lowercase word and at most its typed space
  ([text guards](extension/pythonpath/badi_libreoffice/text.py)); continuations
  are ignored.
- **Mutation.** After the one-shot `commit.prepare`, the extension selects
  exactly `replace_before` to the left of the caret, checks its text, and
  replaces it inside `XUndoManager.enterUndoContext("Badi correction")`, always
  leaving the context. It reports `applied` only when the paragraph reads
  back as the bound prefix plus the correction, the caret is collapsed and the
  newest undo action is that context; otherwise `failed`, or `stale` when the
  binding moved after the grant. Writer's autocorrect does not run on this edit.
- **Threads.** Document access runs on the office main thread
  (`com.sun.star.awt.AsyncCallback`); only broker I/O runs on worker threads, so
  a slow model never freezes Writer and an edit during the wait makes the
  result stale. Any failure on a worker thread ends the attempt; an explicit
  request then shows a content-free notice.

## Check

```sh
npm run libreoffice:check         # text guards, scripted-broker client, controller attempt lifecycle, reproducible build
npm run libreoffice:integration   # real phrase broker + headless soffice, isolated profile
```

The integration lane builds the broker, starts `badi-broker --provider phrase`
and `soffice --headless` under a temporary runtime, configuration, `HOME` and
LibreOffice profile, installs the extension only there, and drives Writer over
a private UNO pipe with disposable text. It checks the Alt+F7 and menu
registration, display without editing, acceptance by key and button, one-step
undo and redo, stale edits, caret moves, selections, tables, change tracking,
another document, ambiguous words (`teh`), pause during display, the Fcitx-only
grant, expiry, an edit landing after the grant (`BADI_LIBREOFFICE_GRANT_DELAY_MS`,
a test seam bounded to 1 s that only delays) and Escape. It prints `SKIPPED`
and exits 0 only when `soffice` or `unopkg` is missing.

The phrase fixture offers the same inference-free dictionary corrections as
the model provider (one unambiguous single edit, such as `adress` → `address`);
ambiguous words go to the model in production and get nothing from the fixture.

## Boundaries

- After a replacement and Ctrl+Z, Writer leaves the restored word selected,
  so the next key typed replaces it: Writer's undo, not a Badi edit.
- Tab is not this extension's key. With the hint showing it stays Writer's
  own Tab, inserts a tab character and withdraws the hint.
- Writer's own AutoCorrect replaces some typos (`teh`) before the extension
  sees them.
- An English word that begins a known word, such as `fing` (`finger`), is
  never a correction target, which also keeps abbreviations such as `dept`.
- The paragraph's language picks the dictionary, as for Writer's own
  spelling check: a short French sentence in a paragraph tagged Spanish was
  corrected from Spanish (`reponce → repone`).
- A wrong correction from the model (`cassa → casas`, `Zeitt → Zeigt` were
  seen) stays a proposal until Alt+F7 accepts it.

### Tested with

Interactive Wayland windows with Fcitx beside them, isolated profiles and
disposable text; the integration lane runs headless LibreOffice 26.8 with
the phrase fixture.

| LibreOffice (installed commit, date) | Result | Boundary |
| --- | --- | --- |
| 26.8.0.3 (2026-10-14) | Alt+F7 showed `Badi: adress → address`, Alt+F7 again replaced it (`applied`) and one Ctrl+Z restored `adress` | |
| 26.8.0.3 with German, French and Spanish dictionaries (2026-10-15) | A finished `Hauss` showed `Hauss → Haus` on its own and Alt+F7 applied it; `muchas grasias` showed `gracias` after about 2 s | |
| 26.8 (`78ff8ca`, 2026-10-17) | The automatic hint showed `adress → address`; Alt+F7 showed and replaced the whole word; one Ctrl+Z restored it | |
| 26.8.1.1 with the same dictionaries (2026-10-19) | After the broker's candidate-choice fix ([cause](../../broker/README.md#suggestion-limits)): `grasias → gracias` and `vacanses → vacances` about 1 s after the space, each in one undo step; `lsit → list` in 0.27 s; the model abstained on `Hauss` | Installed `5e141ed` with Gemma 3 1B, before that fix, corrected only single-candidate words |

Not yet verified: other LibreOffice versions.
