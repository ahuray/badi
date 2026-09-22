import assert from 'node:assert/strict';
import test from 'node:test';
import { request as httpRequest } from 'node:http';
import { createLabServer } from './server.mjs';
import { spellingWorkerRequest } from './spelling-server.mjs';
import { expectedSpellingIdentity, spellingIdentityHash } from './spelling-worker.mjs';

const deferred = () => {
  let resolve; let reject;
  const promise = new Promise((a, b) => { resolve = a; reject = b; });
  return { promise, resolve, reject };
};
const artifact = path => ({ path, sha256: 'a'.repeat(64), bytes: 1 });
const manifest = { path: '/fixture/manifest.json', sha256: 'b'.repeat(64), descriptor: {
  schema: 'badi.spelling-artifact.v1', binary: artifact('/fixture/engine'), library: artifact('/fixture/library'),
  dictionaries: Object.fromEntries(['de', 'fa'].map(language => [language, { id: `fixture-${language}`,
    aff: artifact(`/fixture/${language}.aff`), dic: artifact(`/fixture/${language}.dic`) }])) } };
const input = (overrides = {}) => ({ before: 'Wir treffen uns in Berliin ', language: 'de', protected_words: [], ...overrides });

class FakeSpellingWorker {
  constructor(options, behavior) {
    this.options = options; this.behavior = behavior; this.ready = null; this.calls = []; this.stops = 0;
  }
  async start() {
    if (!this.ready) {
      const identity = expectedSpellingIdentity(this.options.manifest, this.options.language);
      this.ready = { type: 'ready', schema: 'badi.spelling-lab.worker.v1', engine_pid: 12345,
        identity, identity_sha256: spellingIdentityHash(identity) };
    }
    return this.ready.identity;
  }
  result(request) {
    const suggestion = request.language.startsWith('de') && !request.protected_words.includes('Berliin');
    return { type: 'result', schema: 'badi.spelling-lab.result.v1', id: request.id,
      outcome: suggestion ? 'suggestion' : 'abstention', reason: suggestion ? 'single_admissible_returned_candidate' : 'no_target',
      text: suggestion ? 'Berlin ' : null, replace_before: suggestion ? 'Berliin ' : null,
      filtered_candidate_count: suggestion ? 1 : 0, query_count: suggestion ? 2 : 0, query_ms: 1, latency_ms: 2,
      identity: this.ready.identity, identity_sha256: this.ready.identity_sha256,
      warnings: ['returned_candidates_not_exhaustive', 'unknown_proper_names_not_detected'] };
  }
  async check(request, signal) {
    this.calls.push(structuredClone(request));
    const result = this.result(request);
    return this.behavior?.check ? this.behavior.check(request, signal, this, result) : result;
  }
  async stop() {
    this.stops++;
    await this.behavior?.stop?.(this);
    if (this.ready) this.lastCleanup = { process_id: this.ready.engine_pid,
      identity_sha256: this.ready.identity_sha256, reaped: true, exit_code: 0, forced: false };
    this.ready = null;
    return this.lastCleanup ?? null;
  }
}

class FakePredictionWorker {
  constructor() { this.calls = []; this.session = null; }
  async predict(request, signal) {
    this.calls.push(request);
    const gate = deferred(); this.pending = gate; this.entered?.resolve();
    const abort = () => gate.reject(new Error('fixture_cancelled'));
    signal.addEventListener('abort', abort, { once: true });
    try { return await gate.promise; }
    finally { signal.removeEventListener('abort', abort); }
  }
  async stop() { this.pending?.reject(new Error('fixture_stopped')); }
}

async function setup(t, { configured = ['de', 'fa'], behavior = {}, prediction = new FakePredictionWorker() } = {}) {
  const workers = []; const loads = [];
  const lab = await createLabServer({ worker: prediction,
    spellingManifests: Object.fromEntries(configured.map(language => [language, `/fixture/${language}.json`])),
    spellingManifestLoader: async path => { loads.push(path); return structuredClone(manifest); },
    spellingWorkerFactory: options => { const worker = new FakeSpellingWorker(options, behavior); workers.push(worker); return worker; } });
  t.after(async () => { try { await lab.close(); } catch (error) {
    assert.equal(error.message, 'spelling_cleanup_unverified');
  } });
  const page = await fetch(lab.origin);
  const token = (await page.text()).match(/name="badi-lab-token" content="([a-f0-9]{64})"/u)?.[1];
  assert.ok(token);
  const headers = { 'Content-Type': 'application/json', 'X-Badi-Lab-Token': token };
  return { ...lab, workers, loads, prediction, headers,
    post: (body = input(), path = '/api/spelling', extra = {}) => fetch(lab.origin + path,
      { method: 'POST', headers, body: JSON.stringify(body), ...extra }),
    status: () => fetch(lab.origin + '/api/spelling/status', { headers }) };
}

function wire(origin, path, headers, body = '') {
  return new Promise((resolve, reject) => {
    const request = httpRequest(new URL(path, origin), { method: body ? 'POST' : 'GET', headers }, response => {
      const chunks = []; response.on('data', chunk => chunks.push(chunk));
      response.on('end', () => resolve({ status: response.statusCode, body: Buffer.concat(chunks).toString() }));
      response.on('error', reject);
    });
    request.on('error', reject); request.end(body);
  });
}

async function partialUpload(lab, path, body) {
  const entered = deferred();
  lab.server.once('request', request => request.once('data', () => entered.resolve()));
  const response = deferred();
  const request = httpRequest(new URL(path, lab.origin), { method: 'POST', headers: lab.headers }, reply => {
    const chunks = []; reply.on('data', chunk => chunks.push(chunk));
    reply.on('end', () => response.resolve({ status: reply.statusCode, body: Buffer.concat(chunks).toString() }));
  });
  request.on('error', response.reject);
  const text = JSON.stringify(body); request.write(text.slice(0, 12));
  await entered.promise;
  return { finish: () => request.end(text.slice(12)), response: response.promise };
}

const predictionBody = { suite: { schema: 'badi.prediction-suite.v1', name: 'fixture', cases: [
  { id: 'sample', language: 'en', prefix: 'Please review the docum', expected: ['ent'], context: '', style: '' }] },
configs: [{ id: 'baseline', mode: 'production_baseline', budget_ms: 550, max_tokens: 8, cache_prompt: true,
  temperature: 0, seed: 42 }], seed: 42 };

test('native status is capability-bound, exposes only local configured languages and never starts workers', async t => {
  const lab = await setup(t);
  assert.equal((await fetch(lab.origin + '/api/spelling/status')).status, 403);
  for (const extra of [{ Host: 'foreign.example' }, { Origin: 'https://foreign.example' },
    { 'X-Badi-Lab-Token': 'x'.repeat(64) }]) {
    assert.equal((await wire(lab.origin, '/api/spelling/status', { ...lab.headers, ...extra })).status, 403);
  }
  const response = await lab.status();
  assert.deepEqual(await response.json(), { schema: 'badi.spelling-lab.status.v1', configured_languages: ['de', 'fa'], busy: false, ready: null });
  assert.equal(response.headers.get('cache-control'), 'no-store');
  assert.match(response.headers.get('content-security-policy'), /frame-ancestors 'none'/u);
  assert.equal(lab.workers.length, 0); assert.equal(lab.loads.length, 2);
});

test('missing configuration and malformed browser bodies never start native workers or forward references', async t => {
  const lab = await setup(t, { configured: ['de'] });
  assert.equal((await lab.post(input({ language: 'fa' }))).status, 503);
  for (const value of [{ ...input(), expected: 'NEVER_FORWARD' }, { ...input(), manifest: '/private/file' },
    { ...input(), mode: 'anything' }, input({ protected_words: ['x'.repeat(25)] }), input({ before: 'x'.repeat(2049) }),
    input({ protected_words: ['two words'] }), input({ language: 'en' })]) {
    const response = await lab.post(value);
    assert.equal(response.status, 400); assert.deepEqual(await response.json(), { error: 'spelling_invalid_input' });
  }
  assert.equal((await lab.post({}, '/api/spelling', { body: '{bad' })).status, 400);
  assert.equal(lab.workers.length, 0);
  assert.throws(() => spellingWorkerRequest({ ...input(), expected: 'NEVER_FORWARD' }));
  assert.deepEqual(Object.keys(spellingWorkerRequest(input())).sort(), ['before', 'id', 'language', 'protected_words', 'schema']);
});

test('sequential checks share selected language worker and return distinct native identity with exact seam', async t => {
  const lab = await setup(t);
  const first = await lab.post(); assert.equal(first.status, 200);
  const envelope = await first.json();
  assert.equal(envelope.schema, 'badi.spelling-lab.response.v1');
  assert.equal(envelope.result.text, 'Berlin '); assert.equal(envelope.result.replace_before, 'Berliin ');
  assert.equal(envelope.ready.identity_sha256, envelope.result.identity_sha256);
  assert.ok(Number.isFinite(envelope.elapsed_ms)); assert.equal(envelope.cleanup, null);
  assert.equal((await lab.post(input({ language: 'de-DE', protected_words: ['Berliin'] }))).status, 200);
  assert.equal(lab.workers.length, 1); assert.equal(lab.workers[0].calls.length, 2);
  assert.equal(lab.workers[0].calls[1].before, input().before);
  assert.deepEqual(lab.workers[0].calls[1].protected_words, ['Berliin']);
  assert.equal(lab.prediction.calls.length, 0);
  assert.equal((await lab.post(input({ before: 'متن ', language: 'fa' }))).status, 200);
  assert.equal(lab.workers.length, 2); assert.equal(lab.workers[0].stops, 1);
  assert.equal((await lab.post({}, '/api/spelling/reset')).status, 200);
  assert.equal(lab.workers[1].ready, null);
});

test('one active check rejects another; reset waits cleanup and discards a late successful result', async t => {
  const entered = deferred(); const result = deferred(); const stopEntered = deferred(); const cleanup = deferred();
  const lab = await setup(t, { behavior: {
    check: (_request, _signal, _worker, value) => { entered.resolve(value); return result.promise; },
    stop: () => { stopEntered.resolve(); return cleanup.promise; },
  } });
  const first = lab.post(); const value = await entered.promise;
  assert.equal((await lab.post()).status, 409);
  const reset = lab.post({}, '/api/spelling/reset'); await stopEntered.promise;
  assert.equal(lab.spelling.busy, true);
  assert.equal((await lab.post()).status, 409);
  let resetFinished = false; reset.then(() => { resetFinished = true; });
  await new Promise(resolve => setImmediate(resolve)); assert.equal(resetFinished, false);
  result.resolve(value); cleanup.resolve();
  assert.equal((await reset).status, 200);
  const cancelled = await first; assert.equal(cancelled.status, 409);
  assert.deepEqual(await cancelled.json(), { error: 'spelling_cancelled' });
  assert.equal(lab.workers[0].calls.length, 1); assert.equal(lab.workers[0].ready, null);
  assert.equal(lab.spelling.busy, false);
});

test('reset generation fences uploads which began before clearing', async t => {
  const lab = await setup(t);
  const upload = await partialUpload(lab, '/api/spelling', input());
  assert.equal((await lab.post({}, '/api/spelling/reset')).status, 200);
  upload.finish();
  const response = await upload.response;
  assert.equal(response.status, 409); assert.equal(JSON.parse(response.body).error, 'spelling_cancelled');
  assert.equal(lab.workers.length, 0);
});

test('prediction and native requests share capacity before and after body acquisition', async t => {
  const entered = deferred(); const pending = deferred();
  const lab = await setup(t, { behavior: { check: () => { entered.resolve(); return pending.promise; } } });
  const modelUpload = await partialUpload(lab, '/api/run', predictionBody);
  const spelling = lab.post(); await entered.promise;
  modelUpload.finish(); assert.equal((await modelUpload.response).status, 409);
  assert.equal((await lab.post(predictionBody, '/api/run')).status, 409);
  await lab.post({}, '/api/spelling/reset'); assert.equal((await spelling).status, 409);
  const nativeUpload = await partialUpload(lab, '/api/spelling', input());
  lab.prediction.entered = deferred();
  const model = await lab.post(predictionBody, '/api/run'); await lab.prediction.entered.promise;
  nativeUpload.finish(); assert.equal((await nativeUpload.response).status, 409);
  assert.equal((await lab.post()).status, 409);
  await lab.post({}, '/api/reset'); await model.text();
  assert.equal(lab.workers.length, 1); assert.equal(lab.prediction.calls.length, 1);
});

test('HTTP disconnect aborts the request and reset synchronizes with worker cleanup', async t => {
  const entered = deferred(); const stopped = deferred();
  const lab = await setup(t, { behavior: { check: () => { entered.resolve(); return new Promise(() => {}); },
    stop: () => stopped.resolve() } });
  const controller = new AbortController();
  const response = lab.post(input(), '/api/spelling', { signal: controller.signal });
  await entered.promise; controller.abort(); await assert.rejects(response, /abort/iu);
  await stopped.promise;
  assert.equal((await lab.post({}, '/api/spelling/reset')).status, 200);
  assert.equal(lab.workers[0].ready, null); assert.equal(lab.spelling.busy, false);
});

test('invalid output retires its worker and errors never expose content or paths', async t => {
  const lab = await setup(t, { behavior: { check: (_request, _signal, _worker, result) =>
    ({ ...result, replace_before: 'other ' }) } });
  const response = await lab.post();
  assert.equal(response.status, 503); assert.deepEqual(await response.json(), { error: 'spelling_engine_unavailable' });
  assert.equal(lab.workers[0].stops, 1); assert.equal(lab.workers[0].ready, null);
});

test('cleanup failure is explicit, retains unavailable capacity and blocks reuse after reset', async t => {
  const lab = await setup(t, { behavior: { stop: () => { throw new Error('/private/path and typed prose'); } } });
  assert.equal((await lab.post()).status, 200);
  const reset = await lab.post({}, '/api/spelling/reset');
  assert.equal(reset.status, 503); assert.deepEqual(await reset.json(), { error: 'spelling_cleanup_unverified' });
  assert.equal(lab.spelling.busy, true);
  const later = await lab.post(); assert.equal(later.status, 503);
  assert.deepEqual(await later.json(), { error: 'spelling_cleanup_unverified' });
  assert.equal(lab.workers.length, 1);
  assert.equal((await lab.post(predictionBody, '/api/run')).status, 409);
});
