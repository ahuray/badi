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

const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

// Just enough broker for one editor session: handshake, policy, and a
// continuation that streams in growing frames. `received` lists the type of
// every frame the client sent.
async function fakeBroker(onSuggest) {
  const directory = await mkdtemp('/tmp/badi-stream-test-');
  const path = join(directory, 'broker.sock');
  const received = [];
  // Each shown suggestion's text, which its grant carries.
  const texts = new Map();
  const server = createServer(socket => {
    let buffer = Buffer.alloc(0);
    socket.on('data', chunk => {
      buffer = Buffer.concat([buffer, chunk]);
      while (buffer.length >= 4 && buffer.length >= buffer.readUInt32LE() + 4) {
        const message = JSON.parse(buffer.subarray(4, buffer.readUInt32LE() + 4));
        buffer = buffer.subarray(buffer.readUInt32LE() + 4);
        received.push(message.type);
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
          const show = (n, text, ttl = 15000) => {
            texts.set(`s${n}`, text);
            return frame({ id: message.id, type: 'suggestion.show', ...coordinates, payload: {
              fingerprint: message.payload.fingerprint, suggestion_id: `s${n}`, text, accept_word: ' the',
              ttl_ms: ttl, provider: 'local_model' } });
          };
          onSuggest({ socket, show });
        } else if (message.type === 'control.request') {
          reply('commit.prepare', { fingerprint: message.payload.fingerprint, suggestion_id: message.payload.suggestion_id,
            text: texts.get(message.payload.suggestion_id), acceptance: 'all' },
          { session_id: message.session_id, focus_epoch: message.focus_epoch, revision: message.revision });
        }
      }
    });
  });
  server.listen(path);
  await once(server, 'listening');
  chmodSync(path, 0o600);
  return { path, received, close: async () => { server.close(); await rm(directory, { recursive: true, force: true }); } };
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

test('a grant outlives the preview expiry until the editor reports its result', async () => {
  const broker = await fakeBroker(({ socket, show }) => socket.write(show(1, ' the', 300)));
  const client = new BrokerClient('obsidian');
  try {
    assert.equal(await client.connect(broker.path), true);
    assert.equal(await client.suggest('Thank you for', { language: 'en' }), ' the');
    const grant = await client.authorize('all');
    // The editor reports after the 300 ms preview lifetime has run out.
    await delay(400);
    client.report(grant, 'applied');
    await delay(50);
    const closing = broker.received.slice(broker.received.indexOf('control.request'));
    assert.deepEqual(closing, ['control.request', 'commit.result', 'session.close'],
      'the result is reported, then the session closes');
  } finally {
    client.close();
    await broker.close();
  }
});
