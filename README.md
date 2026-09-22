# Badi

Badi (`بعدی`, “next”) is a local writing assistant for Linux, starting with
Omarchy/Hyprland. It uses a verified local LLM to suggest up to four words,
with guarded, explicit acceptance. This is pre-release software.

## Current coverage

| Surface | Implemented behavior | Remaining boundary |
| --- | --- | --- |
| Omawrite / Xournal++ Text tool | Fcitx candidate; Tab request/accept; Escape dismiss | Exact tested native cells; English continuation |
| Dillinger in Chromium | Monaco ghost text, append and narrow spelling correction | Separate opt-in product extension |
| Obsidian desktop | CodeMirror inline words, automatic/Tab request, Tab accepts a word, Ctrl/Command+Right accepts all | Caret at note end; reload the installed vault plugin after an update |
| Bash in a terminal | Grey inline Readline preview; continuation and narrow English spelling correction; Ctrl-X then Tab accepts; native undo | Installed Ghostty 1.3.1 / Bash 5.3.15 physical preview, correction, acceptance and undo passed; explicit shortcut |
| Chromium text inputs / textareas | Automatic/Tab request, Tab accepts a word, Ctrl/Command+Right accepts all, Escape and native undo | Optional site access; top-level fields with stable identity; inline LTR or caret callout |
| Extension-free Chromium / Codex desktop | Experimental accessibility/Fcitx path is quarantined | Chromium preview rendered, but native acceptance failed field/caret binding and separate undo; a safe editor transaction is required |
| Other editors / rich websites / shells | Cooperative integration backlog | Contenteditable, canvas editors, TUIs, Fish/Zsh and Firefox remain unverified |

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

Type an English prefix such as `Please find attached the` at the end of a native
text field. Press **Tab** to request, **Tab again** to accept, or **Escape** to
dismiss. In Xournal++, select the Text tool first. Candidates last five seconds
and disappear when focus or text changes. These previously tested native cells
use manual invocation. `Ctrl+Shift+Space` / `Ctrl+Shift+Y` are alternate shortcuts.
The experimental browser/Codex native path is disabled after failed editing
tests; a site permission alone cannot enable it.

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

The extension-free path uses the [focused accessibility observer](adapters/accessibility/README.md)
and the Fcitx addon. It needs application accessibility and native input-method
support; a browser extension is not part of that path. The opt-in
[pinned Fcitx compatibility frontend](packaging/fcitx5-wayland-compat/README.md)
resolves the measured Chromium v3 publication stall. Physical Chromium trials
showed a grey real-model preview and accepted insertion, but undo also removed
the preceding typed prefix. Sandbox-enabled tests then proved wrong-field and
wrong-caret edits, including with a single native insertion when a page changed
focus or selection during `beforeinput`. The native browser/Codex path is
quarantined, and native replacement is disabled. Editor-owned correction paths
remain separate. Safe extension-free editing needs authority that reaches the
editor transaction; the current external observation and input protocols do not
provide it. See the [source-backed findings](docs/research/linux-architecture.md).
Preview geometry is currently restricted to the measured Chromium 151 native
Wayland LTR cell. Other coordinate conventions fall back to Fcitx's candidate
display. Do not treat this source implementation as completed app coverage.

## Debug missing suggestions

```sh
badi doctor
badi service restart      # clears a failed service's restart limit
badi debug on
badi debug watch           # type in another app; Ctrl+C stops watching
badi debug status
badi debug off
```

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
autostart preference and settings, and backing up replaced files. Use
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
