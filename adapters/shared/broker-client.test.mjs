import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, chmod, rm, lstat } from 'node:fs/promises';
import { spawn, execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { once } from 'node:events';
import { join, resolve } from 'node:path';
import { BrokerClient } from './broker-client.mjs';
import { RecoveringBrokerClient } from './recovering-client.mjs';
import { createInterface } from 'node:readline';

const ROOT = resolve(import.meta.dirname, '../..');
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(predicate, details = []) {
  const deadline = performance.now() + 4000;
  while (!await predicate()) {
    if (performance.now() > deadline) throw new Error(`Integration condition timed out: ${details.join(', ')}`);
    await delay(10);
  }
}

test('editor identities, native reading lease, revocation and one-shot commits against the Rust broker', async () => {
  const root = await mkdtemp('/tmp/badi-editor-test-');
  const environment = { ...process.env, XDG_RUNTIME_DIR: join(root, 'runtime'),
    XDG_CONFIG_HOME: join(root, 'config'), XDG_DATA_HOME: join(root, 'data') };
  const desktopRuntime = process.env.XDG_RUNTIME_DIR;
  process.env.XDG_RUNTIME_DIR = environment.XDG_RUNTIME_DIR;
  const clients = [];
  let broker;
  let bridge;
  let logs = '';
  const control = async (...args) => JSON.parse((await promisify(execFile)(join(ROOT, 'target/debug/badictl'), args,
    { env: environment, encoding: 'utf8' })).stdout);
  try {
    await mkdir(environment.XDG_RUNTIME_DIR, { mode: 0o700 });
    await mkdir(environment.XDG_DATA_HOME, { mode: 0o700 });
    await mkdir(join(environment.XDG_CONFIG_HOME, 'badi'), { recursive: true, mode: 0o700 });
    await writeFile(join(environment.XDG_CONFIG_HOME, 'badi/settings.json'), JSON.stringify({
      schema: 'badi.settings.v2', revision: 1, paused: false,
      subjects: [{ adapter: 'obsidian', app_id: 'obsidian' }, { adapter: 'shell', app_id: 'bash' }].map(identity => ({
        identity: { kind: 'linux_app', ...identity }, permissions: {
          context_read: 'allow', display: 'allow', suggest: 'allow', learn: 'block', retention: { mode: 'none' },
        },
      })),
    }), { mode: 0o600 });
    const startBroker = () => {
      broker = spawn(join(ROOT, 'target/debug/badi-broker'), ['--provider', 'phrase'], { env: environment, stdio: ['ignore', 'pipe', 'pipe'] });
      broker.stdout.on('data', data => { logs += data; });
      broker.stderr.on('data', data => { logs += data; });
    };
    startBroker();
    const endpoint = join(environment.XDG_RUNTIME_DIR, 'badi/broker.sock');
    await until(async () => { assert.equal(broker.exitCode, null, logs); return lstat(endpoint).then(() => true, () => false); });
    for (const profile of ['obsidian', 'terminal']) {
      const client = new BrokerClient(profile);
      const states = [];
      client.on('state', state => states.push(state));
      client.on('diagnostic', reason => states.push(reason));
      client.on('activity', event => states.push(event));
      clients.push(client);
      assert.equal(await client.connect(endpoint), true);
      for (const language of ['e', 'en-', '-en', 'en--x', 'en_US', 'fr-ça', '', 'x'.repeat(36)]) {
        await assert.rejects(client.suggest('thank you', { language }), /Invalid writing language/);
      }
      assert.equal(await client.suggest('thank you', { language: 'en-US' }), ' for your time');
      await delay(800);
      const grant = await client.authorize();
      assert.equal(grant.text, ' for your time');
      await assert.rejects(client.authorize(), /expired/);
      client.report(grant, 'applied');
      assert.equal(await client.suggest('thank you'), ' for your time');
      client.cancel();
      await assert.rejects(client.authorize(), /expired/);
      assert.equal(await client.suggest('thank you'), ' for your time');
      await control('pause', 'on');
      await until(() => !client.allowed);
      await assert.rejects(client.authorize(), /expired/);
      await assert.rejects(client.suggest('thank you'), /disabled/);
      await control('pause', 'off');
      await until(() => client.allowed, states);
      assert.equal(await client.suggest('thank you'), ' for your time');
      client.close();
    }
    await until(async () => (await control('status')).sessions === 0);
    const health = await control('status');
    assert.equal(health.metrics.commits_applied, 2);
    assert.equal(health.metrics.provider_calls, 8);
    assert.equal(health.metrics.provider_errors, 0);
    const readline = await promisify(execFile)('python3', [join(ROOT, 'adapters/shell/test.py')], {
      env: environment, timeout: 15000,
    });
    assert.match(readline.stdout, /no line was submitted/);
    await until(async () => (await control('status')).sessions === 0);
    assert.equal((await control('status')).metrics.commits_applied, 3);
    await chmod(endpoint, 0o666);
    await assert.rejects(new BrokerClient('obsidian').connect(endpoint), /private/);
    await chmod(endpoint, 0o600);

    const recovering = new RecoveringBrokerClient('obsidian');
    clients.push(recovering);
    assert.equal(await recovering.connect(endpoint), true);
    assert.equal(await recovering.suggest('thank you'), ' for your time');

    bridge = spawn(process.execPath, [join(ROOT, 'adapters/shell/bridge.mjs')], {
      env: environment, stdio: ['pipe', 'pipe', 'pipe'],
    });
    bridge.stderr.on('data', data => { logs += data; });
    const replies = createInterface({ input: bridge.stdout })[Symbol.asyncIterator]();
    const reply = async () => {
      let timer;
      try { return (await Promise.race([replies.next(), new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error('Shell bridge reply timed out')), 3000);
      })])).value; }
      finally { clearTimeout(timer); }
    };
    const exchange = command => { bridge.stdin.write(JSON.stringify(command) + '\n'); return reply(); };
    assert.equal(await reply(), 'READY 1');
    assert.equal(await exchange({ operation: 'status' }), 'READY 1');
    assert.match(await exchange({ operation: 'suggest', before: Buffer.from('thank you').toString('base64') }), /^SUGGEST /);
    const exited = once(broker, 'exit'); broker.kill('SIGTERM'); await exited;
    await until(() => !recovering.connected && !recovering.allowed);
    await assert.rejects(recovering.authorize(), /expired/);
    assert.match(await exchange({ operation: 'status' }), /^ERROR model_offline/);
    startBroker();
    await until(async () => lstat(endpoint).then(() => true, () => false));
    await until(() => recovering.connected && recovering.allowed);
    assert.equal((await control('status')).metrics.provider_calls, 0,
      'Automatic recovery must only refresh policy, never replay the old document');
    await assert.rejects(recovering.authorize(), /expired/);
    assert.equal(await recovering.suggest('thank you', { explicit: false }), ' for your time');
    const recoveredGrant = await recovering.authorize('word');
    assert.equal(recoveredGrant.text, ' for');
    await assert.rejects(recovering.authorize('all'), /expired/);
    recovering.report(recoveredGrant, 'applied');
    recovering.close();
    assert.equal(await exchange({ operation: 'status' }), 'READY 2');
    assert.match(await exchange({ operation: 'accept' }), /^ERROR /, 'The old preview cannot acquire a new grant');
    assert.match(await exchange({ operation: 'suggest', before: Buffer.from('thank you').toString('base64') }), /^SUGGEST /);
    assert.equal(await exchange({ operation: 'accept' }), `INSERT ${Buffer.from(' for your time').toString('base64')}`);
    assert.equal(await exchange({ operation: 'result', applied: true }), 'DONE ');
    bridge.stdin.end();
    await once(bridge, 'exit');
    assert.equal(bridge.exitCode, 0, logs);
  } finally {
    if (bridge?.exitCode === null) { bridge.stdin.destroy(); bridge.kill('SIGTERM'); }
    clients.forEach(client => client.close());
    if (broker?.exitCode === null) {
      const exited = once(broker, 'exit');
      broker.kill('SIGTERM');
      const deadline = setTimeout(() => broker.kill('SIGKILL'), 5000);
      await exited; clearTimeout(deadline);
      assert.equal(broker.exitCode, 0, logs);
    }
    await rm(root, { recursive: true, force: true });
    if (desktopRuntime === undefined) delete process.env.XDG_RUNTIME_DIR;
    else process.env.XDG_RUNTIME_DIR = desktopRuntime;
  }
});
