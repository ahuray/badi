import assert from 'node:assert/strict';
import test from 'node:test';
import { request as httpRequest } from 'node:http';
import { createLabServer } from './server.mjs';

const deferred = () => { let resolve; let reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
const input = (overrides = {}) => ({ schema: 'badi.context-lookup.request.v1', id: 'http-fixture',
  before: 'Use the note', language: 'en', context: 'The notebook is ready.', style_examples: [], ...overrides });
const envelope = request => ({ result: { schema: 'badi.context-lookup.result.v1', id: request.id,
  contract_id: 'badi.context-lookup.exact.v1', outcome: 'suggestion', reason: 'unique_source_word',
  text: 'book', matched_word: 'notebook', matched_candidate_count: 1, sources: ['context'], latency_ms: 1,
  warnings: ['context_match_does_not_prove_intent'] }, worker: { sha256: 'a'.repeat(64), bytes: 100 },
cleanup: { process_id: 12345, reaped: true, exit_code: 0, forced: false } });
const predictionBody = { suite: { schema: 'badi.prediction-suite.v1', name: 'fixture', cases: [
  { id: 'sample', language: 'en', prefix: 'Please review the docum', expected: ['ent'], context: '', style: '' }] },
configs: [{ id: 'baseline', mode: 'production_baseline', budget_ms: 550, max_tokens: 8, cache_prompt: true,
  temperature: 0, seed: 42 }], seed: 42 };

async function setup(t, behavior = {}) {
  const workers = [];
  const prediction = { calls: 0, entered: deferred(),
    async predict() { this.calls++; this.pending = deferred(); this.entered.resolve(); return this.pending.promise; },
    async stop() { this.pending?.reject(new Error('fixture_cancelled')); } };
  const spelling = { starts: 0, entered: deferred(), ready: null,
    async start() { this.starts++; this.entered.resolve(); return new Promise(() => {}); },
    async check() { throw new Error('unexpected_spelling_check'); }, async stop() { return null; } };
  const lab = await createLabServer({ worker: prediction, spellingManifests: { de: '/fixture/manifest.json' },
    spellingManifestLoader: async () => ({}), spellingWorkerFactory: () => spelling,
    contextLookupWorkerFactory: () => {
      const worker = { calls: [], stops: 0,
        async check(value, signal) { this.calls.push(structuredClone(value)); return behavior.check ? behavior.check(value, signal) : envelope(value); },
        async stop() { this.stops++; if (behavior.stop) return behavior.stop(); return null; } };
      workers.push(worker); return worker;
    } });
  t.after(async () => { try { await lab.close(); } catch (failure) { assert.equal(failure.message, 'context_lookup_cleanup_unverified'); } });
  const page = await fetch(lab.origin);
  const token = (await page.text()).match(/name="badi-lab-token" content="([a-f0-9]{64})"/u)?.[1]; assert.ok(token);
  const headers = { 'Content-Type': 'application/json', 'X-Badi-Lab-Token': token };
  return { ...lab, workers, prediction, spellingFixture: spelling, headers,
    post: (value = input(), path = '/api/context-lookup', extra = {}) => fetch(lab.origin + path,
      { method: 'POST', headers, body: JSON.stringify(value), ...extra }) };
}

async function partial(lab, path, value) {
  const entered = deferred(); const response = deferred();
  lab.server.once('request', request => request.once('data', () => entered.resolve()));
  const request = httpRequest(new URL(path, lab.origin), { method: 'POST', headers: lab.headers }, reply => {
    const chunks = []; reply.on('data', chunk => chunks.push(chunk));
    reply.on('end', () => response.resolve({ status: reply.statusCode, body: Buffer.concat(chunks).toString() }));
  });
  request.on('error', response.reject);
  const text = JSON.stringify(value); request.write(text.slice(0, 12)); await entered.promise;
  return { finish: () => request.end(text.slice(12)), response: response.promise };
}

test('lookup HTTP guards Host, Origin, capability, strict JSON and reference exclusion before any process', async t => {
  const lab = await setup(t);
  for (const extra of [{ headers: { 'Content-Type': 'application/json' } },
    { headers: { ...lab.headers, Origin: 'null' } }]) {
    assert.equal((await lab.post(input(), '/api/context-lookup', extra)).status, 403);
  }
  const wrongHost = await new Promise((resolveReply, reject) => {
    const request = httpRequest(lab.origin + '/api/context-lookup/status', { headers: { ...lab.headers, Host: 'foreign.example' } }, reply => {
      reply.resume(); reply.on('end', () => resolveReply(reply.statusCode));
    });
    request.on('error', reject); request.end();
  });
  assert.equal(wrongHost, 403);
  for (const value of [input({ expected: ['NEVER'] }), input({ config: {} }), input({ style_examples: [' '] }),
    input({ before: 'x'.repeat(2049) }), input({ language: 'fr' })]) {
    const reply = await lab.post(value); assert.equal(reply.status, 400);
    assert.deepEqual(await reply.json(), { error: 'context_lookup_invalid_input' });
  }
  assert.equal((await lab.post(input(), '/api/context-lookup', { body: '{bad' })).status, 400);
  const status = await fetch(lab.origin + '/api/context-lookup/status', { headers: lab.headers });
  assert.equal(status.headers.get('cache-control'), 'no-store');
  assert.match(status.headers.get('content-security-policy'), /frame-ancestors 'none'/u);
  assert.equal(lab.workers.length, 0); assert.equal(lab.prediction.calls, 0); assert.equal(lab.spellingFixture.starts, 0);
});

test('successful lookup keeps exact snapshot and returns actual worker/cleanup fields without model identity', async t => {
  const lab = await setup(t);
  const reply = await lab.post(); assert.equal(reply.status, 200);
  const value = await reply.json();
  assert.equal(value.schema, 'badi.context-lookup.response.v1');
  assert.deepEqual(value.result, envelope(input()).result); assert.deepEqual(value.cleanup, envelope(input()).cleanup);
  assert.deepEqual(value.worker, envelope(input()).worker); assert.ok(Number.isFinite(value.elapsed_ms));
  assert.equal(value.ready, undefined); assert.equal(value.identity, undefined);
  assert.deepEqual(lab.workers[0].calls, [input()]);
  assert.equal((await lab.post(input({ id: 'another' }))).status, 200);
  assert.equal(lab.workers.length, 2);
  assert.equal(lab.prediction.calls, 0); assert.equal(lab.spellingFixture.starts, 0);
});

test('reset fences old uploads and waits cleanup before rejecting even a successful late reply', async t => {
  const entered = deferred(); const pending = deferred(); const stopping = deferred(); const cleanup = deferred();
  const lab = await setup(t, { check: () => { entered.resolve(); return pending.promise; },
    stop: () => { stopping.resolve(); return cleanup.promise; } });
  const upload = await partial(lab, '/api/context-lookup', input());
  assert.equal((await lab.post({}, '/api/context-lookup/reset')).status, 200);
  upload.finish(); assert.equal((await upload.response).status, 409); assert.equal(lab.workers.length, 0);
  const first = lab.post(); await entered.promise;
  assert.equal((await lab.post()).status, 409);
  const reset = lab.post({}, '/api/context-lookup/reset'); await stopping.promise;
  let resetFinished = false; reset.then(() => { resetFinished = true; });
  await new Promise(resolve => setImmediate(resolve)); assert.equal(resetFinished, false);
  assert.equal(lab.contextLookup.busy, true);
  pending.resolve(envelope(input())); cleanup.resolve();
  assert.equal((await reset).status, 200);
  const cancelled = await first; assert.equal(cancelled.status, 409);
  assert.deepEqual(await cancelled.json(), { error: 'context_lookup_cancelled' });
  assert.equal(lab.contextLookup.busy, false);
});

test('lookup shares capacity with prediction and spelling, including uploads already reading bodies', async t => {
  const entered = deferred();
  const lab = await setup(t, { check: () => { entered.resolve(); return new Promise(() => {}); } });
  const upload = await partial(lab, '/api/run', predictionBody);
  const lookup = lab.post(); await entered.promise;
  upload.finish(); assert.equal((await upload.response).status, 409);
  assert.equal((await lab.post(predictionBody, '/api/run')).status, 409);
  assert.equal((await lab.post({ before: 'word ', language: 'de', protected_words: [] }, '/api/spelling')).status, 409);
  await lab.post({}, '/api/context-lookup/reset'); assert.equal((await lookup).status, 409);
  const lookupUpload = await partial(lab, '/api/context-lookup', input());
  const spelling = lab.post({ before: 'word ', language: 'de', protected_words: [] }, '/api/spelling');
  await lab.spellingFixture.entered.promise;
  lookupUpload.finish(); assert.equal((await lookupUpload.response).status, 409);
  assert.equal((await lab.post()).status, 409);
  await lab.post({}, '/api/spelling/reset'); assert.equal((await spelling).status, 409);
  const prediction = await lab.post(predictionBody, '/api/run'); await lab.prediction.entered.promise;
  assert.equal((await lab.post()).status, 409);
  await lab.post({}, '/api/reset'); await prediction.text();
});

test('disconnect cancels lookup and clear synchronizes its cleanup', async t => {
  const entered = deferred(); const stopped = deferred();
  const lab = await setup(t, { check: () => { entered.resolve(); return new Promise(() => {}); }, stop: () => stopped.resolve() });
  const controller = new AbortController();
  const first = lab.post(input(), '/api/context-lookup', { signal: controller.signal });
  await entered.promise; controller.abort(); await assert.rejects(first, /abort/iu); await stopped.promise;
  assert.equal((await lab.post({}, '/api/context-lookup/reset')).status, 200);
  assert.equal(lab.contextLookup.busy, false);
});

test('invalid response or missing cleanup cannot be displayed; cleanup failure blocks subsequent work', async t => {
  const invalid = await setup(t, { check: value => ({ ...envelope(value), result: { ...envelope(value).result, id: 'foreign' } }) });
  const bad = await invalid.post(); assert.equal(bad.status, 503);
  assert.deepEqual(await bad.json(), { error: 'context_lookup_worker_unavailable' });
  assert.equal(invalid.workers[0].stops, 1);
  const missing = await setup(t, { check: value => ({ ...envelope(value), cleanup: null }) });
  const rejected = await missing.post(); assert.equal(rejected.status, 503);
  assert.deepEqual(await rejected.json(), { error: 'context_lookup_cleanup_unverified' });
  assert.equal(missing.contextLookup.busy, true);
  assert.equal((await missing.post()).status, 503);
});
