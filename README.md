# Badi

Badi (`بعدی`, “next”) is a local writing assistant for Linux, starting with
Omarchy/Hyprland. It uses a verified local LLM to suggest up to four words,
with guarded, explicit acceptance. This is pre-release software.

## Current coverage

| Surface | Implemented behavior | Remaining boundary |
| --- | --- | --- |
| Omawrite / Xournal++ Text tool | Omawrite suggests automatically through the observer (Tab accepts); Xournal++ uses Tab request/accept; Escape dismisses | Omawrite automatic suggestion and chained Tab acceptance passed live 2026-09-27; English continuation |
| Dillinger in Chromium | Monaco ghost text, append and narrow spelling correction | Separate opt-in product extension |
| Obsidian desktop | CodeMirror inline words, automatic/Tab request, Tab accepts a word, Ctrl/Command+Right accepts all | Caret at note end; reload the installed vault plugin after an update |
| Bash in a terminal | Grey inline Readline preview; continuation and narrow English spelling correction; Ctrl-X then Tab accepts; native undo | Installed Ghostty 1.3.1 / Bash 5.3.15 physical preview, correction, acceptance and undo passed; explicit shortcut |
| Chromium text inputs / textareas | Automatic/Tab request, Tab accepts a word, Ctrl/Command+Right accepts all, Escape and native undo | Optional site access; top-level fields with stable identity; inline LTR or caret callout |
| Chromium, VS Code, Cursor (IME-parity, no extension) | Accessibility observer and Fcitx: automatic suggestion, grey inline preview when it fits the field (else the Fcitx panel at the caret), Tab accepts once like typed text, Escape dismisses | Live 2026-09-27 with disposable text: Chromium 152 textarea/input/contenteditable, VS Code 1.138 (`editor.editContext: false`) and Cursor 3.21 each showed a suggestion in 0.5–0.8 s and appended exactly once; password fields sent no request. Needs `--force-renderer-accessibility=complete` (`install-desktop.py --observed-app`); undo may merge with prior typing; no correction |
| Brave Origin (IME-parity) | Same Chromium path and per-site policy | Page-field selection verified live on a Brave page; no full acceptance trial yet |
| Zen browser (Gecko, IME-parity) | Same observer/Fcitx path; grey inline preview from glyph-order direction (Gecko has no `direction` attribute) | Live 2026-09-27 in the user's running Zen: inline preview, one exact append, Escape and password denial passed in textarea, input and contenteditable; undo untested; its urlbar has no Url purpose, so the observer alone denies it; Gecko's surrounding text stops at the caret's paragraph; other Gecko builds are unavailable |
| Telegram desktop | Native Fcitx path (`telegram`); suggestion in the Fcitx panel | Live 2026-09-27 in Saved Messages: suggestion, one exact append, draft cleared; Qt reports no caret geometry for an inline preview |
| Codex desktop (`chatgpt`) | IME-parity identity and flag are installed | Its rich composer (U+FFFC root, trailing line break) is not yet supported: no suggestion |
| Discord | IME-parity identity | Its updater drops accessibility flags and environment, so the observer sees no fields and Badi stays silent |
| Other editors / rich websites / shells | Cooperative integration backlog | Contenteditable, canvas editors, TUIs, Fish/Zsh, Firefox and other Gecko builds remain unverified |

The desktop broker currently uses Qwen3-1.7B Q4_K_M on this workstation. Startup
selects among installed pinned models that fit current resources and verifies
their bytes; a changed hardware recommendation does not require an absent model.
“Model ready” confirms inference startup; it does not prove input is arriving.

English prediction includes guarded partial-word completion. German and Persian
whole-word continuations are experimental; application language controls and
Persian half-spaces are described in the adapter runbooks. Unknown word endings
can abstain. These are defined language paths, not a general multilingual claim.
The [writing evaluation](evaluation/writing/README.md) separates useful suggestions,
abstentions, errors and latency. Cotypist parity has not been measured.

The [Prediction Lab](evaluation/writing/README.md#discover-assess-and-compare-a-model)
can inspect this device, search public Hugging Face models, explain conservative
memory fit, verify pinned downloads, and select an isolated comparison model.
Qualification requires reviewed full additions and measured runtime behavior;
metadata or fast generation alone cannot recommend a replacement. Explicitly
saved receipts retain counts and identities without drafts. The installed model
changes only through a separate authorized installation.
The [three-candidate development screen](evaluation/writing/README.md#measured-shortlist-development-2026-09-10)
found lower memory use but no model meeting the English/German/Persian quality
gates at 550 ms. The installed baseline remains unchanged.

To test prediction quality directly, run `npm run writing:lab` from this checkout.
The local [Prediction Lab](evaluation/writing/README.md#prediction-lab) lets you
enter drafts and expected continuations, compare context/style experiments,
inspect actual model prompts, and explicitly export results. Its owned test
editor needs no extension. The Word completion tab can reuse a uniquely matching
word from supplied context without loading a model. A separate Spelling tab previews conservative German
and Persian corrections when started with the local dictionary configuration
documented in that runbook. It does not enable unsupported editing in other apps
or change installed prediction defaults.

## Desktop use

```sh
badi status
badi settings
badi launch omawrite
badi pause                 # persisted across restart
badi resume
badi app xournalpp on
badi service restart
badi autostart on
```

Where suggestions appear automatically (Omawrite and the extension-free apps),
**Tab** accepts a visible suggestion and is otherwise the normal Tab; **Escape**
dismisses and **Ctrl+Shift+Space** requests explicitly. In a Xournal++ text cell,
type a phrase such as `Please find attached the`, press **Tab** to request and
**Tab again** to accept. Candidates last five seconds and disappear when focus or
text changes.
Chromium-based apps and Zen without an editor integration use the
extension-free IME-parity path below; each needs its own app or site grant.

The new Omarchy speech-bubble **b** mark opens settings; right-click pauses/resumes. The panel also
controls app permissions, model service, login startup and activity diagnostics.

## Obsidian, terminal and browser setup

```sh
python3 scripts/install-editors.py --vault /path/to/vault --bash --chromium
badi app obsidian on
badi app bash on
badi site https://example.com on
```

Reload the Obsidian plugin and open a new Bash shell. In Bash, **Ctrl-X then Tab**
requests/accepts and **Ctrl-X then Escape** dismisses; ordinary Tab stays shell
completion. Predictions only edit the buffer; Enter remains your action.

Load `~/.local/lib/badi/chromium` through **Load unpacked** in `chrome://extensions`,
then enable each site in the Badi popup. The popup explains a missing local policy
grant or offline broker. Browser permission covers the host; Badi policy separately
checks the exact scheme, host and port. Private windows and sensitive fields are
excluded. See [editor integrations](adapters/shared/README.md) for verification
and installation boundaries.

## Extension-free apps (IME-parity)

```sh
python3 scripts/install-desktop.py                            # includes the Fcitx 5.1.22 frontend
python3 scripts/install-desktop.py --vscode-edit-context-off  # also sets VS Code's editor.editContext
python3 scripts/install-desktop.py --observed-app chromium --observed-app code  # renderer accessibility flag
badi app chatgpt on       # Codex desktop; likewise code, cursor, discord, telegram
badi site https://example.com on    # Chromium, Brave and Zen, per exact origin
```

This path uses the [focused accessibility observer](adapters/accessibility/README.md)
and the Fcitx addon; no browser extension is involved. `badi site all on`
therefore allows every Chromium/Brave/Zen origin there with no second host gate,
private windows included. Other Chromium-family browsers, web-app windows and
builds without an observer rule (Edge, Vivaldi, Code-OSS, …) are unavailable, as
are Gecko browsers other than Zen (Firefox and its web-app windows, LibreWolf,
Zen Twilight, Flatpak Zen, …). Each app must run natively
on Wayland with its input method, and the desktop accessibility bus must be on.
Each Chromium-based IME-parity app must also start with
`--force-renderer-accessibility=complete` (Zen needs no flag). Without it Chromium
exposes no page content to the observer, and lighter modes
lack the field details it checks. This costs some browser CPU and memory on every
page. `--observed-app chromium|brave-origin|chatgpt|code|cursor` (repeatable)
adds only that flag to the app's own flags file, with a backup, on the app's next
launch. Discord has no supported flags file; see the
[observer runbook](adapters/accessibility/README.md#renderer-accessibility-flag).

On Fcitx 5.1.22, the full desktop install builds and selects the pinned
[compatibility frontend](packaging/fcitx5-wayland-compat/README.md) by default. It
backports upstream Fcitx 5.1.23's refresh so Chromium-based text-input-v3 clients
keep publishing surrounding text. The backport is protocol-tested only; the
earlier 5.1.21 variant resolved the measured stall physically.
`--no-wayland-compat` skips it, the packaging runbook documents rollback, and
other Fcitx versions get no frontend.

Under the IME-parity decision in [AGENTS.md](AGENTS.md) (2026-09-26), Chromium,
Brave, Codex, VS Code, Cursor, Discord and (by user request) Zen accept a
suggestion through one append-only Fcitx commit that behaves like typed text. Undo can merge it with the
preceding typing, and a page that moves focus or caret during `beforeinput` can
redirect it like a keystroke. Earlier physical Chromium trials showed both
effects; see the [source-backed findings](docs/research/linux-architecture.md).
Exact observer identity, snapshot/caret agreement, revision and expiry binding,
one-shot acceptance, sensitive-field denial and foreign-IME yield still apply.
Replacement stays editor-owned. The grey preview is calibrated against the
app's own frame on every request and appears as inline text after the caret.
LTR text that fits in the field uses it; everything else, including Qt apps
such as Telegram, uses Fcitx's candidate panel. These apps have source and
private nested-session evidence only until live trials. Zen's observer rules
come from a live probe with disposable text; its acceptance and undo are
untested. Gecko exposes no text `direction` attribute, so Zen suggestions use
the Fcitx panel, and its surrounding text covers only the caret's paragraph, so
a field with a newline before the caret fails the snapshot agreement closed.

## Debug missing suggestions

```sh
badi doctor
badi service restart      # clears a failed service's restart limit
badi debug on
badi debug watch           # type in another app; Ctrl+C stops watching
badi debug status
badi debug off
badictl probe -            # type a disposable phrase, Enter, Ctrl+D; JSON result
```

`badi status` counts model requests that showed nothing; `badi doctor` and
`badi debug` name their content-free classes: `request_abstained` (language
missing or unsupported, text after the caret, an empty or spaces-only prefix, a
lone unfinished English word, or a Persian joiner not yet between two letters),
`budget_prefill` / `budget_stream` (the writing budget, 550 ms while typing and
1.2 s after an explicit request, ended before the model answered, or before a
complete word), `model_abstained`, `output_rejected` (language, dictionary,
number, shape or safety checks), `stale`, `timeout` and `provider_error`.
`badictl probe [--language TAG] [--after TEXT] [--replace] [--explicit] TEXT|-`
sends one request through the running broker's provider and display checks and
prints the suggestion or class with latency; `--explicit` uses the 1.2 s budget. It needs no app grant, opens no
session, cannot commit, reports `paused` while paused and changes no counters;
the broker neither logs nor stores the text. Argument text stays in shell
history and the process list, so `-` reads TEXT from standard input instead
(one trailing newline removed).

Both installers write `~/.local/state/badi/receipts/{desktop,editors}.json` with
the checkout HEAD, dirty flag and each installed file's SHA-256; a tree that is
not itself a Git checkout records `unknown`. The broker, `badictl` and native
host print their embedded commit with `--version`, and their receipt entries
record that embedded identity, since the editors installer copies the native
host the desktop installer built.
`badi doctor` shows the receipt identity and reports `broker_build_mismatch`
when the running broker's version or bytes differ from the recorded install.

Debug mode expires after 15 minutes. It records counts and reasons for focus,
input, context, Tab decisions, model requests, display and commit dispatch.
It stores no typed prose or individual key values. `app_disabled` means no app
grant exists; `unidentified_app` means Fcitx supplied no canonical app identity.
`no_input_events` means no instrumented adapter has reported during this debug run.
Obsidian and Bash appear under `editors`; browser requests appear in broker metrics
and transport failures in the page console. Other reasons distinguish
field denial, missing fresh context, caret position and model abstention.
Snapshots live in the private runtime directory and are removed by `debug off`.
A Fcitx commit dispatch cannot itself prove that the application inserted text.
Doctor reports startup failures, missing native integration and degraded settings
without copying arbitrary logs or writing into its output. A paused broker stays
paused across service restart; use `badi resume` to enable predictions.
If the inference process exits, the broker clears sessions and exits too, letting
the desktop service restart it with fresh model verification and editor authority.
Additional cooperative native apps can be granted with `badi app APP_ID on`;
policy is checked before reading. Use the exact debug identity. This cannot add
context support to an application that does not expose it through Fcitx.

## Development

Read [AGENTS.md](AGENTS.md) for architecture and exact checks. Use the root npm
workspace/lockfile and Rust 1.85+. On a provisioned Omarchy development device:

```sh
npm ci
python3 scripts/install-desktop.py
python3 scripts/install-omarchy-ui.py
npm run check
npm run fcitx5:integration
cargo test --workspace --all-features --locked
```

The desktop installer builds user-local binaries and restarts the Badi/Fcitx
user services in an unlocked session, preserving the keyboard profile, existing
autostart preference and settings, and backing up replaced files. On Fcitx
5.1.22 the full install also builds the Wayland compatibility frontend from its
pinned source download and runs its protocol checks. Use
`python3 scripts/install-desktop.py --broker-only` for a model/controller update
that leaves the input method running, including while the desktop is locked.
Model assets must already exist. The UI updater requires an unlocked desktop
and an existing Badi plugin installation. See [native setup and rollback](adapters/fcitx5/README.md),
[Omarchy controls](ui/omarchy-plugin/README.md), and [Chromium development](adapters/chromium/README.md).

Adapters own text acquisition and edits. The broker enforces per-target policy,
focus/revision binding, cancellation and one-shot acceptance. Password fields,
foreign IME composition and stale requests must fail closed. Raw keylogging,
clipboard replacement and blind synthetic typing are not product integrations.

Only [future plans](<future plans.md>) is the active backlog. Read the short
[what has been built](what-have-been.md) for history. Research documents and
immutable capability receipts are on-demand references, not required startup
context. Historical receipts do not qualify changed code or universal app support.
