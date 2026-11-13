import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { createServer } from 'node:net';
import { once } from 'node:events';
import { chmodSync } from 'node:fs';
import { join } from 'node:path';
import { BrokerClient } from './broker-client.mjs';
import { validRevisionText, revisionDiff } from './revision.mjs';

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
async function fakeBroker(onSuggest, onRevision) {
  const directory = await mkdtemp('/tmp/badi-stream-test-');
  const path = join(directory, 'broker.sock');
  const received = [];
  const messages = [];
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
        messages.push(message);
        const reply = (type, payload, extra = {}) => socket.write(frame({ id: message.id, type, payload, ...extra }));
        if (message.type === 'hello') {
          reply('hello.ack', { selected_v: 2, max_frame_bytes: 65536, max_before_chars: 512, max_after_chars: 128,
            enabled_capabilities: message.payload.capabilities });
          socket.write(frame({ type: 'authority.changed', payload: { authority_epoch: 1 } }));
        } else if (message.type === 'policy.query') {
          reply('policy.status', { authority_epoch: 1, settings_revision: 1, paused: false, context_allowed: true,
            display_allowed: true, suggestions_allowed: true, learning_allowed: false, activation: 'always' });
        } else if (message.type === 'revision.request') {
          const coordinates = { session_id: message.session_id, focus_epoch: message.focus_epoch, revision: message.revision };
          const show = (text = 'I went.', extra = {}) => {
            const revisionId = `r${message.revision}`;
            texts.set(revisionId, { text, selection: message.payload.selection, ...extra });
            return frame({ id: message.id, type: 'revision.show', ...coordinates, payload: {
              fingerprint: message.payload.fingerprint, revision_id: revisionId, mode: message.payload.mode,
              original: message.payload.selected, text, ttl_ms: 60000, provider: 'phrase_v1', ...extra } });
          };
          onRevision({ socket, message, show, reply, coordinates });
        } else if (message.type === 'suggest.request') {
          const coordinates = { session_id: message.session_id, focus_epoch: message.focus_epoch, revision: message.revision };
          const show = (n, text, ttl = 15000, extra = {}) => {
            const spec = {
              text,
              acceptWord: extra.accept_word ?? ' the',
              replaceBefore: extra.replace_before,
              grantText: extra.grant_text,
              grantReplaceBefore: extra.grant_replace_before ?? extra.replace_before,
              selection: extra.replace_selection,
            };
            texts.set(`s${n}`, spec);
            return frame({ id: message.id, type: 'suggestion.show', ...coordinates, payload: {
              fingerprint: message.payload.fingerprint, suggestion_id: `s${n}`, text,
              accept_word: spec.acceptWord,
              ...(spec.replaceBefore !== undefined ? { replace_before: spec.replaceBefore } : {}),
              ttl_ms: ttl, provider: 'local_model' } });
          };
          onSuggest({ socket, show });
        } else if (message.type === 'control.request' && message.payload.action === 'alternatives' && !message.payload.word) {
          const coordinates = { session_id: message.session_id, focus_epoch: message.focus_epoch, revision: message.revision };
          reply('control.result', { accepted: true }, coordinates);
          socket.write(frame({ id: 'clear-1', type: 'suggestion.clear', ...coordinates, payload: {
            fingerprint: message.payload.fingerprint, suggestion_id: message.payload.suggestion_id, reason: 'superseded' } }));
          texts.set('s2', { text: ' again soon', acceptWord: ' again' });
          socket.write(frame({ id: message.id, type: 'suggestion.list', ...coordinates, payload: {
            fingerprint: message.payload.fingerprint, suggestion_id: message.payload.suggestion_id,
            words: ['so', 'again'], ttl_ms: 15000, provider: 'local_model' } }));
        } else if (message.type === 'control.request' && message.payload.action === 'alternatives' && message.payload.word) {
          const coordinates = { session_id: message.session_id, focus_epoch: message.focus_epoch, revision: message.revision };
          reply('control.result', { accepted: true }, coordinates);
          socket.write(frame({ id: message.id, type: 'suggestion.show', ...coordinates, payload: {
            fingerprint: message.payload.fingerprint, suggestion_id: 's2', text: ' again soon',
            accept_word: ' again', ttl_ms: 15000, provider: 'local_model' } }));
        } else if (message.type === 'control.request') {
          const spec = texts.get(message.payload.suggestion_id);
          const acceptance = message.payload.action === 'accept_word' ? 'word' : 'all';
          reply('commit.prepare', { fingerprint: message.payload.fingerprint, suggestion_id: message.payload.suggestion_id,
            text: spec?.grantText ?? (acceptance === 'word' ? spec?.acceptWord : spec?.text), acceptance,
            ...(spec?.selection ? { replace_selection: spec.grantSelection ?? spec.selection } : {}),
            ...(spec?.grantReplaceBefore !== undefined ? { replace_before: spec.grantReplaceBefore } : {}) },
          { session_id: message.session_id, focus_epoch: message.focus_epoch, revision: message.revision });
        }
      }
    });
  });
  server.listen(path);
  await once(server, 'listening');
  chmodSync(path, 0o600);
  return { path, received, messages, close: async () => { server.close(); await rm(directory, { recursive: true, force: true }); } };
}

const revisionRequest = { before: '', selected: 'I has went.', after: '', mode: 'grammar', language: 'en',
  selection: { anchor: 0, head: 11, unit: 'unicode_scalar_values' } };

for (const profile of ['neovim', 'obsidian']) test(`${profile} revisions retain one exact session and grant a selection only once`, async () => {
  const broker = await fakeBroker(null, ({ socket, message, show }) => {
    if (message.revision > 1) socket.write(frame({ type: 'revision.clear',
      session_id: message.session_id, focus_epoch: 1, revision: message.revision - 1,
      payload: { fingerprint: 'older', reason: 'superseded' } }));
    socket.write(show());
  });
  const client = new BrokerClient(profile, { selectionRevision: true });
  try {
    await client.connect(broker.path);
    assert.equal((await client.revise(revisionRequest)).text, 'I went.');
    assert.equal((await client.revise({ ...revisionRequest, mode: 'refine' })).text, 'I went.');
    const requests = broker.messages.filter(message => message.type === 'revision.request');
    assert.equal(requests[0].session_id, requests[1].session_id);
    assert.equal(requests[1].revision, requests[0].revision + 1);
    assert.notEqual(requests[0].payload.fingerprint, requests[1].payload.fingerprint);
    const grant = await client.authorizeRevision();
    assert.deepEqual(grant.replaceSelection, revisionRequest.selection);
    assert.equal(grant.text, 'I went.');
    await assert.rejects(client.authorizeRevision(), /expired/);
    client.report(grant, 'applied');
    await client.revise(revisionRequest);
    const last = broker.messages.filter(message => message.type === 'revision.request').at(-1);
    assert.notEqual(last.session_id, requests[0].session_id, 'A completed edit cannot reuse its authority');
  } finally { client.close(); await broker.close(); }
});

test('revision-specific timeouts admit a slow reply; cancellation refuses a late reply', async () => {
  let respond;
  const broker = await fakeBroker(null, ({ socket, show }) => { respond = () => socket.write(show()); });
  const client = new BrokerClient('neovim', { selectionRevision: true });
  try {
    await client.connect(broker.path);
    const pending = client.revise(revisionRequest);
    await delay(2150);
    respond();
    assert.equal((await pending).text, 'I went.');
    const cancelled = client.revise(revisionRequest);
    const rejected = assert.rejects(cancelled, /Context changed/);
    await delay(20);
    client.cancel();
    await rejected;
    respond();
    await delay(20);
    await assert.rejects(client.authorizeRevision(), /expired/);
  } finally { client.close(); await broker.close(); }
});

test('a continuation grant cannot silently acquire selection replacement authority', async () => {
  const broker = await fakeBroker(({ socket, show }) => socket.write(show(1, ' the', 15000,
    { replace_selection: revisionRequest.selection })));
  const client = new BrokerClient('neovim', { selectionRevision: true });
  try {
    await client.connect(broker.path);
    await client.suggest('Thank you for');
    await assert.rejects(client.authorize(), /revoked/);
  } finally { client.close(); await broker.close(); }
});

test('a revision rejects mismatched original, mode, safety, expiry and replacement ranges', async () => {
  for (const extra of [{ original: 'other' }, { mode: 'synonyms' }, { text: 'I\nwent.' }, { ttl_ms: 60001 },
    { grantSelection: { anchor: 1, head: 12, unit: 'unicode_scalar_values' } }]) {
    const broker = await fakeBroker(null, ({ socket, show }) => socket.write(show('I went.', extra)));
    const client = new BrokerClient('neovim', { selectionRevision: true });
    try {
      await client.connect(broker.path);
      if (extra.grantSelection) {
        await client.revise(revisionRequest);
        await assert.rejects(client.authorizeRevision(), /revoked/);
      } else await assert.rejects(client.revise(revisionRequest), /Invalid revision/);
    } finally { client.close(); await broker.close(); }
  }
  assert.throws(() => new BrokerClient('terminal', { selectionRevision: true }), /not supported/);
  assert.doesNotThrow(() => new BrokerClient('obsidian', { selectionRevision: true }));
  assert.equal(validRevisionText('x'.repeat(1100)), true);
  assert.equal(validRevisionText('x'.repeat(1101)), false);
  for (const text of ['a\u200cb', 'a\u2028b', 'a\tb', 'a\u2003b']) assert.equal(validRevisionText(text), false);
  const diff = revisionDiff('I has went. ', 'I went. ');
  assert.equal(diff.filter(part => part.kind !== 'added').map(part => part.text).join(''), 'I has went. ');
  assert.equal(diff.filter(part => part.kind !== 'removed').map(part => part.text).join(''), 'I went. ');
});

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
    assert.equal(client.acceptWord, ' the', 'Previews read what Tab takes of the shown candidate');
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

test('a numbered list replaces the shown suggestion and a pick shows its continuation', async () => {
  const broker = await fakeBroker(({ socket, show }) => socket.write(show(1, ' the opportunity')));
  const client = new BrokerClient('obsidian');
  try {
    assert.equal(await client.connect(broker.path), true);
    assert.equal(await client.suggest('Thank you for', { language: 'en' }), ' the opportunity');
    const list = await client.alternative();
    assert.deepEqual(list, { words: ['so', 'again'] }, 'the chord lists other first words');
    assert.deepEqual(client.alternativeWords, ['so', 'again']);
    await assert.rejects(client.pickAlternative('never'), 'an unlisted word is never picked');
    assert.equal(await client.pickAlternative('again'), ' again soon');
    assert.equal(client.acceptWord, ' again');
    assert.equal(client.alternativeWords, null, 'the pick spends the list');
    await assert.rejects(client.pickAlternative('again'), 'a spent list cannot pick twice');
  } finally {
    client.close();
    await broker.close();
  }
});

test('a negotiated correction binds its exact suffix through word acceptance', async () => {
  const broker = await fakeBroker(({ socket, show }) => socket.write(show(1, 'address ', 15000, {
    accept_word: 'address ', replace_before: 'adress ',
  })));
  const client = new BrokerClient('neovim', { textReplacement: true });
  try {
    assert.equal(await client.connect(broker.path), true);
    assert.equal(await client.suggest('Please check the adress ', { language: 'en' }), 'address ');
    assert.equal(client.replacementBefore, 'adress ');
    assert.equal(client.acceptWord, 'address ');
    await assert.rejects(client.alternative(), /No suggestion to replace/);
    const grant = await client.authorize('word');
    assert.equal(grant.text, 'address ');
    assert.equal(grant.replaceBefore, 'adress ');
  } finally {
    client.close();
    await broker.close();
  }
});

test('a replacement is rejected without negotiated text replacement', async () => {
  const broker = await fakeBroker(({ socket, show }) => socket.write(show(1, 'address ', 15000, {
    accept_word: 'address ', replace_before: 'adress ',
  })));
  const client = new BrokerClient('neovim');
  try {
    assert.equal(await client.connect(broker.path), true);
    await assert.rejects(client.suggest('Please check the adress ', { language: 'en' }),
      /Invalid model suggestion/);
    assert.equal(client.replacementBefore, undefined);
  } finally {
    client.close();
    await broker.close();
  }
});

test('a grant for another replacement suffix is revoked', async () => {
  const broker = await fakeBroker(({ socket, show }) => socket.write(show(1, 'address ', 15000, {
    accept_word: 'address ', replace_before: 'adress ', grant_replace_before: 'other ',
  })));
  const client = new BrokerClient('neovim', { textReplacement: true });
  try {
    assert.equal(await client.connect(broker.path), true);
    assert.equal(await client.suggest('Please check the adress ', { language: 'en' }), 'address ');
    await assert.rejects(client.authorize('word'), /Acceptance was revoked/);
  } finally {
    client.close();
    await broker.close();
  }
});

test('a streamed frame cannot turn a continuation into a replacement', async () => {
  let socket;
  let show;
  const broker = await fakeBroker(request => {
    ({ socket, show } = request);
    socket.write(show(1, ' the'));
  });
  const client = new BrokerClient('neovim', { textReplacement: true });
  try {
    assert.equal(await client.connect(broker.path), true);
    const updates = [];
    client.on('update', text => updates.push(text));
    assert.equal(await client.suggest('Thank you for', { language: 'en' }), ' the');
    socket.write(show(2, ' the address ', 15000, {
      accept_word: ' the address ', replace_before: 'adress ',
    }));
    await delay(50);
    assert.deepEqual(updates, []);
    assert.equal(client.replacementBefore, undefined);
    assert.equal(client.acceptWord, ' the');
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
