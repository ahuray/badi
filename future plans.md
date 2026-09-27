# Future plans

The only active backlog. Goal: dependable local prediction and spelling
assistance throughout Linux, Omarchy first, that stays fast, light and reliable.
Tested coverage is in the [README](README.md). Git history keeps completed work
and the earlier experiment logs.

## Now: working in daily apps

User decisions (2026-09-26): IME-parity for Chromium-based apps and Zen
([ADR 0003](docs/decisions/0003-ime-parity-append-only.md)), promote a quality fix
only without a per-language harm increase, keep the model resident, and give
explicit requests about 1.2 s. Live with disposable text on 2026-09-27:
Chromium, Zen, Telegram, VS Code, Cursor, Omawrite, Obsidian and Bash.

- [ ] Codex desktop composer: rich-composer flattening passed an equivalent
      Chromium fixture; run the live acceptance trial in Codex itself.
- [ ] Brave Origin full acceptance trial; Zen undo grouping after acceptance.
- [ ] Discord: its updater drops command-line flags and environment, so renderer
      accessibility has no supported launcher.
- [ ] Confirm the Fcitx 5.1.22 compatibility frontend across a fresh login; its
      login-order fix has not been through a reboot.
- [ ] Observer lock check: each `inspect` spends 20–100 ms on the Quickshell
      lock query under load, inside a 350 ms budget; an event-driven lock state
      would remove it.

## Code quality: fast, light, reliable (2026-09-27)

- [x] Remove the old evaluator, capability receipts and V3 gates, the browser
      extension and its Dillinger/Monaco build, the legacy panel, the try-native
      registry and research logs (about 42k lines).
- [x] Move the Prediction Lab worker into its own crate; the broker has no Lab
      feature, hook or dependency.
- [x] Broker: a bounded loopback HTTP/1.1 client replaces reqwest (43 crates
      instead of 98, binary 3.6 → 2.8 MB); in-place lexicon lookup (no 2 MiB
      index); no idle wakeups; pidfd exit wait; resident model service.
- [x] Installers: shared helpers, one backup layout keeping three per installer
      (about 300 → 17 MB), compatibility-build reuse (67 s → instant).
- [x] Editors: one Obsidian connection; idle Bash bridges exit.
- [x] Readability: `engine.rs` split into modules (longest function 242 → 82
      lines), one server handler per message kind, observer replies decided
      in the tested Fcitx core, the observer split into desktop, field and
      geometry sides with a contract dispatch table, and one app-identity list
      checked across addon and observer in CI.
- [ ] Remaining readability: a shared `badictl` request helper; one hex
      encoder and one language-tag validator in the broker; one Fcitx transport
      session-open path; the rest of `addon.cpp`'s decisions (invoke routing,
      pre-key handling, Tab reasons) in the tested core.
- [ ] The Lab's paced subprocess test misses its cleanup window under heavy
      parallel builds (load above 7); make it deterministic.

## Prediction quality

Measured baseline: the installed Qwen3-1.7B path is weak on independent sets,
Persian often misses the 550 ms budget, and no ~350M candidate passed the
English/German/Persian gates (2026-09-10). Promotion needs a new hypothesis,
fresh independent cases, native Persian review where ambiguous and the full Lab
qualification.

- [ ] Freeze a new independent multilingual confirmation set (at least 100
      prefixes). Report usefulness, abstention, errors, useful words and
      keystroke savings separately; target warm visible p50 ≤ 250 ms and
      p95 ≤ 500 ms and report misses.
- [ ] Reduce the combined instruction/boundary mode's latency (18/22 useful in
      development, but a 2.8 s cold median) and confirm it independently.
- [ ] Keep facts from style examples from overriding the current draft; the
      Lab's `style_fact_conflict` fence still needs a multilingual confirmation.
- [ ] Tell an intended partial word from a completed one before automatic
      contextual lookup (`mire` → `Mirella` must complete; `marin` must not
      become `Marinella`); dictionary membership is the wrong splitter.
- [ ] German/Persian correction: the 40-case run failed (2 and 4 useful, one
      unwanted change each). Improve names and nearby-word handling on
      independent cases before any application integration.
- [ ] Calibrate token confidence against reviewed full additions per language
      and boundary category.
- [ ] Cotypist parity needs the same tasks on a named Cotypist version and Mac;
      source research cannot establish it.

## Coverage and reliability

- [ ] Validate the installed flow as a matrix: service restart, native undo,
      pause, composition and stale focus, with zero wrong-field or stale edits.
- [ ] Native coverage beyond Omawrite/Xournal++/Telegram through measured
      toolkit integrations.
- [ ] Browser rich editors beyond single-paragraph composers, frames/shadow
      roots and Firefox, keeping exact field binding and per-site consent.
- [ ] Fish/Zsh and terminal editors through their own buffers; never accept by
      executing a generated command.
- [ ] Spelling correction in native fields and Obsidian with exact replacement
      ranges and explicit acceptance.
- [ ] Install, update, reload and uninstall on a fresh Omarchy session.
- [ ] Lower-memory hardware and GPU: cold start, RAM, sustained latency, power
      and crash recovery.

## Later

- [ ] Other Linux desktops and a standard tray integration.
- [ ] Opt-in personalization with explicit retention and clear controls.
- [ ] Release packaging, licensing and naming review. No publication or Git
      history changes without the user's request.

Update this file in place; record durable decisions in
[docs/decisions](docs/decisions/).
