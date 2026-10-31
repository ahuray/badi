# Badi

Badi (`بعدی`, Persian for “next”) is a local writing assistant for Linux,
starting with Omarchy on Hyprland. While you type, a local model suggests up to
four words in grey (`badi length short` shows one or two), and you
accept whatever is showing. Nothing leaves this
machine. Badi is pre-release software.

## Keys

| Key | Action |
| --- | --- |
| **Tab** | Accept only the next word, which the grey text draws a step brighter; the rest returns. Without a suggestion, Tab stays Tab. |
| **Ctrl+→** | Accept all of it. |
| Keep typing | Typing the suggestion's next letters keeps the rest. |
| **Escape** | Dismiss. |
| **Ctrl+Shift+Space** | Ask for a suggestion. On a shown one, show another that starts with a different word; after three, presses cycle through them. On selected text, check its grammar; press it again for a refined wording. **Tab** applies what it shows, **Escape** keeps your text. |
| **Ctrl+Shift+Tab** | Type a real Tab instead of accepting: the suggestion goes away and the app gets Tab. Where Badi would not take Tab, it stays the app's key (a browser's previous tab). |

Every surface uses that order: Obsidian with Ctrl/Command+→ for all of it,
Neovim in prose buffers after a typing pause (leaving Insert mode dismisses
it), and the Fcitx apps below. A spelling correction is still taken whole.
After Tab takes a word, a second Tab pressed before the rest returns is held
for it, so quick Tabs keep typing words instead of reaching the app.
Bash uses Ctrl-X then Tab to request and accept. In a Xournal++
text cell, Tab requests and Tab again accepts a word. Fcitx, Obsidian and Neovim
suggestions last up to 15 seconds; edits, focus changes and dismissal still
remove them immediately. Bash keeps its four-second preview. In the
observed apps below, Badi stays quiet on its own in file dialogs and very
narrow fields; Ctrl+Shift+Space still asks there. `badi escape-quiet 60` (or
the settings panel) also keeps a dismissed field quiet for a minute across
edits; asking explicitly ends that sooner. A next word you turned down (with
Escape, or by typing other letters twice) is not offered again after the same
three words in that app or site, until you ask for it there and take it;
Badi keeps only hashes of those spots, in memory, until it restarts.

A revision works on one line of up to 600 characters in English, German,
French, Spanish or Portuguese, wherever Badi may suggest (not in terminals).
It shows your text with the removed words struck through and the new ones
bold, and changes nothing until you press Tab. Its model, Qwen3 1.7B
(1.3 GB, Apache-2.0), starts on the first request, which takes a few
seconds, and stops after ten idle minutes; `badi revise` says whether it is
installed and `badi revise install` downloads it. A grammar check keeps your
wording and changes at most half of the words; neither mode may drop a
number, address or name, or answer in another language. In Chromium-based
apps it works in plain text fields, not yet in rich editors.

In the middle of a sentence Badi suggests only what fits before the next word:
the rest of a word you are typing, or words the model would write right before
the text that follows. A word you start right in front of another
(`the fin|report`) is finished with a space, so it stays apart (`al ` gives
`the final report`), and English stems that are words themselves, such as `fin`
or `rep`, are finished too. When the caret is already after a space and a word
follows, the suggestion keeps one space so it does not join that word. Where
grey text would cover those words, it shows in a small box under the line
instead, and that box keeps the separating space.

Grey text uses the field's font family, size and weight when accessibility
exposes them, and otherwise a size taken from the caret's glyph. The observer
aligns it using font metrics and disables the Hyprland preview entrance
animation. At a field edge, overflow appears as a compact grey line just below
or above the field. If that cannot fit either, Fcitx uses one unhighlighted
candidate without the help banner
([preview contract](adapters/accessibility/README.md#caret-geometry-and-preview)).
Fcitx 5.1.23
on this machine already supplies emoji: QuickPhrase (Super+semicolon, then a
word such as `smile`) and the Emoji module. They do not expand a colon
shortcode while you type. Badi does not add one. Replacing `:smile` would
delete text, which the append-only apps cannot do, and Badi stays out of the
way while Fcitx is composing.

## Where it works

Tested on the development workstation (Arch Linux, Hyprland 0.56, Fcitx
5.1.23) with disposable text in October 2026. A listed version is not a claim
about Linux in general. Badi needs Fcitx 5.1.23 or later on Hyprland:
earlier versions stop Chromium-based apps sending the text
around the caret.

| App | Notes | Tested |
| --- | --- | --- |
| Chromium and Brave Origin, including Omarchy's web apps (HEY, X, Basecamp, Zoom) in either browser | Any website you allow, including rich editors (ProseMirror, Lexical, Slate, Draft.js, Quill, CKEditor 5, TinyMCE, CodeMirror), iframes and shadow roots | Chromium 152–155, Brave Origin 1.96–1.98 |
| Zen and Zen Twilight | Grey preview; Ctrl+Z undoes an acceptance alone | Zen 1.23b, 1.23.3b; Zen Twilight 1.24t |
| VS Code, Cursor | Needs `"editor.editContext": false` in VS Code | VS Code 1.140 and 1.142, Cursor 3.21 |
| Codex desktop | Composer | 26.930 |
| LibreOffice Writer | Document paragraphs; Calc gets nothing. Spelling: the [Badi extension](adapters/libreoffice/README.md) shows a correction after a finished word, Alt+F7 shows one for the word at the caret, Alt+F7 again replaces it in one undo step | 26.8, 26.8.1 |
| Telegram | Composer (Qt Widgets): grey preview and Tab; as in Omawrite, no typo replacement with fcitx5-qt 5.1.16 | 7.3.2 |
| Omawrite | With fcitx5-qt 5.1.16 (here from 2026-10-18), Qt's input module withdraws the text around the caret after every key; Badi keeps suggesting in Qt fields it observes, but cannot replace a typo there ([details](adapters/fcitx5/README.md#tested-with)) | 0.5.0, 0.7.0 |
| Xournal++ text tool | Tab requests, exact native undo | 1.3.7, 1.3.8 |
| Obsidian | Badi plugin; inline preview, Tab, Ctrl+→, one-step undo and recovery after a broker restart (live runner, real model) | 1.13.7, 1.14.4 |
| Bash | Grey Readline preview and narrow English spelling fixes | Bash 5.3.15 in Ghostty 1.3.1 |
| Ghostty (any TUI, such as a coding agent's prompt) | `badi app com.mitchellh.ghostty on`; suggestions follow the line you type and show in a panel at the cursor; Tab types a word, Ctrl+→ all of it, a correction shows as `teh → the` and Tab fixes the typo, and an automatic miss stays quiet ([how](docs/decisions/0009-terminals-use-the-typed-line.md)) | Ghostty 1.3.1 |
| foot (Omarchy's default terminal) | `badi app foot on`; the line you type, as in Ghostty, including `teh → the` corrections | foot 1.28.0 |
| Neovim | Badi plugin; prose buffers, inline preview, one-step undo; no spelling fixes yet. Live with the real model under LazyVim in foot and Ghostty, including Ctrl+Shift+Space; in Ghostty it takes over from the typed line while focused | 0.12.5, 0.12.6 |
| Discord | Composer; install with `--observed-app discord`, then relaunch Discord | 1.0.166 |
| Grok Bot | Ready, but its composer is untested | 0.35 |
| Claude desktop | Ready, but its composer is untested; `badi app com.anthropic.claude on`, then relaunch it with the flag below | 2.42304 (Electron 44) |

**Not yet:**
- Firefox and Gecko builds other than Zen and Zen Twilight.
- Editors built on Chromium's EditContext (VS Code's default, CodeMirror on
  recent Chromium).
- A caret inside a list or quote.
- Terminals other than Ghostty and foot, and Fish/Zsh line editing.

See [future plans](<future plans.md>).

### How Badi edits Chromium-based apps and Zen

These apps give Badi no editor channel, so it reads the focused field through
its [accessibility observer](adapters/accessibility/README.md) and accepts with
one append, like typed text. Undo may merge the acceptance with your preceding
typing (Brave does; Zen does not), and a page that moves focus while you type
can redirect it like a keystroke
([decision](docs/decisions/0003-ime-parity-append-only.md)). Badi checks field
identity and caret position before showing a suggestion and again before
inserting it, and it never reads a field the toolkit marks as a password.

Spelling replaces the mistyped word in Bash, Obsidian, and native exact
fields such as Omawrite, Telegram and Xournal++. English is built in;
`badi spelling LANG on` adds German, French, Spanish, Italian, Dutch or Polish
from that language's Hunspell dictionary (`badi spelling` lists them and names
the package to install). A single dictionary candidate is offered at once;
when several fit, the local model picks one from the sentence, and text
confidently in a language that is not turned on is never corrected. In LibreOffice Writer the
[Badi extension](adapters/libreoffice/README.md) shows a correction after you
finish a misspelled word, or on Alt+F7 for the word at the caret, and replaces
it through Writer's own text and undo APIs; one Ctrl+Z restores the typo. In
the browsers above and in VS Code, Cursor, Codex, Claude, Discord and Grok Bot,
a typo shows as `→ address` in grey after the word. Tab deletes exactly the
typed typo and types the fix, once the observer confirms the field. Badi then
reads the app's text back: if the correction did not land where it should, a
notice says "Badi could not confirm that correction — check the text". A page
that moves focus at that moment can still split it, the typo deleted in one
field and the fix typed into another
([decision](docs/decisions/0011-checked-corrections-in-append-only-apps.md)).
In Chromium one Ctrl+Z restores the typo. In Omawrite 0.5.0 a spelling
replacement is two native undos: the first removes the correction and the
second restores the mistyped word. With fcitx5-qt 5.1.16, Qt drops the
deletion, so Omawrite 0.7.0 gets no spelling replacement. Telegram and
Xournal++ have not been retried.

Chromium-based apps must start with `--force-renderer-accessibility=complete`.
`install-desktop.py --observed-app chromium|brave-origin|chatgpt|code|com.anthropic.claude|cursor|discord|grok-bot`
adds it to that app's flags file, or for Discord to a user copy of its
launcher ([details](adapters/accessibility/README.md#installation-and-the-renderer-flag));
Zen needs nothing.

## Install

An [AUR package candidate](packaging/aur/README.md) is available for local
review; it has not been published. The current desktop installer remains:

On Omarchy with Fcitx 5, from this checkout:

```sh
python3 scripts/install-desktop.py      # model service, Fcitx addon, observer, session helper, `badi` command
python3 scripts/install-omarchy-ui.py   # bar mark and settings panel
npm ci                                  # once, for the Obsidian plugin's build
python3 scripts/install-editors.py --vault /path/to/vault --bash --neovim   # optional: Obsidian, Bash, Neovim
python3 scripts/install-editors.py --libreoffice   # optional: Writer spelling; close LibreOffice first
```

Prerequisites on the machine: a Wayland session whose user services run
under systemd (X11 is not supported; Badi's units start with
`graphical-session.target`, so a compositor launched from a tty needs
uwsm or a display manager), PipeWire with `pw-dump` (the screen-share
hold), Rust, cmake, ninja, a C++20 compiler, Fcitx 5
headers, PyGObject, GTK 4 and gtk4-layer-shell; Node.js for the editor
integrations (the Bash and Neovim bridges, the Obsidian plugin's build) and
the checks, not for the desktop install. The pinned llama.cpp runtime
needs x86_64 with AVX2 and glibc 2.34 or later.

### Debian and Ubuntu

On Debian 13 and Ubuntu 24.04 (x86-64), build a package with
`packaging/debian/build-deb.sh` and install it with `sudo apt install
./target/debian/badi_*.deb`, or run `python3 scripts/install-desktop.py`
from a checkout as above; there is no settings panel, so `badi` commands set
everything. Each user then downloads the writing assets, enables the
services, chooses Fcitx with `im-config -n fcitx5` and logs in again
([steps](packaging/debian/README.md)). Under GNOME or KDE Plasma, Fcitx's
manual fields, Obsidian, Neovim, Bash and the Writer spelling extension
work; Chromium-based apps, Zen, VS Code and Writer's grey preview need the
field observer, which runs only in a Hyprland session so far. Neither has
been tried in a real GNOME or KDE session yet.

The installers build user-local files, back up everything they replace and
restart Badi's services and, on Omarchy, Fcitx. The newest three backups of each installer
are kept, and the first backup — the files as they were before Badi — is kept
as well. `python3 scripts/badi_install.py restore DIR` rolls one back, reloads
user units when a service file was restored, and updates the install receipt
to those restored bytes. The desktop install needs an unlocked desktop;
`--broker-only` updates only the model service while locked.
Without a writing model, the installer lists the models that fit this
device, the recommended one first, each with its size, its result in Badi's
blind screen and its licence. You pick one; a model that comes with terms
(Gemma, Llama) downloads only after you accept them, and every download is
pinned and hash-checked. Unattended, `--model ID` picks the model and
`--accept-model-license` accepts its terms; without either, the installer
names the models that fit.
After the install, `badi model` switches models ([below](#choose-the-writing-model)). The pinned
llama.cpp runtime is `~/.local/share/badi/runtimes/llama-b11840`, with its
archive in `runtimes/downloads/`; on a new machine, fetch both first as
[Writing assets](packaging/aur/README.md#writing-assets) shows (the model
there is optional, since the installer lists the models that fit). `install-desktop.py`
checks those pinned hashes before it builds, and the broker checks them again
before it starts.

## Choose the writing model

```sh
badi model                          # the model in use, the one recommended for this device, every choice
badi model use gemma-3-1b           # download (pinned, hash-checked), switch and restart
badi model auto                     # follow the device recommendation again
```

Badi recommends, among the models that fit this device's memory and power
state, the one that gave the most useful suggestions in its blind development
screen; the panel's Model row offers the same choice. Gemma and Llama come
with their own licences (the Gemma Terms of Use, the Llama 3.2 Community
License), which `badi model use`, the installer and the settings panel show
and ask you to accept before downloading; each acceptance is noted in
`~/.local/state/badi/model-terms.json`. The broker
uses your choice when it is installed and fits, and the recommendation
otherwise ([how](docs/architecture/model-selection.md)).

## Choose where Badi writes

```sh
badi status
badi pause                          # persists across restarts; badi resume
badi pause toggle                   # what the session helper's shortcut runs
badi snooze                         # off in the focused app or site for 10 minutes; badi resume ends it
badi battery normal                 # on battery: keep mains scheduling (default: quiet)
badi screen-share suggest           # keep suggesting while the screen is shared (default: hold)
badi languages en de fr             # the languages you write in: suggestions and typo fixes in each
badi length short                   # one or two words per suggestion; normal (default) up to four
badi mid-line on                    # suggestions when a word follows the caret; off by default
badi escape-quiet 60                # a dismissal keeps its field quiet for a minute; off by default
badi suggest                        # suggestion languages and their healing
badi suggest fr on                  # also suggest in French (fr, es, pt: experimental)
badi suggest de healing off         # keep a typed space in German prompts; on|off|auto
badi language brave-origin de      # use German in Brave; en, de, fa, fr, es, pt, or reset
badi app telegram on                # allow an app
badi site https://example.com on    # allow a website (Chromium, Brave and Zen share it)
badi autostart on
```

Apps and websites each have a list mode. **Allowlist** (the default) writes only
where you allowed it; **blocklist** writes everywhere Badi can, except where you
blocked it:

```sh
badi apps blocklist                 # every supported app...
badi app discord off                # ...except Discord
badi sites allowlist                # only allowed websites
badi app list                       # mode and rules; likewise badi site list
badi app discord reset              # drop a rule; the app follows the mode
```

In app blocklist mode Badi opens only fields its observer can verify, so
Xournal++, which uses Tab requests, still needs `badi app xournalpp on`.
A field the toolkit marks as a password is refused in every mode. A normal
text field is not treated as sensitive from its label, placeholder, or
autocomplete hint. The **b** mark in the
Omarchy bar opens the same settings as a keyboard-driven popout (j/k, space,
h/l, `/` to search), and right-clicking it pauses Badi. The first time, it
asks which languages you write in. Afterwards it opens compact: the model's
live state, the words saved on each of the last seven days, your languages,
the suggestion length and context. **All settings** (or `a`) shows every
other setting, the acceptance meter and the apps that saved the most
([counter definitions](ui/omarchy-plugin/README.md#statistics)).

A language override takes precedence over detection and the adapter's hint
without granting access to an app or website. `badi language APP reset`
restores automatic selection. For longer text, Badi conservatively detects
English, German, Persian, French, Spanish or Portuguese from the authorized
field; short or uncertain text keeps the app or keyboard hint. Badi suggests
only in the languages you choose (`badi languages`, the panel, or one at a
time with `badi suggest`; English and German by default); French, Spanish
and Portuguese are experimental and not yet qualified. Portuguese has no
typo fixes yet, and Italian, Dutch and Polish have typo fixes only. Persian
is not offered in the panel; `badi suggest fa on` still turns it on.

**Healing** is what Badi does after you type a space: the model writes that
space itself, so it starts the next word as it normally would. Each model
heals the languages its blind review cleared (Gemma 3 1B: all six); `badi suggest LANG healing on|off` overrides that per language, and
`auto` returns to the model's choice. A language that is not healed shows
nothing right after a space. The panel's Suggestions rows set the same
choices.

### Sleep, session switches, screen sharing, battery and shortcuts

The `badi-session` user service, installed with the desktop installer,
withdraws every suggestion and grant before the machine sleeps and while
another session or VT is in the foreground; after resume, each field is
acquired afresh. While you share your screen (a desktop portal screencast,
as browsers, OBS and meeting apps start through `xdg-desktop-portal-hyprland`),
Badi shows no suggestions, so grey text with words from the window or ones it
learned never appears on someone else's screen; they return when sharing
stops. `badi status` and the settings panel say when that hold is on, and
`badi screen-share suggest` (or the panel's **While sharing the screen**
row) keeps suggesting instead. On battery or in the power-saver profile, Badi by default
starts the model only once you pause typing for 300 ms instead of on every
key; `badi battery explicit` shows suggestions only when you ask and `badi
battery normal` keeps mains behavior. The model stays loaded in every mode.
The service also offers **Pause or resume**, **Snooze** and **Open
settings** shortcuts through the desktop portal; bind them in Hyprland with
`hl.dsp.global("io.github.ahuray.badi:pause-toggle")`, `...:snooze` and
`...:open-settings` ([session helper](adapters/session/README.md)). A snooze
turns Badi off for 10 minutes in the app or website of the field you are
typing in, like Cotypist's snooze; it lasts until the model restarts at most,
and `badi resume` or the settings panel's Snoozed row ends it sooner.

### Context (opt-in)

```sh
badi context on                             # or answer the settings panel's question
badi context app com.anthropic.claude off   # per app; badi context site ORIGIN off per site
badi context status | forget | off
```

With context on, once you have typed five letters of a word Badi can finish it
from words in view:

- **The window:** text near the field in web pages and Chromium-based apps,
  never another field or a password, read when you start typing there and
  never kept.
- **Your earlier writing in that app:** words of six letters or more that you
  wrote in two separate fields, kept sealed with a key in your keyring until
  they go 90 days unused.
- **Your own draft.**

It finishes a name from an email's thread or a term from a coding session. A
different word in view that starts with the same five letters can also show as
a wrong suggestion; Tab is still needed to accept it
([decision](docs/decisions/0010-context-finishes-words.md)). Terminals never
teach it words. Turning context off for an app or site deletes its learned
words; `badi context forget` deletes them all.

### Style learning (opt-in)

```sh
badi style on                              # keyring key; nothing is kept yet
badi style site https://mail.example on 30 # keep samples from this allowed site for 30 days
badi style status | export | forget
```

When on, each accepted suggestion on a learning site keeps the sentence you
wrote before it, sealed with a key in your desktop keyring; sentences with
digits, email addresses or links are skipped. No suggestion uses the samples
yet: they are for measuring, in the Lab, whether your own style helps.
`badi style forget` deletes them and the key. Apps other than websites cannot
learn ([details](broker/README.md#style-learning-opt-in)).

### Usage statistics

```sh
badi stats                 # today, the last 7 days, each day and the leading apps and sites
badi stats --json          # the same counts for scripts
badi stats forget          # delete them
```

Badi counts, per day and per app or site, the suggestions it showed, the
acceptances that landed and the words they saved, and keeps those counts 90
days in `~/.local/share/badi/stats.json` (mode 0600). It keeps no text: no
word you typed, no suggestion and no window title. Nothing is counted while
Badi is paused or in an app or site it may not suggest in. Counts reach the
disk within a minute and whenever Badi pauses or stops, so a restart keeps
them ([details](broker/README.md#usage-statistics)).

Badi also uses them to stay quieter where its suggestions are seldom taken:
an app or site with at least 50 suggestions shown in the last 7 days and
fewer than 8 in 100 accepted gets only surer ones, and fewer than 4 in 100
surer still. It never gets bolder than the default, Ctrl+Shift+Space is never
quieted, and `badi stats forget` resets it with the counts
([details](broker/README.md#suggestion-limits)).

## When nothing appears

```sh
badi doctor                  # setup and startup problems
badi doctor --json           # the same diagnosis for tools; exit 1 on problems
badi debug on                # then: badi debug watch, and type in another app
badi debug on --text         # the same trace with context and suggestion text
badictl probe -              # run a disposable phrase through the live model
badi service restart
badi repair                  # restore Badi's lines that an Omarchy refresh dropped
```

Omarchy's refresh commands (`omarchy-refresh-hyprland`,
`omarchy-refresh-chromium`, a browser switch) replace `hyprland.lua` and the
browsers' flag files, which drops Badi's preview rule and accessibility flag.
`badi doctor` reports it, and the desktop installer adds an Omarchy hook that
restores those lines at each login and after `omarchy-update`.

`badi status` and `badi debug` name why nothing was shown, without recording
what you typed:

| Reason | Meaning |
| --- | --- |
| `app_disabled` | The app or site has no grant. |
| `request_abstained` | Unsupported language, a caret inside a word, a word after the caret while mid-sentence suggestions are off, or a lone unfinished word. |
| `budget_prefill`, `budget_stream` | The time budget ran out: 550 ms while typing, 1.2 s after an explicit request. |
| `model_abstained`, `output_rejected` | The model declined, or its words failed a language, dictionary, number or safety check, or did not fit before the text after the caret. |
| `stale`, `timeout`, `provider_error` | The text changed first, or the model was late or failed. |
| `low_confidence` | A word of the suggestion was too unlikely for the model to be sure, so it was withheld ([confidence gate](broker/README.md#suggestion-limits)). |
| `repeated` | You turned down that next word at the same spot in this app or site before: Escape once, or typing other letters twice ([turned-down words](broker/README.md#suggestion-limits)). |

Debug mode expires after 15 minutes. A crashed model service restarts with
backoff; a missing or oversized model stops it for `badi doctor` to explain.

### Debug trace

`badi debug on` writes one line per event to
`$XDG_RUNTIME_DIR/badi/debug-trace.jsonl`, from every part of Badi:

| Source | Events |
| --- | --- |
| `broker` | Each message to and from an adapter (`in context.changed`, `out suggestion.show`, `out suggestion.clear` with its reason, commits), `model start` / `model done` with latency, outcome and content-free token/timing counts, `carry` (type-through kept or the gate that declined), `spelling` (language, candidate count, whether the model chose a correction), `vocabulary` with context on (window and learned word counts, the outcome and its sources; the finished word only with `--text`) and `window_chars` on `context.changed` |
| `fcitx` | `key` with the action taken and whether a suggestion was showing; `ghost shown` as the `overlay` (grey text at the caret) or the `panel` (Fcitx's popup); `ghost hidden`; every blocked request or display with its reason |
| `observer` | Each field request; `preview drawn` with its placement (`inline`, `caret_line`, `chip`, `edge_panel`) and box, `preview declined` with its reason, `preview mapped`, `preview hidden` with its cause |
| `compositor` | While `badi debug watch` runs: whether Hyprland has the grey preview on screen, and where |
| `obsidian`, `neovim`, `terminal` | Editor activity; Obsidian's `ghost shown` and `hidden` |

`badi debug watch` follows it as readable lines, `badi debug log` prints it
(`--json` for the raw lines) and `badi debug off` deletes it. Without
`--text` the trace holds lengths, ids, reasons and positions only. With
`--text` it also holds the text before and after the caret, typed characters
and every suggestion; it stays in memory, ends after 15 minutes and is
deleted when the run ends (or by `badi debug off`, or at logout), a field
that reports itself sensitive contributes no text, and a password field's
keys are not recorded at all ([ADR 0008](docs/decisions/0008-debug-trace-may-hold-text-on-request.md)).
Test commands set `BADI_DEBUG_TRACE=off`, which keeps a process out of a
running trace.

## Model and languages

The broker runs the pinned model that fits this machine (Gemma 3 1B Q4_K_M
here) on the CPU and verifies its bytes at startup. English includes careful
partial-word completion; the other languages complete whole words only, and
French, Spanish and Portuguese are off until you turn them on. The
[Prediction Lab](evaluation/writing/README.md) (`npm run writing:lab`)
compares models and prompts on your own test drafts without changing the
installed model.

## For contributors

| Read | For |
| --- | --- |
| [AGENTS.md](AGENTS.md) | How to work here: architecture, invariants, checks |
| [VISION.md](VISION.md) | The product contract |
| [future plans.md](<future plans.md>) | The only backlog, with the ordered next steps |
| [CHANGELOG.md](CHANGELOG.md) | What shipped, by release |
| [docs/decisions](docs/decisions/) | Durable architecture decisions |
| Component runbooks | [broker](broker/README.md), [Fcitx addon](adapters/fcitx5/README.md), [observer](adapters/accessibility/README.md), [session helper](adapters/session/README.md), [editors](adapters/shared/README.md), [Bash](adapters/shell/README.md), [Neovim](adapters/neovim/README.md), [LibreOffice Writer](adapters/libreoffice/README.md), [Omarchy panel](ui/omarchy-plugin/README.md), [Prediction Lab](evaluation/writing/README.md) |

```sh
npm ci
npm run check
cargo test --workspace --all-features --locked
```

Badi's source and documentation are MIT-licensed ([LICENSE](LICENSE)). Its
binaries also contain open-source components under MIT, Apache-2.0,
MPL-2.0 and Zlib licences, listed with their texts in
[THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt) (regenerate it with
`python3 scripts/third-party-notices.py` after a dependency changes; CI
checks it). Model weights and other third-party files keep their own licences.
