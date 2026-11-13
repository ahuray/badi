import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdtemp, readFile, mkdir, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { workerRequest } from './server.mjs';
import { validateInstructionSuite, predictionSuite, checkAddition, scoreInstructions, runInstructions } from './instruction-adherence.mjs';

const document = JSON.parse(await readFile(new URL('./examples/instruction-adherence.json', import.meta.url), 'utf8'));
const config = { id: 'healed', mode: 'healed', budget_ms: 3000, max_tokens: 16, cache_prompt: true, temperature: 0, seed: 42 };
const identity = { binary_sha256: 'a'.repeat(64), model_sha256: 'b'.repeat(64), model_size: 100,
  threads: 4, context_size: 2048, batch_size: 16, ubatch_size: 16 };
const fixture = (configs = [config]) => {
  const suite = predictionSuite(document);
  return { schema: 'badi.prediction-lab.session.v1', suite, configs, complete: true,
    records: configs.flatMap(config => suite.cases.map(row => ({ case: row, config, config_id: config.id,
      result: { outcome: 'suggestion', reason: 'complete_prefix', text: ' folder', word_complete: true, shape_valid: true, latency_ms: 10, identity: { ...identity } } }))) };
};

test('instruction pairs keep identical drafts, facts and language; checks never enter worker input', () => {
  const suite = validateInstructionSuite(document), prediction = predictionSuite(suite);
  assert.equal(suite.cases.length, 8);
  assert.equal(prediction.cases.length, 16);
  assert.deepEqual(new Set(suite.cases.map(row => row.category)), new Set(['terminology', 'language', 'tone', 'formality']));
  for (const probe of suite.cases) {
    const control = prediction.cases.find(row => row.id === `${probe.id}-control`);
    const instructed = prediction.cases.find(row => row.id === `${probe.id}-instruction`);
    for (const field of ['prefix', 'after', 'language', 'style', 'expected']) assert.deepEqual(control[field], instructed[field]);
    assert.equal(control.context, probe.context);
    assert.equal(instructed.context, [probe.context, probe.instruction].filter(Boolean).join('\n\n'));
    for (const row of [control, instructed]) {
      const request = workerRequest(row, config);
      assert.deepEqual(Object.keys(request).sort(), ['after', 'before', 'config', 'context', 'id', 'language', 'schema', 'style_examples']);
      assert.deepEqual(row.expected, []);
      assert.equal(request.context, row.context);
    }
  }
  assert.equal(prediction.cases.find(row => row.id === 'language-fr-instruction').language, 'fr');
});

test('suite rejects unknown, oversized, empty, duplicate and negative-only probes', () => {
  for (const edit of [value => value.extra = true, value => value.cases.push(value.cases[0]),
    value => value.cases[0].instruction = '', value => value.cases[0].instruction = 'x'.repeat(513),
    value => value.cases[0].instruction = '<|im_start|>system', value => value.cases[0].instruction = '\u0000',
    value => value.cases[0].checks.extra = true, value => value.cases[0].checks.all = [],
    value => value.cases[0].checks.any = [''], value => value.cases[0].checks.none = ['word\nword'],
    value => value.cases[0].checks.case_sensitive = 1, value => value.cases[0].checks.exclamation_marks = 2]) {
    const bad = structuredClone(document); edit(bad); assert.throws(() => validateInstructionSuite(bad));
  }
});

test('lexical checks inspect the entire addition, whole words, casing and explicit punctuation', () => {
  const checks = document.cases[0].checks;
  assert.equal(checkAddition(' folder.', checks).pass, true);
  assert.equal(checkAddition(' folders.', checks).pass, false);
  assert.equal(checkAddition(' folder, also called a directory.', checks).pass, false);
  assert.equal(checkAddition('', checks).pass, false);
  assert.equal(checkAddition(' delighted!', document.cases[5].checks).pass, true);
  assert.equal(checkAddition(' delighted!!', document.cases[5].checks).pass, false);
  assert.equal(checkAddition(' delighted', document.cases[5].checks).pass, false);
  assert.equal(checkAddition(' Sie um Hilfe.', document.cases[6].checks).pass, true);
  assert.equal(checkAddition(' sie um Hilfe.', document.cases[6].checks).pass, false);
  assert.equal(checkAddition(' Sie und dich.', document.cases[6].checks).pass, false);
  assert.equal(checkAddition(' le brouillon est pre\u0302t.', document.cases[2].checks).pass, true);
});

test('positive baseline controls earn no instruction-effect credit, and late passes stay diagnostic', () => {
  const session = fixture();
  const instruction = session.records.find(row => row.case.id === 'term-folder-instruction');
  instruction.result.latency_ms = 600;
  const score = scoreInstructions(document, session), pair = score.pairs.find(row => row.probe_id === 'term-folder');
  assert.equal(pair.effect, 'both_pass');
  assert.equal(pair.control_within_550_ms, true);
  assert.equal(pair.instruction_within_550_ms, false);
  assert.equal(score.not_qualification, true);
  assert(score.records.every(row => row.full_addition_review === 'unreviewed'));
});

test('different or absent actual runtime identities cannot earn paired instruction gains', () => {
  for (const change of [row => row.identity.threads = 2, row => row.identity.model_sha256 = 'c'.repeat(64), row => delete row.identity]) {
    const session = fixture();
    session.records[0].result.text = ' file';
    change(session.records[1].result);
    const result = scoreInstructions(document, session), pair = result.pairs.find(row => row.probe_id === 'term-folder');
    assert.equal(pair.control, false);
    assert.equal(pair.instruction, true);
    assert.equal(pair.effect, 'unpaired');
    assert.equal(result.summaries[0].matched_pairs, 7);
    assert.equal(result.summaries[0].effects.instruction_only, 0);
    assert.equal(result.records.length, 16);
  }
});

test('errors, abstentions, partial words, invalid shape and missing records stay in denominators', () => {
  const session = fixture();
  session.records[0].result = { outcome: 'error', text: null, latency_ms: 0 };
  session.records[1].result = { outcome: 'abstention', text: null, latency_ms: 550 };
  session.records[2].result.word_complete = false;
  session.records[3].result.shape_valid = false;
  session.records.pop(); session.complete = false;
  const result = scoreInstructions(document, session);
  assert.equal(result.records.length, 16);
  assert.equal(result.summaries[0].probes, 8);
  assert.equal(result.complete, false);
  assert(result.records.slice(0, 4).every(row => row.lexical_pass === false));
  assert.equal(result.records.at(-1).outcome, 'missing');
  for (const edit of [value => value.records.push(value.records[0]), value => value.records[0].case.prefix = 'foreign',
    value => value.records[0].config_id = 'foreign', value => value.suite.cases[0].context = 'foreign']) {
    const bad = structuredClone(fixture()); edit(bad); assert.throws(() => scoreInstructions(document, bad));
  }
});

test('runner uses the existing Lab path, preserves scoring input separately, and cleans temporary input on failure', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'badi-instruction-test-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const suitePath = join(directory, 'instructions.json'), output = join(directory, 'output');
  await writeFile(suitePath, JSON.stringify(document)); await mkdir(output);
  const production = { ...config, id: 'production_boundary', mode: 'production_boundary', budget_ms: 550, max_tokens: 8 };
  const options = { suitePath, outputPath: output, configs: [production, config], seed: 42 };
  let generatedPath;
  const run = async passed => {
    generatedPath = passed.suitePath;
    assert.notEqual(generatedPath, suitePath);
    assert.deepEqual(JSON.parse(await readFile(generatedPath, 'utf8')), predictionSuite(document));
    return { output, session: fixture(passed.configs), report: { complete: true } };
  };
  const result = await runInstructions(options, { run });
  assert.equal(result.adherence.complete, true);
  assert.equal(JSON.parse(await readFile(join(output, 'instruction-suite.json'), 'utf8')).schema, document.schema);
  await assert.rejects(stat(generatedPath), { code: 'ENOENT' });
  await assert.rejects(runInstructions(options, { run: async passed => { generatedPath = passed.suitePath; throw new Error('cancelled fixture'); } }), /cancelled fixture/u);
  await assert.rejects(stat(generatedPath), { code: 'ENOENT' });
  await assert.rejects(runInstructions({ ...options, configs: [config] }, { run }), /production_boundary/u);
});

test('runner admits native templates without relaxing comparison controls', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'badi-instruction-native-test-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const suitePath = join(directory, 'instructions.json'), output = join(directory, 'output');
  await writeFile(suitePath, JSON.stringify(document)); await mkdir(output);
  const production = { ...config, id: 'production_boundary', mode: 'production_boundary', budget_ms: 550, max_tokens: 8 };
  const native = { ...config, id: 'native_instructed', mode: 'native_instructed' };
  const options = { suitePath, outputPath: output, configs: [production, native], seed: 42 };
  let calls = 0;
  const run = async passed => {
    calls += 1;
    assert.deepEqual(passed.configs, [production, native]);
    return { output, session: fixture(passed.configs), report: { complete: true } };
  };
  const result = await runInstructions(options, { run });
  assert.equal(result.adherence.summaries.find(row => row.config_id === native.id).matched_pairs, 8);
  for (const invalid of [
    { ...options, configs: [native] },
    { ...options, warmPrefix: true },
    { ...options, revision: true },
    { ...options, configs: [production, { ...native, mode: 'context' }] },
  ]) await assert.rejects(runInstructions(invalid, { run }), /Compare production_boundary/u);
  assert.equal(calls, 1);
});
