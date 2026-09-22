# Badi Fcitx5 module

This tree builds a cooperative Fcitx5 **Module** addon. It does not register or
replace an input method. It observes normal Fcitx events after the active input
method, with a pre-input hook for acceptance and cancellation. It yields whenever
that input method owns preedit or candidates, and never
uses evdev, `wtype`, the clipboard, a virtual keyboard, or global input capture.

## Native application contract

- A canonical `InputContext::program()` identity and an explicit broker app
  rule are required. Each focus/authority epoch queries policy before reading
  text; only a current grant opens a session. Queries expire after two seconds.
  Omawrite and Xournal++ remain the visually tested applications.
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
  must first receive a surrounding-text update; focus-out and capability
  changes reset that freshness latch. Missing or stale capability, sensitive,
  special-purpose, composing, selected, unknown-language, and unknown-app
  states produce zero outbound context.
- A suggestion uses Fcitx's native candidate panel. Tab or `Ctrl+Shift+Y` accepts
  one owned, unexpired, exact-revision candidate; `Escape` dismisses it. Badi
  also shows a bounded thinking, no-continuation, or connection/error notice.
- Acceptance requests broker authorization first. A matching `commit.prepare`
  causes exactly one `InputContext::commitString`; the result is reported as
  `dispatched-unverified`, because Fcitx cannot prove the client applied it.
  The candidate must still be present and owned at dispatch. A local monotonic
  timer also clears the candidate when its lease expires, independently of
  broker clear delivery.

### Browser and Codex native editing is unavailable

The native addon quarantines Chromium/Chrome, Brave, Firefox and the installed
Codex `chatgpt` program aliases. App/site permission cannot enable their native
prediction or acceptance: these bindings acquire no prose and publish no model
context. Browser-origin targets are also rejected independently of the program
alias. Missing, unknown or mismatched observed target metadata retires prior
authority rather than falling back to an app-wide manual grant.

Ordinary Tab and navigation pass through. `Ctrl+Shift+Space` can show “Badi cannot
safely insert suggestions in this app yet”; this requires no surrounding
text or working browser input-method handshake. Unsupported fields never display
an acceptance candidate or “Tab to accept” instruction. The separate manual
unknown-widget contract for native applications remains unchanged.
For the explicitly quarantined app aliases, surrounding-text publication retains
the auxiliary notice until its original five-second expiry. Input, focus loss and
foreign composition still clear it. Observed browser targets with other aliases
retain normal field invalidation and metadata reinspection. Physical visibility
on the stock Wayland frontend remains a separate check from auxiliary-panel
signals in the private fixture.

A sandbox-enabled Chromium 151 physical test on 2026-09-08 confirmed that a single
Fcitx append can reach another field: a `beforeinput` handler moved focus before
the browser chose the insertion target. Moving the caret in that handler also
changed the insertion position. In the benign baseline, Ctrl+Z removed the
previously typed prefix together with the accepted append. External field/caret
checks therefore cannot establish the required transaction or separate undo.
Receipt: `output/extensionless/installed-browser-04/before-focus-result.json`.

The native adapter does **not** negotiate `text_replacement` or dispatch
deletion-based edits, including for known observed fields. A sandbox-enabled
Chromium 151 physical test on 2026-09-08 showed that an input handler can move
focus between Fcitx deletion and insertion: the original field lost `adress `
and another field received `address `. Exact pre-dispatch snapshots cannot make
those two native operations atomic. Unexpected replacement suggestions and
commit grants fail closed at the wire, state and dispatch boundaries. Spelling
remains available through the editor-owned integrations.

The [accessibility observer](../accessibility/README.md) and version-pinned
[Wayland compatibility frontend](../../packaging/fcitx5-wayland-compat/README.md)
retain diagnostic and upstream-research value. Restoring surrounding-text
publication did not establish safe editing. Installer `--observed-app` options
prepare diagnostic observation on a subsequent launch; they do not enable
browser/Codex writing. The quarantine works with the stock Fcitx frontend.
The installed stock-frontend Chromium trial in
`output/extensionless/installed-browser-05/` preserved original Tab navigation
and text, emitted no model context or commit despite an explicit origin allow,
and visibly displayed the unavailable notice. This auxiliary-panel observation
does not qualify caret alignment or browser editing.
Extension-free support requires a cooperating editor transaction or upstream
support that preserves expected field/revision authority through execution; see
the [pinned architecture findings](../../docs/research/linux-architecture.md#2026-09-08-extension-free-editing-transaction-limits).

The actual-addon isolated D-Bus lane proves the quarantine with configured app
and exact-origin allow rules, no prose/model context or CommitString, original
Tab/navigation, and the explicit notice without surrounding text. The
notice-lifetime checks inspect the latest auxiliary panel after repeated
publication, input, focus loss, foreign composition and expiry. Separate
**synthetic desktop** targets exercise the observer's automatic acquisition,
snapshot corroboration, absolute caret/document-length gates, delayed preview
cancellation, field switches, foreign Unicode preedit, pause and broker recovery.
A native manual Omawrite context still invokes and dispatches an exact append.
Those fixtures prove transport/state behavior, not browser compatibility,
application mutation, overlay rendering or undo:

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
IDs, or broad Fcitx compatibility. See the
[native-app handoff](../../what-have-been.md)
for the evidence boundary and observed metrics.

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
`output/playwright/native-installed-htjvil8m/`; they do not requalify historical
capability receipts or establish multilingual native quality.

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
settings and record replaced files in a recoverable backup.

The service creates `$XDG_RUNTIME_DIR/badi` with mode `0700`; its socket is `0600`.
The model stays loaded after editor closure and starts with the graphical session.
A user desktop-file override gives Xournal++ `GTK_IM_MODULE=fcitx`, which supplies
the exact application identity absent from its default Wayland frontend on this
machine. Relaunch existing Xournal++ windows through the desktop launcher or Badi
panel. Omawrite uses the existing `QT_IM_MODULE=fcitx` configuration.

Replaced files are backed up under `~/.local/state/badi/install-backups/<timestamp>`
with `changed-files.json`. Roll back from the actual unlocked graphical session:

1. Select the applicable backup and inspect its changed-file list. An observed-app
   installation also records `accessibility-setting.json`, containing the prior
   `bus_enabled` and `toolkit_accessibility` booleans. For a full rollback of the
   2026-09-07 task, use the earlier `accessibility-before-task.json` receipt copied
   into that backup: the temporary probe preceded installation, so the installer's
   receipt alone can describe an already enabled state. Choose the receipt for
   the intended restore point; do not assume missing receipts mean `false`.
2. **Stop `badi-accessibility.service` before replacing its Python files**, and
   stop `badi-broker.service` before restoring the broker. A broker-only rollback
   leaves the helper and Fcitx running. Disable a service before removing its unit
   only if this installation introduced that service. Existing broker/helper
   autostart choices were preserved by the installer and should remain unchanged;
   do not blanket-disable them. The backup does not record prior service activity
   or enablement, and absence of a backed-up user unit does not rule out an existing
   system-provided unit.
3. Restore the entries in `changed-files.json` from their matching backup paths;
   remove only listed new files confirmed to have had no predecessor. Include the
   observed-app startup files, normally `chromium-flags.conf` and
   `codex-flags.conf` under the configured `XDG_CONFIG_HOME`, plus the Fcitx drop-in
   and launcher overrides when listed. Preserve subsequent unrelated edits rather
   than overwriting them with an older whole file. Run
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

## Try it without changing your desktop

Run from the repository in a terminal in your actual Wayland session:

```sh
python scripts/try-native.py xournalpp
# Alternative supported editor:
python scripts/try-native.py omawrite
```

The trial requires the development dependencies below, Cargo, Python 3,
`dbus-run-session`, `gdbus`, Fcitx5, the target app and its Fcitx toolkit module.
The local model and runtime must already be provisioned, as they are on this
workstation; see the
[local writing implementation and evidence](../../what-have-been.md).
The launcher uses your existing model assets, builds the current broker/addon,
and starts an isolated session with an English/US input profile. It does not
install the addon into or restart your normal desktop Fcitx.

1. In Xournal++, select the **Text** tool and click a blank page. In Omawrite,
   type directly into the document. Use the new trial window.
2. Type `Please find attached the` with the caret at the end and no selection.
3. Press **Tab** to request a short continuation from the local LLM.
   A suggestion appears in the Fcitx candidate panel, when one is available.
4. Press **Tab again** to accept or **Escape** to dismiss. The native candidate
   remains for up to **five seconds**, provided the text and focus stay unchanged.
   A disappeared candidate requires another request. Typing alone does not invoke
   the current manual native implementation.
5. Save any writing you want to keep and close the app normally. Services stop
   automatically; the printed `output/native-trials/` folder, logs, and any files
   saved there remain available.

The current model on this workstation is **Qwen3-1.7B-Q4_K_M**, running locally
through llama.cpp. Native suggestions are manual continuations of up to four
words. English has the historical application proof above; German and Persian
language routing is experimental and follows the active input-method language.
Persian half-spaces are preserved only between the exact permitted Arabic
letters; other invisible formatting remains denied. Both manually identified
and observed native fields remain continuation-only; native spelling replacement
has been withdrawn. Additional cooperative applications can be
granted with `badi app APP_ID on`, using the exact identity from `badi debug status`.
A grant does not add missing toolkit context support. Apps without a canonical
Fcitx identity, browsers, Electron editors and terminals need suitable adapters.

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

For a service and real-model transport check without opening an editor:

```sh
python scripts/try-native.py xournalpp --check
python scripts/try-native.py omawrite --check
```

These checks load the addon and exercise the C++ broker client with the real
model. They do not simulate typing through the editor, prove visible acceptance,
or measure general writing quality. Diagnostics are retained in each trial
folder; learning is blocked and retention is set to `none` for both app policies.

On the configured Omarchy workstation, click the **Badi keyboard icon just left
of Wi-Fi** for launch buttons, a session selector, request/suggestion/error counts,
and pause/resume. A zero request count means inference has not been invoked;
check the Text tool, focus, and Tab first. Counts apply only to the
selected session. See the [desktop controls runbook](../../ui/omarchy-plugin/README.md).

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
After a disconnect or policy epoch change, the native adapter requires a fresh
surrounding-text event before reading again. Typing supplies this in cooperative
fields; the explicit-manual invocation contract remains unchanged.

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

The deterministic tests cover state transitions, exact app identity,
fingerprint salting/binding, UTF-8 and output sanitization, bounded framing,
unchanged-toolkit republish handling, stale focus/revision rejection, sensitive
zero-context behavior, manual key decisions, foreign-IME yielding, duplicate
JSON-key rejection, optional `suggestion.clear` fields, and duplicate commit
authorization.

See [WIRE_PROTOCOL.md](WIRE_PROTOCOL.md) for the isolated v2 assumptions.
