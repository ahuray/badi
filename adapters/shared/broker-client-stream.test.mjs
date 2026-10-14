import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { createServer } from 'node:net';
import { once } from 'node:events';
import { chmodSync } from 'node:fs';
import { join } from 'node:path';
import { BrokerClient } from './broker-client.mjs';

const frame = message => {
  const body = Buffer.from(JSON.stringify({ v: 2, mono_ms: 1, ...message }));
  const header = Buffer.alloc(4);
  header.writeUInt32LE(body.length);
  return Buffer.concat([header, body]);
};

// Just enough broker for one editor session: handshake, policy, and a
// continuation that streams in growing frames.
async function fakeBroker(onSuggest) {
  const directory = await mkdtemp('/tmp/badi-stream-test-');
  const path = join(directory, 'broker.sock');
  const server = createServer(socket => {
    let buffer = Buffer.alloc(0);
    socket.on('data', chunk => {
      buffer = Buffer.concat([buffer, chunk]);
      while (buffer.length >= 4 && buffer.length >= buffer.readUInt32LE() + 4) {
        const message = JSON.parse(buffer.subarray(4, buffer.readUInt32LE() + 4));
        buffer = buffer.subarray(buffer.readUInt32LE() + 4);
        const reply = (type, payload, extra = {}) => socket.write(frame({ id: message.id, type, payload, ...extra }));
        if (message.type === 'hello') {
          reply('hello.ack', { selected_v: 2, max_frame_bytes: 65536, max_before_chars: 512, max_after_chars: 128,
            enabled_capabilities: message.payload.capabilities });
          socket.write(frame({ type: 'authority.changed', payload: { authority_epoch: 1 } }));
        } else if (message.type === 'policy.query') {
          reply('policy.status', { authority_epoch: 1, settings_revision: 1, paused: false, context_allowed: true,
            display_allowed: true, suggestions_allowed: true, learning_allowed: false, activation: 'always' });
        } else if (message.type === 'suggest.request') {
          const coordinates = { session_id: message.session_id, focus_epoch: message.focus_epoch, revision: message.revision };
          const show = (n, text) => frame({ id: message.id, type: 'suggestion.show', ...coordinates, payload: {
            fingerprint: message.payload.fingerprint, suggestion_id: `s${n}`, text, accept_word: ' the',
            ttl_ms: 15000, provider: 'local_model' } });
          onSuggest({ socket, show });
        } else if (message.type === 'control.request') {
          reply('commit.prepare', { fingerprint: message.payload.fingerprint, suggestion_id: message.payload.suggestion_id,
            text: ' the opportunity to help', acceptance: 'all' },
          { session_id: message.session_id, focus_epoch: message.focus_epoch, revision: message.revision });
        }
      }
    });
  });
  server.listen(path);
  await once(server, 'listening');
  chmodSync(path, 0o600);
  return { path, close: async () => { server.close(); await rm(directory, { recursive: true, force: true }); } };
}

test('a streamed continuation grows the shown suggestion and acceptance binds to the newest', async () => {
  let socket;
  let show;
  const broker = await fakeBroker(request => {
    ({ socket, show } = request);
    // Two frames in one write: the second arrives before suggest() resumes.
    socket.write(Buffer.concat([show(1, ' the'), show(2, ' the opportunity')]));
  });
  const client = new BrokerClient('obsidian');
  try {
    assert.equal(await client.connect(broker.path), true);
    const updates = [];
    client.on('update', text => updates.push(text));
    assert.equal(await client.suggest('Thank you for', { language: 'en' }), ' the opportunity');
    socket.write(Buffer.concat([show(3, ' the opportunity to help'), show(4, ' a different start')]));
    await new Promise(resolve => setTimeout(resolve, 50));
    assert.deepEqual(updates, [' the opportunity to help'], 'only a continuation of the shown text');
    const grant = await client.authorize('all');
    assert.equal(grant.suggestion_id, 's3');
  } finally {
    client.close();
    await broker.close();
  }
});
