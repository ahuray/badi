# Badi

Badi (`بعدی`, Persian for “next”) is a local writing assistant for Linux Wayland
sessions, starting with Omarchy on Hyprland and verified live on Arch Linux and
Debian 13 (KDE Plasma 6 and GNOME), with container-tested packaging for Ubuntu
24.04 (live Ubuntu 24.04 graphical sessions are not yet verified). While you
type in an app or editor you allow, a local model suggests up to four next words
(`badi length short` shows one or two), finishes half-typed words, offers
spelling fixes, and — when you select one line and press `Ctrl+Shift+Space` —
checks grammar, refines wording or offers a synonym. Nothing changes in your
document until you accept it (`Tab` for one word, `Ctrl+→` for all of it).
Inference and settings stay on your device. Badi is pre-release software.

**What Badi is not:**
- **Not a cloud service:** the installed model service runs without network
  access; explicit commands only fetch hash-pinned runtime and model files (plus
  optional Hugging Face searches in the standalone Prediction Lab;
  [Privacy](#privacy)).
- **Not a chat bot, agent, code-completion tool or unrequested rewriter:** Badi
  appends a short continuation (or applies one accepted typo fix or single-line
  revision); it never submits forms, executes commands or edits multiple places,
  and `Enter` stays yours.
- **Not an input-method replacement, keylogger, clipboard/screen reader or
  browser extension:** it runs alongside your active Fcitx 5 keyboard or input
  method (yielding whenever another input method composes) and inside opt-in
  editor plugins (Obsidian, Neovim, Bash, LibreOffice Writer).
- **Not universal Linux or X11 coverage:** X11 is unsupported, and any field
  where Badi cannot verify identity, sensitivity or safe insertion stays quiet
  ([product contract](VISION.md)).

## Keys

| Key | Action |
| --- | --- |
| **Tab** | Accept only the next word, which the grey preview or panel draws a step brighter; the rest returns. Spelling corrections and selection revisions apply whole. In manual Fcitx fields (such as Xournal++'s Text tool), `Tab` at the end of a phrase requests first and `Tab` again accepts a word. Without a suggestion or manual request, `Tab` stays `Tab`. |
| **Ctrl+→** | Accept all of it (`Cmd+→` also works in Obsidian; **Ctrl+Shift+Y** also accepts all of a visible suggestion in Fcitx apps). |
| Keep typing | Typing the suggestion's next letters keeps the rest with no new model call ([type-through](docs/decisions/0004-type-through-carries-text-not-authority.md)). |
| **Escape** | Dismiss the suggestion, alternative list or revision preview (in Neovim, leaving Insert mode dismisses a continuation). |
| **Ctrl+Shift+Space** | Ask for a suggestion. On a shown continuation in an Fcitx app, Obsidian or Neovim, list up to four other first words; **`1`–`4`** (or a click in Fcitx) shows that word's continuation, and pressing again lists the rest ([ADR 0014](docs/decisions/0014-numbered-alternative-words.md)). On selected text, check its grammar; press again for a refined wording, and again — for a word or 1–3 word phrase — for a synonym, then another synonym on each further press ([ADR 0012](docs/decisions/0012-explicit-revision-of-one-selection.md), [ADR 0013](docs/decisions/0013-synonyms-for-one-short-selection.md)). **Tab** applies what it shows, **Escape** keeps your text. |
| **Ctrl+Shift+Tab** | In Fcitx apps where Badi would take `Tab`, retire the suggestion and send the app a real `Tab`. Elsewhere it stays the app's key (such as a browser's previous tab). |
| **Bash** | **Ctrl-X** then **Tab** requests and accepts; **Ctrl-X** then **Escape** dismisses; **Ctrl-_** undoes; plain **Tab** stays Bash completion, and commands are never submitted ([Bash runbook](adapters/shell/README.md)). |
| **Writer spelling** | **Alt+F7** shows a correction for the word at or before the caret and **Alt+F7** again replaces it in one undo step. On GNOME, where `Alt+F7` moves windows (`begin-move`), use the automatic infobar or *Tools ▸ Add-Ons* and the infobar's **Replace** button (or rebind `begin-move`; [Writer runbook](adapters/libreoffice/README.md)). |

Every surface uses that word-first order: Obsidian, Neovim in prose buffers
after a typing pause, and the Fcitx apps below.
After `Tab` takes a word, a second `Tab` pressed before the rest returns is held
for it, so quick `Tab` presses keep typing words instead of reaching the app.
Fcitx, Obsidian and Neovim suggestions last up to 15 seconds; edits, focus
changes and dismissal still remove them immediately. Bash keeps its four-second
preview. In the observed apps below, Badi stays quiet on its own in file dialogs
and very narrow fields; `Ctrl+Shift+Space` still asks there. `badi escape-quiet
60` (or the settings panel) also keeps a dismissed field quiet for a minute
across edits; asking explicitly ends that sooner. A next word you turned down
(with `Escape`, or by typing other letters twice) is not offered again after the
same three words in that app or site, until you ask for it there and take it;
Badi keeps only hashes of those spots, in memory, until it restarts.

A revision works on one line of up to 600 characters in English, German,
French, Spanish or Portuguese, in supported Fcitx fields, Obsidian notes and
Neovim prose buffers. In Neovim, select text with characterwise Visual mode (`v`), then
press `Ctrl+Shift+Space`; a marked preview appears beside the selection and `Tab`
replaces the selection in one undo step. Linewise, blockwise and multiline
selections are refused. In Obsidian, select text on one line and use the same
chord, or the grammar, refinement and synonym commands in the command palette;
`Tab` applies the whole revision in one undo step. Bash does not offer selection revision.
The preview marks removed and added text, and changes nothing until you
press `Tab`. Its model, Qwen3 1.7B
(1.3 GB, Apache-2.0), starts on the first request, which takes a few
seconds, and stops after ten idle minutes; `badi revise` says whether it is
present and `badi revise install` verifies or repairs it, downloading it when
needed. The panel's **Revisions** row offers the same setup and recovery.
File presence alone does not prove that the model can run. A grammar check keeps your
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

On Hyprland, grey text uses the field's font family, size and weight when
accessibility exposes them, and otherwise a size taken from the caret's glyph.
The observer aligns it using font metrics and disables the Hyprland preview
entrance animation. At a field edge, overflow appears as a compact grey line
just below or above the field. If that cannot fit either (or on KDE Plasma and
GNOME, where caret geometry is not yet calibrated), Fcitx uses one unhighlighted
candidate without the help banner
([preview contract](adapters/accessibility/README.md#caret-geometry-and-preview)).
Fcitx 5 already supplies emoji: QuickPhrase (`Super+;`, then a word such as
`smile`) and the Emoji module. They do not expand a colon shortcode while you
type. Badi does not add one: replacing `:smile` would delete text, which the
append-only apps cannot do, and Badi stays out of the way while Fcitx is
composing.

## Where it works

Verified with disposable text on Omarchy 4.5 / Arch Linux (Hyprland 0.56, Fcitx
5.1.23; [Fcitx matrix](adapters/fcitx5/README.md#tested-with),
[observer matrix](adapters/accessibility/README.md#tested-with),
[Arch package matrix](packaging/aur/README.md#tested-with)) and in Debian 13.7
Wayland VMs (KDE Plasma 6.3.6 and GNOME 48.7, Fcitx 5.1.12;
[Debian and Ubuntu matrix](packaging/debian/README.md#tested-with)). A listed
version is evidence for that build, not a claim about Linux in general. On
Hyprland (and other `input-method-v2` compositors), Badi needs Fcitx 5.1.23 or
later so Chromium-based apps keep sending text around the caret; GNOME and KDE
Plasma use their own Fcitx frontends.

| App | Notes | Tested |
| --- | --- | --- |
| Chromium and Brave Origin, including `--app` web apps (such as Omarchy's HEY, X, Basecamp, Zoom) in either browser | Any website you allow, including rich editors (ProseMirror, Lexical, Slate, Draft.js, Quill, CKEditor 5, TinyMCE, CodeMirror), iframes and shadow roots ([Fcitx matrix](adapters/fcitx5/README.md#tested-with), [observer matrix](adapters/accessibility/README.md#tested-with), [Debian matrix](packaging/debian/README.md#tested-with)) | Chromium 152–156, Brave Origin 1.96–1.99 |
| Zen and Zen Twilight | Grey preview on Hyprland; `Ctrl+Z` undoes an acceptance alone ([Fcitx matrix](adapters/fcitx5/README.md#tested-with), [observer matrix](adapters/accessibility/README.md#tested-with)) | Zen 1.23b, 1.23.3b; Zen Twilight 1.24t, 1.25t |
| VS Code, Cursor | Needs `"editor.editContext": false` in VS Code ([Fcitx matrix](adapters/fcitx5/README.md#tested-with)) | VS Code 1.140–1.143, Cursor 3.21 |
| Codex desktop | Composer ([Fcitx matrix](adapters/fcitx5/README.md#tested-with)) | 26.930 |
| LibreOffice Writer | Document paragraphs (`libreoffice-gtk3` with `SAL_USE_VCLPLUGIN=gtk3` and `GTK_IM_MODULE=` unset on Plasma/GNOME; Calc gets nothing). Spelling: the [Badi extension](adapters/libreoffice/README.md) shows a correction after a finished word (or on `Alt+F7` / *Tools ▸ Add-Ons*) and replaces it via `Alt+F7` or the infobar's **Replace** button (needed on GNOME where `Alt+F7` moves windows) in one undo step ([Fcitx matrix](adapters/fcitx5/README.md#tested-with), [Writer matrix](adapters/libreoffice/README.md#tested-with), [Debian matrix](packaging/debian/README.md#tested-with)) | 25.2 (Debian 13), 26.8, 26.8.1 |
| Telegram | Composer (Qt Widgets): grey preview and `Tab`; as in Omawrite, no typo replacement with `fcitx5-qt` 5.1.16 ([Fcitx matrix](adapters/fcitx5/README.md#tested-with)) | 7.3.2 |
| Omawrite | With `fcitx5-qt` 5.1.16, Qt's input module withdraws the text around the caret after every key; Badi keeps suggesting in Qt fields it observes, but cannot replace a typo there ([Fcitx matrix](adapters/fcitx5/README.md#tested-with)) | 0.5.0, 0.7.0, 0.7.2 |
| Xournal++ text tool, Kate, Mousepad, GNOME Text Editor | Manual native Fcitx path (`badi app xournalpp|kate|org.kde.kate|mousepad|gnome-text-editor on`): `Tab` at the end of a phrase requests, `Tab` again accepts a word, exact native undo (`GTK_IM_MODULE=fcitx` for GTK apps Xournal++, Mousepad and GNOME Text Editor; [Fcitx matrix](adapters/fcitx5/README.md#tested-with), [Debian matrix](packaging/debian/README.md#tested-with)) | Xournal++ 1.3.7–1.3.8, Kate 25.04, Mousepad 0.6, GNOME Text Editor 48.3 |
| Obsidian | Badi plugin; inline preview, `Tab`, `Ctrl+→`, one-step undo, recovery after a broker restart, and explicit selection revision with marked changes and native undo ([editor matrix](adapters/shared/README.md), [revision limits](adapters/shared/README.md#obsidian-selection-revision)) | 1.13.7, 1.14.4 |
| Bash | Grey Readline preview and narrow English spelling fixes ([Bash matrix](adapters/shell/README.md#verification), [Debian matrix](packaging/debian/README.md#tested-with)) | Bash 5.3.15–5.3.20 in Ghostty 1.3.1–1.4.0 and Konsole |
| Ghostty (any TUI, such as a coding agent's prompt) | `badi app com.mitchellh.ghostty on`; suggestions follow the line you type and show in a panel at the cursor; `Tab` types a word, `Ctrl+→` all of it, a correction shows as `teh → the` and `Tab` fixes the typo, and an automatic miss stays quiet ([ADR 0009](docs/decisions/0009-terminals-use-the-typed-line.md), [Fcitx matrix](adapters/fcitx5/README.md#tested-with)) | Ghostty 1.3.1, 1.4.0 |
| foot (Omarchy's default terminal) | `badi app foot on`; the line you type, as in Ghostty, including `teh → the` corrections ([Fcitx matrix](adapters/fcitx5/README.md#tested-with)) | foot 1.28.0 |
| Neovim | Badi plugin; prose buffers, inline preview, exact spelling replacement, characterwise (`v`) selection revision and one-step undo; in Ghostty or foot it takes over from the typed line while focused ([Neovim matrix](adapters/neovim/README.md#tested-with), [Debian matrix](packaging/debian/README.md#tested-with)) | 0.10.4 (Debian 13), 0.12.5, 0.12.6 |
| Discord | Composer; add the renderer flag below (`--observed-app discord` on Omarchy), then relaunch Discord ([Fcitx matrix](adapters/fcitx5/README.md#tested-with)) | 1.0.166 |
| Grok Bot | Composer; `badi app grok-bot on`, then relaunch it with the renderer flag below ([Fcitx matrix](adapters/fcitx5/README.md#tested-with)) | 1.1.0 |
| Claude desktop | Composer and window words; `badi app com.anthropic.claude on`, then relaunch it with the renderer flag below ([Fcitx matrix](adapters/fcitx5/README.md#tested-with), [observer matrix](adapters/accessibility/README.md#tested-with)) | 5.11492.5 |

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

Spelling replaces the mistyped word in Bash, Obsidian and Neovim, and on native
exact Fcitx fields when the toolkit honours surrounding-text deletion (verified
in Omawrite 0.5.0). English is built in;
`badi spelling LANG on` adds German, French, Spanish, Italian, Dutch or Polish
from that language's Hunspell dictionary (`badi spelling` lists them and names
the package to install). A single dictionary candidate is offered at once;
when several fit, the local model picks one from the sentence, and text
confidently in a language that is not turned on is never corrected. If spelling
finds a possible typo but cannot offer a correction, Badi withholds the
continuation until you resolve that word. In LibreOffice Writer the
[Badi extension](adapters/libreoffice/README.md) shows a correction after you
finish a misspelled word, or on `Alt+F7` for the word at the caret, and replaces
it through Writer's own text and undo APIs; one `Ctrl+Z` restores the typo. In
the browsers above and in VS Code, Cursor, Codex, Claude, Discord and Grok Bot,
a typo shows as `→ address` in grey after the word (`adress → address` in the
panel). `Tab` deletes exactly the typed typo and types the fix, once the
observer confirms the field. Badi then reads the app's text back: if the
correction did not land where it should, a notice says "Badi could not confirm
that correction — check the text". A page that moves focus at that moment can
still split it, the typo deleted in one field and the fix typed into another
([decision](docs/decisions/0011-checked-corrections-in-append-only-apps.md)).
In Chromium one `Ctrl+Z` restores the typo. In Omawrite 0.5.0 a spelling
replacement took two native undos: the first removed the correction and the
second restored the mistyped word. With `fcitx5-qt` 5.1.16, Qt drops
surrounding-text deletion, so Omawrite 0.7.x and Telegram get no spelling
replacement until `fcitx5-qt` is updated; Xournal++ spelling replacement has not
been retried.

Chromium-based apps must start with `--force-renderer-accessibility=complete`.
On Omarchy, `python3 scripts/install-desktop.py --observed-app
chromium|brave-origin|chatgpt|code|com.anthropic.claude|cursor|discord|grok-bot`
adds it to that app's flags file, or for Discord to a user copy of its launcher.
On other desktops, add `--force-renderer-accessibility=complete` directly to
that app's flags file in `~/.config/` or to its launcher
([details](adapters/accessibility/README.md#installation-and-the-renderer-flag)).
Zen needs no flag.

## Install

**System requirements:**
- **CPU and OS:** `x86_64` Linux with AVX2 and `glibc` 2.34 or later (required
  by the pinned `llama.cpp` CPU runtime).
- **Session:** a Wayland session whose user services run under `systemd --user`
  with `graphical-session.target` active (X11 is not supported; a compositor
  started from a TTY needs a display manager, `uwsm`, or `systemctl --user
  start graphical-session.target` in its startup), plus PipeWire with `pw-dump`
  (used to hold suggestions during screen sharing).
- **Input method and observer:** Fcitx 5 (`>= 5.1.23` on Hyprland and other
  `input-method-v2` compositors; `>= 5.1.7` on GNOME and KDE Plasma), AT-SPI2,
  PyGObject and GTK 4 (plus `gtk4-layer-shell` for Hyprland's grey overlay).
- **Building from a checkout:** `git`, Rust (`>= 1.85`; `rustup` uses the
  pinned toolchain in [rust-toolchain.toml](rust-toolchain.toml)), `cmake`,
  `ninja`, a C++20 compiler, Fcitx 5 development headers and `nlohmann-json`.
  Node.js is needed only for the editor integrations (`--vault` needs Node
  `>= 22.20` and `npm ci`; `--bash` also needs `gcc` and Bash/Readline headers)
  and repository checks, not for the desktop install.

**Omarchy and Arch Linux:**

Install build and runtime packages on Arch if needed:

```sh
sudo pacman -S --needed git rust cmake ninja nlohmann-json fcitx5 gtk4 gtk4-layer-shell \
  python python-gobject python-cairo at-spi2-core pipewire curl
```

A local [Arch package candidate](packaging/aur/README.md) (`makepkg` in
`packaging/aur/`, [tested with](packaging/aur/README.md#tested-with)) is
available for review; it is not yet published on the AUR. To install user-locally
from this checkout (make sure `~/.local/bin` is on your `PATH`):

1. Fetch the pinned `llama.cpp` `b11840` runtime into
   `~/.local/share/badi/runtimes/` (with its archive in `runtimes/downloads/`)
   using the first block in [Writing assets](packaging/aur/README.md#writing-assets).
   `scripts/install-desktop.py` verifies those hashes before building, and the
   broker verifies them again at startup.
2. Build and install the desktop services and Fcitx addon (on Omarchy, run from
   an unlocked session; `--broker-only` updates only the broker while locked):

   ```sh
   python3 scripts/install-desktop.py      # model service, Fcitx addon, observer, session helper, `badi` command
   python3 scripts/install-omarchy-ui.py   # Omarchy only: bar mark and settings panel
   ```

   Without an installed writing model, `install-desktop.py` lists the catalog
   models that fit your device (recommended first, with size, blind-screen score
   and licence), asks you to accept any Gemma or Llama terms, and downloads and
   verifies your pick. Unattended, pass `--model ID` (and
   `--accept-model-license` for Gemma or Llama). On plain Arch without Omarchy,
   skip `install-omarchy-ui.py`: the installer writes
   `~/.config/environment.d/50-badi-fcitx5.conf` so Fcitx loads the addon at
   your next login ([panel runbook](ui/omarchy-plugin/README.md),
   [tested with](ui/omarchy-plugin/README.md#tested-with)).
3. Optional editor integrations:

   ```sh
   npm ci                                  # once, for the Obsidian plugin's build
   python3 scripts/install-editors.py --vault /path/to/vault --bash --neovim
   python3 scripts/install-editors.py --libreoffice   # close LibreOffice first
   ```

### Debian and Ubuntu

On Debian 13 (`trixie`) and Ubuntu 24.04 LTS (`noble`, `x86-64`), build a
package with `packaging/debian/build-deb.sh` and install it with `sudo apt
install ./target/debian/badi_*.deb`, or run `python3 scripts/install-desktop.py`
from a checkout after fetching the pinned runtime; there is no Omarchy panel, so
`badi` commands manage every setting. Each desktop user then fetches the
[writing assets](packaging/aur/README.md#writing-assets), enables the user
services, selects Fcitx 5 (`im-config -n fcitx5`, or KDE Plasma's Virtual
Keyboard setting) and logs out and back in
([setup and build dependencies](packaging/debian/README.md)). Under GNOME or KDE
Plasma 6, manual Fcitx fields, Obsidian, Neovim, Bash and the Writer spelling
extension work directly; Chromium-based apps, Zen, VS Code and GTK3 Writer
(`libreoffice-gtk3` with `SAL_USE_VCLPLUGIN=gtk3` and `GTK_IM_MODULE=` unset)
use the field observer, which runs in Hyprland, KDE Plasma 6
([ADR 0015](docs/decisions/0015-kwin-script-reports-plasma-focus.md)) and GNOME
(enable `badi-focus@ahuray.github.io` after logging in again;
[ADR 0016](docs/decisions/0016-gnome-shell-extension-reports-focus.md),
[ADR 0017](docs/decisions/0017-gnome-ibus-takes-observer-app-id.md)). On Plasma
and GNOME, observed fields show suggestions in Fcitx's panel without the grey
overlay (`gnome-shell-extension-kimpanel` on GNOME), verified live in Debian
13.7 Wayland VMs ([tested with](packaging/debian/README.md#tested-with)). Grey
preview calibration on KWin and Mutter, and live Ubuntu 24.04 graphical
sessions, remain unverified.

**Backups and rollback:** each installer backs up everything it replaces under
`~/.local/state/badi/` (`install-backups/`, `ui-backups/`, `editor-backups/`),
keeping the newest three ordinary backups plus the pinned original backup
directory from before Badi (with `accessibility-*.json` receipts from pruned
runs saved under `notes/`). `python3 scripts/badi_install.py restore DIR` rolls
one back, reloads user units when a service file was restored, and updates the
install receipt ([details](adapters/fcitx5/README.md#install-and-roll-back)).

### First run

1. **Finish session setup and check health.** On Omarchy, the installers restart
   Badi's units and Fcitx right away. On Arch (without Omarchy), Debian and
   Ubuntu, log out and back in so Fcitx loads the addon (on GNOME, then run
   `gnome-extensions enable badi-focus@ahuray.github.io`). For a packaged install
   (`badi-git` or `.deb`), complete its model activation step first
   ([Arch](packaging/aur/README.md#user-activation-and-removal),
   [Debian/Ubuntu](packaging/debian/README.md#use-it)) so a verified catalog GGUF
   is in `~/.local/share/badi/models/` before any command starts the broker
   (`badi doctor` inspects passively and never starts an idle broker; `badi
   status`, `badi languages`, `badi app`, `badi site` and `badi resume` start
   `badi-broker.service` on demand):

   ```sh
   badi doctor
   ```

2. **Choose languages, allow apps or websites, and unpause if packaged** (once a
   writing model is installed). `scripts/install-desktop.py` starts **unpaused**
   with only Omawrite and Xournal++ allowed; the Arch and `.deb` packages start
   **paused** with every app and website denied.
   - **On Omarchy:** click the **b** mark in the bar (or run `badi settings`) to
     answer the one-time prompts for your writing languages and allowed websites
     and apps; on a packaged install, also turn on the pause switch (or right-click
     **b**, or run `badi resume`).
   - **From the terminal (any desktop):** set your languages, allow the apps or
     websites you want, unpause (harmless if already unpaused), and inspect status:

     ```sh
     badi languages en de
     badi app omawrite on                  # or nvim, obsidian, bash, libreoffice-writer, ...
     badi site https://example.com on      # or: badi sites blocklist (every http(s) site except blocked ones)
     badi resume
     badi status
     ```

3. **Enable renderer accessibility for Chromium-based apps** (if using Chromium,
   Brave Origin, VS Code, Cursor, Codex, Claude, Discord or Grok Bot): run
   `python3 scripts/install-desktop.py --observed-app APP` on Omarchy or add
   `--force-renderer-accessibility=complete` to the app's flags file on other
   desktops ([flags](adapters/accessibility/README.md#installation-and-the-renderer-flag)),
   then relaunch the app. In VS Code, also set `"editor.editContext": false`.
4. **Type in an allowed field.** `badi-broker.service` starts on demand on the
   first typed key in an observed, terminal typed-line, Obsidian or Neovim
   field, and on the first explicit request in manual native Fcitx fields (`Tab`
   or `Ctrl+Shift+Space`, where the first `Tab` requests and the next `Tab`
   accepts a word) or Bash (`Ctrl-X` then `Tab`). `badi autostart on` starts it
   at login instead, and once loaded it stays up until logout or `badi service
   stop`. Once a suggestion shows, press `Tab` to accept one word, `Ctrl+→` for
   all of it, or `Escape` to dismiss.

## Choose the writing model

```sh
badi model                          # current model, recommendation and its reasons, every choice
badi model use gemma-3-1b           # download (pinned, hash-checked), switch and restart
badi model auto                     # follow the device recommendation again
```

| Model ID | Artifact | Download | Licence | Notes |
| --- | --- | --- | --- | --- |
| `gemma-3-1b` | Gemma 3 1B PT Q4_K_M | 0.81 GB | [Gemma Terms of Use](https://ai.google.dev/gemma/terms) | Recommended on balanced hardware; heals all six writing languages |
| `llama-3.2-1b` | Llama 3.2 1B Q4_K_M | 0.81 GB | [Llama 3.2 Community License](https://www.llama.com/llama3_2/license/) | Balanced alternative |
| `qwen3-1.7b` | Qwen3 1.7B Q4_K_M | 1.28 GB | Apache-2.0 | Balanced Apache-2.0 option; also used on demand for `badi revise` |
| `qwen3-0.6b` | Qwen3 0.6B Q8_0 | 0.64 GB | Apache-2.0 | Compact tier (low-memory or battery-constrained fit) |

Badi recommends, among the models that fit your device's memory and power state,
the one that gave the most useful suggestions in its blind development screen;
the panel's **Model** row offers the same choice. Gemma and Llama come with
their own terms, which `badi model use`, the installer and the settings panel
show and ask you to accept before downloading (unattended: `badi model use ID
--accept-license`, or `install-desktop.py --model ID --accept-model-license`);
each acceptance is recorded locally in `~/.local/state/badi/model-terms.json`.
The broker uses your choice when it is installed and fits, and the
recommendation otherwise ([how](docs/architecture/model-selection.md)).

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
badi language brave-origin de       # use German in Brave; en, de, fa, fr, es, pt, or reset
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

In app blocklist mode Badi opens only fields its observer can verify, so manual
Fcitx apps such as Xournal++ still need an explicit rule (`badi app xournalpp
on`). A field the input method marks as a password or special purpose is refused
in every Fcitx mode, and on observed fields the accessibility observer also
refuses credential-labelled fields
([sensitive fields](adapters/accessibility/README.md#sensitive-fields)). The
**b** mark in the Omarchy bar opens the same settings as a keyboard-driven
popout (`j`/`k`, `Space`, `h`/`l`, `/` to search), and right-clicking it pauses
Badi. On first open it asks which European languages you write in, then where to
suggest (all websites or listed sites, and which installed apps to allow).
Afterwards it opens compact: the model's live state, the words saved on each of
the last seven days, your languages, the suggestion length and context. **All
settings** (or `a`) shows every other setting, the acceptance meter and the apps
that saved the most ([counter definitions](ui/omarchy-plugin/README.md#statistics),
[panel matrix](ui/omarchy-plugin/README.md#tested-with)).

A language override takes precedence over detection and the adapter's hint
without granting access to an app or website. `badi language APP reset`
restores automatic selection, and the panel's **App languages** rows offer the
same European choices and **Auto**. For longer text, Badi conservatively detects
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

The `badi-session` user service, installed with the desktop installer and
packages, withdraws every suggestion and grant before the system sleeps and
while another session or VT is in the foreground; after resume, each field is
acquired afresh ([session helper](adapters/session/README.md),
[tested with](adapters/session/README.md#tested-with)). While you share your
screen (a desktop portal screencast in PipeWire, as browsers, OBS and meeting
apps start through `xdg-desktop-portal-hyprland`), Badi shows no suggestions, so
grey text with words from the window or ones it learned never appears on someone
else's screen; they return when sharing stops. `badi status` and the settings
panel say when that hold is on, and `badi screen-share suggest` (or the panel's
**While sharing the screen** row) keeps suggesting instead. On battery or in the
power-saver profile, Badi by default starts the model only once you pause typing
for 300 ms instead of on every key; `badi battery explicit` shows suggestions
only when you ask and `badi battery normal` keeps mains behavior. The model
stays loaded in every mode. The service also offers **Pause or resume**,
**Snooze** and **Open settings** shortcuts through the desktop portal; bind them
in Hyprland with `hl.dsp.global("io.github.ahuray.badi:pause-toggle")`,
`...:snooze` and `...:open-settings`. A snooze turns Badi off for 10 minutes in
the app or website of the field you are typing in, like Cotypist's snooze; it
lasts until the model restarts at most, and `badi resume` or the settings
panel's Snoozed row ends it sooner.

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
a wrong suggestion; `Tab` is still needed to accept it
([decision](docs/decisions/0010-context-finishes-words.md)). Terminals never
teach it words. Turning context off for an app or site deletes its learned
words; `badi context forget` deletes them all.

### Style learning (opt-in)

```sh
badi style on                               # keyring key; nothing is kept yet
badi style app omawrite on 30               # keep samples from this allowed app for 30 days
badi style site https://mail.example on 30  # keep samples from this allowed site for 30 days
badi style status | export | forget
```

When on, each accepted suggestion in a learning app or site keeps the sentence
you wrote before it, sealed with a key in your desktop keyring; sentences with
digits, email addresses or links are skipped. The panel's **Style learning**
section offers the global switch, per-app and per-site switches (30-day
retention; custom 1–90 day counts stay in `badi style`) and **Forget style
samples**. Desktop suggestions do not use
the samples yet: the Lab offers [three style strengths](evaluation/writing/README.md#style-comparison)
for comparing explicitly supplied examples against style off.
`badi style forget` deletes them and the key. Terminals and shells never learn
([details](broker/README.md#style-learning-opt-in)).

### Usage statistics

```sh
badi stats                 # today, the last 7 days, each day and the leading apps and sites
badi stats --json          # the same counts for scripts
badi stats forget          # delete them
```

Badi counts, per day and per app or site, the suggestions it showed, the
acceptances that landed and the words they saved, and keeps those counts 90
days in `~/.local/share/badi/stats.json` (mode `0600`). It keeps no text: no
word you typed, no suggestion and no window title. Nothing is counted while
Badi is paused or in an app or site it may not suggest in. Counts reach the
disk within a minute and whenever Badi pauses or stops, so a restart keeps
them ([details](broker/README.md#usage-statistics)).

Badi also uses them to stay quieter where its suggestions are seldom taken:
an app or site with at least 50 suggestions shown in the last 7 days and
fewer than 8 in 100 accepted gets only surer ones, and fewer than 4 in 100
surer still. It never gets bolder than the default, `Ctrl+Shift+Space` is never
quieted, and `badi stats forget` resets it with the counts
([details](broker/README.md#suggestion-limits)).

## Privacy

- **Local sandbox and field guards.** `badi-broker.service` runs `badi-broker`
  and `llama-server` with `PrivateNetwork=yes` (no IP network; same-UID Unix
  sockets only in `0700` `$XDG_RUNTIME_DIR/badi`,
  [ADR 0001](docs/decisions/0001-same-uid-trust-boundary.md)),
  `ProtectHome=read-only` outside `~/.config/badi` and `~/.local/share/badi`,
  and `LimitCORE=0` (no core dumps). Adapters check exact app or website policy
  before reading any text; Fcitx refuses input-method password and special-purpose
  fields, and on observed fields the accessibility observer also refuses
  accessible names or descriptions that name credentials (unobserved manual
  Fcitx fields see only input-method flags;
  [context firewall](VISION.md#context-firewall),
  [sensitive fields](adapters/accessibility/README.md#sensitive-fields)).
- **What stays in memory and what touches disk.** Ordinary drafts, nearby window
  words, active snoozes and hashes of turned-down words stay in memory only.
  Persistent files are user-private (`0600` in `0700` directories):
  `~/.config/badi/settings.json` (settings and permissions),
  `~/.local/share/badi/stats.json` (90-day counts only, zero prose;
  [Usage statistics](#usage-statistics)),
  `~/.local/share/badi/personalization.json` (per-app and per-site daily
  shown/dismissed/accepted counts with zero prose; cleared with `badictl memory
  clear`), and — only when you opt in — `learned-words.json`
  ([Context](#context-opt-in)) and `style.json`
  ([Style learning](#style-learning-opt-in)), whose word lists and sentences are
  XChaCha20-Poly1305 encrypted with your keyring key while each record's app or
  site identity and day stay in the clear for key-free expiry and per-subject
  deletion, never collected from terminals or shells.
- **What the debug trace may hold.** Tracing is off by default, lives in RAM
  (`$XDG_RUNTIME_DIR/badi/debug-trace.jsonl`), and ends automatically after 15
  minutes (`badi debug off` or logout deletes it sooner). Plain `badi debug on`
  records message types, lengths, opaque ids, timings and refusal reasons — no
  typed or suggested text. Only `badi debug on --text` records text before and
  after the caret, typed characters and suggestions
  ([Debug trace](#debug-trace),
  [ADR 0008](docs/decisions/0008-debug-trace-may-hold-text-on-request.md));
  sensitive fields still contribute no text and password fields record no keys.
- **Downloads and model licences.** `badi-broker` has no network code. Network
  requests happen only when you explicitly run the
  [writing-asset commands](packaging/aur/README.md#writing-assets) (`llama.cpp`
  `b11840`, MIT, from GitHub Releases), download a model through the installer,
  `badi model use ID`, `badi revise install` or the panel (from Hugging Face,
  pinned by revision, byte size and SHA-256), or search Hugging Face in the
  standalone [Prediction Lab](evaluation/writing/README.md). Model weights keep
  their own licences — **Apache-2.0** (`qwen3-0.6b`, `qwen3-1.7b`), the
  **Gemma Terms of Use** (`gemma-3-1b`) and the **Llama 3.2 Community License**
  (`llama-3.2-1b`) — and Gemma or Llama weights download only after you accept
  their terms ([Choose the writing model](#choose-the-writing-model)).

## When nothing appears

```sh
badi doctor                                           # setup and startup problems
badi doctor --json                                    # the same diagnosis for tools; exit 1 on problems
badi debug on                                         # then: badi debug watch, and type in another app
badi debug on --text                                  # the same trace with context and suggestion text
printf 'Please find attached the' | badictl probe -   # run a disposable phrase through the live model
badi service restart
badi repair                                           # restore Badi's lines that an Omarchy refresh dropped
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
| `repeated` | You turned down that next word at the same spot in this app or site before: `Escape` once, or typing other letters twice ([turned-down words](broker/README.md#suggestion-limits)). |
| `typo_suspected` | Spelling found a possible typo, but this field cannot replace it or no correction was found. Badi withholds the continuation until you resolve the word. |

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

The broker runs the pinned catalog model that fits your device (`Gemma 3 1B
Q4_K_M` by default on balanced x86-64 hardware) on the CPU and verifies its
bytes at startup. English includes careful partial-word completion; the other
languages complete whole words only, and French, Spanish and Portuguese are off
until you turn them on. The [Prediction Lab](evaluation/writing/README.md)
(`npm run writing:lab`) compares models and prompts on your own test drafts
without changing the installed model.

## For contributors

| Read | For |
| --- | --- |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Outside contributor setup, checks and pull requests |
| [SECURITY.md](SECURITY.md) | Private vulnerability reporting and security scope |
| [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) | Community standards and enforcement |
| [Issue templates](.github/ISSUE_TEMPLATE/) | Bug reports and feature requests |
| [AGENTS.md](AGENTS.md) | How to work on Badi: architecture, invariants, checks |
| [VISION.md](VISION.md) | The product contract |
| [future plans.md](<future plans.md>) | The only backlog, with the ordered next steps |
| [CHANGELOG.md](CHANGELOG.md) | What shipped, by release |
| [docs/decisions](docs/decisions/) | Durable architecture decisions |
| Component runbooks | [broker](broker/README.md), [Fcitx addon](adapters/fcitx5/README.md), [observer](adapters/accessibility/README.md), [GNOME Shell extension](adapters/gnome-shell/README.md), [session helper](adapters/session/README.md), [editors](adapters/shared/README.md), [Bash](adapters/shell/README.md), [Neovim](adapters/neovim/README.md), [LibreOffice Writer](adapters/libreoffice/README.md), [Omarchy panel](ui/omarchy-plugin/README.md), [Arch package](packaging/aur/README.md), [Debian/Ubuntu package](packaging/debian/README.md), [Prediction Lab](evaluation/writing/README.md) |

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
