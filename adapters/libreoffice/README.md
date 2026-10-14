# LibreOffice Writer extension

An editor-owned channel for **English spelling replacement** in Writer. It
corrects the word before the caret through Writer's own UNO text API, in one
native undo step, and reads the paragraph back before it reports `applied`.
Continuations and the grey preview in Writer stay on the
[IME-parity path](../../docs/decisions/0003-ime-parity-append-only.md), which
remains append-only; this extension never appends.

## Use

| Action | Key |
| --- | --- |
| Show a correction for the word before the caret | **Alt+F7** (or Tools ▸ Add-Ons ▸ Badi: Correct Last Word) |
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
  broker serves this target only explicit requests.
- **Where it works.** The collapsed caret at the end of a body-text paragraph
  of an editable, plain Writer document, with no selection or multi-selection.
  Tables, frames, headers, footnotes, fields, protected sections, read-only and
  Web/master documents, change tracking (`RecordChanges`) and paragraphs with
  objects (U+FFFC) are refused before anything is sent. The broker replaces
  only at the end of the field, so text after the caret means no request.
- **Context.** The paragraph before the caret (last 512 characters) and its
  character locale (`en-US`); nothing from other paragraphs. Text is held in
  memory only for the attempt and never logged.
- **Binding.** The frame, controller, document (`RuntimeUID`), paragraph text
  before and after the caret and its language are captured at the request and
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
  result stale.

## Check

```sh
npm run libreoffice:check         # text guards, scripted-broker client tests, reproducible build
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

Tested headless with LibreOffice 26.8 on this workstation, with the phrase
fixture. Live on 2026-10-14 (LibreOffice 26.8.0.3, interactive Wayland
window, Fcitx beside it, installed broker, isolated profile, disposable
text): Alt+F7 showed the infobar `Badi: adress → address`, Alt+F7 again
replaced it (`applied`) and one Ctrl+Z restored `adress`. Writer then leaves
the restored word selected, so the next key typed replaces it; that is
Writer's undo, not a Badi edit. Not yet verified: a model-backed correction
of an ambiguous word and other LibreOffice versions. English only, like the
broker's spelling contract.
