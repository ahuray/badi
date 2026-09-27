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

Password/sensitive fields, foreign composition, noncollapsed selections and stale
context are excluded. Obsidian currently requires the caret at the note end. Bash
hooks do not inspect terminal output, password prompts or other programs. Browser
fields use the extension-free [IME-parity path](../../README.md#browsers-and-chromium-based-apps-ime-parity)
instead; no universal Linux compatibility is claimed.

## Install

From the repository after the desktop model is provisioned:

```sh
python3 scripts/install-editors.py --vault /path/to/vault --bash
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

## Check

```sh
npm run editors:check
npm run editors:integration
node adapters/obsidian/live.mjs --headless --local
node adapters/shared/evaluate-writing.mjs
```

The model smoke explicitly sends 20 synthetic prefixes to the running, opted-in
Obsidian adapter and writes ignored `output/writing/smoke.json`. Its
counts/latencies are a smoke result, not a held-out accuracy benchmark.
Physical checks use an unlocked desktop and disposable text. English/German
Latin text uses the application language hint. Within the EN/DE/FA scope, an
Arabic-script letter nearest the caret selects Persian from the already
authorized context window. Other explicit language tags are retained. Persian
U+200C half-spaces are preserved only between the exact Arabic letter ranges in
[the orthographic fixture](../../protocol/orthographic-joiner-fixtures.json);
other invisible formatting and bidi controls remain rejected.

The Obsidian live runner uses the installed application with its own temporary
vault, profile, plugin and broker. It trusts only that synthetic vault. Headless
Ozone exercises the real renderer and editor transactions without touching the
desktop; omitting `--headless` requires a normally unlocked Wayland session.
Screenshots and results remain under ignored `output/playwright/obsidian-*`.
The Escape regression covers Obsidian preventing the DOM event before
CodeMirror handles it; a passive editor-local observer dismisses only Badi's
owned preview. These application checks establish editing behavior, not the
usefulness of Persian or German prose or Cotypist parity.
