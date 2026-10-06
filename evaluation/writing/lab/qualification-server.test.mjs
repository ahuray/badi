import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createHash } from 'node:crypto';
import { mkdtemp, rm, readdir, readFile } from 'node:fs/promises';
import { request as httpRequest } from 'node:http';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createLabServer } from './server.mjs';
import { EvidenceStore } from './qualification-evidence.mjs';
import { CONFIRMATION_REVIEW_PROTOCOL } from './qualification-server.mjs';

const hash = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
const config = { id: 'baseline', mode: 'healed', budget_ms: 550, max_tokens: 8, cache_prompt: true, temperature: 0, seed: 42 };
const cases = [
  { id: 'english', language: 'en', prefix: 'Private English draft ', context: '', style: '', expected: [' expected secret'] },
  { id: 'german', language: 'de', prefix: 'Privater deutscher Entwurf ', context: '', style: '', expected: [' Erwartung'] },
  { id: 'persian', language: 'fa', prefix: 'یک متن خصوصی ', context: '', style: '', expected: [' پاسخ'] },
];
const selection = { artifact_id: '1'.repeat(64), candidate: { candidate_id: '2'.repeat(64), sha256: 'a'.repeat(64), bytes: 100 },
  artifact: { descriptor: { sha256: 'a'.repeat(64), bytes: 100 } } };
const runtimeIdentity = { model_sha256: 'a'.repeat(64), model_size: 100, threads: 4, context_size: 2048, batch_size: 16, ubatch_size: 16, gpu_layers: 0, binary_sha256: 'b'.repeat(64) };
const recommendation = { schema: 'badi.model-recommendation.v1', status: 'no_qualified_model', candidate_id: null, ranking: [], reasons: ['Fixture intentionally never qualifies.'] };
const gate = () => { let resolve; const promise = new Promise(value => { resolve = value; }); return { promise, resolve }; };
class Worker {
  constructor() { this.calls = []; this.stops = 0; this.session = null; }
  async selectArtifact(artifact, beforeStart) { await this.stop(); this.modelArtifact = artifact; this.beforeStart = beforeStart; }
  async predict(request, signal) {
    signal?.throwIfAborted();
    if (!this.session) { await this.beforeStart?.(signal); this.session = { identity: runtimeIdentity }; }
    this.calls.push(request);
    if (this.failAt === this.calls.length) throw new Error('Owned fixture request failed');
    return { outcome: 'suggestion', text: ' useful addition', latency_ms: 100, word_complete: true, terminal_received: true, shape_valid: true,
      identity: runtimeIdentity, reused_prompt_tokens: 0,
      local_measurement: { worker_startup_ms: 500, request_roundtrip_ms: 101, peak_runtime_rss_bytes: 123456, gpu_memory_bytes: 0 } };
  }
  async stop() { this.stops++; this.session = null; return { reaped: true, challenge_completed: true }; }
}
class Qualification {
  constructor() { this.calls = []; this.device = 'c'.repeat(64); this.rankCalls = []; }
  async inspect() { return { fixture: true }; }
  async assess(candidate, signal, options = {}) {
    signal?.throwIfAborted(); this.calls.push(options);
    const generation = options.config ?? config;
    return { schema: 'badi.device-qualification.v1', candidate_id: candidate.candidate_id,
      identity: { candidate_sha256: 'd'.repeat(64), device_fingerprint: this.device, settings_sha256: hash(generation) },
      metadata: { sha256: 'a'.repeat(64), artifact_bytes: 100 },
      settings: { prompt_format: generation.mode, generation_settings_sha256: hash(generation), languages: options.languages ?? ['de', 'en', 'fa'],
        backend: 'cpu', threads: 4, context_tokens: 2048, batch_tokens: 16 },
      load_allowed: true, rejection_reasons: [], recommended: false, evidence_valid: Boolean(options.evidence),
      language_measurements: options.evidence?.languages ?? [], stages: [{ state: 'estimated_fit', passed: true, reasons: [] }], prompt_profile: 'base_continuation' };
  }
  async rank(entries, signal) { signal?.throwIfAborted(); this.rankCalls.push(entries); return recommendation; }
}
async function setup(t, options = {}) {
  const directory = await mkdtemp(join(tmpdir(), 'badi-qualification-http-'));
  const evidencePath = join(directory, 'evidence');
  const worker = new Worker(), qualification = new Qualification();
  const lab = await createLabServer({ worker, qualification, evidenceStore: new EvidenceStore({ directory: evidencePath }),
    discoveryService: { selection: async () => selection }, ...options });
  t.after(async () => { await lab.close(); await rm(directory, { recursive: true, force: true }); });
  const token = (await (await fetch(lab.origin)).text()).match(/name="badi-lab-token" content="([a-f0-9]{64})"/u)[1];
  const headers = { 'Content-Type': 'application/json', 'X-Badi-Lab-Token': token };
  const post = (path, body, extra = {}) => fetch(lab.origin + path, { method: 'POST', headers, body: JSON.stringify(body), ...extra });
  const status = async () => (await fetch(lab.origin + '/api/qualification/status', { headers })).json();
  const select = await post('/api/discovery/select', { artifact_id: selection.artifact_id });
  assert.equal(select.status, 200, await select.clone().text());
  return { ...lab, worker, qualification, directory, evidencePath, headers, post, status };
}
async function run(lab, overrides = {}) {
  const response = await lab.post('/api/run', { suite: { schema: 'badi.prediction-suite.v1', name: 'Synthetic private fixture', cases }, configs: [config], ...overrides });
  assert.equal(response.status, 200);
  const events = (await response.text()).trim().split('\n').map(JSON.parse);
  return { events, id: events[0].run_id, state: await lab.status() };
}
function reviews(state) {
  return state.run.configurations[0].review_targets.map(target => ({ ...target, judgment: target.outcome === 'suggestion' ? 'useful' : 'neutral',
    full_addition_reviewed: true, substantive: target.outcome === 'suggestion' }));
}

test('full HTTP run/review/save/load path binds owned results and never stores drafts by default', async t => {
  const lab = await setup(t); const result = await run(lab);
  assert.equal(result.events.at(-1).type, 'complete');
  assert.equal(result.events.filter(event => event.type === 'record').every(event => event.review_target.result_sha256.length === 64), true);
  assert.equal(result.state.run.finished, true);
  assert.equal(result.state.run.configurations[0].reviewed, 0);
  assert.equal(result.state.run.configurations[0].evidence, null);
  assert.equal(JSON.stringify(result.state).includes('Private English'), false);
  await assert.rejects(readdir(lab.evidencePath), { code: 'ENOENT' });
  const body = { run_id: result.id, config_id: config.id, reviews: reviews(result.state) };
  const reviewed = await lab.post('/api/qualification/review', body); assert.equal(reviewed.status, 200, await reviewed.clone().text());
  const measured = await reviewed.json();
  assert.equal(measured.evidence.loaded_and_exercised, true);
  assert.equal(measured.evidence.confirmation_untouched, false);
  assert.deepEqual(measured.evidence.languages.map(row => row.useful_on_time), [1, 1, 1]);
  assert.equal(measured.recommendation.status, 'no_qualified_model');
  await assert.rejects(readdir(lab.evidencePath), { code: 'ENOENT' });
  const saved = await (await lab.post('/api/qualification/review', { ...body, save: true })).json();
  assert.equal(saved.saved_evidence_id.length, 64);
  const bytes = await readFile(join(lab.evidencePath, `${saved.saved_evidence_id}.json`), 'utf8');
  assert.equal(bytes.includes('Private English'), false); assert.equal(bytes.includes('useful addition'), false);
  const loaded = await lab.post('/api/qualification/load', { run_id: result.id, config_id: config.id, evidence_id: saved.saved_evidence_id });
  assert.equal(loaded.status, 200, await loaded.clone().text()); assert.equal((await loaded.json()).loaded_evidence, true);
  assert.ok(lab.qualification.rankCalls.length >= 3);
  assert.ok(lab.qualification.calls.some(call => call.config && JSON.stringify(call.languages) === '["de","en","fa"]'));
  assert.equal(lab.worker.calls.some(request => JSON.stringify(request).includes('expected secret')), false);
  await lab.post('/api/reset', {}); assert.equal((await lab.status()).run, null);
  assert.equal((await lab.post('/api/qualification/review', body)).status, 409);
});

test('HTTP review rejects client measurements, wrong full-output hashes, foreign origins and missing tokens', async t => {
  const lab = await setup(t); const result = await run(lab);
  const body = { run_id: result.id, config_id: config.id, reviews: reviews(result.state) };
  assert.equal((await fetch(lab.origin + '/api/qualification/status')).status, 403);
  assert.equal((await lab.post('/api/qualification/review', body, { headers: { ...lab.headers, Origin: 'https://foreign.example' } })).status, 403);
  for (const spoof of [{ ...body, evidence: {} }, { ...body, peak_rss_bytes: 1 }, { ...body, reviews: [{ ...body.reviews[0], latency_ms: 1 }] },
    { ...body, reviews: [{ ...body.reviews[0], result_sha256: 'f'.repeat(64) }] }, { ...body, reviews: [{ ...body.reviews[0], full_addition_reviewed: false }] }]) {
    assert.equal((await lab.post('/api/qualification/review', spoof)).status, 400);
  }
  assert.equal((await lab.status()).run.configurations[0].evidence, null);
});

test('failed comparison keeps missing opportunities in review targets and usefulness denominator', async t => {
  const lab = await setup(t); lab.worker.failAt = 2;
  const result = await run(lab);
  assert.equal(result.events.at(-1).type, 'error');
  const targets = result.state.run.configurations[0].review_targets;
  assert.equal(targets.length, 3); assert.equal(targets.filter(target => target.outcome === 'missing').length, 2);
  const response = await lab.post('/api/qualification/review', { run_id: result.id, config_id: config.id, reviews: reviews(result.state) });
  assert.equal(response.status, 200, await response.clone().text());
  const receipt = (await response.json()).evidence;
  assert.equal(receipt.languages.reduce((sum, row) => sum + row.total, 0), 3);
  assert.equal(receipt.languages.reduce((sum, row) => sum + row.abstained_or_failed, 0), 2);
});

test('fresh device/runtime identity drift rejects review and saved load instead of relabeling old runs', async t => {
  const lab = await setup(t); const result = await run(lab);
  const body = { run_id: result.id, config_id: config.id, reviews: reviews(result.state), save: true };
  const saved = await (await lab.post('/api/qualification/review', body)).json();
  lab.qualification.device = 'e'.repeat(64);
  assert.equal((await lab.post('/api/qualification/review', body)).status, 409);
  assert.equal((await lab.post('/api/qualification/load', { run_id: result.id, config_id: config.id, evidence_id: saved.saved_evidence_id })).status, 409);
});

test('confirmation is frozen before inference and cannot be reused by reset or changed protocol', async t => {
  const lab = await setup(t);
  const result = await run(lab, { confirmation: { untouched: true, review_protocol: CONFIRMATION_REVIEW_PROTOCOL } });
  assert.equal(result.events.find(event => event.type === 'qualification_plan').confirmation_untouched, true);
  const measured = await (await lab.post('/api/qualification/review', { run_id: result.id, config_id: config.id, reviews: reviews(result.state) })).json();
  assert.equal(measured.evidence.confirmation_untouched, true);
  await lab.post('/api/reset', {});
  const previousCalls = lab.worker.calls.length;
  const reused = await run(lab, { confirmation: { untouched: true } });
  assert.equal(reused.events.at(-1).type, 'error'); assert.equal(lab.worker.calls.length, previousCalls);
});

test('diagnostics receive only server-held cases/configuration and measured flags remain truthful', async t => {
  const calls = [];
  const lab = await setup(t, { qualificationDiagnostics: async options => {
    calls.push(options); await options.worker.beforeStart(options.signal);
    options.onEvent({ phase: 'fixture', elapsed_seconds: 0 });
    return { schema: 'badi.qualification-diagnostics.v1', complete: true, identity: runtimeIdentity,
      evidenceMetrics: { preparation_ms: 20, sustained_seconds: 30, cancellation_recovery_passed: false, cleanup_verified: true, prompt_reuse_measured: true, paced_typing_measured: true }, records: [], limitations: [] };
  } });
  const result = await run(lab);
  assert.equal((await lab.post('/api/qualification/diagnose', { run_id: result.id, config_id: config.id, duration_seconds: 30, metrics: {} })).status, 400);
  const response = await lab.post('/api/qualification/diagnose', { run_id: result.id, config_id: config.id, duration_seconds: 30 });
  assert.equal(response.status, 200, await response.clone().text());
  const measured = await response.json();
  assert.equal(calls.length, 1); assert.equal(calls[0].durationSeconds, 30); assert.deepEqual(calls[0].config, config);
  assert.equal(calls[0].cases[0].prefix, cases[0].prefix);
  assert.equal(measured.evidence.preparation_ms, 20); assert.equal(measured.evidence.sustained_seconds, 30);
  assert.equal(measured.evidence.cancellation_recovery_passed, false); assert.equal(measured.evidence.confirmation_untouched, false);
  assert.equal(measured.evidence.languages.reduce((sum, row) => sum + row.reviewed, 0), 0);
  assert.equal(measured.recommendation.status, 'no_qualified_model');
});

test('diagnostic shared slot excludes inference/device/discovery and reset waits for owned cleanup', async t => {
  const entered = gate(), cleanup = gate(); let aborted = false;
  const lab = await setup(t, { qualificationDiagnostics: async options => {
    entered.resolve(); await new Promise(resolve => options.signal.addEventListener('abort', () => { aborted = true; resolve(); }, { once: true }));
    await cleanup.promise;
    return { complete: false, cancelled: true, identity: runtimeIdentity, evidenceMetrics: { cleanup_verified: true } };
  } });
  const result = await run(lab);
  const diagnose = lab.post('/api/qualification/diagnose', { run_id: result.id, config_id: config.id, duration_seconds: 30 });
  await entered.promise;
  assert.equal((await lab.status()).busy, true);
  assert.equal((await fetch(lab.origin + '/api/device', { headers: lab.headers })).status, 409);
  assert.equal((await lab.post('/api/run', { suite: { schema: 'badi.prediction-suite.v1', name: 'busy', cases }, configs: [config] })).status, 409);
  assert.equal((await lab.post('/api/discovery/select', { artifact_id: selection.artifact_id })).status, 409);
  let resetFinished = false;
  const reset = lab.post('/api/reset', {}).then(response => { resetFinished = true; return response; });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(resetFinished, false);
  cleanup.resolve(); assert.equal((await reset).status, 200);
  assert.equal((await diagnose).status, 409); assert.equal(aborted, true);
  assert.equal((await lab.status()).run, null); assert.equal((await lab.status()).busy, false);
});

test('model reselection erases held reviews and cannot reuse previous run IDs', async t => {
  const lab = await setup(t); const result = await run(lab);
  assert.equal((await lab.post('/api/discovery/select', { artifact_id: selection.artifact_id })).status, 200);
  assert.equal((await lab.status()).run, null);
  assert.equal((await lab.post('/api/qualification/review', { run_id: result.id, config_id: config.id, reviews: reviews(result.state) })).status, 409);
});

test('review upload started before reset cannot restore erased evidence', async t => {
  const lab = await setup(t); const result = await run(lab);
  const body = JSON.stringify({ run_id: result.id, config_id: config.id, reviews: reviews(result.state) });
  const response = gate();
  const request = httpRequest(new URL('/api/qualification/review', lab.origin), { method: 'POST', headers: { ...lab.headers, 'Content-Length': Buffer.byteLength(body) } }, reply => {
    reply.resume(); reply.on('end', () => response.resolve(reply.statusCode));
  });
  request.write(body.slice(0, 20));
  await new Promise(resolve => setTimeout(resolve, 10));
  await lab.post('/api/reset', {}); request.end(body.slice(20));
  assert.equal(await response.promise, 409);
  assert.equal((await lab.status()).run, null);
});

test('custom frozen review notes add to fixed thresholds and renamed exposed inputs cannot become untouched', async t => {
  const lab = await setup(t);
  const custom = 'Judge substantive prose, names, dates, script seams and unwanted tails in the full displayed addition.';
  const result = await run(lab, { confirmation: { untouched: true, review_protocol: custom } });
  const protocol = result.events.find(event => event.type === 'qualification_plan').review_protocol;
  assert.ok(protocol.startsWith(CONFIRMATION_REVIEW_PROTOCOL)); assert.ok(protocol.includes(custom));
  await lab.post('/api/reset', {});
  const callsBefore = lab.worker.calls.length;
  const renamed = cases.map(row => ({ ...row, id: `new-${row.id}`, expected: [' changed reference'] }));
  const repeated = await run(lab, { suite: { schema: 'badi.prediction-suite.v1', name: 'Renamed inputs', cases: renamed }, confirmation: { untouched: true } });
  assert.equal(repeated.events.at(-1).type, 'error'); assert.equal(lab.worker.calls.length, callsBefore);
});

test('incomplete diagnostics stay inspectable without granting passing metrics or cleanup', async t => {
  const lab = await setup(t, { qualificationDiagnostics: async () => ({ schema: 'badi.qualification-diagnostics.v1', complete: false,
    error: 'resource_pressure', identity: runtimeIdentity, evidenceMetrics: { sustained_seconds: 1800, cleanup_verified: false,
      cancellation_recovery_passed: true, paced_typing_measured: true }, records: [], limitations: ['Fixture failure'] }) });
  const result = await run(lab);
  const response = await lab.post('/api/qualification/diagnose', { run_id: result.id, config_id: config.id, duration_seconds: 30 });
  assert.equal(response.status, 200, await response.clone().text());
  const measured = await response.json();
  assert.equal(measured.diagnostics.complete, false);
  assert.equal(measured.evidence.sustained_seconds, 0); assert.equal(measured.evidence.cleanup_verified, false);
  assert.equal(measured.evidence.cancellation_recovery_passed, false);
});

test('worker error messages retain the verified session identity and all planned outcomes', async t => {
  const lab = await setup(t);
  const predict = lab.worker.predict.bind(lab.worker);
  lab.worker.predict = async (...args) => { const result = await predict(...args); return { type: 'error', error: 'fixture rejected', local_measurement: result.local_measurement }; };
  const result = await run(lab);
  const records = result.events.filter(event => event.type === 'record').map(event => event.record);
  assert.equal(records.length, 3);
  assert.ok(records.every(record => record.result.outcome === 'error' && record.result.identity.model_sha256 === runtimeIdentity.model_sha256));
  assert.ok(records.every(record => record.result.local_measurement.peak_runtime_rss_bytes === 123456));
});

async function positiveFixture(lab, result) {
  const original = lab.qualification.assess.bind(lab.qualification);
  lab.qualification.assess = async (...args) => ({ ...await original(...args), recommended: Boolean(args[2]?.evidence) });
  lab.qualification.rank = async (_entries, signal) => { signal.throwIfAborted(); return { ...recommendation, status: 'recommended', candidate_id: selection.candidate.candidate_id, ranking: [selection.candidate.candidate_id] }; };
  const response = await lab.post('/api/qualification/review', { run_id: result.id, config_id: config.id, reviews: reviews(result.state) });
  assert.equal(response.status, 200, await response.clone().text());
  assert.equal((await response.json()).recommendation.status, 'recommended');
}

test('GET status rechecks positive recommendations and clears stale identity claims', async t => {
  const lab = await setup(t); const result = await run(lab); await positiveFixture(lab, result);
  const previous = lab.qualification.calls.length;
  const healthy = await lab.status();
  assert.equal(healthy.recommendation.status, 'recommended'); assert.equal(healthy.busy, false);
  assert.ok(lab.qualification.calls.length > previous);
  lab.qualification.device = 'e'.repeat(64);
  const stale = await lab.status();
  assert.equal(stale.recommendation.status, 'no_qualified_model');
  assert.equal(stale.run.configurations[0].assessment.recommended, false);
  assert.equal(stale.run.configurations[0].assessment.stale, true);
  assert.equal(stale.run.configurations[0].evidence.languages.length, 3);
});

test('failed fresh POST invalidates cached recommendation and status never revives it', async t => {
  const lab = await setup(t); const result = await run(lab); await positiveFixture(lab, result);
  lab.qualification.device = 'e'.repeat(64);
  assert.equal((await lab.post('/api/qualification/review', { run_id: result.id, config_id: config.id, reviews: reviews(result.state) })).status, 409);
  const stale = await lab.status();
  assert.equal(stale.recommendation.status, 'no_qualified_model');
  assert.equal(stale.run.configurations[0].assessment.recommended, false);
});

test('unrefreshed snapshots label historical positives pending rather than currently recommended', async t => {
  const lab = await setup(t); const result = await run(lab); await positiveFixture(lab, result);
  const historical = lab.qualificationReview.snapshot();
  assert.equal(historical.recommendation.status, 'pending_recheck');
  assert.equal(historical.recommendation.candidate_id, null);
  assert.equal(historical.run.configurations[0].assessment.recommended, false);
  assert.equal(historical.run.configurations[0].assessment.requires_recheck, true);
});

for (const cleanupVerified of [false, true]) test(`cancelled diagnostics clear previous passing runtime evidence (cleanup ${cleanupVerified})`, async t => {
  const entered = gate(), interrupted = gate(), cleanup = gate(); let calls = 0;
  const measured = { schema: 'badi.qualification-diagnostics.v1', complete: true, identity: runtimeIdentity,
    evidenceMetrics: { preparation_ms: 20, sustained_seconds: 1800, cancellation_recovery_passed: true,
      cleanup_verified: true, prompt_reuse_measured: true, paced_typing_measured: true }, records: [], limitations: [] };
  const lab = await setup(t, { qualificationDiagnostics: async options => {
    if (++calls === 1) return measured;
    entered.resolve();
    await new Promise(resolve => options.signal.addEventListener('abort', resolve, { once: true }));
    interrupted.resolve(); await cleanup.promise;
    return { ...measured, complete: false, cancelled: true,
      evidenceMetrics: { ...measured.evidenceMetrics, cleanup_verified: cleanupVerified } };
  } });
  const result = await run(lab);
  const bound = { run_id: result.id, config_id: config.id };
  assert.equal((await lab.post('/api/qualification/diagnose', { ...bound, duration_seconds: 1800 })).status, 200);
  const passes = evidence => evidence?.cleanup_verified === true && evidence.sustained_seconds >= 1800
    && evidence.cancellation_recovery_passed === true;
  const originalAssess = lab.qualification.assess.bind(lab.qualification);
  lab.qualification.assess = async (...args) => ({ ...await originalAssess(...args), recommended: passes(args[2]?.evidence) });
  lab.qualification.rank = async (entries, signal) => {
    signal.throwIfAborted();
    return entries.some(({ evidence }) => passes(evidence))
      ? { ...recommendation, status: 'recommended', candidate_id: selection.candidate.candidate_id, ranking: [selection.candidate.candidate_id] }
      : recommendation;
  };
  const review = { ...bound, reviews: reviews(result.state) };
  const saved = await (await lab.post('/api/qualification/review', { ...review, save: true })).json();
  assert.equal(saved.recommendation.status, 'recommended');
  assert.equal(saved.evidence.sustained_seconds, 1800);
  assert.equal(saved.evidence.cleanup_verified, true);
  assert.equal(saved.saved_evidence_id.length, 64);

  const controller = new AbortController();
  const pending = lab.post('/api/qualification/diagnose', { ...bound, duration_seconds: 1800 }, { signal: controller.signal }).catch(error => error);
  try {
    await entered.promise;
    const running = await lab.status();
    assert.equal(running.recommendation.status, 'no_qualified_model');
    assert.equal(running.run.configurations[0].evidence, null);
    assert.equal(running.run.configurations[0].saved_evidence_id, null);
    assert.equal(running.run.configurations[0].reviewed, cases.length);
    controller.abort();
    assert.equal((await pending).name, 'AbortError');
    await interrupted.promise; cleanup.resolve();
    for (let attempt = 0; lab.qualificationReview.busy && attempt < 100; attempt++) await new Promise(resolve => setImmediate(resolve));
    assert.equal(lab.qualificationReview.busy, false);

    const after = await lab.status();
    assert.equal(after.recommendation.status, 'no_qualified_model');
    assert.equal(after.run.configurations[0].assessment.recommended, false);
    assert.equal(after.run.configurations[0].diagnostics.cancelled, true);
    assert.equal(after.run.configurations[0].diagnostics.evidenceMetrics.cleanup_verified, cleanupVerified);
    const reassessed = await (await lab.post('/api/qualification/review', review)).json();
    assert.equal(reassessed.recommendation.status, 'no_qualified_model');
    assert.equal(reassessed.evidence.sustained_seconds, 0);
    assert.equal(reassessed.evidence.cancellation_recovery_passed, false);
    assert.equal(reassessed.evidence.cleanup_verified, cleanupVerified);
    assert.equal(reassessed.evidence.languages.reduce((count, row) => count + row.reviewed, 0), cases.length);
    assert.equal((await lab.status()).recommendation.status, 'no_qualified_model');
    if (!cleanupVerified) {
      const loaded = await lab.post('/api/qualification/load', { ...bound, evidence_id: saved.saved_evidence_id });
      assert.equal(loaded.status, 409);
      assert.match((await loaded.json()).error, /unverified runtime cleanup/u);
      assert.equal((await lab.status()).recommendation.status, 'no_qualified_model');
    }
  } finally { controller.abort(); cleanup.resolve(); await pending; }
});
