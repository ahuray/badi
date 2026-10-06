# Badi

Badi (`بعدی`, Persian for “next”) is a local writing assistant for Linux,
starting with Omarchy on Hyprland. While you type, a local model suggests up to
four words in grey, and you accept them explicitly. Nothing leaves this
machine. Badi is pre-release software.

## Keys

| Key | Action |
| --- | --- |
| **Tab** | Accept the suggestion. Without one, Tab stays Tab. |
| **Ctrl+→** | Accept only the next word; the rest stays. |
| Keep typing | Typing the suggestion's next letters keeps the rest. |
| **Escape** | Dismiss. |
| **Ctrl+Shift+Space** | Ask for a suggestion. |

Obsidian swaps the first two: Tab takes a word and Ctrl/Command+→ takes all.
Bash uses Ctrl-X then Tab to request and accept. In a Xournal++ text cell, Tab
requests and Tab again accepts. A suggestion lasts up to five seconds.

## Where it works

Tested on the development workstation (Arch Linux, Hyprland 0.56, Fcitx 5.1.22)
with disposable text in October 2026. A listed version is not a claim about
Linux in general.

| App | Notes | Tested |
| --- | --- | --- |
| Chromium and Brave Origin, including Omarchy's web apps (HEY, X, Basecamp, Zoom) | Any website you allow, including rich editors (ProseMirror, Lexical, Slate, Draft.js, Quill, CKEditor 5, TinyMCE, CodeMirror), iframes and shadow roots | Chromium 152, Brave Origin 1.96 |
| Zen | Grey preview; Ctrl+Z undoes an acceptance alone | Zen 1.23b |
| VS Code, Cursor | Needs `"editor.editContext": false` in VS Code | VS Code 1.140, Cursor 3.21 |
| Codex desktop | Composer | 26.930 |
| LibreOffice Writer | Document paragraphs; Fcitx panel; Calc gets nothing | 26.8 |
| Telegram | Fcitx panel | Telegram desktop (Qt) |
| Omawrite | | 0.5.0 |
| Xournal++ text tool | Tab requests, exact native undo | 1.3.7 |
| Obsidian | Badi plugin; caret at the end of a note | 1.13.7 |
| Bash | Grey Readline preview and narrow English spelling fixes | Bash 5.3.15 in Ghostty 1.3.1 |
| Grok Bot | Ready, but its composer is untested | 0.35 |

**Not yet:**
- Discord: it drops the accessibility flag Badi needs.
- Firefox and Gecko builds other than Zen.
- Editors built on Chromium's EditContext (VS Code's default, CodeMirror on
  recent Chromium).
- A caret inside a list or quote.
- Terminals and TUIs other than Bash, and Fish/Zsh.

See [future plans](<future plans.md>).

### How Badi edits Chromium-based apps and Zen

These apps give Badi no editor channel, so it reads the focused field through
its [accessibility observer](adapters/accessibility/README.md) and accepts with
one append, like typed text. Undo may merge the acceptance with your preceding
typing (Brave does; Zen does not), and a page that moves focus while you type
can redirect it like a keystroke
([decision](docs/decisions/0003-ime-parity-append-only.md)). Badi checks field
identity and caret position before showing a suggestion and again before
inserting it, and it never reads password fields.

Chromium-based apps must start with `--force-renderer-accessibility=complete`.
`install-desktop.py --observed-app chromium|brave-origin|chatgpt|code|cursor|grok-bot`
adds it to that app's flags file; Zen needs nothing.

## Install

On Omarchy with Fcitx 5, from this checkout:

```sh
npm ci
python3 scripts/install-desktop.py      # model service, Fcitx addon, observer, `badi` command
python3 scripts/install-omarchy-ui.py   # bar mark and settings panel
python3 scripts/install-editors.py --vault /path/to/vault --bash   # optional: Obsidian, Bash
```

The installers build user-local files, back up everything they replace and
restart Badi's services and Fcitx. The newest three backups are kept, and
`python3 scripts/badi_install.py restore DIR` rolls one back. The desktop
install needs an unlocked desktop; `--broker-only` updates only the model
service while locked. On Fcitx 5.1.22 it also installs a
[backported Fcitx frontend](packaging/fcitx5-wayland-compat/README.md) so
Chromium-based apps keep sending their text (`--no-wayland-compat` skips it).
The model files must already be present (`badictl models writing` prints the
pinned download plan).

## Choose where Badi writes

```sh
badi status
badi pause                          # persists across restarts; badi resume
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
Password and sensitive fields are refused in every mode. The **b** mark in the
Omarchy bar opens the same settings, and right-clicking it pauses Badi.

## When nothing appears

```sh
badi doctor                  # setup and startup problems
badi debug on                # then: badi debug watch, and type in another app
badictl probe -              # run a disposable phrase through the live model
badi service restart
```

`badi status` and `badi debug` name why nothing was shown, without recording
what you typed:

| Reason | Meaning |
| --- | --- |
| `app_disabled` | The app or site has no grant. |
| `request_abstained` | Unsupported language, text after the caret, or a lone unfinished word. |
| `budget_prefill`, `budget_stream` | The time budget ran out: 550 ms while typing, 1.2 s after an explicit request. |
| `model_abstained`, `output_rejected` | The model declined, or its words failed a language, dictionary, number or safety check. |
| `stale`, `timeout`, `provider_error` | The text changed first, or the model was late or failed. |

Debug mode expires after 15 minutes. A crashed model service restarts with
backoff; a missing or oversized model stops it for `badi doctor` to explain.

## Model and languages

The broker runs the pinned model that fits this machine (Qwen3-1.7B Q4_K_M
here) on the CPU and verifies its bytes at startup. English includes careful
partial-word completion; German and Persian are experimental and complete whole
words only. The [Prediction Lab](evaluation/writing/README.md) (`npm run
writing:lab`) compares models and prompts on your own test drafts without
changing the installed model.

## For contributors

| Read | For |
| --- | --- |
| [AGENTS.md](AGENTS.md) | How to work here: architecture, invariants, checks |
| [VISION.md](VISION.md) | The product contract |
| [future plans.md](<future plans.md>) | The only backlog |
| [docs/decisions](docs/decisions/) | Durable architecture decisions |
| Component runbooks | [broker](broker/README.md), [Fcitx addon](adapters/fcitx5/README.md), [observer](adapters/accessibility/README.md), [editors](adapters/shared/README.md), [Bash](adapters/shell/README.md), [Omarchy panel](ui/omarchy-plugin/README.md), [Fcitx frontend backport](packaging/fcitx5-wayland-compat/README.md), [Prediction Lab](evaluation/writing/README.md) |

```sh
npm ci
npm run check
cargo test --workspace --all-features --locked
```

Badi's source and documentation are MIT-licensed ([LICENSE](LICENSE)); model
weights and other third-party files keep their own licenses.
