# Editor integrations

All clients check exact app/site policy before acquiring text. The local model
suggests up to four words. Each acceptance is bound to its document, caret,
revision, fingerprint and expiring one-shot broker grant. Typed text is not
retained by these adapters; `badi debug on` enables private metadata snapshots
for 15 minutes. English, German and Persian routing is implemented; language
quality remains an experimental boundary evaluated separately from editing safety.

| Integration | Input and acceptance | Verification |
| --- | --- | --- |
| Obsidian | CodeMirror at note end; automatic/Tab request, Tab accepts one word, Ctrl/Command+Right accepts all, Escape dismisses; isolated undo transaction | Obsidian 1.13.7 / Electron 43.4.1, actual app with headless Ozone and Qwen: EN/DE/FA display, acceptance, undo, Escape and same-note broker restart |
| Bash | Readline buffer at prompt end; Ctrl-X then Tab request/accept, Ctrl-X then Escape dismiss; no command submission | Physical Ghostty/Bash and real Qwen; PTY tests cover completion, undo, pause and broker restart |
| Chromium web | Top-level text inputs/textarea with stable identity; automatic/Tab request, Tab accepts one word, Ctrl/Command+Right accepts all; optional host access plus exact broker-origin policy | Normal Brave profile on Omarchy plus isolated Chromium/Qwen; typing, partial/full acceptance and native undo |

Password/sensitive fields, foreign composition, noncollapsed selections and stale
context are excluded. Obsidian currently requires the caret at the note end. Bash
hooks do not inspect terminal output, password prompts or other programs. Browser
contenteditable, canvas editors, frames and shadow-root fields need their own
integration; no universal Linux compatibility is claimed.

## Install

From the repository after the desktop model is provisioned:

```sh
python3 scripts/install-editors.py --vault /path/to/vault --bash --chromium
badi app obsidian on
badi app bash on
```

The installer preserves existing plugin enablement and Bash configuration, keeps
backups and a path map under `~/.local/state/badi/editor-backups/`, and replaces
files atomically. Restore a mapped backup to its original path to roll back; a
null backup denotes a newly installed file. It does not change Obsidian restricted
mode or edit any note. Reload Obsidian normally. The command palette includes
**Badi: Request or accept words** and **Badi: Reconnect local model**.
Open notes reconnect automatically after a broker restart with bounded backoff;
the adapter obtains fresh policy and never replays an earlier document or grant.
After ten unsuccessful attempts, typing, focus or the reconnect command retries.
Language commands select the application locale, English, German or Persian.
Each accepted word gets a separate undo transaction and a fresh model request.

Open a new interactive Bash shell, or source
`~/.local/lib/badi/editors/shell/badi.bash` in an existing one. Node must be on PATH.
The Readline hook only changes `READLINE_LINE`; no generated text is evaluated.
Set `BADI_LANGUAGE=de` or `BADI_LANGUAGE=fa` in the environment before starting
the bridge to override its locale. The shell shortcut continues to accept all.

For Chromium, load `~/.local/lib/badi/chromium` as an unpacked extension. Open its
Badi popup on a website and enable access, then grant the exact origin locally:

```sh
badi site https://example.com on
badi site https://example.com off
```

The fixture, Dillinger and ordinary-web builds share the current development
extension ID; use one build per browser profile. The browser's host grant can
cover multiple ports; Badi's separate policy still distinguishes every origin.
The extension never requests blanket access during installation. See Chrome's
[permission API](https://developer.chrome.com/docs/extensions/reference/api/permissions)
and [script registration](https://developer.chrome.com/docs/extensions/reference/api/scripting).

Chromium's `insertText` edit command preserves native undo; direct `setRangeText`
does not. Reasserting the unchanged caret first separates the edit from preceding
typing in native undo. This deprecated API is tested on Chromium and Brave, with no second mutation if
it refuses the edit. The [API compatibility note](https://developer.mozilla.org/en-US/docs/Web/API/Document/execCommand)
explains this remaining platform dependency.

## Check

```sh
npm run editors:check
npm run editors:integration
npm run build:web --workspace @badi/chromium
npm run live:web --workspace @badi/chromium -- --local
node adapters/obsidian/live.mjs --headless --local
node adapters/shared/evaluate-writing.mjs
```

The browser runner uses an isolated profile and synthetic page; it never touches
an existing profile. The model smoke explicitly sends 20 synthetic prefixes to
the running, opted-in Obsidian adapter and writes ignored `output/writing/smoke.json`.
Its counts/latencies are a smoke result, not a held-out accuracy benchmark.
Physical checks use an unlocked desktop and disposable text. Historical
capability receipts remain separate. Untagged web fields use the browser locale;
English/German Latin text uses the page/application language hint. Within the
EN/DE/FA scope, an Arabic-script letter nearest the caret selects Persian from
the already authorized context window. Other explicit language tags are retained.
Persian U+200C half-spaces are preserved only between the exact Arabic letter
ranges in [the orthographic fixture](../../protocol/orthographic-joiner-fixtures.json);
other invisible formatting and bidi controls remain rejected.

Ordinary-web requests have a two-second generation budget, including debounce;
the historical exact fixture retains its 600 ms bound. Both retain immediate
revision/focus cancellation. Successful browser reconnects renew the retry
budget so several independent broker/worker restarts do not strand a page.
Ordinary-web bootstrap failures additionally retry with bounded backoff (ten
attempts, capped at five seconds). A later eligible focus renews an exhausted
budget. Native-host EOF closes the Chrome port even while browser stdin stays
open, so a retired preview clears promptly instead of waiting for its lease.
Disconnect notices from failed reconnects retain the current backoff instead
of consuming the entire budget before a cold local model is ready. The
2026-09-07 Chromium 151.0.7922.173 local-model run exercised the same open page
through broker shutdown, preview clearing, cold restart, fresh acceptance and
native undo. Its EN/DE/FA samples are integration checks, not language quality
scores; see the [writing evaluation](../../evaluation/writing/README.md).

The Obsidian live runner uses the installed application with its own temporary
vault, profile, plugin and broker. It trusts only that synthetic vault. Headless
Ozone exercises the real renderer and editor transactions without touching the
desktop; omitting `--headless` requires a normally unlocked Wayland session.
Screenshots and results remain under ignored `output/playwright/obsidian-*`.
The Escape regression covers Obsidian preventing the DOM event before
CodeMirror handles it; a passive editor-local observer dismisses only Badi's
owned preview. These application checks establish editing behavior, not the
usefulness of Persian or German prose or Cotypist parity.
