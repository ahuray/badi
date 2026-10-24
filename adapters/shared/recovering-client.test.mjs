import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { RecoveringBrokerClient } from './recovering-client.mjs';

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
