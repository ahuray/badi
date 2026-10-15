# Changelog

What shipped, newest first, by merged pull request. This is history only:
current behavior lives in the [README](README.md) and the runbooks, open work
in [future plans](<future plans.md>), and lasting decisions in
[docs/decisions](docs/decisions/).

## 2026-10-13: desktop signals, new surfaces and a clean code base

- [#84](https://github.com/ahuray/badi/pull/84) The bar mark opens a keyboard-first
  Omarchy popout: mode chip, activity spinner, acceptance meter, settings
  search and rows for battery, spelling languages and Ghostty.
- [#83](https://github.com/ahuray/badi/pull/83) `badi spelling` turns on German,
  French, Spanish, Italian, Dutch and Polish spelling from installed Hunspell
  dictionaries; the model chooses between candidates.
- [#82](https://github.com/ahuray/badi/pull/82) Writer checks each finished word
  on its own and Alt+F7 corrects the word at the caret; the broker serves the
  Writer target corrections only.
- [#81](https://github.com/ahuray/badi/pull/81) Ghostty: suggestions follow the
  line typed into an allowed terminal and Tab types them
  ([ADR 0009](docs/decisions/0009-terminals-use-the-typed-line.md)).
- [#80](https://github.com/ahuray/badi/pull/80) Obsidian and Neovim show the
  whole streamed suggestion and accept the newest; Writer suggests after a
  trailing space at a paragraph end.
- [#79](https://github.com/ahuray/badi/pull/79) `badi debug on [--text]`
  traces each suggestion from model to screen across the broker, Fcitx,
  observer, editors and compositor ([ADR 0008](docs/decisions/0008-debug-trace-may-hold-text-on-request.md));
  Writer's trailing-space sentence lookup no longer refuses the caret.
- [#78](https://github.com/ahuray/badi/pull/78) Type-through keeps the
  carried remainder when a browser field is re-admitted with the same
  context, instead of asking the model again; the observer's overflow chip
  sits under the caret inside the field when nothing follows it. Corrects
  #77: the doubled pill was cotype, a separate autocomplete, not Badi.
- [#77](https://github.com/ahuray/badi/pull/77) Records only: the
  2026-10-13 evening live install/trial results in the plan — session
  helper and binds proven, the observer chip-doubling defect in browsers
  and Writer, and live type-through carry counters. No behavior change.
- [#76](https://github.com/ahuray/badi/pull/76) `badictl status` counts
  type-through carry outcomes by declining gate (`type_through`), and a
  browser-scoped single-space carry is regression-tested; the live
  Brave/Zen gate identification stays open in the plan.
- [#75](https://github.com/ahuray/badi/pull/75)
  [ADR 0007](docs/decisions/0007-retire-protocol-v1.md) retiring protocol
  v1: what keeps it alive and the removal order. Every shipped client
  already negotiates v2 only.
- [#74](https://github.com/ahuray/badi/pull/74) The `badi-session` helper
  (logind sleep and session state, UPower and power-profile state,
  GlobalShortcuts portal, keyring delivery of the style key) and
  [ADR 0005](docs/decisions/0005-desktop-signals-withhold-authority.md);
  battery scheduling (`badi battery`, default `quiet`); the Neovim plugin; the
  LibreOffice Writer spelling extension; Lab-only same-window context; opt-in
  sealed style learning (`badi style`). The same branch removed the dated
  parity snapshot and reports, recorded the overlay decision as
  [ADR 0006](docs/decisions/0006-grey-preview-is-an-overlay.md), turned the
  backlog into an ordered plan and split the largest source files (broker
  settings, engine tests, the Fcitx addon) into focused modules without
  behavior changes.
- [#73](https://github.com/ahuray/badi/pull/73) Installed preview checks and
  the Linux feature research behind #74.
- [#72](https://github.com/ahuray/badi/pull/72) Inline previews with longer
  negotiated reading windows and quiet grey overflow.

## 2026-10-12: parity work, speed and the Lab

- [#70](https://github.com/ahuray/badi/pull/70),
  [#71](https://github.com/ahuray/badi/pull/71) Conservative field-language
  selection, the AUR package candidate, observer lock-state handling and
  runtime verification; dismissal ends when a native field's text changes.
- [#51](https://github.com/ahuray/badi/pull/51)–[#55](https://github.com/ahuray/badi/pull/55),
  [#40](https://github.com/ahuray/badi/pull/40)–[#49](https://github.com/ahuray/badi/pull/49)
  Speed: a known field starts the model on the key, the session is reused
  while typing, each finished word shows while the rest generates; the first
  grey word reached about 220 ms (Omawrite) and 250 ms (Chromium) in the
  recorded samples.
- [#57](https://github.com/ahuray/badi/pull/57),
  [#58](https://github.com/ahuray/badi/pull/58),
  [#59](https://github.com/ahuray/badi/pull/59) Measured and not adopted:
  Q4_0, speculative decoding and a following-line prompt.
- [#61](https://github.com/ahuray/badi/pull/61)–[#66](https://github.com/ahuray/badi/pull/66)
  The preview follows scrolls, the panel polls only while open, Bash's
  preview timer runs only while a suggestion shows, and type-through waits
  for typed characters to land.
- [#37](https://github.com/ahuray/badi/pull/37),
  [#38](https://github.com/ahuray/badi/pull/38) Exact spelling replacement on
  native fields that support it; the grey preview uses the field's font.
- [#35](https://github.com/ahuray/badi/pull/35),
  [#36](https://github.com/ahuray/badi/pull/36) Mid-sentence suggestions stay
  off; Lab frame, field-context and warm-prefix runs.
- [#32](https://github.com/ahuray/badi/pull/32)–[#34](https://github.com/ahuray/badi/pull/34)
  Audit fixes: toolchain, editing contract, sandbox and Lab follow-ups.
- Records and maintenance: [#39](https://github.com/ahuray/badi/pull/39),
  [#56](https://github.com/ahuray/badi/pull/56),
  [#60](https://github.com/ahuray/badi/pull/60),
  [#67](https://github.com/ahuray/badi/pull/67)–[#69](https://github.com/ahuray/badi/pull/69).

## 2026-10-06: type-through and wider coverage

- [#28](https://github.com/ahuray/badi/pull/28),
  [#29](https://github.com/ahuray/badi/pull/29) Type-through, next-word
  acceptance and search-box quieting
  ([ADR 0004](docs/decisions/0004-type-through-carries-text-not-authority.md)).
- [#27](https://github.com/ahuray/badi/pull/27) Rich editors on any website;
  allowlist and blocklist modes for apps and sites.
- [#26](https://github.com/ahuray/badi/pull/26) Omarchy web apps, LibreOffice
  Writer and Grok Bot through IME-parity; a VS Code 1.140 fix.

## 2026-09-27: daily apps

- [#24](https://github.com/ahuray/badi/pull/24) Badi working in daily apps:
  IME-parity for Chromium, Brave, Zen, Codex, VS Code, Cursor and Discord with
  no browser extension ([ADR 0003](docs/decisions/0003-ime-parity-append-only.md));
  the Fcitx 5.1.22 compatibility frontend; a resident model service. The old
  evaluator, browser extension and legacy panel were removed, and the Lab
  worker became its own crate.

## 2026-08-30: foundation

- [#4](https://github.com/ahuray/badi/pull/4) Hardened foundation after an
  adversarial audit ([ADR 0001](docs/decisions/0001-same-uid-trust-boundary.md)).
- [#5](https://github.com/ahuray/badi/pull/5)–[#13](https://github.com/ahuray/badi/pull/13)
  Curated dependency updates and their CI policy.
