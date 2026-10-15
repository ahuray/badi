# What to consider next (2026-10-15)

A review after the work merged in #78–#84, ordered by what matters most. The
ordered backlog stays in [future plans](<future plans.md>); this file is the
reasoning behind the next choices and links there instead of repeating it.
Delete it once its points are decided or moved into the backlog.

## 1. Settle what just shipped

- **Review the seven pull requests.** They were written and verified in one
  long session, with CI and live trials but without a second reader. The
  riskiest parts for a review:
  - the terminal typed line, `lineKey()` in `adapters/fcitx5/src/addon_keys.cpp`;
  - Writer's trailing-space binding, `restoreObservedTrailingSpaces()`;
  - the spelling-only Writer target in the broker engine;
  - the widened replacement shape, which is identical in four validators.
- **Reinstall from `main`** after merging, so `badi doctor` names a clean commit
  instead of a modified branch: `scripts/install-desktop.py`,
  `scripts/install-editors.py` (same targets) and `scripts/install-omarchy-ui.py`.
- **Install the spelling dictionaries.** German, French and Spanish are turned
  on but correct nothing until `sudo pacman -S hunspell-de
  hunspell-fr-comprehensive hunspell-es_es`. `badi spelling` shows the state.
- **Fix the git credential helper** with `gh auth setup-git`. It still points
  at a removed `gh` 2.98.0.

## 2. Decisions only you can make

- **Terminal privacy.** Ghostty follows the typed line automatically. A
  terminal cannot mark a password prompt, so a passphrase of two or more
  words in an allowed terminal can reach the local model
  ([ADR 0009](docs/decisions/0009-terminals-use-the-typed-line.md)). The
  options:
  - keep it;
  - make terminals explicit-only (Ctrl+Shift+Space);
  - pause the line after prompts such as `password:`. Badi cannot read
    terminal output, so it cannot detect them.
- **Spelling languages beyond English are unqualified.** Dictionary candidates
  and the model's choice were tried on a handful of words, not measured. VISION
  asks for evidence before enabling a language's behavior. They are opt-in
  today. Decide whether that is enough, or whether each needs the held-out
  measurement in [Suggestion quality](<future plans.md#suggestion-quality>)
  first.
- **The `--text` debug trace** records what you write for 15 minutes in memory
  ([ADR 0008](docs/decisions/0008-debug-trace-may-hold-text-on-request.md)).
  Confirm that is the trade you want, or limit it to suggestions only.

## 3. Speed, the biggest gap you will feel

- **Missed suggestions are mostly time, not quality.** Live traces showed
  `budget_prefill` and `budget_stream` most often: the 550 ms typing budget ends
  before a whole word arrives.
  [Speed and footprint](<future plans.md#speed-and-footprint>) holds the
  options. A cached spelling prompt (its own llama.cpp slot) would also bring
  the spelling choice between candidates (1–2.5 s now) into typing budgets.
- **Overlay redraw while streaming.** Each longer version hides the preview and
  draws it again about 1 ms later. Check for flicker, then move the surface
  instead ([Interaction](<future plans.md#interaction>)).

## 4. Trials still owed

These are the steps already in the backlog, with what changed since:

- **Installed-flow matrix** (step 1): suspend and resume, a VT switch, a
  service restart and stale focus, now also covering Ghostty's typed line.
  After a restart it starts empty, so check that it stays quiet.
- **Neovim live** (step 2): streamed suggestions now grow in place, but they
  were tested only headless and by unit test.
- **Writer** (step 2): an ambiguous English correction end to end, and the
  new automatic hint with the dictionaries installed. Writer leaves the
  restored word selected after Ctrl+Z, so the next key replaces it. A Writer
  behavior worth noting in its runbook if it bothers you.
- **The panel:** a rounded-corner theme, a second monitor and fractional
  scaling. It was tried live only on Tokyo Night with square corners.
- **Zen's overlay fallback** and Telegram's host font
  ([Interaction](<future plans.md#interaction>)).

## 5. Engineering upkeep

- **A flaky CI step:** `observed-desktop.py` timed out once at "candidate
  before timed quiet" (#81) and passed on rerun. If it recurs, give that wait
  the same bound as its neighbours, or find the timing it races.
- **No QML test covers `SettingsView.qml`'s keyboard model.** The lifecycle
  harness covers only the client. A headless `qs` test that loads the view
  with a fake controller and drives `move()`/`activate()` would guard j/k,
  h/l and search.
- **One replacement-shape rule lives in four places:** `protocol.rs`, the
  schema, `text-safety.mjs` and `text.py`. Tests keep their cases equal by
  hand. A shared fixture file, like `orthographic-joiner-fixtures.json`, would
  make drift impossible.

## 6. Worth building after that

- More terminals on the typed-line path (kitty, foot, Alacritty), each after a
  live trial ([Remaining surfaces](<future plans.md#remaining-surfaces>)).
- Panel rows for per-app language overrides and style learning, which are
  still terminal-only.
- Spelling for Persian, which has no cased letters and needs its own word
  shape rather than the European one.
