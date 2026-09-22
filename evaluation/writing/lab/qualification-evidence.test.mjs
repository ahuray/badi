import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createHash } from 'node:crypto';
import { mkdtemp, readFile, writeFile, rm, stat, chmod, symlink, link, mkdir, readdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createEvidence, reviewTargets, registerConfirmation, validateEvidence, EvidenceStore } from './qualification-evidence.mjs';

const hash = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
const config = { id: 'baseline', mode: 'healed', budget_ms: 550, max_tokens: 8, cache_prompt: true, temperature: 0, seed: 42 };
const makeCase = (id, language = 'en') => ({ id, language, prefix: 'Private typed prose ', context: 'Private current context.', style: 'Private writing example.', expected: [' answer'] });
function input({ cases = [makeCase('one')], results, languages = ['en'] } = {}) {
  const assessment = { identity: { candidate_sha256: 'a'.repeat(64), device_fingerprint: 'b'.repeat(64), settings_sha256: 'c'.repeat(64) },
    metadata: { sha256: 'd'.repeat(64), artifact_bytes: 100 },
    settings: { languages, prompt_format: 'healed', generation_settings_sha256: hash(config) } };
  const result = { outcome: 'suggestion', text: ' helpful answer', latency_ms: 200, word_complete: true, terminal_received: true, shape_valid: true,
    replace_before: null, identity: { model_sha256: 'd'.repeat(64), model_size: 100 },
    local_measurement: { worker_startup_ms: 900.2, request_roundtrip_ms: 201.1, peak_runtime_rss_bytes: 123456, gpu_memory_bytes: 0 } };
  const records = cases.map((row, index) => ({ case: row, config: structuredClone(config), config_id: config.id,
    result: { ...structuredClone(result), ...(results?.[index] ?? {}) }, score: { canonical_exact_match: true, first_word_match: true } }));
  return { assessment, records, plannedCases: cases, configId: config.id, runId: 'test-run', reviews: [],
    runtimeMeasurements: { identity: structuredClone(assessment.identity), measured_at_unix_s: 1000,
      loaded_and_exercised: true, actual_backend: 'cpu', artifact_verified: true, cleanup_verified: true, peak_vram_bytes: 0 } };
}
function reviewAll(value, judgment = 'useful') {
  return reviewTargets(value).map(target => ({ ...target, judgment, full_addition_reviewed: true, substantive: judgment === 'useful' }));
}
function evidence() {
  const value = input(); value.reviews = reviewAll(value); return createEvidence(value);
}
async function storeFixture(t) {
  const root = await mkdtemp(join(tmpdir(), 'badi-evidence-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const directory = join(root, 'private', 'evidence');
  return { root, directory, store: new EvidenceStore({ directory }) };
}

test('reference agreement grants no automatic usefulness or human review', () => {
  const value = input();
  const result = createEvidence(value);
  assert.equal(result.languages[0].total, 1);
  assert.equal(result.languages[0].reviewed, 0);
  assert.equal(result.languages[0].useful_on_time, 0);
  assert.equal(result.languages[0].generic_or_incorrect, 1);
  assert.equal(result.confirmation_untouched, false);
  assert.equal(result.review_protocol_sha256, '');
  assert.equal(result.cold_start_ms, 901);
  assert.equal(result.preparation_ms, 0);
  assert.equal(result.sustained_seconds, 0);
  assert.equal(result.paced_typing_measured, false);
});

test('every planned outcome remains in each language denominator, including missing and errors', () => {
  const cases = [makeCase('good'), makeCase('failed'), makeCase('timeout'), makeCase('absent'), makeCase('missing'), makeCase('german', 'de'), makeCase('persian', 'fa')];
  const value = input({ cases, languages: ['en', 'de', 'fa'] });
  value.records.splice(4, 1);
  Object.assign(value.records[1].result, { outcome: 'error', text: null, latency_ms: 5 });
  Object.assign(value.records[2].result, { outcome: 'deadline', text: null, latency_ms: 553 });
  Object.assign(value.records[3].result, { outcome: 'abstention', text: null, latency_ms: 20 });
  value.reviews = reviewTargets(value).map(target => ({ ...target, judgment: target.outcome === 'suggestion' ? 'useful' : 'neutral', full_addition_reviewed: true, substantive: target.outcome === 'suggestion' }));
  const result = createEvidence(value);
  assert.deepEqual(result.languages.map(row => [row.language, row.total, row.reviewed, row.useful_on_time, row.abstained_or_failed]), [['en', 5, 5, 1, 4], ['de', 1, 1, 1, 0], ['fa', 1, 1, 1, 0]]);
  assert.equal(result.languages[0].p50_complete_word_ms, 551);
  assert.equal(result.languages[0].p95_complete_word_ms, 553);
});

test('full displayed harmful tails override matching first words and stay visible above deadline', () => {
  const value = input({ results: [{ text: ' answer, but the wrong appointment date is Friday.', latency_ms: 1800 }] });
  value.reviews = reviewAll(value, 'harmful');
  const result = createEvidence(value);
  assert.equal(result.languages[0].harmful, 1);
  assert.equal(result.languages[0].useful_on_time, 0);
  assert.equal(result.languages[0].p95_complete_word_ms, 1800);
});

test('neutral and unhelpful additions receive no credit; generic useful label requires substantive review', () => {
  for (const judgment of ['neutral', 'unhelpful']) {
    const value = input(); value.reviews = reviewAll(value, judgment);
    assert.equal(createEvidence(value).languages[0].generic_or_incorrect, 1);
  }
  const value = input(); value.reviews = reviewAll(value); value.reviews[0].substantive = false;
  assert.throws(() => createEvidence(value), /substantive/);
});

test('worker round-trip latency and complete terminal additions determine the typing target', () => {
  for (const change of [ { terminal_received: false }, { word_complete: false }, { shape_valid: false }, { replace_before: 'typed' },
    { local_measurement: { request_roundtrip_ms: 550.1 } }, { latency_ms: 8000 } ]) {
    const value = input({ results: [change] }); value.reviews = reviewAll(value);
    const result = createEvidence(value);
    assert.equal(result.languages[0].useful_on_time, 0);
    assert.ok(result.languages[0].p95_complete_word_ms > 550);
  }
  const value = input(); value.records[0].result.latency_ms = 550; value.records[0].result.local_measurement.request_roundtrip_ms = 550;
  value.reviews = reviewAll(value);
  assert.equal(createEvidence(value).languages[0].useful_on_time, 1);
});

test('review cannot be reused for another run, configuration, case, or modified displayed output', () => {
  const original = input(); original.reviews = reviewAll(original);
  for (const mutate of [value => { value.runId = 'another-run'; }, value => { value.reviews[0].config_id = 'other'; },
    value => { value.reviews[0].case_id = 'other'; }, value => { value.records[0].result.text += ' unwanted tail'; },
    value => { value.records[0].result.latency_ms++; }, value => { value.reviews[0].full_addition_reviewed = false; }]) {
    const value = structuredClone(original); mutate(value);
    assert.throws(() => createEvidence(value), /review/);
  }
});

test('rejects duplicate, unplanned and identity-mismatched records rather than shrink denominators', () => {
  for (const mutate of [value => { value.records.push(value.records[0]); },
    value => { value.records[0].case = makeCase('unplanned'); },
    value => { value.records[0].result.identity.model_sha256 = 'e'.repeat(64); },
    value => { value.records[0].run_id = 'another-run'; },
    value => { value.records[0].config.max_tokens = 64; }]) {
    const value = input(); mutate(value); assert.throws(() => createEvidence(value), /Qualification evidence/);
  }
  const noLanguage = input({ languages: ['en', 'de'] });
  assert.throws(() => createEvidence(noLanguage), /every assessed language/);
});

test('malformed and omitted runtime measurements cannot grant loaded/performance claims', () => {
  const value = input(); value.runtimeMeasurements = {};
  const result = createEvidence(value);
  assert.equal(result.loaded_and_exercised, false);
  assert.equal(result.cleanup_verified, false);
  assert.equal(result.artifact_verified, false);
  assert.equal(result.actual_backend, 'unknown');
  assert.equal(result.measured_at_unix_s, 0);
  value.runtimeMeasurements = { ...input().runtimeMeasurements, peak_rss_bytes: -1 };
  assert.throws(() => createEvidence(value), /integer/);
  value.runtimeMeasurements = { ...input().runtimeMeasurements, identity: { ...value.assessment.identity, settings_sha256: 'f'.repeat(64) } };
  assert.throws(() => createEvidence(value), /stale identity/);
  value.runtimeMeasurements = { ...input().runtimeMeasurements, notes: 'typed prose' };
  assert.throws(() => createEvidence(value), /unknown runtime/);
});

test('reviews cannot smuggle raw measurements or invent useful missing outcomes', () => {
  const value = input(); value.reviews = reviewAll(value); value.reviews[0].peak_rss_bytes = 1;
  assert.throws(() => createEvidence(value), /measurements/);
  value.records = []; value.reviews = reviewAll(value);
  assert.throws(() => createEvidence(value), /absent additions/);
  value.reviews = reviewAll(value, 'neutral');
  assert.equal(createEvidence(value).languages[0].reviewed, 1);
});

test('confirmation requires an opaque pre-run registration bound to exact set, run, settings and protocol', () => {
  const value = input();
  value.confirmation = registerConfirmation({ ...value, reviewProtocol: 'Review the full displayed addition; at least 60 percent substantive useful within 550ms, zero harmful per language, minimum 40 cases.', registeredAtUnixS: 900 });
  value.runStartedAtUnixS = 901;
  assert.equal(createEvidence(value).confirmation_untouched, true);
  assert.equal(createEvidence(value).review_protocol_sha256.length, 64);
  const token = value.confirmation;
  value.confirmation = structuredClone(token);
  assert.throws(() => createEvidence(value), /confirmation registration/);
  value.confirmation = token; value.runStartedAtUnixS = 899;
  assert.throws(() => createEvidence(value), /confirmation registration/);
  value.runStartedAtUnixS = 901; value.plannedCases[0].expected.push(' alternate');
  assert.throws(() => createEvidence(value), /confirmation registration/);
});

test('strict evidence validator rejects prose, malformed counts, unreviewed useful credit and hash type coercion', () => {
  for (const mutate of [value => { value.notes = 'typed prose'; }, value => { value.identity.candidate_sha256 = ['a'.repeat(64)]; },
    value => { value.languages[0].total = 2; }, value => { value.languages[0].reviewed = 0; },
    value => { value.languages[0].p95_complete_word_ms = 0; }, value => { value.actual_backend = 'private backend prose'; }]) {
    const value = evidence(); mutate(value); assert.throws(() => validateEvidence(value), /Qualification evidence/);
  }
});

test('explicit save is private, bounded and content-free; unchanged saves reuse their ID', async t => {
  const { directory, store } = await storeFixture(t);
  await assert.rejects(stat(directory), { code: 'ENOENT' });
  const value = evidence(); const id = await store.save(value);
  assert.equal(id.length, 64);
  assert.equal(await store.save(value), id);
  assert.equal((await stat(directory)).mode & 0o777, 0o700);
  const path = join(directory, `${id}.json`);
  assert.equal((await stat(path)).mode & 0o777, 0o600);
  assert.equal((await stat(path)).nlink, 1);
  const saved = await readFile(path, 'utf8');
  for (const prose of ['Private', 'helpful answer', 'test-run', 'baseline']) assert.equal(saved.includes(prose), false);
  assert.deepEqual(await store.load(id, { assessment: input().assessment, nowUnixS: 1001 }), value);
  assert.deepEqual(await readdir(directory), [`${id}.json`]);
});

test('saved evidence is invalidated by changed identity, language set, future time and age', async t => {
  const { store } = await storeFixture(t); const id = await store.save(evidence());
  const assessment = input().assessment;
  await assert.rejects(store.load(id, { assessment: { ...assessment, identity: { ...assessment.identity, device_fingerprint: 'f'.repeat(64) } }, nowUnixS: 1001 }), /stale/);
  await assert.rejects(store.load(id, { assessment: { ...assessment, settings: { ...assessment.settings, languages: ['en', 'de'] } }, nowUnixS: 1001 }), /language coverage/);
  await assert.rejects(store.load(id, { assessment, nowUnixS: 999 }), /future-dated/);
  await assert.rejects(store.load(id, { assessment, nowUnixS: 1001 + 30 * 24 * 3600 }), /older than/);
  await assert.rejects(store.load(id), /freshly assessed/);
});

test('save rejects exposed directory permissions and symlinked directory ancestors', async t => {
  const { root, directory, store } = await storeFixture(t);
  await mkdir(directory, { recursive: true, mode: 0o700 }); await chmod(directory, 0o755);
  await assert.rejects(store.save(evidence()), /private permissions/);
  await rm(join(root, 'private'), { recursive: true });
  const outside = join(root, 'outside'); await mkdir(outside, { mode: 0o700 });
  await symlink(outside, join(root, 'private'));
  await assert.rejects(store.save(evidence()));
  assert.deepEqual(await readdir(outside), []);
});

test('load rejects symlinks, hard links, exposed permissions and oversized files', async t => {
  const { root, directory, store } = await storeFixture(t); const id = await store.save(evidence());
  const path = join(directory, `${id}.json`), copy = join(root, 'copy.json');
  const bytes = await readFile(path);
  await writeFile(copy, bytes, { mode: 0o600 }); await rm(path); await symlink(copy, path);
  await assert.rejects(store.load(id, { assessment: input().assessment, nowUnixS: 1001 }));
  await rm(path); await link(copy, path);
  await assert.rejects(store.load(id, { assessment: input().assessment, nowUnixS: 1001 }), /singly linked/);
  await rm(path); await writeFile(path, bytes, { mode: 0o644 });
  await assert.rejects(store.load(id, { assessment: input().assessment, nowUnixS: 1001 }), /private/);
  await chmod(path, 0o600); await writeFile(path, Buffer.alloc(64 * 1024 + 1));
  await assert.rejects(store.load(id, { assessment: input().assessment, nowUnixS: 1001 }), /bounded/);
});

test('load rejects truncated, duplicated or tampered evidence instead of importing claims', async t => {
  const { directory, store } = await storeFixture(t); const id = await store.save(evidence());
  const path = join(directory, `${id}.json`); const original = await readFile(path, 'utf8');
  for (const bytes of ['{', original.replace('"actual_backend":"cpu"', '"actual_backend":"unknown","actual_backend":"cpu"'),
    original.replace('"useful_on_time":1', '"useful_on_time":0'), original + '\n']) {
    await writeFile(path, bytes);
    await assert.rejects(store.load(id, { assessment: input().assessment, nowUnixS: 1001 }), /Qualification evidence/);
  }
});

test('store is bounded and preserves existing evidence without automatic eviction', async t => {
  const { directory, store } = await storeFixture(t);
  const original = evidence(); const id = await store.save(original);
  for (let index = 0; index < 255; index++) await writeFile(join(directory, `preserved-${index}`), '', { mode: 0o600 });
  await assert.rejects(store.save({ ...original, measured_at_unix_s: 1001 }), /store is full/);
  assert.equal(await store.save(original), id);
  assert.equal((await readdir(directory)).length, 256);
});

test('loaded state requires terminal non-error exercise and exact observed runtime resources', () => {
  for (const result of [{ terminal_received: false }, { outcome: 'error', text: null }]) {
    assert.equal(createEvidence(input({ results: [result] })).loaded_and_exercised, false);
  }
  const value = input();
  Object.assign(value.assessment.settings, { backend: 'cpu', threads: 4, context_tokens: 2048, batch_tokens: 16 });
  Object.assign(value.records[0].result.identity, { gpu_layers: 0, threads: 4, context_size: 2048, batch_size: 16, ubatch_size: 16 });
  assert.equal(createEvidence(value).loaded_and_exercised, true);
  for (const [key, changed] of [['gpu_layers', 10], ['threads', 8], ['context_size', 8192], ['batch_size', 64], ['ubatch_size', 64]]) {
    const stale = structuredClone(value); stale.records[0].result.identity[key] = changed;
    assert.throws(() => createEvidence(stale), /runtime configuration/);
  }
});

test('all missing assignments still produce rejected, reviewable evidence without imaginary measurements', () => {
  const value = input(); value.records = []; value.runtimeMeasurements = {};
  const result = createEvidence(value);
  assert.equal(result.languages[0].total, 1);
  assert.equal(result.languages[0].abstained_or_failed, 1);
  assert.equal(result.languages[0].reviewed, 0);
  assert.equal(result.languages[0].p50_complete_word_ms, 551);
  assert.equal(result.cold_start_ms, 0);
  assert.equal(result.peak_rss_bytes, 0);
  assert.equal(result.loaded_and_exercised, false);
  assert.equal(reviewTargets(value)[0].outcome, 'missing');
});
