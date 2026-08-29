# Omatype vision

> Working-name warning: an unrelated [OmaType dictation project](https://github.com/Aayush9029/OmaType)
> already serves the Linux/Omarchy community. Keep `omatype` as the repository
> codename for this research, but resolve the public name before launch.

## The promise

Omatype is the quiet co-writer Linux is missing: start a sentence in the app
you already use, see a short continuation, accept exactly the useful words, and
keep moving. It should feel like part of the desktop, not like a chatbot pasted
on top of it.

It will work toward broad coverage across browsers, Obsidian and other Electron
apps, native editors, chat clients, and terminals. “Everywhere” is a direction,
not a dishonest compatibility claim: Omatype will show what each app can safely
support and fall back gracefully when Linux cannot provide enough context.

## What makes it the real deal

### 1. Native to Linux's shape

Linux is not one desktop API. Omatype uses an integration ladder: input-method
support for broad text entry, accessibility where an app exposes reliable text
and caret data, small first-party bridges for high-value apps, and explicit
manual activation where ambient completion would be unsafe. Each adapter speaks
one stable internal protocol, so the prediction engine is not coupled to one
compositor or toolkit.

### 2. Quiet enough to trust

A missing suggestion is cheaper than a distracting or dangerous one. Omatype
will suppress low-confidence, late, badly spaced, duplicate, or contextually
unsafe completions. It will optimize for useful accepted text and avoided
keystrokes—not for how often it manages to display AI output.

### 3. A context firewall, not a universal keylogger

Every app and website receives one of three policies:

- **Always** — ambient suggestions are allowed.
- **Manual** — Omatype responds only to an explicit request.
- **Never** — no context collection, inference, learning, or UI.

Password, PIN, hidden, and sensitive fields are always `Never` when the input
stack identifies them. Unknown contexts fail toward `Manual`, and a global
pause is always one shortcut away. The UI will explain which integration saw
which context and whether anything left the machine.

### 4. Local first, provider optional

Short completions should run locally and offline by default. Users can choose a
different local model or explicitly configure a remote provider, but remote
traffic is visually distinct and governed by a separate policy. Personalization
data remains local, inspectable, exportable, and deletable.

### 5. Terminal-aware, not terminal-reckless

Shell completion, indentation, and command safety outrank Omatype. Ambient
completion is off at ordinary shell prompts. Natural-language agent prompts can
opt in through a terminal or agent bridge; otherwise the user invokes Omatype
manually. Accepting a suggestion must never execute it.

## The interaction

1. The user types normally.
2. After a short, adaptive pause, Omatype produces at most one short suggestion.
3. The next accepted word is visually stronger than the remaining phrase.
4. A configurable action accepts one word; a second action accepts the rest.
5. Continuing to type narrows or replaces the suggestion without punishment.
6. Escape dismisses it; the normal Tab key remains normal unless the current
   app profile explicitly assigns it to Omatype.

Long-form generation and rewriting are useful later, but the defining loop is
co-writing one small decision at a time.

## Product ideas worth protecting

- **Capability receipt:** the first time Omatype appears in an app, it can show
  a compact receipt: context source, insertion method, local/remote model, and
  active policy. Trust should be observable.
- **Quiet score:** measure late suggestions, immediate dismissals, overlap,
  spacing errors, and false activations. The system must earn the right to
  appear more often.
- **Acceptance ledger:** keep local aggregate statistics for accepted words,
  saved keystrokes, latency, and per-app usefulness without retaining raw prose.
- **Voice without surveillance:** personalization can learn approved vocabulary
  and phrases from accepted completions or explicitly imported writing. Raw
  cross-app typing history is not the default training set.
- **Adapter kit:** browser, Obsidian, editor, terminal, and future compositor
  bridges share a versioned local protocol and a reusable compatibility suite.
- **Two-speed prediction:** an instant deterministic guard/phrase layer may show
  a safe completion while a small local language model prepares a better one;
  stale generations are cancelled, never queued.

## First proof, not first fantasy

The initial proof targets this machine: Omarchy/Arch, Hyprland/Wayland, 16 GB
RAM, Intel i7-12700H, and integrated graphics. It should demonstrate:

- a short local suggestion in one browser text field, Obsidian, and Ghostty;
- accept-one-word, accept-all, dismiss, pause, and manual activation;
- `Always`, `Manual`, and `Never` app policies;
- no activity in a controlled password-field test;
- warm visible-suggestion latency measured rather than described as “instant”;
- an honest compatibility report for every tested target.

The proof is successful when the complete loop is reliable in three real apps.
It is not successful merely because a model can produce text in a demo window.

## Non-goals for the first two days

- claiming every Linux distribution, compositor, toolkit, and sandbox works;
- capturing raw input devices as the normal production architecture;
- screen-wide OCR or periodic screenshots;
- automatic completion of ordinary shell commands;
- cross-device sync, accounts, billing, teams, or a plugin marketplace;
- fine-tuning a model before a measured base-model bake-off;
- a polished settings application before the core loop is trustworthy.

## North-star sentence

**Omatype helps you write your own next words, everywhere Linux can support it,
without making you surrender your flow or your trust.**
