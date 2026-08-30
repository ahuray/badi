# Omatype broker and native bridge

This crate contains the local policy broker, its control CLI, and the narrow
Chromium native-messaging bridge. It is Linux/Unix-socket only in the current
proof and never drives a keyboard, clipboard, or accessibility API.

## Binaries

- `omatype-broker` owns policy, session state, cancellation, the deterministic
  provider, content-free metrics, and the private Unix socket.
- `omatypectl` sends explicit control and health requests to that socket.
- `omatype-native-host` translates Chrome native-message frames to validated
  Omatype protocol frames on the existing broker socket. It does not start the
  broker or provide suggestions itself.
- `omatype-native-manifest` prints one deterministic native-host manifest to
  standard output. It never installs or writes that manifest.

Build all four without installing them:

```sh
cargo build --workspace --bins
```

## Native-message boundary

The development host accepts only this caller origin:

```text
chrome-extension://ckkiehcjbclcjckkkajohopoikeejkoa/
```

That ID is derived from the public development key in the Chromium manifest.
The host and generated native manifest both pin it exactly; wildcards and
caller-selected origins are rejected. A future production identity therefore
requires rebuilding the host as well as changing the extension manifest.

Chrome uses a native-endian unsigned 32-bit length followed by UTF-8 JSON.
Chrome's transport permits larger messages, but this bridge applies Omatype's
65,536-byte encoded-envelope ceiling in both directions and rejects an
oversized declared input before allocating its body. Every frame is decoded as
a strict protocol envelope before relay. The bridge verifies the broker socket
metadata and peer UID, writes no content to logs, and treats expected EOF or a
closed Chrome output pipe as a clean disconnect.

The host uses `$XDG_RUNTIME_DIR/omatype/broker.sock` by default. An absolute
`--socket` override exists for direct development tests; Chrome itself supplies
only the caller origin.

## Broker shutdown and socket cleanup

`omatype-broker` registers Linux SIGINT and SIGTERM handlers before exposing
its socket. Either signal returns through the normal server path, drops the
socket guard, and removes the private socket before the process exits
successfully. Cleanup rechecks the socket's device and inode, so it will not
unlink a path that another process replaced.

Process-level tests send both signals, require a zero exit status and empty
stdout/stderr, and observe the socket disappear within a bounded interval. A
runner should therefore wait for that disappearance rather than force-deleting
the socket.

## Print-only manifest workflow

The checked-in [example manifest](native-messaging/io.omatype.broker.example.json)
is reproducible with:

```sh
target/debug/omatype-native-manifest \
  --host-path /opt/omatype/omatype-native-host
```

The supplied path must be absolute UTF-8 without `.` or `..` components. The
command prints JSON and makes no profile, user-configuration, or system change.
The isolated live runner creates its own temporary HOME and writes the emitted
manifest only below that disposable directory.

## Verify

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Tests cover fragmented frames, empty/truncated input, a 65,537-byte declaration
rejected from its header alone, strict caller-origin validation, deterministic
manifest output, bidirectional socket relay, EOF, broken output pipes, and
observed SIGINT/SIGTERM socket cleanup.
