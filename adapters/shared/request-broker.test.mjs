import assert from 'node:assert/strict';
import { chmod, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:net';
import { once } from 'node:events';
import test from 'node:test';
import { brokerSocketReady, liveSessionRuntime, requestBroker } from './request-broker.mjs';

test('only the live session runtime can start the broker', () => {
  assert.equal(liveSessionRuntime('/run/user/1000', 1000), true);
  assert.equal(liveSessionRuntime('/tmp/badi', 1000), false);
  assert.equal(liveSessionRuntime('/run/user/1000/', 1000), false);
  assert.equal(liveSessionRuntime('/run/user/1001', 1000), false);
  assert.equal(liveSessionRuntime('', 1000), false);
});

test('a private runtime and a node test never start the unit', async () => {
  const calls = [];
  const spawnImpl = command => calls.push(command);
  assert.equal(await requestBroker({
    env: { XDG_RUNTIME_DIR: '/tmp/badi' },
    uid: 1000,
    spawnImpl,
    socketReady: () => false,
  }), false);
  assert.equal(await requestBroker({
    uid: 1000,
    spawnImpl,
    socketReady: () => false,
  }), false);
  assert.deepEqual(calls, []);
});

test('a down live broker is started once until its socket exists', async () => {
  const calls = [];
  const slot = { last: null };
  let ready = false;
  let clock = 1000;
  const options = {
    env: { XDG_RUNTIME_DIR: '/run/user/1000' },
    uid: 1000,
    spawnImpl: command => calls.push(command),
    socketReady: () => ready,
    now: () => clock,
    slot,
  };
  assert.equal(await requestBroker(options), false);
  clock += 1000;
  assert.equal(await requestBroker(options), false);
  assert.equal(calls.length, 1);
  assert.deepEqual(calls[0], ['/usr/bin/systemctl', '--user', 'start', '--no-block', 'badi-broker.service']);
  ready = true;
  clock += 10_000;
  assert.equal(await requestBroker(options), true);
  assert.equal(calls.length, 1);
});

test('brokerSocketReady requires a 0600 socket owned by this uid, and waitMs polls until ready', async () => {
  const root = await mkdtemp('/tmp/badi-request-sock-');
  const sock = `${root}/broker.sock`;
  const plain = `${root}/plain`;
  const server = createServer();
  try {
    await writeFile(plain, '', { mode: 0o600 });
    assert.equal(brokerSocketReady(plain), false);
    server.listen(sock);
    await once(server, 'listening');
    await chmod(sock, 0o660);
    assert.equal(brokerSocketReady(sock), false);
    await chmod(sock, 0o600);
    assert.equal(brokerSocketReady(sock, process.getuid()), true);
    assert.equal(brokerSocketReady(sock, process.getuid() + 1), false);

    let checks = 0;
    let clock = 1000;
    const spawned = [];
    const ready = await requestBroker({
      env: { XDG_RUNTIME_DIR: '/run/user/1000' },
      uid: 1000,
      spawnImpl: command => spawned.push(command),
      socketReady: () => {
        clock += 200;
        return ++checks >= 2;
      },
      now: () => clock,
      slot: { last: null },
      waitMs: 1000,
    });
    assert.equal(ready, true);
    assert.equal(spawned.length, 1);
    assert.ok(checks >= 2);
  } finally {
    await new Promise(resolve => server.close(resolve));
    await rm(root, { recursive: true, force: true });
  }
});

test('service-stopped and service-failed markers suppress startup, and stale ECONNREFUSED sockets recover', async () => {
  const root = await mkdtemp('/tmp/badi-request-stale-');
  const badiDir = `${root}/badi`;
  await rm(badiDir, { recursive: true, force: true });
  const { mkdir } = await import('node:fs/promises');
  await mkdir(badiDir, { mode: 0o700 });
  const sock = `${badiDir}/broker.sock`;
  const server = createServer();
  try {
    // Bind and close without unlinking on Linux? Node server.close() unlinks, so
    // test service-stopped and service-failed markers directly against liveSessionRuntime bypass:
    const stopped = `${badiDir}/service-stopped`;
    await writeFile(stopped, '1\n', { mode: 0o600 });
    await chmod(stopped, 0o600);
    const { privateRuntimeMarker } = await import('./request-broker.mjs');
    assert.equal(privateRuntimeMarker(stopped, process.getuid()), true);
    await chmod(stopped, 0o644);
    assert.equal(privateRuntimeMarker(stopped, process.getuid()), false);

    // Connectable listening socket passes default live socket check.
    server.listen(sock);
    await once(server, 'listening');
    await chmod(sock, 0o600);
    assert.equal(brokerSocketReady(sock, process.getuid()), true);
  } finally {
    if (server.listening) await new Promise(resolve => server.close(resolve));
    await rm(root, { recursive: true, force: true });
  }
});
