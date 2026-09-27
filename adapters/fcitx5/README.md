# Badi Fcitx5 module

This tree builds a cooperative Fcitx5 **Module** addon. It does not register or
replace an input method. It observes normal Fcitx events after the active input
method, with a pre-input hook for acceptance and cancellation. It yields whenever
that input method owns preedit or candidates, and never
uses evdev, `wtype`, the clipboard, a virtual keyboard, or global input capture.

## Native application contract

- A canonical app identity and an explicit broker rule are required.
  `InputContext::program()` counts only when it is an ASCII identifier
  (`^[A-Za-z][A-Za-z0-9_-]*(\.[A-Za-z][A-Za-z0-9_-]*)*$`, at most 128 bytes);
  it is folded to lowercase once at focus-in (Qt's `Telegram` becomes
  `telegram`) and that id is used for policy, sessions, debug and the observer.
  Each focus/authority epoch queries policy before reading text; only a current
  grant opens a session. A reply still missing after two seconds is logged as
  overdue; the connection and other sessions continue, and the late reply
  still applies to its own session. Omawrite and
  Xournal++ remain the visually tested applications. The app class below
  selects which edit paths exist at all.
- **Tab** invokes at the end of a nonempty phrase with an English, German or
  Persian input-method language and no selection;
  it accepts an already visible owned candidate. Empty or ineligible fields
  and foreign IME panels retain normal Tab handling. `Ctrl+Shift+Space`
  remains an explicit invocation/refresh chord. In the manual path, until one is pressed,
  surrounding-text events only invalidate local revision state; no prose is
  serialized or sent.
- Manual invocation requires a collapsed, non-composing, non-sensitive
  surrounding-text snapshot, the live Fcitx `SurroundingText` capability, and
  a validated language from the active input-method entry. Each focus epoch
  must first receive a surrounding-text update; focus-out, capability changes
  and changed broker authority reset that freshness latch. Missing or stale
  capability, sensitive, special-purpose, composing, selected,
  unknown-language, and unknown-app states produce zero outbound context.
- A suggestion uses Fcitx's native candidate panel. Tab or `Ctrl+Shift+Y` accepts
  one owned, unexpired, exact-revision candidate; `Escape` dismisses it. Badi
  also shows a bounded thinking, no-continuation, or connection/error notice.
- Acceptance requests broker authorization first. A matching `commit.prepare`
  causes exactly one `InputContext::commitString`; the result is reported as
  `dispatched-unverified`, because Fcitx cannot prove the client applied it.
  The candidate must still be present and owned at dispatch. A local monotonic
  timer also clears the candidate when its lease expires, independently of
  broker clear delivery.

### App classes and IME parity

One classification of the canonical id (`classifyNativeApp` in `src/state.cpp`)
decides every native path:

| Class | App ids | Edit path |
| --- | --- | --- |
| Native exact | any other granted id, e.g. `omawrite`, `com.github.xournalpp.xournalpp`, `telegram` | Manual unknown-identity contract or an observed desktop field; unchanged |
| IME-parity browser | `chromium`, `chromium-browser`, `chrome`, `google-chrome`, `brave`, `brave-origin`, `brave-browser`, `zen` (Gecko) | Observed browser-origin field only, origin policy (exact rule, else `badi site all on`), append-only |
| IME-parity desktop | `chatgpt` (Codex), `code`, `cursor`, `discord` | Observed desktop field of the same app id only, app policy, append-only |
| Unavailable | Gecko-family browsers other than `zen`: Firefox and its channels (`firefox*`, `org.mozilla.firefox*`), PWAsForFirefox windows (`ffpwa-*`), other Zen builds (`zen-*`, `app.zen_browser.*`, `io.github.zen_browser.*`) and forks (LibreWolf, Floorp, Waterfox, Mullvad and Tor Browser, IceCat, …); `obsidian`; Chromium-family ids without an observer rule: web-app windows (`chrome-*`, `crx_*`, `brave-*`, `msedge-*`), Flatpak ids (`com.google.chrome`, `org.chromium.*`, `com.brave.*`, …), other Chromium browsers and channels (Edge, Vivaldi, Opera, Helium, `google-chrome-*`, …), `electron*`, and other VS Code/Discord builds (`code-oss`, `vscodium`, `discord-canary`, `vesktop`, …) | None; Obsidian's editor plugin owns its fields |

Case folding admits mixed-case window classes such as `Vivaldi-stable` or
`chrome-app.hey.com__-Default`; the Chromium- and Gecko-family rules keep them
from becoming native exact apps. Gecko mail clients (Thunderbird) are not
browsers and keep the native exact contract. An Electron app with an unrelated
id (for example a chat client) cannot be recognized by id: granting it uses the
native exact manual contract, whose append has the same typed-text semantics as
IME-parity.

IME-parity (user decision, 2026-09-26) accepts through one Fcitx
`commitString` that behaves like typed text. Undo may coalesce the append with
preceding typing, and page script that moves focus or the caret during
`beforeinput` may redirect it exactly like a keystroke. The result is
`dispatched-unverified`; Badi claims neither exact undo nor verified field
authority there. Every other guard stays:

- IME-parity apps never use the unknown-identity manual path, even with a
  `linux_app` grant. Without an observed field, Tab and navigation pass through
  and no context is sent. `Ctrl+Shift+Space` requests an explicit inspection;
  an absent observer or unobservable window shows “Badi cannot see this text
  field — check badi doctor” (`ime_parity_observer_unavailable`), and a field
  the observer denies (password, purpose, selection) shows “Badi cannot read
  this text field” (`ime_parity_field_denied`).
- On an observed field Tab only accepts a visible suggestion; without one it
  stays the application's Tab (next field, indentation). Ctrl+Shift+Space is the
  explicit observed request: the field is inspected again and policy is queried
  for its exact target.
- The observer's target kind must match the class. Otherwise authority is
  retired without reading prose (`ime_parity_target_mismatch`, or
  `ime_parity_target_invalid` for malformed metadata).
- A browser field needs an origin allow: its exact `browser_origin` rule, or
  `badi site all on` for every origin without one. There is no browser
  host-permission gate on this path, and the observer cannot tell private
  windows apart, so site-all covers private windows here.
- Before context publication, display and `commitString`, a fresh observer
  snapshot must agree with Fcitx's live surrounding text, absolute caret and
  document length. Disagreement or an unanswered RPC fails closed with a
  content-free debug reason: `observer_context_mismatch`,
  `observer_display_mismatch`/`observer_display_unavailable`,
  `observer_dispatch_mismatch`/`observer_dispatch_unavailable`, or
  `ime_parity_observer_unavailable`.
- Replacement is never negotiated or dispatched; sensitive/purpose denial,
  foreign-IME yield, revision/fingerprint/expiry binding and one-shot
  acceptance are unchanged. There are no retries or synthetic keys.

Display prefers the observer's caret preview. When the observer verifies the
field but reports `rendered:false` (preview unavailable, e.g. uncalibrated
geometry), Badi shows its owned Fcitx panel: the suggestion text as the single
candidate with “Badi · Tab to accept · Escape to dismiss”. On the `wayland_v2`
frontend Fcitx's own cursor rectangle is `[0,0,0,0]`; Hyprland places the input
popup at the application's text-input caret rectangle. Foreign preedit or
candidates still take the panel over. The addon asks the observer to hide its
preview only while one may be on screen, not on every key.

Automatic inspection runs 120 ms after Fcitx input pauses. When the observer
invalidates a field without Fcitx input (keys or changed surrounding text) since
the last inspection, a change of the field itself waits for the next input;
other invalidations inspect again after 240, 480 and 960 ms and then wait for
input (`observer_awaiting_input`), so a busy page cannot drive an inspection
loop. With `badi debug on`, a missing or invalid debug control is rechecked at
most once a second and at every focus-in.

Nested-session facts (stock Fcitx 5.1.22, Hyprland 0.56.2): Chromium 152, Brave
Origin (Chromium 154) and Electron 42 apps use text-input-v3 by default and
arrive through `wayland_v2` with their Wayland app id as `program()`;
Telegram (Qt, `QT_IM_MODULE=fcitx`) arrives through `dbus` as `Telegram`.
Chromium/Electron send no multiline, email or other purpose hints for page
fields, so the observer's field-purpose checks carry sensitive-field denial.
Chromium's own omnibox does send the `Url` purpose: live on Chromium 152
(2026-09-26) its context had capabilities `0x1072` against `0x80072` for a page
textarea. `allowsNativeContext()` therefore denies omnibox typing before any
observer request, even though AT-SPI keeps the page field focused. The stock
frontend's surrounding text stalls, which fails the snapshot comparison closed;
the backported [compatibility frontend](../../packaging/fcitx5-wayland-compat/README.md)
makes it exact after every character. VS Code's default EditContext sends
corrupted surrounding text, which also fails closed; `editor.editContext: false`
makes it exact. These are probe facts, not per-app proof.

Zen 1.22.3b (Gecko 156.0.1), live on this desktop (2026-09-26, compatibility
frontend): Zen arrives through `wayland_v2` with `program()` exactly `zen`. One
input context serves every page field and the urlbar. A field change arrives as
blur, focus and `capabilities_changed`. Text fields, the contenteditable and the
urlbar all had capabilities `0x72` (Preedit, FormattedPreedit,
ClientUnfocusCommit, SurroundingText). There was no Multiline, spell-check or
`Url` hint. The password field added Password and Sensitive (`0x100000007a`)
and was recorded as `field_denied`. Every typed character produced surrounding-text
updates. Fcitx exposes no call that returns that text, so whether it equals the
page text was not checked live; the snapshot comparison fails closed if it does
not. Gecko's GTK input method publishes only the caret's paragraph
(`IMContextWrapper::GetCurrentParagraph`, Firefox source 2026-09), so observed
requests agree only while no newline precedes the caret in the field.
Because the urlbar carries no `Url` purpose, `allowsNativeContext()` cannot deny
it; the observer's Gecko rule denies it instead (see the
[observer runbook](../accessibility/README.md#authority-boundary)). How Gecko
applies a Badi `commitString` (plain insertion or a composition commit), its
undo grouping and page events are not live-tested. Typed text there undid in
one Ctrl+Z step.

Rich editors: Codex desktop (`openai-codex-desktop` 26.915.31945, Chromium 153,
`chatgpt`) composes in a ProseMirror contenteditable, whose paragraphs the
[observer](../accessibility/README.md#rich-editors) flattens. Live on 2026-09-27
with the previous build, the disposable phrase typed at the end of the composer
gave Fcitx all 24 bytes before the caret and more text after it
(`caret_not_at_end`), although the paragraph had nothing after the caret.
Chromium 152's own text-input-v3 requests, recorded with `WAYLAND_DEBUG=1` on an
isolated window, show why: `set_surrounding_text` ends every `<p>` with `"\n\n"`,
with or without margins, when anything is rendered after the editor. A phrase at
the end of one paragraph was sent as `"Please find attached the"` + `"\n\n"`.
Two paragraphs were sent as `"Hello\n\nPlease find attached the"` + `"\n\n"`.
A paragraph that already ends in a line break gets one more. A bare-text
contenteditable ends at the caret.

For an observed IME-parity field only, `normalizeObservedParagraphEnd()`
therefore treats after-caret text of exactly `"\n\n"` as end of field. Tab
eligibility and the broker context use `after: ""` at the unchanged caret, and
the fingerprint binds the raw text. Request, display and dispatch agreement
still compare the raw `"\n\n"` (`observedAfter()`), which the observer
reproduces from the paragraphs. A request that used the rule records
`observed_field_request_paragraph_end`. Any other suffix (one or three line
breaks, text after them) stays `caret_not_at_end`, and `"\r\n"` is never
context. An observer that disagrees fails as `observer_context_mismatch`. A
textarea whose text continues with exactly `"\n\n"` after the caret behaves the
same, and the append lands at the caret. Native exact apps and unobserved
contexts keep the unnormalized contract.

Verified live with this build: in isolated Chromium 152 (fixture origin, no
extensions), ProseMirror-shaped `<div contenteditable role=textbox><p>` editors
with zero and default paragraph margins each displayed a suggestion for the
typed phrase. Tab appended one continuation; Escape dismissed without a commit.
The narrow window tile put the caret outside Hyprland's window size, so the Fcitx
panel was shown instead of the overlay. Not verified live with this build: the
Codex composer itself, whose window was closed during the session (Chromium 153
is assumed to serialize like 152), and the Zen regression, since Zen was no
longer running. The Zen regression (textarea, input, contenteditable, password
denial, focus change) passed on the intermediate installed build, whose plain
paths are unchanged.

Unavailable apps acquire no prose and are never inspected. Ordinary Tab and
navigation pass through; `Ctrl+Shift+Space` can show “Badi cannot safely insert
suggestions in this app yet”, which needs no surrounding text. Surrounding-text
publication retains that notice until its five-second expiry; input, focus loss
and foreign composition clear it. A browser-origin target for a native exact app
is also unavailable. Missing, unknown or mismatched observed target metadata
retires prior authority rather than falling back to an app-wide manual grant.

A sandbox-enabled Chromium 151 physical test on 2026-09-08 established the
IME-parity semantics above: a `beforeinput` handler that moved focus redirected a
single Fcitx append to another field, moving the caret changed its position, and
Ctrl+Z removed the previously typed prefix together with the append. External
field/caret checks cannot make this a transaction or separate undo.
Receipt: `output/extensionless/installed-browser-04/before-focus-result.json`.

The native adapter does **not** negotiate `text_replacement` or dispatch
deletion-based edits, including for known observed fields. A sandbox-enabled
Chromium 151 physical test on 2026-09-08 showed that an input handler can move
focus between Fcitx deletion and insertion: the original field lost `adress `
and another field received `address `. Exact pre-dispatch snapshots cannot make
those two native operations atomic. Unexpected replacement suggestions and
commit grants fail closed at the wire, state and dispatch boundaries. Spelling
remains available through the editor-owned integrations.

The [accessibility observer](../accessibility/README.md) supplies IME-parity
field identity and corroboration; the version-pinned
[Wayland compatibility frontend](../../packaging/fcitx5-wayland-compat/README.md)
supplies exact surrounding text. Neither provides an editor transaction:
IME-parity accepts typed-text semantics instead. The earlier stock-frontend
Chromium trial in `output/extensionless/installed-browser-05/` verified the
former quarantine (original Tab, no context or commit despite an origin allow);
it does not qualify IME-parity. Exact editor transactions remain limited to
editor-owned integrations; see the
[IME-parity decision](../../docs/decisions/0003-ime-parity-append-only.md).

The actual-addon isolated D-Bus lane covers each class with configured
`linux_app` and exact-origin allow rules. Unavailable apps keep Tab/navigation,
read no prose and send no context; notice-lifetime checks inspect the latest
auxiliary panel after publication, input, focus loss, foreign composition and
expiry. For IME-parity browsers (`brave-origin`, with Chromium's
`UppercaseWords` hint, and `zen` without hints) and a desktop target (`code`)
it proves: a declined field or absent observer service sends no manual context
despite the app grant, a mismatched target kind or observer-denied password
field reads no prose, automatic suggestion in the Fcitx panel fallback with its
hint, one exact Tab dispatch, explicit Tab re-inspection, and
request/display/dispatch disagreement and an unanswered dispatch snapshot
failing closed with their reasons. For `zen`, an observer-denied urlbar on the
same unhinted context reads no prose, sends no context and keeps Tab.
`brave-origin` also covers foreign Unicode preedit yield and a rendered preview;
`brave-origin` and `code` cover a peer-injected replacement suggestion or
commit grant closing the transport. `chatgpt` covers the paragraph end: an
agreed `"\n\n"` displays and dispatches one exact append, the broker receives
`after: ""`, and observer disagreement, one or three line breaks and text after
them fail closed. `Telegram` is canonicalized
for observer, policy and session, and a non-identifier program gets no binding.
**Synthetic desktop** targets still
exercise snapshot corroboration, absolute caret/document-length gates, delayed
preview cancellation, field switches, pause, broker recovery and Tab after an
unchanged-authority reconnect; a native manual Omawrite context still dispatches
an exact append. These fixtures prove transport/state behavior, not app
compatibility, application mutation, overlay rendering or undo:

```sh
python3 adapters/fcitx5/tests/observed-desktop.py
```

This lane needs the built addon, debug broker/CLI, Fcitx D-Bus frontend and Unicode module/data,
`dbus-run-session` and Python GObject/Gio. It owns a private D-Bus session and
temporary configuration; it does not touch normal Fcitx or the desktop.

### Verified compatibility cells

The following cells passed on Arch Linux under native Wayland, Hyprland 0.56.2,
and Fcitx5 5.1.21. These are exact proof cells, not toolkit-wide claims or
runtime widget selectors. Fcitx supplies the process identity and text context,
but no stable widget identity; the user must explicitly invoke Badi in the
focused field. Context frames therefore declare field identity and purpose
unknown, so only the broker's explicit-manual path can authorize them. Other
fields in the two allowlisted processes remain unverified.

| Application | Native stack | Result |
| --- | --- | --- |
| Omawrite 0.5.0 | Qt 6 | 20/20 visible invoke, candidate, accept, clear, save, and undo trials |
| Xournal++ 1.3.7 | GTK 3 text tool | 20/20 visible invoke, candidate, accept, clear, saved `.xopp` inspection, and native document-undo trials; a separate Escape dismissal left the document unchanged |

The selected input method remained `keyboard-us`. The module does not claim
other fields, versions, Qt/GTK applications, terminals, unknown application
IDs, or broad Fcitx compatibility.

The 2026-09-07 installed update was rechecked on Omawrite 0.5.0-1 with Qt
6.11.2-2 and Fcitx5 5.1.21-1: the visible local-model candidate was accepted,
saved to a disposable file, and one native undo restored the exact prefix.
Xournal++ 1.3.7-1 also displayed and saved the prediction into an existing text
object; after leaving text-edit mode with Escape, one document undo restored the
saved prefix. Ctrl+Z inside a new empty text editor is not document undo.
These trials used guarded test input on the real Wayland desktop. Separate
synthetic D-Bus contexts verified both native app identities for Tab, Escape,
empty/selected/mid-text fields, stale text and sensitive-field denial. That
D-Bus lane proves input-method dispatch; the saved-file desktop trials prove
the observed application mutations. Current synthetic artifacts are retained in
`output/playwright/native-installed-htjvil8m/`; they do not establish
multilingual native quality.

## Desktop installation

`python scripts/install-desktop.py` builds the release broker and addon, installs
them under `~/.local/lib`, enables `badi-broker.service` on its initial installation,
preserves an existing autostart choice, and loads the addon into
`omarchy-fcitx5.service` using a user drop-in. It requires the provisioned local
model/runtime and an active graphical Omarchy/Fcitx session. The installer verifies
model health, addon loading, and an unchanged keyboard profile before reporting
success. No system package or global toolkit environment is changed.
The mapped-file check compares the current process, exact path and inode, and
the mapping device. On Btrfs it obtains the device from a read-only mapping of
the installed file because `stat` may report a different subvolume device;
deleted mappings cannot verify an update. This requires no privileged access
to process memory or `/proc/PID/map_files`.

The complete installation also installs `badi-accessibility.service` with its
Python/AT-SPI/GTK preview helper and verifies its private socket and exact service
process before restarting Fcitx. Existing helper autostart preferences are
preserved. A responsive helper does not establish application accessibility or
editing support; `badi doctor` reports its readiness and the accessibility bus
enablement separately.

The complete installation requires an explicitly unlocked Omarchy session.
`python scripts/install-desktop.py --broker-only` updates just the broker and
commands while preserving the running input method; use the complete installer
after normal unlock to apply a changed native addon. Both paths preserve existing
settings and record replaced files in a recoverable backup. They also remove what
the retired Chromium extension left installed, backed up the same way:
`~/.local/lib/badi/chromium/`, `badi-native-host`, `badi-native-manifest`, and
`io.github.ahuray.badi.json` native-messaging manifests in Chromium, Chrome and
Brave configuration folders that name that host. Remove the unpacked extension
itself in `brave://extensions` if it was loaded.

The service creates `$XDG_RUNTIME_DIR/badi` with mode `0700`; its socket is `0600`.
The model stays loaded after editor closure and starts with the graphical session.
A user desktop-file override gives Xournal++ `GTK_IM_MODULE=fcitx`, which supplies
the exact application identity absent from its default Wayland frontend on this
machine. Relaunch existing Xournal++ windows through the desktop launcher or Badi
panel. Omawrite uses the existing `QT_IM_MODULE=fcitx` configuration.

Each installer run that changes a file backs up what it replaced under
`~/.local/state/badi/install-backups/<UTC time>/` and prints that directory.
`changes.json` lists every change in order: its home-relative `path`, `action`
(`create`, `replace` or `remove`), the original's location in the backup
(`saved`, under `home/`) and the `sha256` (or `symlink`) the installation left.
Files whose bytes and mode already match are neither rewritten nor backed up, so
an unchanged reinstall creates no backup. The installer keeps the newest three
backups and deletes older ones in that directory, including earlier-format
backups named by nanosecond or local `YYYYmmdd-HHMMSS` time. Roll back from the
actual unlocked graphical session:

1. Select the applicable backup and inspect its `changes.json`. An observed-app
   installation also records `accessibility-setting.json`, containing the prior
   `bus_enabled` and `toolkit_accessibility` booleans. For a full rollback of the
   2026-09-07 task, use the earlier `accessibility-before-task.json` receipt copied
   into that backup: the temporary probe preceded installation, so the installer's
   receipt alone can describe an already enabled state. Choose the receipt for
   the intended restore point; do not assume missing receipts mean `false`.
   Pruning an older backup first moves its `accessibility-*.json` receipts to
   `~/.local/state/badi/install-backups/notes/<backup name>/`.
2. **Stop `badi-accessibility.service` before replacing its Python files**, and
   stop `badi-broker.service` before restoring the broker. A broker-only rollback
   leaves the helper and Fcitx running. Disable a service before removing its unit
   only if this installation introduced that service. Existing broker/helper
   autostart choices were preserved by the installer and should remain unchanged;
   do not blanket-disable them. The backup does not record prior service activity
   or enablement, and absence of a backed-up user unit does not rule out an existing
   system-provided unit.
3. Restore the files, newest backup first when undoing several installations:

   ```sh
   python3 scripts/badi_install.py restore ~/.local/state/badi/install-backups/<UTC time>
   ```

   It restores each replaced or removed original, deletes files that had no
   predecessor, and leaves (and lists, exiting 1) every path changed since that
   installation, preserving later unrelated edits. That covers the observed-app
   startup files under the configured `XDG_CONFIG_HOME` (`chromium-flags.conf`,
   `brave-origin-flags.conf`, `codex-flags.conf`, `code-flags.conf` or
   `cursor-flags.conf`), the Fcitx drop-ins, launcher overrides and command
   links. `--only PATH` limits it to one home-relative path or folder. To undo
   only the renderer accessibility flag, delete the
   `# Badi: renderer accessibility for the focused-field observer` line and the
   flag line after it instead. Run
   `systemctl --user daemon-reload`, restore the prior broker/helper running states,
   and restart `omarchy-fcitx5.service` for a complete native rollback. Do not start
   a newly introduced helper whose unit has just been removed. Save work and
   relaunch affected applications normally so they use the restored startup flags.
4. After service and application recovery, restore the two accessibility values
   from the selected receipt. Set the live `org.a11y.Bus` property **first**, then
   restore the persistent GNOME toolkit setting: changing `IsEnabled` can itself
   write `toolkit-accessibility`. With `BADI_ACCESSIBILITY_RECEIPT` set to the
   selected receipt's absolute path, run:

   ```sh
   python3 - "$BADI_ACCESSIBILITY_RECEIPT" <<'PY'
   import json
   import subprocess
   import sys

   with open(sys.argv[1]) as stream:
       previous = json.load(stream)
   for key in ("bus_enabled", "toolkit_accessibility"):
       if type(previous.get(key)) is not bool:
           raise SystemExit("Receipt must contain both prior boolean values")
   subprocess.run(["busctl", "--user", "--timeout=2s", "set-property",
       "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Status", "IsEnabled", "b",
       str(previous["bus_enabled"]).lower()], check=True, timeout=3)
   subprocess.run(["gsettings", "set", "org.gnome.desktop.interface",
       "toolkit-accessibility", str(previous["toolkit_accessibility"]).lower()],
       check=True, timeout=3)
   PY
   busctl --user --timeout=2s get-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled
   gsettings get org.gnome.desktop.interface toolkit-accessibility
   ```

   Verify both results match the receipt after services settle. If another
   accessibility client changes them again, inspect that client rather than
   silently accepting a different restore state. A rollback without either
   receipt must not guess or reset these desktop-wide settings.

Initial private Badi settings and downloaded model files are retained. Service
recovery and restored settings do not establish physical prediction, insertion,
or undo behavior in an application; those remain separate verification steps.

## Try it

After the desktop installation below, open an editor from the Badi panel or
with `badi launch xournalpp` or `badi launch omawrite`. The local model and
runtime must already be provisioned; `badictl models writing` prints the pinned
download plan.

1. In Xournal++, select the **Text** tool and click a blank page. In Omawrite,
   type directly into the document.
2. Type `Please find attached the` with the caret at the end and no selection.
3. Omawrite suggests on its own. In Xournal++, press **Tab** to request a short
   continuation from the local LLM. A suggestion appears in the Fcitx candidate
   panel when one is available.
4. Press **Tab** to accept or **Escape** to dismiss. The candidate remains for
   up to **five seconds**, provided the text and focus stay unchanged; a
   disappeared candidate needs another request.

The current model on this workstation is **Qwen3-1.7B-Q4_K_M**, running locally
through llama.cpp. Native suggestions are continuations of up to four
words. English has the historical application proof above; German and Persian
language routing is experimental and follows the active input-method language.
Persian half-spaces are preserved only between the exact permitted Arabic
letters; other invisible formatting remains denied. Both manually identified
and observed native fields remain continuation-only; native spelling replacement
has been withdrawn. Additional cooperative applications can be
granted with `badi app APP_ID on`, using the exact identity from `badi debug status`.
A grant does not add missing toolkit context support. Apps without a canonical
Fcitx identity, Gecko browsers other than Zen, and terminals need suitable
adapters. The listed Chromium-based apps and Zen use the observed IME-parity
path above; other Chromium-family ids are unavailable.

### Judge usefulness and editing safety separately

Try at least 20 prefixes from your normal writing, including short emails,
technical prose, names, and incomplete words. Do not use only the demo sentence.
For each request, record whether the suggestion arrived in time, fit your
intended meaning, preserved tone, and was worth accepting. Count abstentions,
irrelevant continuations, and invented specifics separately. This small personal
trial helps expose problems; it is not a general quality certification.

Also check that acceptance inserts exactly once, Escape leaves the text intact,
and the app's native undo removes the accepted text. Type another character or
move focus while a suggestion is visible: an obsolete suggestion must not be
inserted. Never interpret transport acknowledgements as proof of visible edits.

`badictl probe -` runs disposable text through the installed model without an
editor; it does not prove visible acceptance or measure general writing quality.

On the configured Omarchy workstation, click the **Badi keyboard icon just left
of Wi-Fi** for launch buttons, request/suggestion/error counts, and
pause/resume. A zero request count means inference has not been invoked; check
the Text tool, focus, and Tab first. See the
[desktop controls runbook](../../ui/omarchy-plugin/README.md).

## Build and test

Requirements are CMake, Ninja, a C++20 compiler, Fcitx5Core, and nlohmann-json.

```sh
cmake -S adapters/fcitx5 -B adapters/fcitx5/build -G Ninja \
  -DCMAKE_BUILD_TYPE=Release
cmake --build adapters/fcitx5/build
ctest --test-dir adapters/fcitx5/build --output-on-failure
```

The repeatable C++/Rust integration check additionally requires Cargo and
Python 3:

```sh
npm run fcitx5:integration
```

It runs the shipped transport and session state against the real broker for
both allowlisted identities, verifies exact context/accept/report/dismiss
counters, and checks disconnect and shutdown cleanup in temporary XDG roots.
It does not install the addon, start an input method, edit a native application,
or replace the dated visible-app evidence above. The native Arch CI job runs
the same check without requiring Node.

The deterministic transport lane also restarts its private broker while a
candidate is visible. The client reconnects without a new focus/key event,
obtains fresh policy, and rejects the retired candidate. Production reconnect
uses at most ten retries with backoff capped at five seconds; an explicit key
or new focus can renew the budget. Handshakes time out after two seconds.
The broker keeps an idle policy-capable connection open. A closed connection,
such as a broker restart, retires every session, context revision, candidate,
commit grant and pending observer reply. After fresh policy the adapter reopens
the session and republishes context before any request. Surrounding text is
Fcitx state: when the new connection reports the same authority epoch, settings
revision and pause state, Tab reads it without new input, while automatic
observed requests still wait for new surrounding text. A changed authority, on
the same or a new connection, requires a fresh surrounding-text event before
reading again. Typing supplies this in cooperative fields; the explicit-manual
invocation contract remains unchanged.

After desktop installation, the opt-in check below uses Gio to drive synthetic
input contexts through the **installed Fcitx addon and local model**:

```sh
PYTHONDONTWRITEBYTECODE=1 python adapters/fcitx5/tests/desktop-tab.py
```

It checks Tab request/accept, Escape, stale-text rejection, empty/selected/mid-text
pass-through, and unsupported/sensitive contexts. It temporarily creates and
focuses its own Fcitx contexts and destroys them afterward; run it while not
typing elsewhere. It requires the provisioned Qwen model, an unpaused broker,
both native app policies, and Python Gio. It does not prove pixels, file saves,
or native undo in an editor.

For a staged install without changing the live Fcitx configuration:

```sh
DESTDIR="$PWD/adapters/fcitx5/stage" \
  cmake --install adapters/fcitx5/build --prefix /usr
```

The output module is `libbadi-fcitx5.so`; `badi.conf` declares it as
`Category=Module`.

## Disposable user-local evaluation

This path is for an exact development cell. A release package should own the
normal system addon paths instead of persisting custom environment variables.

```sh
cmake --install adapters/fcitx5/build --prefix "$HOME/.local"

FCITX_DATA_DIRS="$HOME/.local/share/fcitx5:/usr/local/share/fcitx5:/usr/share/fcitx5" \
FCITX_ADDON_DIRS="$HOME/.local/lib/fcitx5:/usr/lib/fcitx5" \
  fcitx5 -r
```

The explicit search paths are required on the tested machine for a user-local
module to be discovered. Before starting the addon, run the matching broker v2
and install explicit `linux_app` rules for only `omawrite` and
`com.github.xournalpp.xournalpp`. Learning is blocked for native applications,
and retention stays `none` in this slice.

Verify that the addon loaded and the user's input method did not change:

```sh
gdbus call --session --dest org.fcitx.Fcitx5 \
  --object-path /controller \
  --method org.fcitx.Fcitx.Controller1.CurrentInputMethod
```

Use `Ctrl+Shift+Space` in a supported focused text cell to request a suggestion,
`Ctrl+Shift+Y` to accept the owned candidate, and `Escape` to dismiss it. A
shortcut is consumed only for the matching local action; otherwise it passes
through to the application.

### Rollback

Stop the evaluation Fcitx process, remove only the two user-local Badi files,
and start the ordinary session again:

```sh
rm "$HOME/.local/lib/fcitx5/libbadi-fcitx5.so"
rm "$HOME/.local/share/fcitx5/addon/badi.conf"
fcitx5 -rd
```

Rollback must also stop the disposable broker and remove its isolated
HOME/XDG/runtime data. Do not remove or rewrite the user's existing Fcitx
configuration.

The deterministic tests cover state transitions, app classes and canonical
app ids, observed-only append-only IME parity, fingerprint salting/binding,
UTF-8 and output sanitization, bounded framing, unchanged-toolkit republish
handling, stale focus/revision rejection, sensitive
zero-context behavior, manual key decisions, foreign-IME yielding, duplicate
JSON-key rejection, optional `suggestion.clear` fields, duplicate commit
authorization, and reconnect freshness without surviving candidates or grants.

See [WIRE_PROTOCOL.md](WIRE_PROTOCOL.md) for the isolated v2 assumptions.
