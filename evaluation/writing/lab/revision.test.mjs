import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import { REVISION_LANGUAGES, validateRevisionSuite, validateRevisionCase, revisionRequest, planRevision, scoreRevision,
  summarizeRevision } from './revision.mjs';

const error = (overrides = {}) => ({ id: 'en-g', language: 'en', mode: 'grammar', kind: 'error', selected: 'She dont know.',
  expected: ["She doesn't know."], ...overrides });
const correct = (overrides = {}) => ({ id: 'en-c', language: 'en', mode: 'grammar', kind: 'correct', selected: 'She knows.', ...overrides });
const refine = (overrides = {}) => ({ id: 'en-r', language: 'en', mode: 'refine', selected: 'pls call Tom at 14:30', facts: ['Tom', '14:30'], ...overrides });
const synonym = (overrides = {}) => ({ id: 'en-s', language: 'en', mode: 'synonyms', before: 'The view is ', selected: 'beautiful',
  expected: ['lovely', 'pretty'], ...overrides });
const result = (overrides = {}) => ({ type: 'revision_result', outcome: 'shown', text: "She doesn't know.", output: "She doesn't know.",
  latency_ms: 100, generated_tokens: 6, output_language: null, field_language: null, ...overrides });
const suite = cases => ({ schema: 'badi.revision-suite.v1', name: 'Revision fixture', cases });

test('the development suite loads and covers every revision language and mode', async () => {
  const loaded = validateRevisionSuite(JSON.parse(await readFile(new URL('./examples/revision-development.json', import.meta.url), 'utf8')));
  for (const language of Object.keys(REVISION_LANGUAGES)) {
    const rows = loaded.cases.filter(row => row.language === language);
    const grammar = rows.filter(row => row.mode === 'grammar');
    assert.ok(grammar.filter(row => row.kind === 'error').length >= 12, language);
    assert.ok(grammar.filter(row => row.kind === 'correct').length >= 4, language);
    assert.ok(rows.filter(row => row.mode === 'refine').length >= 8, language);
    // Mid-sentence selections exercise the gate's case and final-mark rules.
    assert.ok(grammar.filter(row => row.before && row.after).length >= 2, language);
  }
});

test('a case is refused when its answer is missing, misplaced or unreachable', () => {
  assert.equal(validateRevisionCase(error()).kind, 'error');
  assert.deepEqual(validateRevisionCase(refine()).facts, ['Tom', '14:30']);
  assert.deepEqual(validateRevisionCase(synonym()).expected, ['lovely', 'pretty']);
  assert.equal(validateRevisionCase(correct({ selected: 'Tu viens ?' })).selected, 'Tu viens ?');
  for (const invalid of [
    error({ expected: undefined }), error({ expected: [] }), error({ expected: ['She  dont know. '] }),
    error({ facts: ['She'] }), correct({ expected: ['She knows!'] }), correct({ kind: undefined }),
    refine({ facts: ['Anna'] }), refine({ facts: [] }), refine({ kind: 'error' }), refine({ expected: ['x'] }),
    synonym({ expected: undefined }), synonym({ expected: ['beautiful'] }), synonym({ kind: 'error' }),
    synonym({ facts: ['view'] }), synonym({ selected: 'one two three four' }), synonym({ selected: 'word.' }),
    synonym({ selected: 'room 101' }),
    error({ selected: 'two\nlines' }), error({ selected: 'tab\there' }), error({ selected: 'zero​width' }),
    error({ selected: '   ' }), error({ selected: 'a'.repeat(601) }), error({ before: 'b'.repeat(513) }), error({ after: 'a'.repeat(129) }),
    error({ language: 'fa' }), error({ language: 'en-US' }), error({ mode: 'rewrite' }), error({ note: 'x' }), error({ id: '-bad' }),
  ]) assert.throws(() => validateRevisionCase(invalid));
  for (const row of [error({ before: 'I said ' }), correct(), refine(), synonym()]) {
    assert.deepEqual(validateRevisionCase(validateRevisionCase(row)), validateRevisionCase(row));
  }
  assert.throws(() => validateRevisionSuite(suite([error(), error()])));
  assert.throws(() => validateRevisionSuite({ ...suite([error()]), schema: 'badi.prediction-suite.v1' }));
});

test('the worker request carries the selection and its surroundings, never the answer', () => {
  const request = revisionRequest(error({ before: 'I said ', after: ' twice.', expected: ['PRIVATE_EXPECTED'] }));
  assert.deepEqual(Object.keys(request).sort(), ['after', 'before', 'id', 'language', 'mode', 'schema', 'selected']);
  assert.deepEqual({ ...request, id: null }, { schema: 'badi.prediction-lab.revision-request.v1', id: null, language: 'en',
    mode: 'grammar', before: 'I said ', selected: 'She dont know.', after: ' twice.' });
  assert.ok(!JSON.stringify(revisionRequest(refine({ facts: ['Tom'] }))).includes('facts'));
});

test('the plan runs each case once, in seeded blocks of mode and then language', () => {
  const cases = ['en', 'de', 'fr'].flatMap(language => [1, 2, 3].flatMap(index => [
    error({ id: `${language}-g-${index}`, language }), refine({ id: `${language}-r-${index}`, language })]));
  const plan = planRevision(cases, ['grammar', 'refine'], 7);
  assert.deepEqual(plan, planRevision(cases, ['grammar', 'refine'], 7));
  assert.deepEqual(plan.map(item => item.case_id).sort(), cases.map(row => row.id).sort());
  const blocks = plan.map(item => `${item.mode}:${item.case_id.slice(0, 2)}`).filter((key, index, keys) => key !== keys[index - 1]);
  assert.equal(blocks.length, 6);
  assert.equal(new Set(blocks).size, 6);
  assert.notDeepEqual([7, 8, 9, 10, 11].map(seed => JSON.stringify(planRevision(cases, ['grammar', 'refine'], seed))),
    Array(5).fill(JSON.stringify(plan)));
  assert.ok(planRevision(cases, ['refine'], 7).every(item => item.mode === 'refine'));
  assert.throws(() => planRevision(cases, ['grammar', 'grammar'], 7));
});

test('a correction matches after whitespace normalization only, and facts are read in what is shown', () => {
  const french = error({ language: 'fr', selected: 'Tu vien ?', expected: ['Tu viens ?'] });
  assert.equal(scoreRevision(french, result({ text: 'Tu  viens ?' })).gold_match, true);
  assert.equal(scoreRevision(error(), result({ text: "she doesn't know." })).gold_match, false);
  assert.equal(scoreRevision(error(), result({ outcome: 'edits', text: null })).gold_match, false);
  assert.equal(scoreRevision(correct(), result({ outcome: 'unchanged', text: null })).left_unchanged, true);
  assert.equal(scoreRevision(correct(), result({ text: 'She does know.' })).left_unchanged, false);
  assert.equal(scoreRevision(error(), result()).left_unchanged, null);
  assert.equal(scoreRevision(synonym(), result({ text: 'lovely' })).gold_match, true);
  assert.equal(scoreRevision(synonym(), result({ text: 'gorgeous' })).gold_match, false);
  assert.equal(scoreRevision(synonym(), result({ outcome: 'unchanged', text: null })).gold_match, false);

  const lost = scoreRevision(refine(), result({ text: 'Please call Tom at half past two.' }));
  assert.deepEqual([lost.facts_kept, lost.missing_facts], [false, ['14:30']]);
  const refused = scoreRevision(refine(), result({ outcome: 'facts', text: null, output: 'Please call Tom at 14:30, Monday.' }));
  assert.deepEqual([refused.shown, refused.facts_kept], [false, true]);
  assert.equal(scoreRevision(refine(), result({ outcome: 'timeout', text: null, output: null })).facts_kept, null);

  assert.equal(scoreRevision(error(), result({ output_language: 'deu' })).other_language, true);
  assert.equal(scoreRevision(error(), result({ output_language: 'eng' })).other_language, false);
  assert.equal(scoreRevision(error(), result({ field_language: 'de' })).field_language_differs, true);
  assert.throws(() => scoreRevision(error(), result({ outcome: 'unchanged' })));
  assert.throws(() => scoreRevision(error(), result({ outcome: 'guess' })));
});

test('the summary counts fixes, wrong fixes, unchanged controls and abstention classes per mode and language', () => {
  const records = [
    [error({ id: 'a' }), result({ latency_ms: 400 })],
    [error({ id: 'b' }), result({ text: 'She do not know.', latency_ms: 100 })],
    [error({ id: 'c' }), result({ outcome: 'edits', text: null, latency_ms: 300 })],
    [correct({ id: 'd' }), result({ outcome: 'unchanged', text: null, latency_ms: 200 })],
    [correct({ id: 'e' }), result({ text: 'She knows it.', output_language: 'deu', latency_ms: 500 })],
    [correct({ id: 'f' }), result({ outcome: 'language', text: null, latency_ms: 600 })],
    [error({ id: 'g', language: 'de', selected: 'Er gehen.', expected: ['Er geht.'] }), result({ outcome: 'error', text: null, output: null, latency_ms: 9 })],
    [refine({ id: 'h' }), result({ text: 'Please call Tom at 14:30.' })],
    [refine({ id: 'i' }), result({ text: 'Please call him at 14:30.' })],
    [refine({ id: 'j' }), result({ outcome: 'facts', text: null, output: 'Call Tom at 14:30 on Monday.' })],
    [synonym({ id: 'k' }), result({ text: 'lovely', latency_ms: 150 })],
    [synonym({ id: 'l' }), result({ text: 'gorgeous', latency_ms: 250 })],
    [synonym({ id: 'm' }), result({ outcome: 'unchanged', text: null, latency_ms: 350 })],
  ].map(([row, outcome]) => ({ case: row, mode: row.mode, result: outcome }));
  const summary = summarizeRevision(records).by_mode;
  const english = summary.grammar.by_language.en;
  assert.deepEqual(english.errors, { cases: 3, fixed: 1, shown_wrong: 1, not_shown: { edits: 1 } });
  assert.deepEqual(english.correct, { cases: 3, unchanged: 1, changed: 1, other: { language: 1 } });
  assert.equal(english.other_language, 1);
  assert.deepEqual(english.latency_ms, { count: 6, p50: 300, p90: 600, max: 600 });
  // A failed request stays in the counts but not in the model timings.
  assert.deepEqual(summary.grammar.by_language.de.errors.not_shown, { error: 1 });
  assert.equal(summary.grammar.by_language.de.latency_ms.count, 0);
  assert.equal(summary.grammar.overall.cases, 7);
  assert.deepEqual({ ...summary.refine.overall, latency_ms: null, generated_tokens: null }, { cases: 3, shown: 2, shown_rate: 2 / 3,
    not_shown: { facts: 1 }, facts_kept: 1, facts_abstentions_keeping_declared_facts: 1, other_language: 0, field_language_differs: 0,
    latency_ms: null, generated_tokens: null });
  assert.deepEqual({ ...summary.synonyms.overall, latency_ms: null, generated_tokens: null }, { cases: 3, matched: 1, shown_other: 1,
    not_shown: { unchanged: 1 }, other_language: 0, field_language_differs: 0, latency_ms: null, generated_tokens: null });
});
