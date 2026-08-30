# ADR 0001: same-UID processes are inside the local trust boundary

- Status: accepted
- Date: 2026-08-30
- Scope: M1/M2A local broker and `badictl`

## Context

Badi exposes a private Unix socket below `$XDG_RUNTIME_DIR`. The socket parent
is user-only, the socket is mode `0600`, and both clients and the broker verify
peer UID and socket metadata. This prevents other operating-system users from
connecting, but it does not authenticate one process owned by the logged-in
user against another process owned by that same user.

The distinction matters because protocol capabilities are negotiation, not
credentials. A same-UID process can declare the control capability. `badictl`
also intentionally discovers and controls the sole active adapter session, so
requiring connection ownership for every addressed control would break an
explicit product function.

## Decision

For the current local-only product boundary, Badi treats processes running as
the logged-in UID as trusted local principals. UID checks, private filesystem
permissions, protocol validation, bounded frames, and session/revision
coordinates are safety and integrity controls; they are not application-level
authentication.

Consequently:

- `badictl` cross-connection pause, request, accept, and dismiss behavior is
  intentional;
- capability lists must never be described as authorization credentials;
- health and normal error output remain content-free;
- the broker must not expose a network listener or broaden socket permissions;
- receipts and product documentation must disclose same-UID impersonation as a
  residual risk; and
- tests must not imply protection from malicious debuggers, injected code,
  compromised browser extensions, or other processes already executing as the
  user.

## Alternatives rejected

### A static secret in configuration

Rejected. Every same-UID process able to read the user's files can usually read
the secret, so this would add ceremony without changing the attacker boundary.

### Per-connection session ownership for all controls

Rejected for current product behavior. It would prevent `badictl` from acting
as the user's separate control surface without solving same-UID observation or
process compromise.

### Broker-issued capabilities in a protected launcher channel

Deferred. This is the appropriate direction only if a future sandboxed adapter,
privilege separation, or multi-principal deployment creates an OS-enforced
boundary capable of protecting the issued capability.

## Consequences

The design stays compact and honest for a single-user Linux desktop. It does
not claim confidentiality or control integrity against malicious same-UID
software. Any future remote listener, system service, shared account, sandbox
boundary, or untrusted plugin model invalidates this decision and requires a
new threat model before implementation.
