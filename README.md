# Badi

Badi (`بعدی`, “next”) is a local writing assistant for Linux, starting with
Omarchy/Hyprland. A verified local LLM suggests up to four words and you accept
them explicitly; predictions run on this machine. This is pre-release software.

## Coverage

Tested on the development workstation with disposable text. A listed version is
not a general Linux claim.

| Surface | How it works | Tested boundary |
| --- | --- | --- |
| Omawrite | Automatic suggestion through the observer; Tab accepts | Live 2026-09-27, including chained Tab acceptance |
| Xournal++ Text tool | Tab requests, Tab again accepts | Live, with exact native undo |
| Obsidian | Vault plugin: inline words, Tab accepts a word, Ctrl/Command+Right all | Caret at note end; reload the plugin after an update |
| Bash | Grey Readline preview, narrow English correction; Ctrl-X then Tab requests/accepts | Ghostty 1.3.1 / Bash 5.3.15: preview, correction, acceptance and undo |
| Chromium, VS Code, Cursor | IME-parity: automatic, grey inline preview (else the Fcitx panel), Tab accepts once | Live 2026-09-27: Chromium 152, VS Code 1.138, Cursor 3.21; password fields denied. 2026-10-06: Chromium 152 again, VS Code 1.140 (Fcitx panel) |
| Zen | IME-parity for the exact `zen` identity (Gecko) | Live 2026-10-06, Zen 1.23b: preview, acceptance, Escape, password denial; Ctrl+Z undoes the acceptance alone |
| Brave Origin | IME-parity with a per-site grant | Live 2026-10-06 in the user's profile, Brave Origin 1.96: preview, acceptance, Escape, password denial |
| Websites in Chromium and Brave (`badi site all on`, or per site) | IME-parity: plain fields, contenteditable `<p>`/`<div>`/`<br>` lines, ProseMirror, Lexical, Slate, Draft.js, Quill, CKEditor 5, TinyMCE and CodeMirror editors, iframe and shadow-root fields | Live 2026-10-06 on local editor builds and the official Lexical, Slate, CKEditor 5, ProseMirror and TinyMCE demo pages, one exact append each. Unavailable: EditContext editors (CodeMirror on recent Chromium), a caret inside a list or quote |
| Omarchy web apps (HEY, X, Basecamp, Zoom) | Brave Origin `--app` windows, per-site grant | Live 2026-10-06 on a local fixture `--app` window: preview, acceptance, Escape, password denial |
| LibreOffice Writer | IME-parity for document paragraphs, `badi app libreoffice on`; suggestion in the Fcitx panel | Live 2026-10-06, LibreOffice 26.8: acceptance, Escape, Ctrl+Z undoes the acceptance alone; Calc gets none |
| Telegram | Observed native Fcitx path; suggestion in the Fcitx panel | Live 2026-09-27; Qt reports no caret geometry for an inline preview |
| Codex desktop (`chatgpt`) | IME-parity with rich-composer flattening | Live 2026-10-06, Codex desktop 26.930: composer preview and one append |
| Discord | IME-parity identity | Unsupported: its updater drops the accessibility flag, and Discord resets its own switch list after one launch |
| Grok Bot | IME-parity, `badi app grok-bot on` | 0.35 exposes its web tree with the flag; composer acceptance needs a signed-in session and is untested |

On every Fcitx field above, typing the next characters of a suggestion keeps
its remainder without a new model call, and Ctrl+→ accepts only the next word
([type-through](docs/decisions/0004-type-through-carries-text-not-authority.md)).
Live 2026-10-06 in Chromium 152 (textarea, input, contenteditable; grey
preview) and Telegram (Fcitx panel), with no model call:
- the rest returned 0.05–0.36 s after the last typed key, against 0.39–0.68 s
  for a new suggestion;
- Ctrl+→ inserted one word, and the rest carried on.

Other browsers and Gecko builds, canvas editors, TUIs and Fish/Zsh are
unavailable or unverified; see [future plans](<future plans.md>).

## Install

On Omarchy with Fcitx 5, from this checkout:

```sh
npm ci
python3 scripts/install-desktop.py      # broker, model service, Fcitx addon, observer, badi
python3 scripts/install-omarchy-ui.py   # bar mark and settings panel
python3 scripts/install-editors.py --vault /path/to/vault --bash   # optional
```

The installers build user-local binaries, back up every replaced file (the
newest three backups per installer are kept; `python3 scripts/badi_install.py
restore DIR` rolls one back) and restart the Badi and Fcitx user services. The
desktop install needs an unlocked desktop; `--broker-only` updates the broker
and controls while locked and leaves the input method running. On Fcitx 5.1.22
it also installs the pinned [compatibility frontend](packaging/fcitx5-wayland-compat/README.md)
so Chromium-based apps keep publishing surrounding text (`--no-wayland-compat`
skips it). Model assets must already be present.

## Use

**Tab** accepts a visible suggestion and is otherwise the normal Tab, **Escape**
dismisses and **Ctrl+Shift+Space** requests. In Xournal++, Tab requests and Tab
again accepts. A suggestion lasts five seconds and disappears when focus or text
changes. The **b** mark in the Omarchy bar opens settings; right-click pauses.

```sh
badi status
badi pause                          # persists across restarts; badi resume
badi app chatgpt on                 # likewise code, cursor, telegram, obsidian, bash
badi site https://example.com on    # one browser origin, shared by Chromium, Brave and Zen
badi autostart on
```

Apps and websites each have a list mode. The **allowlist** (the default) lets
Badi write only where you allowed it; the **blocklist** lets it write everywhere
it can except where you blocked it:

```sh
badi apps blocklist                 # every supported app except blocked ones
badi app discord off                # ... except Discord
badi sites allowlist                # only allowed websites
badi site https://mail.example.com on
badi app list                       # mode and rules; likewise badi site list
badi app discord reset              # drop the rule; the app follows the mode
```

In app blocklist mode Badi opens only fields its accessibility observer can
verify, so an app that needs Tab requests (Xournal++) still needs `badi app ...
on`. Password and sensitive fields are refused in every mode. The settings
panel's Applications page offers the same choices.

### Chromium-based apps and Zen (IME-parity)

These apps have no editor-owned channel, so Badi reads the focused field through
the [accessibility observer](adapters/accessibility/README.md) and accepts with
one append-only Fcitx commit that behaves like typed text. Undo may merge it
with your preceding typing (it did in Brave; Zen undoes it alone), and a page
that moves focus during input can redirect it like a keystroke
([decision](docs/decisions/0003-ime-parity-append-only.md)).
Field identity, caret agreement, expiry, one-shot acceptance, password denial
and foreign-IME yield still apply; corrections stay editor-owned.

Each Chromium-based app must start with `--force-renderer-accessibility=complete`:
`install-desktop.py --observed-app chromium|brave-origin|chatgpt|code|cursor|grok-bot`
adds it to that app's flags file (Zen needs none). VS Code also needs
`"editor.editContext": false`.
`badi site all on` allows every browser origin, private windows included.

## Troubleshooting

```sh
badi doctor                  # setup, build identity and startup problems
badi debug on                # then: badi debug watch, and type in another app
badictl probe -              # run a disposable phrase through the live model
badi service restart
```

`badi status` and `badi debug` name why nothing was shown without recording
typed text: `request_abstained` (unsupported language, text after the caret, a
lone unfinished word), `budget_prefill` / `budget_stream` (the 550 ms typing
budget, or 1.2 s after an explicit request, ran out), `model_abstained`,
`output_rejected` (language, dictionary, number or safety checks), `stale`,
`timeout` and `provider_error`; `app_disabled` means the app has no grant.
Debug mode expires after 15 minutes. If the model process exits, the broker
exits and its service restarts it with backoff; a missing or oversized model
exits with status 78 and stays stopped for `badi doctor` to report.

## Model and languages

The broker runs the installed pinned model that fits current resources
(Qwen3-1.7B Q4_K_M here) and verifies its bytes at startup. English includes
guarded partial-word completion; German and Persian whole-word continuation is
experimental. The [Prediction Lab](evaluation/writing/README.md)
(`npm run writing:lab`) compares models, prompts and spelling on your own test
drafts without changing the installed model.

## Development

Read [AGENTS.md](AGENTS.md) for architecture, invariants and checks. The
[vision](VISION.md) is the product contract, [decisions](docs/decisions/) record
durable choices and [future plans](<future plans.md>) is the only backlog.

```sh
npm ci
npm run check
cargo test --workspace --all-features --locked
```

Badi's source and documentation are MIT-licensed ([LICENSE](LICENSE)); model
weights and other third-party artifacts keep their own licenses.
