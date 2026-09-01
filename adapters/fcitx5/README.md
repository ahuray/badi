# Badi Fcitx5 module

This tree builds a cooperative Fcitx5 **Module** addon. It does not register or
replace an input method. It observes normal Fcitx events after the active input
method, yields whenever that input method owns preedit or candidates, and never
uses evdev, `wtype`, the clipboard, a virtual keyboard, or global input capture.

## Deliberately narrow product slice

- Exact `InputContext::program()` allowlist: `omawrite` and
  `com.github.xournalpp.xournalpp`. The noncanonical value `xournalpp` is denied.
- `Ctrl+Shift+Space` is the only invocation/refresh chord. Until it is pressed,
  surrounding-text events only invalidate local revision state; no prose is
  serialized or sent.
- Manual invocation requires a collapsed, non-composing, non-sensitive
  surrounding-text snapshot, the live Fcitx `SurroundingText` capability, and
  a validated language from the active input-method entry. Each focus epoch
  must first receive a surrounding-text update; focus-out and capability
  changes reset that freshness latch. Missing or stale capability, sensitive,
  special-purpose, composing, selected, unknown-language, and unknown-identity
  states produce zero outbound context.
- A suggestion uses Fcitx's native candidate panel. `Ctrl+Shift+Y` accepts one
  owned, unexpired, exact-revision candidate; `Escape` dismisses it. Every other
  key passes through.
- Acceptance requests broker authorization first. A matching `commit.prepare`
  causes exactly one `InputContext::commitString`; the result is reported as
  `dispatched-unverified`, because Fcitx cannot prove the client applied it.

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
[native-app handoff](../../docs/delivery/2026-09-01-fcitx5-native-app-handoff.md)
for the evidence boundary and observed metrics.

## Build and test

Requirements are CMake, Ninja, a C++20 compiler, Fcitx5Core, and nlohmann-json.

```sh
cmake -S adapters/fcitx5 -B adapters/fcitx5/build -G Ninja \
  -DCMAKE_BUILD_TYPE=Release
cmake --build adapters/fcitx5/build
ctest --test-dir adapters/fcitx5/build --output-on-failure
```

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

FCITX_DATA_DIRS="$HOME/.local/share:/usr/local/share:/usr/share" \
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
