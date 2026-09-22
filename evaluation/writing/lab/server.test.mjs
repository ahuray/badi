import assert from 'node:assert/strict';
import { test } from 'node:test';
import { request as httpRequest } from 'node:http';
import { createLabServer, validateConfigs, workerRequest, labOptions, renderLabPage } from './server.mjs';
import { readFile } from 'node:fs/promises';

const config = (overrides = {}) => ({ id: 'baseline', mode: 'production_baseline', budget_ms: 550, max_tokens: 8,
  cache_prompt: true, temperature: 0, seed: 42, ...overrides });
const testCase = (overrides = {}) => ({ id: 'sample', language: 'en', prefix: 'Please review the docum', expected: ['ent'], context: '', style: '', ...overrides });
const payload = (overrides = {}) => ({ suite: { schema: 'badi.prediction-suite.v1', name: 'HTTP fixture', cases: [testCase()] }, configs: [config()], seed: 42, ...overrides });
const prediction = (overrides = {}) => ({ outcome: 'suggestion', text: 'ent', latency_ms: 2, word_complete: true, ...overrides });
const deferred = () => { let resolve; let reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };

class FakeWorker {
  constructor(behavior = async () => prediction()) {
    this.behavior = behavior; this.calls = []; this.stops = 0; this.epoch = 0; this.session = null; this.pending = null;
  }
  async predict(value, signal) {
    if (!this.session) this.session = { identity: { model: 'disposable-fixture', epoch: ++this.epoch } };
    this.calls.push({ request: structuredClone(value), epoch: this.epoch });
    const gate = deferred(); this.pending = gate;
    const abort = () => gate.reject(new Error('Fixture cancelled'));
    signal?.addEventListener('abort', abort, { once: true });
    try { return await Promise.race([this.behavior(value, signal, this), gate.promise]); }
    finally { signal?.removeEventListener('abort', abort); if (this.pending === gate) this.pending = null; }
  }
  async stop() { this.stops++; this.pending?.reject(new Error('Fixture stopped')); this.session = null; }
}

async function setup(t, worker = new FakeWorker()) {
  const lab = await createLabServer({ worker });
  t.after(() => lab.close());
  const page = await fetch(lab.origin);
  assert.equal(page.status, 200);
  const token = (await page.text()).match(/name="badi-lab-token" content="([a-f0-9]{64})"/u)?.[1];
  assert.ok(token, 'page supplies the session capability');
  const headers = { 'Content-Type': 'application/json', 'X-Badi-Lab-Token': token };
  return { ...lab, worker, headers,
    post: (body, extra = {}) => fetch(`${lab.origin}/api/run`, { method: 'POST', headers, body: JSON.stringify(body), ...extra }) };
}

async function events(response) {
  assert.equal(response.status, 200, await response.clone().text());
  assert.match(response.headers.get('content-type'), /application\/x-ndjson/u);
  return (await response.text()).trim().split('\n').filter(Boolean).map(JSON.parse);
}

function wire(origin, path, { method = 'GET', headers = {}, body = '' } = {}) {
  return new Promise((resolve, reject) => {
    const req = httpRequest(new URL(path, origin), { method, headers }, response => {
      const chunks = [];
      response.on('data', chunk => chunks.push(chunk));
      response.on('end', () => resolve({ status: response.statusCode, body: Buffer.concat(chunks).toString() }));
      response.on('error', reject);
    });
    req.on('error', reject); req.end(body);
  });
}

test('real HTTP boundary rejects missing capability, wrong Host and foreign Origin before model access', async t => {
  const lab = await setup(t);
  assert.equal((await fetch(`${lab.origin}/api/status`)).status, 403);
  assert.equal((await fetch(`${lab.origin}/api/status`, { headers: { 'X-Badi-Lab-Token': 'x'.repeat(64) } })).status, 403);
  assert.equal((await wire(lab.origin, '/', { headers: { Host: 'foreign.example' } })).status, 403);
  assert.equal((await wire(lab.origin, '/', { headers: { Origin: 'https://foreign.example' } })).status, 403);
  assert.equal((await lab.post(payload(), { headers: { ...lab.headers, Origin: 'null' } })).status, 403);
  const status = await fetch(`${lab.origin}/api/status`, { headers: lab.headers });
  assert.deepEqual(await status.json(), { schema: 'badi.prediction-lab.status.v1', active: false, identity: null, selected_model: null, assessment: null });
  const page = await fetch(lab.origin);
  assert.equal(page.headers.get('cache-control'), 'no-store');
  assert.match(page.headers.get('content-security-policy'), /frame-ancestors 'none'/u);
  assert.equal(lab.worker.calls.length, 0);
});

test('opening the raw file explains server startup; only served HTML exposes enabled workbench controls', async t => {
  const raw = await readFile(new URL('./public/index.html', import.meta.url), 'utf8');
  assert.match(raw, /id="lab-ui" hidden/u);
  assert.match(raw, /id="launch-help" class=/u);
  assert.match(raw, /npm run writing:lab -- --port 36889/u);
  assert.match(raw, /href="\.\/style\.css"/u);
  assert.throws(() => renderLabPage(raw, '__LAB_CAPABILITY__'));
  const lab = await setup(t);
  const served = await (await fetch(lab.origin)).text();
  assert.match(served, /id="lab-ui">/u);
  assert.match(served, /id="launch-help" hidden/u);
  assert.ok(!served.includes('__LAB_CAPABILITY__'));
  assert.equal(lab.worker.calls.length, 0);
});

test('server launch accepts a reusable local port and exactly the supported prefill batches', () => {
  assert.deepEqual(labOptions([]), { port: 0, prefillBatch: 16, spellingManifests: {} });
  assert.deepEqual(labOptions(['--port', '36889', '--prefill-batch', '64']),
    { port: 36889, prefillBatch: 64, spellingManifests: {} });
  for (const args of [['--port', '65536'], ['--port', '-1'], ['--port', '12x'], ['--prefill-batch', '32'], ['--host', '0.0.0.0']]) {
    assert.throws(() => labOptions(args));
  }
});

test('invalid JSON, case/config values and oversized HTTP bodies never reach the model', async t => {
  const lab = await setup(t);
  assert.equal((await lab.post(payload(), { body: '{invalid' })).status, 400);
  assert.equal((await lab.post(payload(), { headers: { ...lab.headers, 'Content-Type': 'text/plain' } })).status, 400);
  for (const body of [payload({ seed: -1 }), payload({ configs: [] }), payload({ configs: [config({ temperature: .5 })] }),
    payload({ suite: { ...payload().suite, cases: [testCase({ expected: [7] })] } }),
    payload({ suite: { ...payload().suite, cases: [testCase({ secret_prompt: 'invalid field' })] } }),
    payload({ suite: { ...payload().suite, cases: Array.from({ length: 301 }, (_, i) => testCase({ id: `case-${i}` })) } })]) {
    assert.equal((await lab.post(body)).status, 400);
  }
  const large = JSON.stringify({ data: 'a'.repeat(512 * 1024) });
  const oversized = await wire(lab.origin, '/api/run', { method: 'POST', headers: { ...lab.headers, 'Content-Length': Buffer.byteLength(large) }, body: large });
  assert.ok([400, 413].includes(oversized.status));
  assert.equal(lab.worker.calls.length, 0);
});

test('single-config streamed run scores a snapshot, excludes expectations from inference, and stops its runtime', async t => {
  const lab = await setup(t);
  const input = payload({ suite: { ...payload().suite, cases: [testCase({ expected: ['UNIQUE_EXPECTATION'], context: 'Visible text', style: 'Example one\n\nExample two' })] } });
  const output = await events(await lab.post(input));
  assert.equal(output[0].type, 'plan');
  const record = output.find(event => event.type === 'record').record;
  assert.equal(record.score.raw_exact_match, false);
  assert.equal(output.at(-1).type, 'complete');
  assert.equal(lab.worker.calls.length, 1);
  const request = lab.worker.calls[0].request;
  assert.equal(JSON.stringify(request).includes('UNIQUE_EXPECTATION'), false);
  assert.equal(request.before, input.suite.cases[0].prefix);
  assert.equal(request.context, 'Visible text');
  assert.deepEqual(request.style_examples, ['Example one', 'Example two']);
  assert.equal(request.config.id, undefined);
  assert.ok(lab.worker.stops >= 1);
  assert.equal(lab.worker.session, null);
  assert.equal((await (await fetch(`${lab.origin}/api/status`, { headers: lab.headers })).json()).active, false);
});

test('comparison preserves trace order and resets cache between each configuration/group', async t => {
  const lab = await setup(t);
  const cases = [testCase({ id: 'trace0', trace_id: 'typing', step: 0, at_ms: 0 }),
    testCase({ id: 'trace1', trace_id: 'typing', step: 1, at_ms: 120, prefix: 'Please review the docume' }),
    testCase({ id: 'independent' })];
  const configs = [config(), config({ id: 'candidate', mode: 'context' })];
  const output = await events(await lab.post(payload({ suite: { ...payload().suite, cases }, configs })));
  const plan = output[0].plan;
  const records = output.filter(event => event.type === 'record');
  assert.equal(records.length, 6);
  for (let i = 0; i < plan.length; i++) {
    assert.equal(records[i].record.case.id, plan[i].case_id);
    assert.equal(records[i].record.config.id, plan[i].config_id);
    if (!i) continue;
    const sameGroup = plan[i].group_id === plan[i - 1].group_id && plan[i].config_id === plan[i - 1].config_id;
    assert.equal(lab.worker.calls[i].epoch === lab.worker.calls[i - 1].epoch, sameGroup);
  }
  assert.equal(output.at(-1).type, 'complete');
});

test('worker malformed/invalid outcomes become explicit error records and retain request denominators', async t => {
  const responses = [{ type: 'error', error: 'fixture input rejected' }, prediction({ outcome: 'invalid', text: 'unfinished fragment' }),
    prediction({ latency_ms: NaN })];
  const lab = await setup(t, new FakeWorker(async () => responses.shift()));
  const cases = Array.from({ length: 3 }, (_, i) => testCase({ id: `case-${i}` }));
  const output = await events(await lab.post(payload({ suite: { ...payload().suite, cases } })));
  const records = output.filter(event => event.type === 'record');
  assert.equal(records.length, 3);
  assert.ok(records.every(event => event.record.result.outcome === 'error' && event.record.result.text === null));
  assert.equal(output.at(-1).type, 'complete');
  assert.equal(output.at(-1).summaries[0].summary.overall.errors, 3);
  assert.equal(output.at(-1).summaries[0].summary.overall.raw_exact.requests, 3);
});

test('active inference rejects concurrent runs; reset cancels, clears ownership and permits a later run', async t => {
  const entered = deferred(); const wait = deferred();
  const lab = await setup(t, new FakeWorker(async () => { entered.resolve(); return wait.promise; }));
  const first = await lab.post(payload());
  await entered.promise;
  const status = await (await fetch(`${lab.origin}/api/status`, { headers: lab.headers })).json();
  assert.equal(status.active, true);
  assert.equal((await lab.post(payload())).status, 409);
  const reset = await fetch(`${lab.origin}/api/reset`, { method: 'POST', headers: lab.headers, body: '{}' });
  assert.equal(reset.status, 200);
  const output = await events(first);
  assert.equal(output.at(-1).type, 'cancelled');
  assert.equal(lab.worker.session, null);
  lab.worker.behavior = async () => prediction();
  assert.equal((await events(await lab.post(payload()))).at(-1).type, 'complete');
});

test('HTTP client cancellation aborts its in-flight worker and prevents a completed run claim', async t => {
  const entered = deferred(); const stopped = deferred();
  const worker = new FakeWorker(async (_value, signal) => { entered.resolve(); signal.addEventListener('abort', () => stopped.resolve(), { once: true }); return new Promise(() => {}); });
  const lab = await setup(t, worker);
  const controller = new AbortController();
  const response = await lab.post(payload(), { signal: controller.signal });
  await entered.promise;
  controller.abort();
  await assert.rejects(response.text(), /abort/iu);
  await stopped.promise;
  // A reset is an explicit synchronization boundary, avoiding timing polls.
  await fetch(`${lab.origin}/api/reset`, { method: 'POST', headers: lab.headers, body: '{}' });
  assert.equal(worker.session, null);
});

test('configuration and inference request allowlists keep arbitrary model parameters and references out', () => {
  for (const invalid of [config({ model: '/private/model.gguf' }), config({ budget_ms: 549 }), config({ id: '../bad' }),
    config({ mode: 'instructed', temperature: 2 }), config({ mode: 'instructed', max_tokens: 65 }),
    config({ mode: 'instructed', cache_prompt: 'true' })]) assert.throws(() => validateConfigs([invalid]));
  assert.throws(() => validateConfigs([config(), config()]));
  assert.equal(JSON.stringify(workerRequest(testCase({ expected: ['NEVER_FORWARD_THIS'] }), config())).includes('NEVER_FORWARD_THIS'), false);
});

test('production-boundary mode requires the production budget, nominal tokens, cache and decoding settings', () => {
  const boundary = config({ id: 'boundary', mode: 'production_boundary' });
  assert.deepEqual(validateConfigs([boundary]), [boundary]);
  for (const changed of [{ budget_ms: 551 }, { max_tokens: 9 }, { cache_prompt: false }, { temperature: .1 }, { seed: 43 }]) {
    assert.throws(() => validateConfigs([{ ...boundary, ...changed }]), /fixed production settings/u);
  }
  const modes = ['production_baseline', 'production_boundary', 'context', 'context_confidence', 'instructed', 'healed', 'instructed_healed', 'instructed_word', 'healed_attested', 'native_instructed'];
  const all = modes.map(mode => config({ id: mode, mode }));
  assert.equal(validateConfigs(all).length, 10);
  assert.throws(() => validateConfigs([...all, config({ id: 'extra' })]), /10 configurations/u);
});

test('one-word mode is explicit, preserves input and cannot supply a custom grammar', async t => {
  const lab = await setup(t);
  const html = await (await fetch(lab.origin)).text();
  const choice = [...html.matchAll(/<input\b[^>]*name="mode"[^>]*>/gu)]
    .map(match => match[0]).filter(value => value.includes('value="instructed_word"'));
  assert.equal(choice.length, 1);
  assert.doesNotMatch(choice[0], /\bchecked\b/u);
  const selected = config({ id: 'one', mode: 'instructed_word' });
  const sample = testCase({ prefix: 'We still need proofr', context: 'Check the report before sending it.',
    style: 'Keep the report brief.', expected: ['PRIVATE_ONE_WORD_REFERENCE'] });
  const result = await events(await lab.post(payload({ suite: { ...payload().suite, cases: [sample] }, configs: [selected] })));
  assert.equal(result.at(-1).type, 'complete');
  const request = lab.worker.calls[0].request;
  assert.equal(request.config.mode, 'instructed_word');
  assert.equal(request.before, sample.prefix);
  assert.equal(request.context, sample.context);
  assert.deepEqual(request.style_examples, [sample.style]);
  assert.equal(JSON.stringify(request).includes('PRIVATE_ONE_WORD_REFERENCE'), false);
  assert.throws(() => validateConfigs([{ ...selected, grammar: 'root ::= "answer"' }]));
});

test('instructed healing is unchecked in the served UI and forwards only explicit model input', async t => {
  const lab = await setup(t);
  const html = await (await fetch(lab.origin)).text();
  const choices = [...html.matchAll(/<input\b[^>]*name="mode"[^>]*>/gu)].map(match => match[0]);
  const combinedChoice = choices.filter(choice => choice.includes('value="instructed_healed"'));
  assert.equal(combinedChoice.length, 1);
  assert.doesNotMatch(combinedChoice[0], /\bchecked\b/u);
  assert.deepEqual(choices.filter(choice => /\bchecked\b/u.test(choice))
    .map(choice => choice.match(/value="([^"]+)"/u)[1]), ['production_baseline', 'healed']);
  const combined = config({ id: 'instructed_healed', mode: 'instructed_healed', budget_ms: 1500,
    max_tokens: 16, cache_prompt: false, temperature: .2 });
  const sample = testCase({ prefix: 'Please review the  ', context: 'The meeting is Friday.',
    style: 'Keep the note brief.', expected: ['INSTRUCTED_REFERENCE_NEVER_FORWARD'] });
  const output = await events(await lab.post(payload({ suite: { ...payload().suite, cases: [sample] }, configs: [combined] })));
  assert.equal(output.at(-1).type, 'complete');
  assert.equal(lab.worker.calls.length, 1);
  const request = lab.worker.calls[0].request;
  const { id, ...expectedConfig } = combined;
  assert.deepEqual(request.config, expectedConfig);
  assert.equal(request.before, sample.prefix);
  assert.equal(request.context, sample.context);
  assert.deepEqual(request.style_examples, [sample.style]);
  assert.equal(JSON.stringify(request).includes('INSTRUCTED_REFERENCE_NEVER_FORWARD'), false);
});

test('contextual word completion is unchecked and forwards only bounded explicit sources', async t => {
  const lab = await setup(t);
  const html = await (await fetch(lab.origin)).text();
  const choices = [...html.matchAll(/<input\b[^>]*name="mode"[^>]*>/gu)].map(match => match[0]);
  const attested = choices.filter(choice => choice.includes('value="healed_attested"'));
  assert.equal(attested.length, 1);
  assert.doesNotMatch(attested[0], /\bchecked\b/u);
  assert.match(html, /Complete words from context/u);
  assert.deepEqual(choices.filter(choice => /\bchecked\b/u.test(choice))
    .map(choice => choice.match(/value="([^"]+)"/u)[1]), ['production_baseline', 'healed']);
  const selected = config({ id: 'healed_attested', mode: 'healed_attested', budget_ms: 5000, max_tokens: 32 });
  const sample = testCase({ prefix: 'Die Tagesord', language: 'de', context: 'Die Tagesordnung liegt bereit.',
    style: 'Ich ergänze die Tagesordnung.', expected: ['ATTESTED_REFERENCE_NEVER_FORWARD'] });
  const output = await events(await lab.post(payload({ suite: { ...payload().suite, cases: [sample] }, configs: [selected] })));
  assert.equal(output.at(-1).type, 'complete');
  assert.equal(lab.worker.calls.length, 1);
  const request = lab.worker.calls[0].request;
  assert.equal(request.config.mode, 'healed_attested');
  assert.equal(request.before, sample.prefix);
  assert.equal(request.context, sample.context);
  assert.deepEqual(request.style_examples, [sample.style]);
  assert.equal(JSON.stringify(request).includes('ATTESTED_REFERENCE_NEVER_FORWARD'), false);
  assert.throws(() => validateConfigs([{ ...selected, expected: ['nung'] }]));
});

test('clearing an attested request prevents a late suffix from using its former context', async t => {
  const entered = deferred(); const pending = deferred();
  const lab = await setup(t, new FakeWorker(async () => { entered.resolve(); return pending.promise; }));
  const selected = config({ id: 'healed_attested', mode: 'healed_attested' });
  const sample = testCase({ language: 'de', prefix: 'Die Tagesord', context: 'Die Tagesordnung liegt bereit.', expected: ['nung'] });
  const request = payload({ suite: { ...payload().suite, cases: [sample] }, configs: [selected] });
  const first = await lab.post(request);
  await entered.promise;
  assert.equal((await fetch(`${lab.origin}/api/reset`, { method: 'POST', headers: lab.headers, body: '{}' })).status, 200);
  pending.resolve(prediction({ text: 'nung', reason: 'attested_context_word_suffix' }));
  const output = await events(first);
  assert.equal(output.at(-1).type, 'cancelled');
  assert.equal(output.some(event => event.type === 'record' && event.record.result.outcome === 'suggestion'), false);
  assert.equal(lab.worker.session, null);
  request.suite.cases[0].context = '';
  lab.worker.behavior = async value => {
    assert.equal(value.context, '');
    return prediction({ outcome: 'abstention', text: null, word_complete: false });
  };
  const next = await events(await lab.post(request));
  assert.equal(next.at(-1).type, 'complete');
  assert.equal(next.find(event => event.type === 'record').record.result.outcome, 'abstention');
});

test('HTTP boundary forwards the opt-in production mode and exact trailing-space input without expected text', async t => {
  const lab = await setup(t);
  const boundary = config({ id: 'boundary', mode: 'production_boundary' });
  const sample = testCase({ prefix: 'We will visit the old town ', expected: ['BOUNDARY_REFERENCE_NEVER_FORWARD'] });
  const output = await events(await lab.post(payload({ suite: { ...payload().suite, cases: [sample] }, configs: [boundary] })));
  assert.equal(output.at(-1).type, 'complete');
  assert.equal(lab.worker.calls.length, 1);
  const request = lab.worker.calls[0].request;
  assert.equal(request.config.mode, 'production_boundary');
  assert.equal(request.before, sample.prefix);
  assert.equal(request.config.budget_ms, 550);
  assert.equal(request.config.max_tokens, 8);
  assert.equal(JSON.stringify(request).includes('BOUNDARY_REFERENCE_NEVER_FORWARD'), false);
});

test('an upload begun before reset cannot start inference after reset completes', async t => {
  const lab = await setup(t);
  const firstChunk = deferred();
  const observe = request => {
    if (request.url === '/api/run') request.once('data', () => firstChunk.resolve());
  };
  lab.server.on('request', observe);
  const body = JSON.stringify(payload());
  let upload;
  const response = new Promise((resolve, reject) => {
    upload = httpRequest(new URL('/api/run', lab.origin), { method: 'POST',
      headers: { ...lab.headers, 'Content-Length': Buffer.byteLength(body) } }, reply => {
      const chunks = [];
      reply.on('data', chunk => chunks.push(chunk));
      reply.on('end', () => resolve({ status: reply.statusCode, body: Buffer.concat(chunks).toString() }));
      reply.on('error', reject);
    });
    upload.on('error', reject);
    upload.write(body.slice(0, 20));
  });
  t.after(() => upload.destroy());
  await firstChunk.promise;
  const reset = await fetch(`${lab.origin}/api/reset`, { method: 'POST', headers: lab.headers, body: '{}' });
  assert.equal(reset.status, 200);
  upload.end(body.slice(20));
  const rejected = await response;
  assert.equal(rejected.status, 409);
  assert.match(JSON.parse(rejected.body).error, /before the lab was cleared/u);
  assert.equal(lab.worker.calls.length, 0);
  lab.server.off('request', observe);
  assert.equal((await events(await lab.post(payload()))).at(-1).type, 'complete');
  assert.equal(lab.worker.calls.length, 1);
});
