# Badi Fcitx5 module

`libbadi-fcitx5.so` is a cooperative Fcitx5 **Module** addon (`badi.conf`
declares `Category=Module`). It does not register or replace an input method.
It observes Fcitx events after the active input method, with a pre-input hook
for acceptance and cancellation, and yields whenever that input method owns
preedit or candidates. It never uses evdev, `wtype`, the clipboard, a virtual
keyboard or global input capture. [WIRE_PROTOCOL.md](WIRE_PROTOCOL.md) holds
the broker and observer wire details.

## Native application contract

- **Identity.** `InputContext::program()` counts only when it is an ASCII
  identifier (`^[A-Za-z][A-Za-z0-9_-]*(\.[A-Za-z][A-Za-z0-9_-]*)*$`, at most
  128 bytes). It is folded to lowercase once at focus-in (Qt's `Telegram`
  becomes `telegram`), and that one id is used for policy, sessions, debug and
  the observer. Any other program gets no binding.
- **Policy.** Each focus or authority epoch queries broker policy before
  reading text; only a current grant opens a session. A reply still missing
  after two seconds is logged as overdue; the connection and other sessions
  continue, and the late reply still applies to its own session.
- **Keys.** **Tab** accepts a visible owned suggestion. On the manual path it
  also requests at the end of a nonempty phrase with an English, German or
  Persian input-method language and no selection. Otherwise, and whenever a
  foreign IME owns the panel, it stays the application's Tab.
  **Ctrl+Shift+Space** requests or refreshes, **Ctrl+Shift+Y** accepts, and
  **Escape** dismisses a suggestion or closes a notice. A key is consumed only
  for its matching local action.
- **Manual request.** It needs a collapsed, non-composing, non-sensitive
  surrounding-text snapshot, the live Fcitx `SurroundingText` capability and a
  valid language from the active input-method entry. Each focus epoch must
  first receive a surrounding-text update; focus-out, capability changes and
  changed broker authority reset that freshness. Until a request,
  surrounding-text events only invalidate local state and no prose is
  serialized. Missing or stale capability, sensitive, special-purpose,
  composing, selected, unknown-language and unknown-app states send no context.
- **Display.** A suggestion appears as the observer's inline preview or as the
  single candidate of Badi's own Fcitx panel, with “Badi · Tab to accept ·
  Escape to dismiss”. Thinking, no-continuation and connection notices expire
  after five seconds.
- **Acceptance.** The broker authorizes first. A matching `commit.prepare`
  causes exactly one `InputContext::commitString`, reported as
  `dispatched-unverified` because Fcitx cannot prove the client applied it. The
  candidate must still be present and owned at dispatch, and a local monotonic
  timer clears it when its lease expires, whether or not the broker's clear
  arrives.
- **Reconnect.** A closed connection retires every session, context revision,
  candidate, commit grant and pending observer reply. Reconnection uses at
  most ten retries with backoff capped at five seconds, renewed by an explicit
  key or new focus; handshakes time out after two seconds. After fresh policy
  the adapter reopens the session and republishes context before any request.
  When the new connection reports the same authority epoch, settings revision
  and pause state, Tab may read Fcitx's unchanged surrounding text while
  automatic observed requests wait for new text; changed authority needs a
  fresh surrounding-text event.
- **No replacement.** The module never negotiates `text_replacement` or
  dispatches deletion-based edits, even for observed fields: an `input` handler
  can move focus between a Fcitx deletion and insertion
  ([decision](../../docs/decisions/0003-ime-parity-append-only.md)). Unexpected
  replacement suggestions and commit grants fail closed at the wire, state and
  dispatch boundaries. Spelling stays with the editor-owned integrations.

### App classes and IME parity

One classification of the canonical id (`classifyNativeApp` in `src/state.cpp`)
decides every native path:

| Class | App ids | Edit path |
| --- | --- | --- |
| Native exact | any other granted id, e.g. `omawrite`, `com.github.xournalpp.xournalpp`, `telegram` | Manual unknown-identity contract or an observed desktop field |
| IME-parity browser | `chromium`, `chromium-browser`, `brave-origin` (also its `--app` windows, `brave-<host>__<path>-<profile>`), `zen` (Gecko) | Observed browser-origin field only, origin policy (exact rule, else `badi site all on`), append-only |
| IME-parity desktop | `chatgpt` (Codex), `code` (VS Code 1.140's `com.microsoft.VSCode` maps to it), `cursor`, `discord`, `grok-bot`; native `libreoffice` (every `libreoffice-*` program id) | Observed desktop field of the same app id only, app policy, append-only |
| Unavailable | Gecko-family browsers other than `zen`: Firefox and its channels (`firefox*`, `org.mozilla.firefox*`), PWAsForFirefox windows (`ffpwa-*`), other Zen builds (`zen-*`, `app.zen_browser.*`, `io.github.zen_browser.*`) and forks (LibreWolf, Floorp, Waterfox, Mullvad and Tor Browser, IceCat, …); `obsidian`; Chromium-family ids without an observer rule: Google Chrome and Brave (`chrome`, `google-chrome*`, `brave`, `brave-browser*`), web-app windows (`chrome-*`, `crx_*`, `msedge-*`, and `brave-*` without `__`, such as installed PWAs), Flatpak ids (`com.google.chrome`, `org.chromium.*`, `com.brave.*`, …), other Chromium browsers and channels (Edge, Vivaldi, Opera, Helium, …), `electron*`, and other VS Code/Discord builds (`code-oss`, `vscodium`, `discord-canary`, `vesktop`, …) | None; Obsidian's editor plugin owns its fields |

Case folding admits mixed-case window classes such as `Vivaldi-stable` or
`chrome-app.hey.com__-Default`; the Chromium- and Gecko-family rules keep them
from becoming native exact apps. Gecko mail clients (Thunderbird) are not
browsers and keep the native exact contract. An Electron app with an unrelated
id (a chat client, say) cannot be recognized by id: granting it uses the native
exact manual contract, whose append has the same typed-text semantics.

IME parity ([decision](../../docs/decisions/0003-ime-parity-append-only.md))
accepts through one Fcitx `commitString` that behaves like typed text. Undo may
coalesce the append with preceding typing, and page script that moves focus or
the caret during `beforeinput` may redirect it like a keystroke. The result is
`dispatched-unverified`; Badi claims neither exact undo nor verified field
authority there. Every other guard stays:

- IME-parity apps never use the unknown-identity manual path, even with a
  `linux_app` grant. Without an observed field, Tab and navigation pass through
  and no context is sent. `Ctrl+Shift+Space` requests an inspection: an absent
  observer or unobservable window shows “Badi cannot see this text field —
  check badi doctor” (`ime_parity_observer_unavailable`), and a field the
  observer denies (password, purpose, selection) shows “Badi cannot read this
  text field — run badi debug status” (`ime_parity_field_denied`).
- On an observed field Tab only accepts a visible suggestion; without one it
  stays the application's Tab (next field, indentation). `Ctrl+Shift+Space`
  inspects the field again and queries policy for its exact target.
- The observer's target kind must match the class. Otherwise authority is
  retired without reading prose (`ime_parity_target_mismatch`, or
  `ime_parity_target_invalid` for malformed metadata).
- A browser field needs an origin allow: its exact `browser_origin` rule, or
  `badi site all on` for every origin without one. The observer cannot tell
  private windows apart, so site-all covers them too.
- An app allowed only by the app blocklist mode (policy reason
  `matched_default`) opens only an observed field; on the manual
  unknown-identity path the addon treats it as denied (`app_rule_required`), so
  an app outside Badi's integrations keeps its own Tab until it gets an exact
  rule.
- Before context publication, display and `commitString`, a fresh observer
  snapshot must agree with Fcitx's live surrounding text, absolute caret and
  document length. Disagreement or an unanswered RPC fails closed with a
  content-free reason: `observer_context_mismatch`,
  `observer_display_mismatch`/`observer_display_unavailable`,
  `observer_dispatch_mismatch`/`observer_dispatch_unavailable`, or
  `ime_parity_observer_unavailable`.
- Replacement is never negotiated or dispatched. Sensitive and purpose denial,
  foreign-IME yield, revision/fingerprint/expiry binding and one-shot
  acceptance are unchanged. There are no retries or synthetic keys.

Unavailable apps acquire no prose and are never inspected. Tab and navigation
pass through; `Ctrl+Shift+Space` shows “Badi cannot safely insert suggestions
in this app yet”, which needs no surrounding text. Surrounding-text
publication keeps that notice until its five-second expiry; input, focus loss
and foreign composition clear it. A browser-origin target for a native exact
app is also unavailable. Missing, unknown or mismatched observed target
metadata retires prior authority rather than falling back to an app-wide
manual grant.

### Observed fields

The [accessibility observer](../accessibility/README.md) supplies field
identity and corroboration, and the pinned
[Wayland compatibility frontend](../../packaging/fcitx5-wayland-compat/README.md)
exact surrounding text. Neither provides an editor transaction.

Display prefers the observer's caret preview. When the observer verifies the
field but reports `rendered:false` (no calibrated geometry, for example), Badi
shows its own Fcitx panel instead. On the `wayland_v2` frontend Fcitx's cursor
rectangle is `[0,0,0,0]`; Hyprland places that panel at the application's
text-input caret rectangle. Foreign preedit or candidates take the panel over.
The addon asks the observer to hide its preview only while one may be on
screen, not on every key.

Automatic inspection runs 120 ms after Fcitx input pauses. When the observer
invalidates a field without Fcitx input (keys or changed surrounding text)
since the last inspection, a change of the field itself waits for the next
input; other invalidations inspect again after 240, 480 and 960 ms and then
wait for input (`observer_awaiting_input`), so a busy page cannot drive an
inspection loop. With `badi debug on`, a missing or invalid debug control is
rechecked at most once a second and at every focus-in.

Chromium's text-input surrounding text ends every `<p>` with `"\n\n"` and
every `<div>` line with `"\n"` when anything is rendered after the editor, so a
caret at the end of a rich editor's last block reports that suffix. For an
observed IME-parity field only, `normalizeObservedParagraphEnd()` treats
after-caret text of exactly `"\n\n"` or `"\n"` as end of field: Tab
eligibility and the broker context use `after: ""` at the unchanged caret,
while the fingerprint and every observer agreement check use the raw text
(`observedAfter()`), which the observer reproduces from the blocks
([rich editors](../accessibility/README.md#rich-editors)). Such a request
records `observed_field_request_paragraph_end`. Any other suffix (three line
breaks, text after them) stays `caret_not_at_end`, `"\r\n"` is never context,
and observer disagreement fails as `observer_context_mismatch`, recording the
content-free `before_bytes` and `after_bytes`.

Agreement is exact with two narrow equivalences. An empty Fcitx `after` agrees
with an observed block end alone, because both name the end of the field (an
EditContext editor sends its own text, and an editor last on its page gets no
block end). A snapshot with `scope: "block"`, the caret's block only because an
opaque list, quote or table lies in the window, agrees when Fcitx's `before`
equals it or ends with it right after a line break. Native exact apps and
unobserved contexts keep the unnormalized contract.

## Tested versions and boundaries

Arch Linux, native Wayland, Hyprland 0.56.2; Omawrite and Xournal++ on Fcitx
5.1.21, the rest on Fcitx 5.1.22. These are exact probe and proof cells, not
toolkit-wide claims or runtime widget selectors; the root
[README](../../README.md#coverage) holds the current per-app coverage.

| Application | How it reaches Fcitx | Boundary |
| --- | --- | --- |
| Omawrite 0.5.0 (Qt 6) | `QT_IM_MODULE=fcitx` | 20/20 visible invoke, accept, clear, save and undo trials; one native undo restores the exact prefix |
| Xournal++ 1.3.7 (GTK 3 text tool) | `GTK_IM_MODULE=fcitx` launcher override | 20/20 trials including saved `.xopp` inspection; after leaving text-edit mode with Escape, one document undo restores the prefix (Ctrl+Z inside a new empty text editor is not document undo) |
| Chromium 152, Brave Origin (Chromium 154), Electron 42 apps | text-input-v3 through `wayland_v2`, Wayland app id as `program()` | No multiline, email or other purpose hints for page fields, so the observer's purpose checks carry sensitive-field denial. The omnibox sends the `Url` purpose (capabilities `0x1072`, a page textarea `0x80072`), so `allowsNativeContext()` denies it before any observer request. The stock frontend's surrounding text stalls and fails closed; the compatibility frontend is exact after every character |
| VS Code | as above | Its default EditContext sends corrupted surrounding text, which fails closed; `"editor.editContext": false` makes it exact. 1.140 reports the Wayland app id `com.microsoft.VSCode`, and its hidden textarea is only caret-wide, so the glyph is outside the field and the Fcitx panel replaces the inline preview |
| Codex desktop (`chatgpt`, Chromium 153) | as above | ProseMirror composer: the paragraph-end rule displayed, appended once and dismissed on an isolated Chromium 152 fixture of the same shape, and in the Codex desktop 26.930 composer itself (inline preview, one append) |
| Zen 1.22.3b and 1.23b (Gecko 156.0.1, 157.0) | `wayland_v2`, `program()` exactly `zen` | One input context serves every page field and the urlbar; a field change arrives as blur, focus and `capabilities_changed`. Fields and the urlbar carry capabilities `0x72` and no `Url` hint, so the observer's Gecko rule, not `allowsNativeContext()`, denies the urlbar ([observer runbook](../accessibility/README.md#authority-boundary)). The password field adds Password and Sensitive and is denied. Gecko publishes only the caret's paragraph, so observed requests agree only while no newline precedes the caret. On 1.23b one Ctrl+Z undoes Badi's `commitString` alone and leaves the typed text, where Brave Origin undoes both together |
| Telegram (Qt) | `dbus`, `program()` `Telegram` | Canonicalized to `telegram` for observer, policy and session |

Fcitx supplies the process identity and text context but no stable widget
identity, so manual context frames declare field identity and purpose unknown
and only the broker's explicit-manual path can authorize them. The selected
input method stayed `keyboard-us`. Other fields, versions, toolkits,
terminals and unknown application ids are unverified.

## Install and roll back

`python3 scripts/install-desktop.py` builds the release broker and addon,
installs them under `~/.local/lib`, and loads the addon into
`omarchy-fcitx5.service` through a user drop-in. It enables
`badi-broker.service` on the first installation and otherwise preserves the
autostart choice; likewise for `badi-accessibility.service`, whose private
socket and exact service process it verifies before restarting Fcitx. It needs
the provisioned local model and runtime, an active graphical Omarchy/Fcitx
session and an explicitly unlocked desktop, and reports success only after
model health, addon loading and an unchanged keyboard profile check out. The
loaded-addon check compares Fcitx's mapping of the exact installed file (path,
inode and device; on Btrfs through a read-only mapping of the file) without
privileged access to process memory. No system package or global toolkit
environment changes.

`--broker-only` updates just the broker and commands while locked and leaves
the input method running; a changed addon needs the complete install after
unlock. Both keep existing settings. They also remove files of Badi's retired
browser extension and native-messaging host, backed up like any other change;
remove an unpacked Badi extension in `brave://extensions` yourself.

The broker creates `$XDG_RUNTIME_DIR/badi` with mode `0700` and a `0600`
socket, keeps the model loaded after editors close, and starts with the
graphical session. A user desktop-file override gives Xournal++
`GTK_IM_MODULE=fcitx`, which supplies the exact app identity its default
Wayland frontend lacks on this machine; relaunch it through the desktop
launcher or Badi panel. A responsive observer service does not
establish application accessibility; `badi doctor` reports it and the
accessibility bus separately.

Every run that changes a file backs up what it replaced under
`~/.local/state/badi/install-backups/<UTC time>/` and prints that directory.
`changes.json` lists each change in order: its home-relative `path`, `action`
(`create`, `replace` or `remove`), the original's location in the backup
(`saved`, under `home/`) and the `sha256` (or `symlink`) left installed. An
unchanged reinstall writes no backup. The newest three backups are kept; older
ones are deleted after their `accessibility-*.json` receipts move to
`~/.local/state/badi/install-backups/notes/<backup name>/`.

To roll back, from the unlocked graphical session:

1. Pick the backup and read its `changes.json`. An observed-app installation
   also saved `accessibility-setting.json` with the prior `bus_enabled` and
   `toolkit_accessibility` booleans. Choose the receipt of the intended restore
   point; a missing receipt does not mean `false`.
2. Stop `badi-accessibility.service` before replacing its Python files and
   `badi-broker.service` before restoring the broker; a broker-only rollback
   leaves the observer and Fcitx running. Disable a service before removing its
   unit only if this installation introduced it: existing autostart choices were
   preserved and stay. The backup records neither service state nor system
   units.
3. Restore, newest backup first when undoing several installations:

   ```sh
   python3 scripts/badi_install.py restore ~/.local/state/badi/install-backups/<UTC time>
   ```

   It restores replaced and removed originals, deletes files that had no
   predecessor, and leaves and lists (exiting 1) every path changed since that
   installation. That covers the observed-app flags files under
   `XDG_CONFIG_HOME` (`chromium-flags.conf`, `brave-origin-flags.conf`,
   `codex-flags.conf`, `code-flags.conf`, `cursor-flags.conf`), the Fcitx
   drop-ins, launcher overrides and command links; `--only PATH` limits it to
   one home-relative path or folder. To undo only the renderer accessibility
   flag, delete the `# Badi: renderer accessibility for the focused-field
   observer` line and the flag line after it. Then run
   `systemctl --user daemon-reload`, restore the prior broker and observer
   running states, restart `omarchy-fcitx5.service`, and relaunch affected
   applications. Do not start an observer whose unit was just removed.
4. After services and applications recover, restore the accessibility values
   from the receipt, the live `org.a11y.Bus` property first (changing
   `IsEnabled` can write `toolkit-accessibility`):

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

   `BADI_ACCESSIBILITY_RECEIPT` is the receipt's absolute path. Both results
   must match it once services settle; if another accessibility client changes
   them again, inspect that client. Without a receipt, leave these
   desktop-wide settings alone.

Private settings and model files stay. Recovered services and settings do not
prove prediction, insertion or undo in an application; check those separately.

## Try it

Open an editor from the Badi panel or with `badi launch xournalpp` or
`badi launch omawrite`. The local model must be provisioned; `badictl models
writing` prints the pinned download plan.

1. In Xournal++, select the **Text** tool and click a blank page. In Omawrite,
   type directly into the document.
2. Type `Please find attached the` with the caret at the end and no selection.
3. Omawrite suggests on its own. In Xournal++, press **Tab** to request.
4. Press **Tab** to accept or **Escape** to dismiss. A suggestion lasts up to
   five seconds while text and focus stay unchanged.

Suggestions continue by up to four words. German and Persian routing follows
the active input-method language and is experimental; Persian half-spaces are
kept only between the permitted Arabic letters, and other invisible formatting
is denied. Grant another cooperative app with `badi app APP_ID on`, using the
exact identity `badi debug status` reports; a grant adds no missing toolkit
context support. Apps without a canonical Fcitx identity, Gecko browsers other
than Zen, and terminals need their own adapters.

Judge usefulness and editing safety separately. Try at least 20 prefixes from
your own writing, and count late, irrelevant and invented suggestions apart
from useful ones; this is a personal trial, not a quality score. Check that
acceptance inserts exactly once, Escape leaves the text intact, native undo
removes the accepted text, and typing or moving focus while a suggestion is
visible never inserts it. A transport acknowledgement is not a visible edit.
`badictl probe -` runs disposable text through the installed model without an
editor. The [desktop controls](../../ui/omarchy-plugin/README.md) show request
and suggestion counts; a zero request count means inference never ran, so check
the Text tool, focus and Tab first.

## Build and test

Requirements are CMake, Ninja, a C++20 compiler, Fcitx5Core and nlohmann-json.

```sh
npm run fcitx5:check
```

It builds the module and runs the deterministic suites: app classes and
canonical ids, IME-parity ids equal to the observer's web-app rules
(`tests/observer-identities.py`), append-only observed fields, the pre-input
key, invoke, Tab and inspection decisions, fingerprint binding, UTF-8 and output
sanitization, bounded framing, strict wire shapes, stale focus and revision
rejection, sensitive zero-context behavior, foreign-IME yield, one-shot commit
authorization, the observer socket client, overdue policy replies, the
replacement boundary, content-free debug snapshots and reconnect freshness.

```sh
npm run fcitx5:integration
```

This also needs Cargo and Python 3. It runs the shipped transport and session
state against the real broker for both native identities in temporary XDG
roots: exact context, accept, report and dismiss counters, disconnect and
shutdown cleanup, and a broker restart while a candidate is visible, after
which the client reconnects without a new focus or key, gets fresh policy and
rejects the retired candidate. It installs nothing and edits no application.
The native Arch CI job runs it without Node.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 adapters/fcitx5/tests/observed-desktop.py
```

The actual addon in a private D-Bus session with its own Fcitx, broker and
configuration; it touches neither normal Fcitx nor the desktop. It needs the
built addon, the debug broker and CLI, Fcitx's D-Bus frontend and Unicode
module, `dbus-run-session` and Python GObject/Gio. With configured `linux_app`
and exact-origin rules it covers each app class: unavailable apps keep
Tab/navigation, read no prose and send no context, and their notice lives and
clears as described. For `brave-origin` (with Chromium's `UppercaseWords`
hint), `zen` (no hints) and `code` it proves that a declined field or absent
observer sends no manual context despite the app grant; a mismatched target
kind or observer-denied password reads no prose; automatic suggestion appears
in the Fcitx panel with its hint; Tab dispatches once and re-inspects on
request; and request, display and dispatch disagreement, or an unanswered
dispatch snapshot, fail closed with their reasons. It also covers the Zen
urlbar, foreign Unicode preedit and a rendered preview (`brave-origin`), a
peer-injected replacement closing the transport, the `chatgpt` paragraph end,
`Telegram` canonicalization, and on synthetic desktop targets absolute
caret/length gates, delayed preview cancellation, field switches, pause,
broker recovery and a native manual Omawrite append. These fixtures prove
transport and state behavior, not app compatibility, application mutation,
overlay rendering or undo.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 adapters/fcitx5/tests/desktop-tab.py
```

Opt-in, after desktop installation: synthetic Gio input contexts drive the
**installed** addon and local model through Tab request/accept, Escape,
stale-text rejection, empty/selected/mid-text pass-through and
unsupported/sensitive denial. It creates, focuses and destroys its own
contexts, so run it while not typing elsewhere. It needs the provisioned model,
an unpaused broker, both native app policies and Python Gio, and proves no
pixels, file saves or native undo.

A staged install leaves the live Fcitx configuration alone:

```sh
DESTDIR="$PWD/adapters/fcitx5/stage" cmake --install adapters/fcitx5/build --prefix /usr
```
