// The Neovim plugin and its bridge against the real Rust broker with the
// deterministic phrase provider: policy, display, acceptance, carry, pause,
// restart and content-free diagnostics.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, execFile } from 'node:child_process';
import { mkdtemp, mkdir, writeFile, readFile, rm, lstat } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import { join } from 'node:path';
import { once } from 'node:events';
import { promisify } from 'node:util';
import { delay, neovimSkipReason, PLUGIN, ROOT, startNeovim } from './tests/harness.mjs';

const skip = neovimSkipReason();
const permissions = { context_read: 'allow', display: 'allow', suggest: 'allow', learn: 'block', retention: { mode: 'none' } };
const subject = (adapter, app_id) => ({ identity: { kind: 'linux_app', adapter, app_id }, permissions });

test('Neovim edits through the real broker only under its exact grant', { skip }, async t => {
  const root = await mkdtemp('/tmp/badi-neovim-broker-');
  const environment = { ...process.env, XDG_RUNTIME_DIR: join(root, 'runtime'),
    XDG_CONFIG_HOME: join(root, 'config'), XDG_DATA_HOME: join(root, 'data') };
  const control = async (args, input = '') => {
    const running = promisify(execFile)(join(ROOT, 'target/debug/badictl'), args, { env: environment, encoding: 'utf8' });
    running.child.stdin.end(input);
    return JSON.parse((await running).stdout);
  };
  // The broker counts a commit once the editor reports it applied.
  const committed = async count => {
    const end = performance.now() + 4000;
    while ((await control(['status'])).metrics.commits_applied !== count) {
      assert.ok(performance.now() < end, `commits_applied never reached ${count}`);
      await delay(20);
    }
  };
  let broker;
  let logs = '';
  const startBroker = () => {
    broker = spawn(join(ROOT, 'target/debug/badi-broker'), ['--provider', 'phrase'], { env: environment, stdio: ['ignore', 'pipe', 'pipe'] });
    broker.stdout.on('data', data => { logs += data; });
    broker.stderr.on('data', data => { logs += data; });
  };
  const endpoint = join(environment.XDG_RUNTIME_DIR, 'badi/broker.sock');
  const stopBroker = async () => {
    if (broker?.exitCode !== null) return;
    const exited = once(broker, 'exit');
    broker.kill('SIGTERM');
    await exited;
    assert.equal(broker.exitCode, 0, logs);
  };
  t.after(async () => { await stopBroker(); await rm(root, { recursive: true, force: true }); });
  await mkdir(environment.XDG_RUNTIME_DIR, { mode: 0o700 });
  await mkdir(environment.XDG_DATA_HOME, { mode: 0o700 });
  await mkdir(join(environment.XDG_CONFIG_HOME, 'badi'), { recursive: true, mode: 0o700 });
  // Bash is allowed, Neovim is not: one app's grant is no other's.
  await writeFile(join(environment.XDG_CONFIG_HOME, 'badi/settings.json'), JSON.stringify({
    schema: 'badi.settings.v2', revision: 1, paused: false, subjects: [subject('shell', 'bash')],
  }), { mode: 0o600 });
  startBroker();
  const deadline = performance.now() + 5000;
  while (await lstat(endpoint).then(() => false, () => true)) {
    assert.ok(performance.now() < deadline && broker.exitCode === null, logs);
    await delay(10);
  }
  // Content-free debug activity, as `badi debug on` would enable it.
  await writeFile(join(environment.XDG_RUNTIME_DIR, 'badi/debug-control.json'),
    JSON.stringify({ id: randomUUID(), expires_at: Math.floor(Date.now() / 1000) + 600 }), { mode: 0o600 });
  // The broker appends to a trace the controller created; it never creates one.
  await writeFile(join(environment.XDG_RUNTIME_DIR, 'badi/debug-trace.jsonl'), '', { mode: 0o600 });

  const editor = await startNeovim({ env: { XDG_RUNTIME_DIR: environment.XDG_RUNTIME_DIR },
    config: { bridge: join(PLUGIN, 'bridge.mjs'), delay_ms: 40, language: 'en' } });
  t.after(() => editor.close());
  await editor.command('setfiletype markdown');
  await editor.send('ithank you');
  // The refused request and the bridge's first policy refresh may land in
  // either order; both name the missing grant.
  let state = await editor.until(current =>
    ['app_disabled_or_model_paused', 'app_disabled'].includes(current.status.status), 'denied');
  assert.equal(state.preview, null);
  assert.equal((await control(['status'])).metrics.provider_calls, 0, 'An app without a grant is never read');

  const current = await control(['settings', 'show', '--json']);
  const revision = current.revision;
  // Canonical settings keep subjects in adapter-name order, as `badi app` writes them.
  current.subjects.unshift(subject('neovim', 'nvim'));
  current.revision = revision + 1;
  await control(['settings', 'replace', '--if-revision', String(revision), '--json', '-'], JSON.stringify(current));
  // Type only once the bridge has resolved the new grant.
  await editor.until(current => current.status.status === 'ready', 'grant resolved');
  await editor.send(' <BS>');
  state = await editor.until(current => current.preview === ' for your time', 'granted preview');
  assert.equal(state.screen, 'thank you for your time');
  await editor.send('<C-Right>');
  state = await editor.until(current => current.lines[0] === 'thank you for your time', 'acceptance');
  await committed(1);
  await editor.send('<Esc>u');
  await editor.until(current => current.lines[0] === 'thank you', 'one-step undo');

  // One word, then the broker's carried remainder without another model call.
  await editor.send('A <BS>');
  await editor.until(current => current.preview === ' for your time', 'preview again');
  const calls = (await control(['status'])).metrics.provider_calls;
  await editor.send('<Tab>');
  state = await editor.until(current => current.preview === ' your time', 'carried remainder');
  assert.deepEqual(state.lines, ['thank you for']);
  assert.equal((await control(['status'])).metrics.provider_calls, calls, 'Type-through carries text without inference');
  await editor.send('<C-Right>');
  await editor.until(current => current.lines[0] === 'thank you for your time', 'rest accepted');
  await committed(3);

  // The request key on a shown suggestion asks the broker for one that
  // starts with another word; leaving Insert mode then dismisses it.
  await editor.send('<CR>thank you');
  await editor.until(current => current.preview === ' for your time', 'preview before an alternative');
  await editor.send('<C-S-Space>');
  state = await editor.until(current => current.preview === ' so much', 'an alternative');
  assert.deepEqual(state.chunks, [[' so', 'BadiSuggestionWord'], [' much', 'BadiSuggestion']]);
  await editor.send('<C-S-Space>');
  await editor.until(current => current.preview === ' again', 'a second alternative');
  await editor.send('<Esc>ddA');
  await editor.until(current => current.preview === null && current.lines.length === 1, 'line removed');

  // Pause withdraws the preview at once and refuses explicit requests.
  await editor.send('<CR>looking forward');
  await editor.until(current => current.preview === ' to hearing from you', 'preview before pause');
  await control(['pause', 'on']);
  await editor.until(current => current.preview === null, 'pause withdraws');
  const refused = await editor.command('lua require("badi").request(true)');
  await editor.until(current => current.status.status === 'app_disabled_or_model_paused', 'paused');
  assert.notEqual(await editor.expr('get(maparg("<Tab>", "i", 0, 1), "desc", "")'), 'Badi', 'No preview key remains');
  await control(['pause', 'off']);
  await delay(50);
  await editor.send('<C-S-Space>');
  await editor.until(current => current.preview === ' to hearing from you', 'explicit request after resume');
  assert.equal(typeof refused, 'string');

  // A broker restart withdraws everything; recovery replays no text.
  await stopBroker();
  await editor.until(current => current.preview === null, 'disconnect withdraws');
  startBroker();
  while (await lstat(endpoint).then(() => false, () => true)) await delay(10);
  await delay(600);
  assert.equal((await control(['status'])).metrics.provider_calls, 0, 'Reconnecting does not resend the buffer');
  await editor.send(' ');
  await editor.send('<BS>');
  state = await editor.until(current => current.preview === ' to hearing from you', 'preview after restart');

  // Holding the terminal's focus withholds the input method's native path,
  // survives a broker restart through the bridge's new connection, and ends
  // with FocusLost.
  const focusReports = async () => (await readFile(join(environment.XDG_RUNTIME_DIR, 'badi/debug-trace.jsonl'), 'utf8'))
    .split('\n').filter(Boolean).map(line => JSON.parse(line))
    .filter(entry => entry.ev === 'out editor.status').map(entry => entry.native_withheld);
  const reported = async (count, withheld, label) => {
    const end = performance.now() + 4000;
    for (;;) {
      const reports = await focusReports();
      if (reports.length >= count && reports.at(-1) === withheld) return;
      assert.ok(performance.now() < end, `${label}: ${JSON.stringify(reports)}`);
      await delay(20);
    }
  };
  const before = (await focusReports()).length;
  await editor.command('doautocmd FocusGained');
  await reported(before + 1, true, 'focus withholds the native path');
  assert.equal((await editor.state()).status.holds_terminal, true);
  await stopBroker();
  startBroker();
  await reported(before + 2, true, 'the claim returns with the new connection');
  await editor.command('doautocmd FocusLost');
  await reported(before + 3, false, 'FocusLost releases it');

  const activity = await readFile(join(environment.XDG_RUNTIME_DIR, 'badi/debug-neovim.json'), 'utf8');
  assert.equal(JSON.parse(activity).app_id, 'nvim');
  const messages = (await editor.state()).messages;
  for (const text of ['thank you', 'looking forward', 'your time', 'hearing']) {
    assert.doesNotMatch(activity, new RegExp(text), 'Debug activity is content-free');
    assert.doesNotMatch(logs, new RegExp(text), 'The broker logs no text');
    assert.doesNotMatch(messages, new RegExp(text), 'Neovim shows no text in messages');
  }
  assert.equal((await control(['status'])).metrics.provider_errors, 0);
});
