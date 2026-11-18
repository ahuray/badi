import { test } from 'node:test';
import assert from 'node:assert/strict';
import { chmod, mkdtemp, rm } from 'node:fs/promises';
import { createServer } from 'node:net';
import { once } from 'node:events';
import { RecoveringBrokerClient } from './recovering-client.mjs';

const frame = message => {
  const body = Buffer.from(JSON.stringify({ v: 2, mono_ms: 1, ...message }));
  const header = Buffer.alloc(4);
  header.writeUInt32LE(body.length);
  return Buffer.concat([header, body]);
};

const settle = async () => { for (let turn = 0; turn < 12; turn++) await Promise.resolve(); };

test('unavailable editor transport retries with a bound and can be retried by an explicit action', async context => {
  const root = await mkdtemp('/tmp/badi-recovery-');
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const client = new RecoveringBrokerClient('obsidian');
  const states = [];
  client.on('state', state => states.push(state));
  try {
    await assert.rejects(client.connect(`${root}/missing.sock`), /ENOENT/);
    for (let retry = 0; retry < 12; retry++) {
      context.mock.timers.tick(5000);
      await settle();
    }
    assert.equal(states.filter(state => state === 'offline').length, 10);
    assert.equal(states.at(-1), 'offline_retry');
    assert.equal(client.connected, false);
    assert.equal(client.allowed, false);
    await assert.rejects(client.suggest('never replay this'), /disabled/);
    await assert.rejects(client.connect(), /ENOENT/);
    assert.equal(states.filter(state => state === 'offline').length, 11);
    client.close();
    context.mock.timers.tick(60_000);
    await settle();
    assert.equal(states.filter(state => state === 'offline').length, 11);
  } finally {
    client.close();
    context.mock.timers.reset();
    await rm(root, { recursive: true, force: true });
  }
});

test('immediate connect bypasses an armed backoff timer after an on-demand broker start', async () => {
  const root = await mkdtemp('/tmp/badi-recovery-now-');
  const endpoint = `${root}/broker.sock`;
  const client = new RecoveringBrokerClient('obsidian');
  let server;
  try {
    await assert.rejects(client.connect(endpoint), /ENOENT/);
    // A plain connect yields to the scheduled backoff timer.
    assert.equal(await client.connect(endpoint), false);
    server = createServer(socket => {
      let buffer = Buffer.alloc(0);
      socket.on('data', chunk => {
        buffer = Buffer.concat([buffer, chunk]);
        while (buffer.length >= 4 && buffer.length >= buffer.readUInt32LE() + 4) {
          const message = JSON.parse(buffer.subarray(4, buffer.readUInt32LE() + 4));
          buffer = buffer.subarray(buffer.readUInt32LE() + 4);
          if (message.type === 'hello') {
            socket.write(frame({
              id: message.id, type: 'hello.ack',
              payload: { selected_v: 2, max_frame_bytes: 65536, max_before_chars: 512, max_after_chars: 128,
                enabled_capabilities: message.payload.capabilities },
            }));
            socket.write(frame({ type: 'authority.changed', payload: { authority_epoch: 1 } }));
          } else if (message.type === 'policy.query') {
            socket.write(frame({
              id: message.id, type: 'policy.status',
              payload: { authority_epoch: 1, settings_revision: 1, paused: false, context_allowed: true,
                display_allowed: true, suggestions_allowed: true, learning_allowed: false, activation: 'always' },
            }));
          }
        }
      });
    });
    server.listen(endpoint);
    await once(server, 'listening');
    await chmod(endpoint, 0o600);
    assert.equal(await client.connect(endpoint, { immediate: true }), true);
    assert.equal(client.connected, true);
    assert.equal(client.allowed, true);
  } finally {
    client.close();
    if (server) await new Promise(resolve => server.close(resolve));
    await rm(root, { recursive: true, force: true });
  }
});

test('held terminal focus keeps watching past 10 retries, sends editor.focus once up, and stops on blur', async context => {
  const root = await mkdtemp('/tmp/badi-recovery-focus-');
  const endpoint = `${root}/broker.sock`;
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const client = new RecoveringBrokerClient('neovim');
  const states = [];
  const focusMessages = [];
  let attemptsSeen = 0;
  client.on('state', state => states.push(state));
  let server;
  try {
    await client.reportFocus(true);
    await assert.rejects(client.connect(endpoint), /ENOENT/);
    for (let retry = 0; retry < 14; retry++) {
      context.mock.timers.tick(5000);
      await settle();
    }
    assert.equal(states.filter(state => state === 'offline').length, 10);
    assert.equal(states.filter(state => state === 'offline_retry').length, 1);

    // Losing focus stops the post-budget focus poll.
    await client.reportFocus(false);
    context.mock.timers.reset();

    server = createServer(socket => {
      attemptsSeen++;
      let buffer = Buffer.alloc(0);
      socket.on('data', chunk => {
        buffer = Buffer.concat([buffer, chunk]);
        while (buffer.length >= 4 && buffer.length >= buffer.readUInt32LE() + 4) {
          const message = JSON.parse(buffer.subarray(4, buffer.readUInt32LE() + 4));
          buffer = buffer.subarray(buffer.readUInt32LE() + 4);
          if (message.type === 'hello') {
            socket.write(frame({
              id: message.id, type: 'hello.ack',
              payload: { selected_v: 2, max_frame_bytes: 65536, max_before_chars: 512, max_after_chars: 128,
                enabled_capabilities: message.payload.capabilities },
            }));
            socket.write(frame({ type: 'authority.changed', payload: { authority_epoch: 1 } }));
          } else if (message.type === 'policy.query') {
            socket.write(frame({
              id: message.id, type: 'policy.status',
              payload: { authority_epoch: 1, settings_revision: 1, paused: false, context_allowed: true,
                display_allowed: true, suggestions_allowed: true, learning_allowed: false, activation: 'always' },
            }));
          } else if (message.type === 'editor.focus') {
            focusMessages.push(message.payload.focused);
            socket.write(frame({
              id: message.id, type: 'editor.status',
              payload: { native_withheld: message.payload.focused === true },
            }));
          }
        }
      });
    });
    server.listen(endpoint);
    await once(server, 'listening');
    await chmod(endpoint, 0o600);

    await new Promise(resolve => setTimeout(resolve, 1200));
    assert.equal(attemptsSeen, 0, 'blurred client stopped polling after 10 attempts');

    // Regaining focus resumes the quiet poll and claims editor.focus once connected.
    await client.reportFocus(true);
    const deadline = Date.now() + 3000;
    while (focusMessages.length === 0 && Date.now() < deadline) {
      await new Promise(resolve => setTimeout(resolve, 25));
    }
    assert.deepEqual(focusMessages, [true]);
    assert.equal(client.connected, true);
  } finally {
    client.close();
    context.mock.timers.reset();
    if (server) await new Promise(resolve => server.close(resolve));
    await rm(root, { recursive: true, force: true });
  }
});

test('pauseRetries suppresses retry scheduling even when called while an attempt is in flight', async context => {
  const root = await mkdtemp('/tmp/badi-recovery-pause-');
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const client = new RecoveringBrokerClient('obsidian');
  const states = [];
  client.on('state', state => states.push(state));
  try {
    const pending = client.connect(`${root}/missing.sock`);
    client.pauseRetries();
    await assert.rejects(pending, /ENOENT/);
    for (let retry = 0; retry < 5; retry++) {
      context.mock.timers.tick(5000);
      await settle();
    }
    assert.equal(states.filter(state => state === 'offline').length, 1);
  } finally {
    client.close();
    context.mock.timers.reset();
    await rm(root, { recursive: true, force: true });
  }
});
