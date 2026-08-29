# Linux architecture recommendation

Status: decision-ready research, 2026-08-30

## Executive decision

Build Omatype as a local Rust broker with several small, target-owned adapters. Do not build it as a Wayland keylogger plus synthetic typer.

The first Omarchy/Hyprland slice should prove one broker and three integrations:

1. A Chromium Manifest V3 extension for ordinary HTML text fields.
2. An Obsidian desktop plugin for the Markdown editor.
3. A C++20 Fcitx5 module for a **manual-only** session in a live Codex prompt inside Ghostty.

The browser and Obsidian adapters own their text context, rendering, and insertion. The Fcitx adapter owns only the terminal session it explicitly arms, renders one candidate through Fcitx, and inserts through Fcitx InputContext::commitString. Hyprland supplies shortcuts and corroborating focus events; it is not a text API.

This is intentionally not a claim of universal Wayland support. On Linux, the application, toolkit, compositor, input-method route, sandbox, and field type all affect what is possible. Omatype should publish support as tested capability tuples rather than a single “Linux supported” badge.

The two-day result is a credible integration proof, not a shippable universal daemon. Browser and Obsidian can become useful quickly. Ghostty/Codex is the early go/no-go spike because it is the least certain route.

## Decisions at a glance

| Concern | Decision for the first slice | Long-term direction |
|---|---|---|
| Core | Rust 2024 broker, CLI, native-message shim, Unix-domain socket | Stable protocol and provider/adapter SDKs |
| Browser | Strict TypeScript MV3 extension, content script plus service worker | Chromium and Firefox builds; site-specific adapters for complex editors |
| Obsidian | Strict TypeScript desktop-only plugin using the official Editor API and CodeMirror 6 | First-party plugins for high-value note/editing apps |
| Terminal | C++20 Fcitx5 module; user arms it manually before typing a Codex prompt | Fcitx plus optional IBus and explicit agent/terminal bridges |
| Global control | Hyprland bindings call omatypectl; socket2 focus events | XDG GlobalShortcuts where reliable, desktop-specific adapters where needed |
| Inference | Owned llama.cpp server sidecar plus deterministic fast path | Swappable local provider interface and benchmarked model profiles |
| UI | In-app ghost text for browser/Obsidian; Fcitx candidate panel for terminal | App-owned UI first; GTK4 layer-shell only for status or coarse fallback UI |
| Insertion | DOM, CodeMirror transaction, or Fcitx commitString | Never silently fall through to generic synthetic typing |
| Policy | Local-only by default; Always, Manual, Never per app/origin; hard sensitive-field gates | Explicit remote-provider and learning grants, separately scoped |
| Packaging | Cargo workspace plus npm workspaces | Reproducible packages per distribution and signed browser releases |

## What was established from sources

This section contains sourced facts and direct observations. Recommendations and inferences begin in the next section.

### Direct observation on the target workstation

The following was observed locally on the Omarchy workstation on 2026-08-29/30. It is useful as the acceptance-test baseline, not evidence for other installations:

- Hyprland 0.56.2 and xdg-desktop-portal-hyprland 1.4.1.
- Fcitx5 and Fcitx5Core 5.1.21, with Fcitx running.
- Ghostty 1.3.1 as a native Wayland GTK application.
- Chromium 151.0.7922.173.
- Obsidian 1.13.7 as a native Wayland application.
- rustc/cargo 1.98.0, Node 26.8.1, npm 11.19.0, GCC 16.2.1, and Clang 22.1.8.
- gtk4-layer-shell 1.3.0 is present.
- CMake and pnpm are absent. The Fcitx spike therefore has an explicit CMake prerequisite; npm workspaces avoid adding pnpm to the 48-hour critical path.
- llama-server is not on PATH, and no local GGUF was found in the bounded workspace/home search. Provisioning the runtime and an evaluated model is therefore a model-demo prerequisite; the deterministic provider does not depend on it.
- The observed Hyprland window classes include com.mitchellh.ghostty and md.obsidian.Obsidian.

### Wayland and input-method boundaries

The Wayland text-input-v3 protocol lets a focused client provide surrounding text, cursor/anchor positions, content hints and purpose, and a cursor rectangle to an input method. Its content purposes include password, PIN, and terminal, while hints include hidden and sensitive data. The protocol asks clients to keep surrounding text bounded and explicitly permits them to omit it. Its preedit and commit operations are synchronized by client state and serials. These are client/input-method contracts, not a compositor-neutral API by which any daemon can inspect another application's field. See the official [text-input-v3 protocol XML](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/unstable/text-input/text-input-unstable-v3.xml).

The external zwp_input_method_v2 protocol used by wlroots-family stacks gives the compositor-selected input method commit, preedit, surrounding-text, keyboard-grab, and popup-surface operations. It permits at most one input-method object per seat. A separate Omatype input-method client would therefore compete with Fcitx rather than complement it. See the upstream wlroots [input-method-v2 protocol XML](https://github.com/swaywm/wlroots/blob/master/protocol/input-method-unstable-v2.xml). Wayland-protocols now also carries a newer, explicitly experimental [xx-input-method-v2 proposal](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/experimental/xx-input-method/xx-input-method-v2.xml); it retains the one-input-method-per-seat constraint and is not assumed to replace the observed Hyprland/Fcitx route.

Layer shell creates separate compositor-managed surfaces. It does not place an inline widget inside an unrelated application's text layout. See the upstream wlroots [wlr-layer-shell protocol XML](https://github.com/swaywm/wlroots/blob/master/protocol/wlr-layer-shell-unstable-v1.xml).

The virtual-keyboard protocol is a compositor-exposed integration with synthetic keyboard semantics, not a field-aware edit operation. It cannot prove the recipient, selection, revision, password state, or framework acceptance. See the upstream wlroots [virtual-keyboard protocol XML](https://github.com/swaywm/wlroots/blob/master/protocol/virtual-keyboard-unstable-v1.xml).

### Fcitx5 and IBus

Fcitx documents substantial route differences among GTK, Qt, Chromium/Electron, XWayland, compositors, and native Wayland. It also documents popup-position limitations and notes that a compositor input-method route may expose one global input context, making application identity harder to determine. This is the strongest reason to test precise environment tuples rather than infer support from “Wayland.” See [Using Fcitx 5 on Wayland](https://fcitx-im.org/wiki/Using_Fcitx_5_on_Wayland/en) and [How an application talks to Fcitx](https://fcitx-im.org/wiki/How_does_an_application_talk_to_Fcitx).

Fcitx's native InputContext API exposes the program identifier, capabilities, surrounding text, cursor rectangle, preedit/user-interface updates, and commitString. See [Fcitx InputContext](https://github.com/fcitx/fcitx5/blob/master/src/lib/fcitx/inputcontext.h). Fcitx capability flags include password, sensitive-data, surrounding-text, and preedit signals; see [fcitxflags.h](https://github.com/fcitx/fcitx5-gtk/blob/master/gtk2/fcitxflags.h). Fcitx's own keyboard engine avoids prediction in password, sensitive, and no-spell contexts; see [keyboard.cpp](https://github.com/fcitx/fcitx5/blob/master/src/im/keyboard/keyboard.cpp). Its Qt integration also suppresses surrounding text for password/sensitive fields; see [qfcitxplatforminputcontext.cpp](https://github.com/fcitx/fcitx5-qt/blob/master/qt5/platforminputcontext/qfcitxplatforminputcontext.cpp).

IBus Engine has corresponding preedit, commit, surrounding-text, content-type, and cursor-location operations. IBus input purposes include password and PIN. See the official [IBusEngine reference](https://ibus.github.io/docs/ibus-1.5/IBusEngine.html) and [IBus types](https://ibus.github.io/docs/ibus-1.5/ibus-ibustypes.html).

### Hyprland and portals

Hyprland exposes two Unix sockets. Its event socket includes active-window changes, so a broker can subscribe rather than repeatedly run synchronous hyprctl calls. Hyprland warns that excessive synchronous hyprctl calls can slow the compositor. See [Hyprland IPC](https://wiki.hypr.land/IPC/) and [Using hyprctl](https://wiki.hypr.land/Configuring/Advanced-and-Cool/Using-hyprctl/).

The XDG GlobalShortcuts portal creates user-approved shortcut sessions and reports activations independently of application focus. It is a suitable future control-plane option. It does not provide text context or insertion. See the [GlobalShortcuts portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html).

The portal collection covers deliberately scoped desktop capabilities. InputCapture is a compositor-mediated, session-based pointer/keyboard capture mechanism, not permission for ambient keylogging or foreign text editing. See the [portal API overview](https://flatpak.github.io/xdg-desktop-portal/docs/) and [InputCapture](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.InputCapture.html).

### AT-SPI

AT-SPI Text can expose text, caret offsets, and character/range extents, while Component exposes screen/window geometry. Applications may expose roles such as password text and terminal. These are application-provided accessibility interfaces; their presence, completeness, and timing vary. See [AT-SPI Text](https://gnome.pages.gitlab.gnome.org/at-spi2-core/devel-docs/doc-org.a11y.atspi.Text.html), [AT-SPI Component](https://gnome.pages.gitlab.gnome.org/at-spi2-core/devel-docs/doc-org.a11y.atspi.Component.html), and [AT-SPI roles](https://gnome.pages.gitlab.gnome.org/at-spi2-core/libatspi/enum.Role.html).

### Browser, Obsidian, and Electron

Browser content scripts can inspect and modify a permitted page's DOM, but run in an isolated world and have only a subset of extension APIs. A service worker or extension page must relay native messages. Chrome launches a registered native host over length-prefixed JSON on stdin/stdout; the host manifest restricts allowed extension origins. See [content scripts](https://developer.chrome.com/docs/extensions/develop/concepts/content-scripts), [native messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging), and [optional permissions](https://developer.chrome.com/docs/extensions/reference/api/permissions). Firefox documents the same content-script-to-background relay requirement in [native messaging](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/Native_messaging).

Optional host permissions allow an extension to request sites at runtime rather than start with all-sites access. This supports per-origin consent. See [Chrome permissions](https://developer.chrome.com/docs/extensions/reference/api/permissions) and [Firefox optional_host_permissions](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/manifest.json/optional_host_permissions).

Obsidian's supported Editor API includes reads and replaceRange, and its editor-extension API supports CodeMirror 6 extensions. CodeMirror exposes state transactions, decorations, and widgets. See [Obsidian Editor](https://docs.obsidian.md/Plugins/Editor/Editor), [Obsidian editor extensions](https://github.com/obsidianmd/obsidian-developer-docs/blob/main/en/Plugins/Editor/Editor%20extensions.md), [CodeMirror reference](https://codemirror.net/docs/ref/), and [decorations](https://codemirror.net/examples/decoration/). Obsidian's community requirements distinguish desktop-only plugins that use Node/Electron APIs; the broker bridge must declare itself desktop-only. See [plugin submission requirements](https://docs.obsidian.md/community-directory/submission-requirements-for-plugins).

Electron recommends context isolation and narrow, validated IPC bridges. A generic preload monkeypatch across arbitrary Electron apps would violate that boundary and is not a viable product integration. See [context isolation](https://www.electronjs.org/docs/latest/tutorial/context-isolation) and [Electron security](https://www.electronjs.org/docs/latest/tutorial/security).

### Terminals and confinement

Ghostty shell integration can mark ordinary shell prompt/output boundaries, and Ghostty has static keybinding actions. Those signals describe a shell, not an interactive Codex TUI's internal prompt. They cannot be used as evidence that a Codex prompt is safe or active. See [Ghostty shell integration](https://ghostty.org/docs/features/shell-integration), [configuration reference](https://ghostty.org/docs/config/reference), and [keybind actions](https://ghostty.org/docs/config/keybind/reference).

The current Ghostty documentation exposes shell integration and static keybinding actions, but no supported general Linux remote-control path was found; an upstream feature request likewise reports the absence of a kitty-like API. Linux accessibility support was still being discussed as an out-of-tree proof of concept, so this design must not depend on Ghostty AT-SPI text access. See the upstream [remote-control issue](https://github.com/ghostty-org/ghostty/issues/11447) and [Linux accessibility discussion](https://github.com/ghostty-org/ghostty/discussions/13746). Kitty's explicitly enabled remote-control interface is a possible future terminal-specific adapter, not a Ghostty fallback; see [kitty remote control](https://sw.kovidgoyal.net/kitty/remote-control/).

Strictly confined browsers complicate native-messaging host discovery and execution. Firefox's native-messaging portal work is explicitly a design in progress, and Flatpak's xdg-native-messaging-proxy warns about its security limitations. Treat Flatpak/Snap browser support as unverified, not inherited from the unpackaged browser. See [Firefox native-messaging portal design](https://firefox-source-docs.mozilla.org/toolkit/components/extensions/webextensions/native-messaging-portal-design.html) and [xdg-native-messaging-proxy](https://github.com/flatpak/xdg-native-messaging-proxy/blob/main/README.md).

### Local inference

llama.cpp's server offers local completion endpoints, streaming, prompt caching, and an OpenAI-compatible HTTP surface. See the official [llama-server documentation](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md) and [speculative decoding documentation](https://github.com/ggml-org/llama.cpp/blob/master/docs/speculative.md).

## Architectural inferences and recommendations

Everything in this section is a design inference from the constraints above. It must be validated on the named workstation.

### Why the prior prototype is not the base

The read-only cotype prototype is useful as a demo and risk register: it has a deterministic predictor, a basic stale-snapshot check, Hyprland focus lookup, a layer-shell pill, raw evdev capture, and wtype insertion. It should not become the production base.

Its pointer position is not a caret position. Window-title substrings are not a sensitive-field guarantee. Raw input observes much more than Omatype needs, and synthetic typing cannot atomically prove field identity or revision. Keep only the lessons: latest-wins prediction, visible state, explicit pause/dismiss, and an easily replayed deterministic backend.

### Component topology

    Chromium content script ─ service worker ─ native host ┐
    Obsidian desktop plugin ───────────────────────────────┤
    Fcitx5 C++ module ─────────────────────────────────────┤
    omatypectl / Hyprland bindings ────────────────────────┤
                                                           ▼
                                                Rust broker over UDS
                                             policy · sessions · cancellation
                                                provider arbitration · metrics
                                                  │                  │
                                      deterministic provider   llama-server

Hyprland socket2 events feed focus evidence to the broker. They never authorize an edit by themselves. Each adapter remains the authority on its own active field and performs the final revalidation and insertion.

### Languages and frameworks

**Broker, CLI, and native host: Rust 2024.** Use Tokio for asynchronous sockets/process ownership, Serde plus JSON Schema for the protocol, tokio-util CancellationToken for latest-wins cancellation, keyed BLAKE3 with a per-session random key for ephemeral context fingerprints, and tracing with a metadata-only subscriber. Rust makes the concurrency and cancellation state machine explicit and produces small native binaries. The local Rust 1.98 toolchain is sufficient.

**Browser: strict TypeScript, MV3/WebExtension APIs, npm workspaces, and esbuild.** Do not add React for a small shadow-DOM overlay. A content script owns field observation/rendering and a service worker owns native messaging. Build Chromium first; keep a small compatibility layer for Firefox. Runtime-granted origin permissions are mandatory.

**Obsidian: strict TypeScript and the supported Obsidian/CodeMirror 6 APIs.** Render a decoration/widget, insert with one CodeMirror transaction or Editor.replaceRange, and register the plugin as desktop-only. Do not inject an Electron preload or patch internal Obsidian modules.

**Terminal: C++20 Fcitx5 module addon.** This is the native extension language and API. Build with CMake and link Fcitx5Core; pin a header-only JSON codec such as nlohmann/json for the protocol subset. Installing CMake is the first host prerequisite. The module/addon approach is preferred over a new input-method engine because it can coexist inside the running Fcitx instance, but coexistence and event ordering are hypotheses that the first spike must prove.

**Later IBus adapter: C with GLib/GObject.** Implement a native IBus engine/extension only after the Fcitx slice. An IBus engine can displace the user's currently selected engine, so do not promise seamless coexistence with an existing CJK engine until tested.

**Fallback/status UI: Rust GTK4 plus gtk4-layer-shell, later.** It may show pause state, errors, or a coarse manual popup. It is not inline text and is not a terminal acceptance fallback.

Use a Cargo workspace and npm workspaces in the first 48 hours. pnpm is not required.

### Integration ladder

Use the following order, per target:

1. **App-owned adapter:** browser DOM, Obsidian/CodeMirror, or another supported editor API. This has the best context, caret, composition, and insertion semantics.
2. **Existing input-method framework:** Fcitx5 on the target Hyprland stack, later IBus on GNOME-oriented stacks. Support is conditional on real capabilities.
3. **AT-SPI observation:** opportunistic context/caret assistance only when role, state, bounds, and timing are reliable. It is not the generic insertion path.
4. **Explicit manual/status UI:** useful when an app cannot host inline UI, but only if a safe native insertion adapter still exists.
5. **Unsupported:** if there is no trustworthy insertion route, say so.

There is no production rung for raw evdev capture, screenshots/OCR, clipboard scraping, wtype, virtual-keyboard injection, or xdotool.

### Broker and adapter protocol

Maintain one versioned JSON Schema as the source of truth. Generate Rust and TypeScript types and a small C++ DTO subset. The protocol is an adapter boundary, not a model-vendor API.

The Unix transport is a filesystem socket at $XDG_RUNTIME_DIR/omatype/broker.sock. Create the parent directory as mode 0700 and the socket as 0600; reject peers whose SO_PEERCRED UID differs. Use a four-byte little-endian length followed by UTF-8 JSON and impose a 64 KiB limit in both directions.

The Chromium native host uses the browser's native byte-order length framing externally and the same versioned JSON body. It validates the extension origin passed by the browser, then relays to the Unix socket. It never listens on TCP. Chrome's allowed_origins manifest entry is part of the installation.

The common envelope is:

    {
      "v": 1,
      "id": "optional request id",
      "type": "context.changed",
      "session_id": "128-bit random id",
      "focus_epoch": 12,
      "revision": 47,
      "mono_ms": 81234567,
      "payload": {}
    }

Each connection begins with hello carrying protocol minimum/maximum, adapter name/version, target identity, and capabilities. The broker selects exactly one protocol version and returns enabled capabilities. Unknown versions or undeclared capabilities close the connection.

An adapter session is identified by three values:

- session_id: random and unique until the target closes.
- focus_epoch: increments whenever the target field/input context loses or gains focus.
- revision: increments on text, selection, composition, field-purpose, or policy-relevant changes.

Required message families are:

| Direction | Message | Purpose |
|---|---|---|
| Adapter → broker | session.open, session.close | Advertise and retire a target |
| Adapter → broker | context.changed | Send bounded context and capabilities after local gating |
| Adapter → broker | suggest.request, suggest.cancel | Start or cancel one generation |
| Broker → adapter | suggestion.show, suggestion.clear | Render or clear a revision-bound suffix |
| CLI/adapter → broker | control.request | Manual request, accept-word, accept-all, dismiss, pause |
| Broker → adapter | commit.prepare | Ask the sole current adapter to revalidate and insert |
| Adapter → broker | commit.result | Applied, dispatched-unverified, stale, blocked, or failed |
| Any → broker | health/status | Capability and diagnostic data without prose |

context.changed contains only what the policy permits: a bounded before/after window, selection range, language hint, composition state, field purpose, editable/multiline flags, app identifier, and browser origin. Browser scope is scheme/host/port only, never path, query, or page title. Obsidian does not send vault path or note title. Terminal context is the explicit armed buffer described below, never scrollback or PTY output.

The adapter computes a context fingerprint over the normalized bounded text, selection, target token, focus epoch, and revision. The broker echoes it with the suggestion. The hash detects races inside a trusted same-user deployment; it is not authentication.

The first protocol supports suffix insertion only. Rewrites and arbitrary replacement ranges are deferred because their revalidation and user-understanding costs are higher.

### Event, cancellation, and insertion flow

The end-to-end invariant is: **only the adapter that owns the still-focused, unchanged target can insert.**

1. The adapter sees an eligible change and first checks editability, field purpose, composition, app/origin policy receipt, and its local target identity.
2. It increments revision, clears any visible suggestion, and emits context.changed. Sensitive failures emit only a reason code and zero text bytes.
3. The broker cancels the preceding session task immediately. There is at most one live generation per session and one displayable suggestion per target.
4. After the configured debounce, the broker evaluates policy and selects deterministic or local-model inference. A provider may continue computing after cancellation, but its result can no longer become displayable.
5. suggestion.show carries session_id, focus_epoch, revision, fingerprint, suggestion_id, expiry, and insertion text. The adapter drops any mismatch.
6. A local key or Hyprland binding sends a control request. The broker accepts it only when exactly one current focused session has a non-expired suggestion. Ambiguity clears all candidates.
7. The broker derives accept-word or accept-all text, removes forbidden controls, and sends commit.prepare to that same adapter.
8. The adapter re-reads focus, composition, text/selection, purpose, revision, and fingerprint. Any difference returns stale or blocked and inserts nothing.
9. The adapter performs one native mutation: DOM edit, CodeMirror transaction, or one Fcitx commitString dispatch. It never retries with a different mechanism.
10. The adapter clears its UI and reports the most accurate outcome. Browser/Obsidian can report applied after verifying their own state. Fcitx commitString has no reliable application-level acknowledgment in this design, so terminal reports dispatched-unverified rather than claiming insertion.

Any new keystroke, selection movement, composition event, target focus change, origin navigation, policy change, pause, provider restart, or timeout cancels and clears. UI hiding should not wait for the model process to acknowledge cancellation.

accept-word uses a shared Unicode word-boundary fixture. Leading whitespace attaches to the first accepted word. The unaccepted suffix may remain cached only after an app-owned adapter reports a matching applied revision. The first terminal adapter disarms after either acceptance and never carries a suffix across a commit.

### Browser adapter

The content script supports these first:

- text-like input elements excluding sensitive types/purposes;
- textarea;
- simple contenteditable regions whose DOM selection and input behavior pass the conformance suite.

It keeps a WeakRef-like target token, watches beforeinput, input, selectionchange, focus, blur, compositionstart/end, and navigation, and owns the ghost-text overlay. For input/textarea it revalidates value and selection, uses setRangeText, emits the appropriate input signal, then verifies value/selection. For contenteditable it uses a tested browser editing operation and verifies the resulting DOM selection.

Framework-controlled fields can reject programmatic value changes, and script-created events are not trusted user events. Therefore React-controlled editors, cross-origin frames, canvas editors, Google Docs, Monaco, ProseMirror, and CodeMirror-in-web-pages are conditional or unsupported until each adapter/test passes. Do not claim that “contenteditable” means universal support.

The extension starts with no broad host access. The user grants an origin, then selects Always, Manual, or Never for that origin. The content script hard-denies password/hidden fields and autocomplete purposes for current/new password, one-time code, and payment secrets before any text crosses extension messaging.

### Obsidian adapter

Scope the first version to the desktop Markdown source editor. The plugin gets editor state through the supported API, uses a CodeMirror StateField/Decoration for ghost text, and commits through one transaction tagged with a distinct user-event annotation so undo is coherent.

The target receipt includes EditorView identity, document length/hash window, selection, composition state, focus epoch, and revision. On any mismatch it clears. Reading view, Canvas, Properties controls, embedded web views, and third-party editor views are outside the first support cell.

The desktop-only plugin connects directly to the broker's Unix socket using its allowed Node runtime. It sends neither note title nor vault/file path. A per-vault policy is possible later, but the initial scope is the Obsidian application plus Manual/Always/Never mode.

### Exact terminal MVP contract

The 48-hour terminal result is narrowly defined:

> On the named Hyprland/Fcitx5/Ghostty versions, an Fcitx5 C++ module connects to the Rust broker. While a live Codex TUI prompt is focused, the user explicitly arms a session, types a one-line prompt, sees one Fcitx candidate near the terminal caret, and accepts text through one Fcitx commitString dispatch. Nothing runs automatically in an ordinary shell.

There is **no automatic shell-versus-agent detection** in this milestone. Ghostty shell integration, OSC 133 markers, process-tree inspection, window titles, and prompt heuristics do not change this contract.

The state machine is:

    DISARMED
      └─ explicit manual request → ARMED
           ├─ printable key/backspace → ARMED (ephemeral buffer updated)
           ├─ idle debounce → GENERATING → SHOWING
           ├─ accept-word/all → COMMITTING → DISARMED
           └─ Enter, Escape, navigation, paste chord, focus loss,
              existing IME composition, timeout, pause, or dismiss → DISARMED

While DISARMED, the addon sends no terminal text, keeps no key buffer, and requests no suggestion. Arming is focus-bound and lasts at most 60 seconds. The user must arm **before** typing the prompt. The MVP observes and forwards ordinary keys; it reconstructs only printable post-arm key events and Backspace for its ephemeral buffer. It does not read terminal scrollback, PTY output, process memory, or pre-arm text. Cursor movement, selection, paste, control/meta editing, Enter, or an unmodeled key disarms and zeroes the buffer without swallowing the user's key.

The addon first denies Fcitx contexts marked password or sensitive. Ghostty generally cannot know that a TUI is showing sudo/password input, so this is not a complete password guarantee. Terminal mode is consequently forced to Manual, local inference only, no learning, no persistence, and a visible armed indicator. The residual risk is explicit user arming at a secret prompt.

If another input method has active preedit or candidates, Omatype yields, clears, and does not consume the key. The addon may not disturb the active engine or change the user's input-method group. Its revalidation distinguishes its own candidate from any foreign/competing candidate state.

Suggestions are one line, at most 64 Unicode scalar values and eight words. Before commit, reject all C0/C1 controls, DEL, escape, newline, carriage return, and tab; do not merely escape them. Omatype never appends Enter. A terminal completion must be inert text until the user separately submits it.

#### Terminal pass, fallback, and fail semantics

Presentation is the standard Fcitx candidate/input panel tied to the current InputContext. There is no “inline terminal ghost text” claim.

The positioning cell passes only if all of the following hold in 20 repeated live trials:

- the current Fcitx InputContext exposes a non-empty cursor rectangle;
- the candidate appears on the same monitor/workspace as Ghostty;
- the closest candidate-panel edge is within 96 logical pixels of the advertised cursor rectangle;
- the panel does not take keyboard focus, cover the typed line, or remain after clear/focus loss.

If the cursor rectangle is missing or implausible, suppress the suggestion before display. If the actual Fcitx panel is misplaced, blinking unusably, on the wrong monitor, or stuck, mark the Ghostty compatibility profile disabled with reason candidate-position-unreliable. A content-free status notification may explain the failure. A pointer-position layer-shell pill is **not** a passing fallback.

Commit passes only if 20 repeated live trials insert exactly the prepared UTF-8 bytes into the same active Codex input, leave the cursor after that text, and never submit or execute it. Immediately before dispatch, the addon must still have the same current InputContext, focus epoch, armed revision, and no competing preedit/candidate state.

At runtime commitString is dispatched once and UI is cleared. Because it does not prove what the TUI accepted, the result is recorded as dispatched-unverified and never auto-retried. If no current context exists, the call cannot be made, observed bytes differ, text lands in another target, or a trial submits input, mark terminal insertion unsupported and fail the terminal milestone.

There is no functional fallback for failed Fcitx positioning or commit in the two-day slice: no wtype, virtual keyboard, clipboard/paste, raw evdev, Ghostty static keybind, layer-shell insertion, or shell function. Browser and Obsidian may still work, but the claimed three-app vertical slice has failed.

A Bash/readline hook can be researched later for ordinary interactive shell lines. It does not see or control a Codex raw-mode TUI and **does not count as Codex terminal support**.

### Hyprland control plane

Use explicit, configurable Hyprland bindings to invoke:

- omatypectl request
- omatypectl accept-word
- omatypectl accept-all
- omatypectl dismiss
- omatypectl pause toggle

The CLI sends a control request to the broker; it does not synthesize a key. App adapters may also offer local shortcuts.

Subscribe once to Hyprland socket2 for active-window changes. A focus change increments the broker focus epoch and clears candidates. Query activewindow only for startup/recovery, not on every keystroke. Window class is policy evidence, never proof of the active field. Do not write a Hyprland compositor plugin for this slice: it would add in-process ABI/version coupling without adding text semantics.

### Policy and privacy model

Policy has independent axes:

- activation: Always, Manual, Never;
- context access: none or bounded text;
- inference: deterministic/local, and later explicitly granted remote;
- learning: off or explicitly enabled;
- retention: ephemeral or explicitly configured.

Evaluation order is hard field deny, global pause, origin Never, app Never, terminal constraints, explicit Manual arm, scoped Always, then default Manual. Deny wins over allow. Unknown app identity, origin, or field capability falls back to Manual/local/no-learning; it never inherits Always.

Hard-deny before transport:

- browser password/hidden fields and recognized password, OTP, PIN, or payment-secret purposes;
- Fcitx/IBus password or sensitive capabilities;
- active composition/preedit that Omatype does not own;
- lock-screen/session-authentication surfaces;
- any non-editable, ambiguous, or multiply focused target.

Absence of a sensitive hint is not proof of safety. This matters especially in terminals.

The initial product is local-only. llama-server binds to 127.0.0.1 on a random owned port and is started/stopped by the broker. No LAN binding and no remote provider. A future remote provider requires a separate provider grant and app/origin grant, plus a visible indicator; enabling an origin must not silently enable remote transmission.

Retain no raw prose in logs. Structured diagnostics may include adapter/version, policy result, byte counts, cancellation reason, latency bucket, and error code. Do not log origins by default; use a keyed local pseudonym if aggregate per-origin diagnostics are explicitly enabled. Do not use clipboard, screen capture, shell history, note paths, or browser URL paths.

Keep text in bounded in-memory buffers and zero terminal armed buffers on disarm. Disable core dumps for the service. Be honest that managed strings, allocator copies, swap, and crash behavior make “zero physical remnants” impossible to guarantee without a deeper memory-hardening effort.

Socket permissions and SO_PEERCRED protect against other Unix users. They do not provide cryptographic application attestation against a malicious process already running as the same user; that is outside the first threat boundary.

### Local inference strategy

Run two providers behind one Rust trait:

1. A deterministic phrase/n-gram provider that is instant, replayable, and guarantees an integration result even while model plumbing is being debugged.
2. An owned llama-server child process using a small quantized GGUF profile, streaming completions over loopback with prompt caching.

Do not select a model by reputation alone. Run a short bake-off of 1–3B-class, 4-bit local models on a user-owned/synthetic browser/note/agent-prompt corpus and record first-token latency, memory, cancellation behavior, repetition, and human acceptance. The deterministic result proves plumbing; the local-model result is required before claiming useful semantic prediction.

Prompts should request a short suffix, never an answer to the user's prose. Stop at newline and cap output at eight words/64 characters. The broker validates and truncates provider output again.

## Compatibility matrix

Statuses are intentionally narrow:

- **First-slice:** required to pass on the observed tuple.
- **Conditional:** architecture exists but the tuple/field must pass tests.
- **Manual:** no ambient suggestions; explicit session only.
- **Unsupported:** no safe route is claimed.

| Environment / target | Context and insertion route | Status |
|---|---|---|
| Chromium 151, unpackaged, ordinary input/textarea | MV3 content script, DOM revalidation and insertion | First-slice |
| Chromium simple contenteditable | Content script plus selection/DOM verification | Conditional; include only passing fixtures |
| Google Docs, canvas editors, Monaco, complex web editors | Requires site/editor-specific integration | Unsupported in first slice |
| Cross-origin iframe | Requires explicit frame origin permission and tests | Unsupported in first slice |
| Firefox, unpackaged | Shared WebExtension design plus Firefox native host | Planned/conditional, not a 48-hour acceptance cell |
| Flatpak/Snap Chromium or Firefox | Confinement-specific native messaging/portal work | Unsupported in first slice |
| Obsidian 1.13.7 desktop Markdown editor | Official Editor and CodeMirror 6 plugin | First-slice |
| Obsidian Canvas, reading view, Properties, third-party views | Per-view integration required | Unsupported in first slice |
| Other Electron/note apps | First-party plugin or app API | No generic preload injection; unsupported until adapter exists |
| Ghostty 1.3.1 + live Codex prompt + Fcitx5 5.1.21 + Hyprland 0.56.2 | Manually armed Fcitx addon, candidate panel, commitString | First-slice manual proof gate |
| Ordinary Ghostty shell | Same adapter only if manually armed | No ambient behavior; not evidence of Codex support |
| Ghostty without usable Fcitx key/cursor/commit path | None | Unsupported; fail terminal cell |
| Bash/readline hook | Shell buffer hook | Ordinary shell experiment only; never Codex TUI coverage |
| kitty with user-enabled remote control | Future terminal-specific adapter | Conditional/later |
| GTK3/4 or Qt5/6 app under Wayland | Fcitx toolkit/compositor route | Conditional per tuple |
| XWayland app | Fcitx XIM/toolkit route | Conditional per tuple |
| GNOME/IBus environment | Future IBus adapter | Planned, not inherited from Fcitx result |
| AT-SPI-exposing app | Context/caret assistance | Conditional observation only |
| Layer-shell status/pill | Separate surface | Status/manual UI only; never safe insertion |

## Forty-eight-hour implementation plan

The work is protocol-first and risk-first. Parallel agents should use isolated worktrees/branches, own disjoint directories, and integrate only through frozen protocol fixtures. One integration owner controls the schema and release branch.

| Time | Integration owner | Broker/model lane | Browser lane | Obsidian lane | Fcitx/Hyprland lane | QA/privacy lane |
|---|---|---|---|---|---|---|
| 0–4 h | Freeze v1 schema, golden transcripts, support tuple | UDS broker skeleton and fake provider; provision llama-server/model off the critical path | MV3 scaffold | Plugin scaffold | Install CMake; prove addon loads and sees Ghostty context | Acceptance harness and sentinel cases |
| 4–12 h | Review capability handshake | Session/cancel state machine, CLI | input/textarea context and overlay | CM6 decoration/context | **Go/no-go:** manual arm, printable events, cursor rect, panel | Protocol/policy unit tests |
| 12–24 h | Integrate fake-provider E2E | deterministic provider and policy | native host, verified insert | broker socket, verified transaction | broker socket and commitString proof | focus/composition/password races |
| 24–34 h | Freeze feature scope | llama-server sidecar/streaming | optional origins/modes | app modes and undo | exact control stripping, yield to IME | latency and redaction harness |
| 34–44 h | Three-app integration | cancellation/health hardening | real Chromium matrix | isolated test vault | 20-trial live Codex test | chaos, stale result, sentinel execution |
| 44–48 h | Tag capability report | measured metrics | package/test notes | package/test notes | pass or explicit unsupported result | privacy report and demo script |

The Fcitx go/no-go happens early. If the current Ghostty path does not deliver ordinary printable Fcitx key events, a usable cursor rectangle/panel, and exact commitString behavior in the Codex TUI, stop that lane and report unsupported. Do not spend the remaining time disguising the failure with a shell hook or synthetic typer.

Multi-agent merge rules:

- The schema and golden transcript are the contract; adapters develop against a fake broker and the broker tests replay adapter transcripts.
- Each lane owns separate packages/directories. No two agents edit the schema concurrently.
- Every adapter publishes a machine-readable capability manifest and an unsupported-reason code.
- Changes merge only after schema conformance, stale-result, and sensitive-zero-byte tests pass.
- One integration captain runs the real desktop tests because focus, Fcitx, and Hyprland state cannot be reliably shared across concurrent GUI test agents.

## Acceptance criteria

The vertical slice is complete only when all required cells pass on the observed workstation:

1. Chromium: type in a normal input and textarea, see a suggestion, accept word/all, dismiss, pause, and switch Manual/Always/Never for an origin.
2. Obsidian: repeat in the Markdown editor; acceptance is one undoable transaction and survives mid-line editing/selection tests.
3. Ghostty/Codex: follow the exact manual terminal contract and pass all positioning/commit trials. An ordinary shell produces zero requests while not armed.
4. A browser password field emits zero context bytes, zero suggest.request messages, and zero provider calls. Fcitx password/sensitive fixtures do the same.
5. A focus change, revision change, selection change, composition start, or late provider result produces zero stale displays and zero stale inserts.
6. Terminal sentinel text containing shell metacharacters remains inert, and attempted newline, carriage return, tab, ESC, C0/C1, and DEL provider output is rejected. Nothing is submitted.
7. Killing/restarting the local model, native host, or adapter clears UI and does not insert cached text.
8. The final compatibility report names exact versions and records unsupported cells; no result is generalized to all Wayland sessions.

If browser and Obsidian pass but the Fcitx/Ghostty/Codex proof fails, the result is a useful two-adapter prototype, not completion of the three-app goal.

## Test strategy

### Protocol and broker

- JSON Schema conformance in Rust, TypeScript, and C++; golden encode/decode transcripts.
- Frame-size, malformed UTF-8/JSON, unknown version, capability escalation, and reconnect tests.
- Property/fuzz tests for the session state machine, Unicode accept-word behavior, and terminal sanitizer.
- Fake-clock tests for debounce, expiry, focus epochs, revision changes, cancellation, and late streaming chunks.
- Policy-table tests for deny precedence and a spy provider that proves zero calls/bytes for hard-denied fields.
- Log snapshot tests that reject raw context, origins, paths, and suggestion text.

### Browser

Use Playwright against real Chromium fixtures for input, textarea, mid-line caret, selection, undo, composition, password, framework-controlled input, simple contenteditable, focus races, navigation, and removed DOM nodes. Only fixtures that verify exact insertion enter the supported matrix. Add Firefox after Chromium rather than inferring compatibility.

### Obsidian

Test pure CodeMirror state transitions/decorations in unit tests, then use a disposable test vault for focus, selection, composition, undo/redo, view changes, plugin reload, broker death, and rapid edits. Do not run the tests against the user's vault.

### Fcitx/terminal

Use unit fixtures for the arm/disarm reducer and sanitizer, an Fcitx test frontend where possible, then real nested/isolated Hyprland smoke tests. The release gate is the current real Hyprland/Fcitx/Ghostty/Codex tuple. Test existing IME preedit/candidates, focus races, missing cursor rectangles, multi-monitor placement, broker death, max TTL, control/meta keys, paste, terminal resize, and the 20 positioning/commit trials.

Capture a PTY/test-command sentinel only to verify that accepted text is not submitted or executed. It is not a context source. A Bash/readline success cannot replace the Codex TUI test.

### Provider and quality

Replay user-owned or synthetic browser prose, Markdown, code-adjacent notes, and agent prompts. Record latency, empty/bad suffix rate, repetition, memory, and blinded acceptability. Keep functional integration pass/fail separate from model-quality claims.

## Latency and resource budgets

These are targets to measure, not current results:

| Metric | Target on the named i7-12700H/16 GB workstation |
|---|---|
| Keystroke debounce | 120–180 ms, adaptive |
| Deterministic provider computation | p95 ≤ 15 ms |
| Deterministic visible result after debounce | p95 ≤ 50 ms |
| Warm local-model first usable suffix, end-to-end | p50 ≤ 250 ms, p95 ≤ 500 ms |
| Late-result TTL | 600 ms; later results never display |
| Accept to verified app insertion | p95 ≤ 30 ms browser/Obsidian |
| Accept to Fcitx dispatch | p95 ≤ 50 ms, still reported unverified |
| Cancel/focus-loss to hidden UI | ≤ 32 ms |
| Stale display or insertion | exactly zero in the race suite |
| Suggestion size | ≤ 8 words and ≤ 64 Unicode scalars |
| In-flight work | one generation per session |

Model cold start is excluded from the interactive loop and must be surfaced separately. Keep the sidecar warm after opt-in, measure resident memory, and degrade to deterministic/manual mode if memory pressure or repeated timeout occurs.

## Long-term architecture

Phase 1 hardens the three adapters, installer, crash containment, capability report, origin/app policy UI, and benchmarked local model. It does not widen the generic Wayland claim.

Phase 2 adds Firefox, specific high-value web-editor adapters, and more first-party note/editor plugins. Each retains target-owned insertion.

Phase 3 tests Fcitx tuples across GTK/Qt/XWayland and implements an IBus adapter for GNOME-oriented systems. Coexistence with existing IMEs remains a release gate. XDG GlobalShortcuts can replace compositor-specific bindings where its portal implementation is reliable.

Phase 4 adds explicit terminal/agent bridges where a terminal or agent exposes a supported API. A kitty adapter may use user-enabled remote control. An ordinary-shell Bash/readline adapter remains labeled shell-only. Do not infer agent-prompt state from OSC markers, process names, or titles.

Phase 5 may use AT-SPI to enrich context/caret placement for applications with proven accessibility behavior. It should remain capability-gated and should not become a reason to claim universal insertion.

The durable architecture is therefore a broker plus capability-negotiated adapters, not one “Linux hook.” Adding an app means proving context, sensitive state, caret/UI, revalidation, insertion, and cancellation for that app's actual integration surface.

## Major risks and open questions

| Risk / unknown | Consequence | Earliest validation / mitigation |
|---|---|---|
| Fcitx addon event ordering/coexistence is insufficient | Interferes with the user's IME or misses keys | First four-hour addon spike; yield on any foreign preedit/candidate |
| Ghostty/Codex raw TUI bypasses expected Fcitx behavior | No terminal context or safe insertion | Early live Codex go/no-go; report unsupported, no Bash substitute |
| Fcitx cursor rectangle/popup is wrong on Hyprland/Ghostty | Misplaced or unusable candidate | Exact 20-trial placement gate; disable tuple on failure |
| commitString lacks application acknowledgment | Telemetry could overclaim success | Record dispatched-unverified; no retry; real repeated acceptance test |
| Terminal cannot identify sudo/password prompts | Manual session could capture a secret | Terminal forced Manual/local/ephemeral with visible arm and TTL; document residual risk |
| Framework-controlled web fields reject DOM mutation | Display works but app state does not | Verify after insertion; conditional matrix and site-specific adapters |
| Complex editors/cross-origin frames | False “browser-wide” claim | Explicitly unsupported until dedicated permission and tests |
| Obsidian internal/API drift | Plugin breaks across releases | Stay on supported API, pin/test versions, avoid internal modules |
| Same-UID malicious client spoofs an adapter | Policy identity can be forged | State threat boundary; permissions/peer UID; future packaging attestation research |
| Provider ignores cancellation | Wasted compute and possible late result | Broker latest-wins token and output gate; kill/restart unhealthy child |
| Local model quality/latency on 16 GB | Suggestions are slow or distracting | Small-model bake-off, deterministic fallback, short suffix, manual mode |
| Native messaging under confinement | Flatpak/Snap browsers cannot reach host | Separate compatibility work; do not claim support |
| AT-SPI data is incomplete/stale | Wrong context/caret | Observation-only, per-app capability tests |
| Memory/swap/crash remnants | Privacy promise exceeds implementation | Bounded buffers, local-only, no raw logs/core dumps; avoid absolute claims |
| Global shortcut collisions | User loses expected app keys | Configurable bindings and portal consent later; local shortcut fallback |

## Bottom line

Ship the first proof as “Chromium + Obsidian + manual Ghostty/Codex on this Omarchy tuple,” backed by one Rust broker and explicit capability receipts. Treat application-owned APIs as the durable path, Fcitx/IBus as conditional breadth, Hyprland/portals as control planes, and AT-SPI as optional observation.

The architecture succeeds when it fails closed and describes unsupported targets honestly. A suggestion that cannot be tied to one current, unchanged, non-sensitive field must disappear; an adapter that cannot insert natively must not synthesize its way around the failure.
