# Badi vision

Badi (`بعدی`, Persian for “next”) is local, capability-aware co-writing for
Linux, starting with Omarchy. It helps you write your own next words inside the
app you already use. A short continuation appears only when the current field
can be understood and edited safely; you accept it, type through it, or ignore
it without leaving your flow. The model is replaceable. The product is the
interaction, the exact editing authority and the policy around them.

## Character: precise, light, native

- One polished interaction before a second one. Features that miss latency,
  safety or usefulness gates stay off.
- Quietness is model quality: optimize retained useful text per interruption,
  not text generated. Abstain when confidence or target state is weak; late,
  overlapping or off-voice suggestions are product failures.
- Shared contracts stay small; target-specific behavior stays in adapters.
- The UI follows the Linux desktop and Omarchy's calm, direct style.
- Inference is local on hardware-selected, pinned and verified models. A slow or
  unavailable model degrades to silence, never to a hidden cloud call.

## Authorship and editing contract

- Badi appends a short suffix. It never answers for you, rewrites several
  places, executes commands or submits text; Enter stays your action.
- Acceptance is explicit and one-shot. Spelling replacement is a separate,
  narrow mode that requires negotiated replacement support, an exactly bound
  range and explicit acceptance.
- Adapters own document acquisition and mutation. The broker cannot edit an
  app; it authorizes one commit bound to the exact target identity, revision,
  fingerprint, caret, focus and expiry, and the adapter revalidates before one
  target-API edit. Anything stale or ambiguous fails closed.
- Native undo is preserved. A mutation the target cannot verify is reported as
  dispatched, never as applied.

## Context firewall

- Context is the bounded text of the focused field, read only after the exact
  app or site is allowed and the field passes its checks. Password and other
  sensitive fields, foreign IME composition and unknown authority yield nothing.
- Activation, context, inference, learning and retention are separate
  decisions; deny wins. Prose is not retained, and typed text and credentials
  stay out of diagnostics, which record content-free counts and reasons.
- Raw keylogging, clipboard or screen scraping and blind synthetic typing are
  never product integrations.

## Integration order

1. **Editor-owned integrations** (Obsidian, Bash Readline) own context, display,
   the edit and native undo.
2. **Cooperative Fcitx5 addon** for native apps that expose surrounding text. It
   yields to foreign preedit and candidates, and its commit is dispatched,
   not verified.
3. **IME-parity** for Chromium-based apps and Zen without an editor channel: the
   focused accessibility observer plus Fcitx. Acceptance is one append-only
   commit that behaves like typed text; every identity, binding and denial
   guard still applies. There is no browser extension.
4. **Unsupported** is the correct answer when identity, sensitivity, revision,
   placement or insertion cannot be established.

## Non-goals

- Claiming support for every Linux app, toolkit, compositor or distribution.
- Global input capture, virtual keyboards or synthetic typing as architecture.
- Chat, arbitrary rewrite, agent actions or code execution while typing.
- Remote inference, raw-history learning, accounts, sync or telemetry by default.

## North star

**Help me write my own next words across the Linux apps that can support it,
quietly, locally, and with an honest account of what Badi saw and did.**
