# Omatype

Omatype is a Linux-native, privacy-conscious ghostwriter that can assist in the
apps where people already write: browsers, note-taking tools, editors, chat
clients, and terminals.

This repository currently contains the decision-ready research package. The
recommended product shape is one local broker with small, capability-gated
adapters—not a global keylogger or synthetic typer.

> **Working-name warning:** an unrelated
> [OmaType dictation project](https://github.com/Aayush9029/OmaType) already
> serves the Linux/Omarchy niche. Keep this repository name as a research
> codename, but resolve the public product and technical namespaces before
> launch.

## Research package

- [Vision](VISION.md) — product promise, trust principles, interaction, and
  first-proof boundaries.
- [Competitive landscape](docs/research/competitive-landscape.md) — Cotypist's
  strongest features, open-source alternatives, Linux gaps, and source-backed
  market opportunity.
- [Linux architecture](docs/research/linux-architecture.md) — decisive
  Rust/TypeScript/Fcitx5 design, protocol, policy, compatibility matrix, and
  measurable acceptance criteria.
- [Adversarial review](docs/research/adversarial-review.md) — the independent
  grill, rejected shortcuts, kill criteria, and decisions still owed.
- [Two-day delivery plan](docs/plan/two-day-delivery.md) — risk-first H0–H48
  multi-agent workflow, ownership, fixtures, gates, rollback, and live-session
  contract.

## Recommendation

Proceed after user review with a bounded 48-hour experiment on the observed
Omarchy/Hyprland workstation:

1. Chromium via an app-owned Manifest V3 adapter.
2. Obsidian via an app-owned CodeMirror plugin.
3. A manually armed Fcitx5 addon in a live Codex prompt inside Ghostty.

The third target is deliberately a go/no-go test. If native Fcitx candidate or
commit behavior is unreliable, report terminal support as failed; do not replace
it with `evdev`, `wtype`, clipboard insertion, or a Bash-only demonstration.

## Status

Research completed on 2026-08-30. No product implementation has started; the
next step is user feedback on the research package and unresolved decisions.
