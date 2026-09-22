import assert from 'node:assert/strict';
import { test } from 'node:test';
import { validateCase, validateSuite, modelInputForCase, scorePrediction, scoreCompleteWordPrefixes, summarizeResults, planComparison } from './cases.mjs';

const row = (overrides = {}) => ({ id: 'case-1', language: 'en', prefix: 'Please review the docum', expected: ['ent'], ...overrides });
const result = (overrides = {}) => ({ outcome: 'suggestion', text: 'ent', latency_ms: 100, word_complete: true, ...overrides });
const suite = cases => ({ schema: 'badi.prediction-suite.v1', name: 'Explicit development cases', cases });

test('multiword reference agreement binds complete affected words across a partial-word seam', () => {
  const sample = row({ expected: ['entation before our meeting', 'ent before lunch'] });
  assert.deepEqual(scoreCompleteWordPrefixes(sample, result({ text: 'entation before our meeting' })), {
    2: { scorable: true, match: true }, 3: { scorable: true, match: true }, 4: { scorable: true, match: true },
  });
  const short = scoreCompleteWordPrefixes(sample, result({ text: 'ent before lunch' }));
  assert.equal(short[2].match, true);
  assert.equal(short[3].match, true);
  assert.equal(short[4].match, false);
  assert.equal(scoreCompleteWordPrefixes(sample, result({ text: 'entation before your meeting' }))[3].match, false);
  assert.equal(scoreCompleteWordPrefixes(sample, result({ text: ' entation before our meeting' }))[2].match, false);
});

test('multiword agreement preserves unfinished-token, abstention and unscored-reference boundaries', () => {
  const sample = row({ prefix: 'Meet at the ', expected: ['garden entrance tomorrow morning'] });
  const fragment = scoreCompleteWordPrefixes(sample, result({ text: 'garden entrance tomor', word_complete: false }));
  assert.equal(fragment[2].match, true);
  assert.equal(fragment[3].match, false);
  const uncertified = scoreCompleteWordPrefixes(sample, result({ text: 'garden entrance', word_complete: false }));
  assert.equal(uncertified[2].match, false);
  assert.equal(scoreCompleteWordPrefixes(sample, result({ text: 'garden entrance ', word_complete: false }))[2].match, true);
  for (const outcome of ['abstention', 'deadline', 'error']) {
    assert.deepEqual(scoreCompleteWordPrefixes(sample, result({ outcome, text: null }))[2], { scorable: true, match: false });
  }
  for (const value of [row({ expected: [] }), row({ expected: ['ent'] })]) {
    assert.deepEqual(scoreCompleteWordPrefixes(value, result())[2], { scorable: false, match: null });
  }
  assert.deepEqual(scoreCompleteWordPrefixes(sample, result({ text: 'garden entrance', replace_before: 'gardne' }))[2], { scorable: false, match: null });
});

test('multiword agreement is case-sensitive NFC lexical agreement, including Persian joiners', () => {
  const sample = row({ prefix: 'The caf', expected: ['é opens tomorrow morning'] });
  assert.equal(scoreCompleteWordPrefixes(sample, result({ text: 'e\u0301 opens tomorrow morning' }))[4].match, true);
  assert.equal(scoreCompleteWordPrefixes(sample, result({ text: 'é Opens tomorrow morning' }))[2].match, false);
  assert.equal(scoreCompleteWordPrefixes(sample, result({ text: 'é, opens tomorrow morning' }))[4].match, true);
  const persian = row({ language: 'fa', prefix: 'او می‌توا', expected: ['ند فردا کتاب بیاورد'] });
  assert.equal(scoreCompleteWordPrefixes(persian, result({ text: 'ند فردا کتاب بیاورد' }))[4].match, true);
  assert.equal(scoreCompleteWordPrefixes(persian, result({ text: 'ند امروز کتاب بیاورد' }))[2].match, false);
});

test('case boundary retains exact whitespace, languages and explicit context/style, without mutating input', () => {
  const raw = row({ prefix: '  لطفاً توض', language: 'fa', context: 'او می‌تواند بنویسد.', style: 'مختصر بنویس.\n\nرسمی بنویس.', expected: ['یح بده.'] });
  const value = validateCase(raw);
  assert.equal(value.prefix, raw.prefix);
  assert.equal(value.context, raw.context);
  assert.equal(value.style, raw.style);
  assert.equal(value.trace_id, null);
  value.expected.push('یح بیشتر');
  assert.equal(raw.expected.length, 1);
  assert.deepEqual(validateCase(row({ expected: [] })).expected, []);
  for (const invalid of [row({ language: 'not a locale' }), row({ prefix: '\ud800' }), row({ prefix: 'a\0b' }),
    row({ context: 'a\u0001b' }), row({ expected: [''] }), row({ expected: ['ent', 'ent'] }),
    row({ prefix: 'a'.repeat(2049) }), row({ context: 'a'.repeat(4097) }), row({ style: 'a'.repeat(1025) }),
    row({ style: Array(9).fill('Example').join('\n\n') }), row({ arbitrary_prompt: 'answer leaked' })]) {
    assert.throws(() => validateCase(invalid));
  }
});

test('model input excludes all expected answers and trace metadata using an explicit allowlist', () => {
  const input = modelInputForCase(row({ expected: ['UNIQUE_REFERENCE_NOT_FOR_MODEL'], context: 'visible context', style: 'style text',
    trace_id: 'typing', step: 0, at_ms: 0 }));
  assert.deepEqual(input, { language: 'en', prefix: 'Please review the docum', context: 'visible context', style: 'style text' });
  assert.equal(JSON.stringify(input).includes('UNIQUE_REFERENCE_NOT_FOR_MODEL'), false);
  assert.equal(Object.hasOwn(input, 'expected'), false);
});

test('reference agreement completes the word across the prefix/suffix seam and recognizes alternatives', () => {
  const sample = row({ expected: ['ent now', 'entation later'] });
  const matched = scorePrediction(sample, result({ text: 'entation later' }));
  assert.equal(matched.predicted_first_word, 'documentation');
  assert.equal(matched.first_word_match, true);
  assert.equal(matched.raw_exact_match, true);
  const differentPhrase = scorePrediction(sample, result({ text: 'ent tomorrow' }));
  assert.equal(differentPhrase.first_word_match, true);
  assert.equal(differentPhrase.canonical_exact_match, false);
  assert.equal(scorePrediction(sample, result({ text: ' ent now' })).first_word_match, false);
  assert.equal(scorePrediction(sample, result({ text: 'entertainment' })).first_word_match, false);
});

test('raw token fragments cannot certify a completed first word from reference agreement alone', () => {
  const raw = scorePrediction(row(), result({ word_complete: false }));
  assert.equal(raw.raw_exact_match, true);
  assert.equal(raw.first_word_complete, false);
  assert.equal(raw.first_word_match, false);
  assert.equal(scorePrediction(row(), result({ word_complete: undefined })).first_word_match, false);
  assert.equal(scorePrediction(row(), result({ text: 'ent ', word_complete: false })).first_word_match, true);
  assert.equal(scorePrediction(row(), result({ text: 'ent.', word_complete: false })).first_word_match, true);
  assert.equal(scorePrediction(row(), result({ text: 'ent', word_complete: true })).first_word_match, true);
  assert.equal(scorePrediction(row(), result({ text: 'ent-', word_complete: false })).first_word_match, false);
  assert.equal(scorePrediction(row({ prefix: 'Please do', expected: ["n't"] }), result({ text: "n't'", word_complete: false })).first_word_match, false);
});

test('German and Persian partial words, NFC and grapheme agreement preserve real Unicode distinctions', () => {
  assert.equal(scorePrediction(row({ language: 'de', prefix: 'Eine Zusammenf', expected: ['assung'] }), result({ text: 'assung' })).predicted_first_word, 'Zusammenfassung');
  const persian = scorePrediction(row({ language: 'fa', prefix: 'او می‌توا', expected: ['ند'] }), result({ text: 'ند' }));
  assert.equal(persian.predicted_first_word, 'می‌تواند');
  assert.equal(persian.first_word_match, true);
  const decomposed = scorePrediction(row({ prefix: 'A caf', expected: ['é nearby'] }), result({ text: 'e\u0301 nearby' }));
  assert.equal(decomposed.raw_exact_match, false);
  assert.equal(decomposed.canonical_exact_match, true);
  assert.equal(decomposed.first_word_match, true);
  assert.equal(decomposed.matching_reference_graphemes, 8);
  assert.equal(scorePrediction(row({ prefix: 'Open the', expected: [' Window'] }), result({ text: ' window' })).first_word_match, false);
  assert.equal(scorePrediction(row({ prefix: 'Done', expected: ['.'] }), result({ text: '.' })).first_word_match, false);
  assert.equal(scorePrediction(row({ prefix: 'Done', expected: ['.'] }), result({ text: '.' })).canonical_exact_match, true);
});

test('potential matching graphemes are neither fuzzy semantic correctness nor observed keystroke savings', () => {
  const scored = scorePrediction(row({ prefix: 'Go to the', expected: [' station', ' market'] }), result({ text: ' market square' }));
  assert.equal(scored.canonical_exact_match, false);
  assert.equal(scored.first_word_match, true);
  assert.equal(scored.matching_reference_graphemes, 7);
  assert.equal(scored.observed_keystrokes_saved, null);
  const noReference = scorePrediction(row({ expected: [] }), result());
  assert.equal(noReference.scorable, false);
  assert.equal(noReference.first_word_match, null);
  assert.equal(noReference.matching_reference_graphemes, null);
});

test('spelling replacement cannot gain append-reference credit or a false disagreement', () => {
  const scored = scorePrediction(row({ prefix: 'My adress', expected: ['address'] }), result({ text: 'address', replace_before: 'adress' }));
  assert.equal(scored.scorable, false);
  assert.equal(scored.unscored_reason, 'correction_requires_replacement_reference');
  assert.equal(scored.raw_exact_match, null);
  assert.equal(scored.first_word_match, null);
  assert.equal(scored.matching_reference_graphemes, null);
});

test('errors, deadlines and abstentions stay in request denominators and have distinct counters', () => {
  const records = [
    { case: row(), result: result(), config_id: 'baseline' },
    { case: row({ id: 'abstain' }), result: result({ outcome: 'abstention', text: null, latency_ms: 1 }), config_id: 'baseline' },
    { case: row({ id: 'late', language: 'de' }), result: result({ outcome: 'deadline', text: null, latency_ms: 550 }), config_id: 'baseline' },
    { case: row({ id: 'error', language: 'fa' }), result: result({ outcome: 'error', text: null, latency_ms: 600 }), config_id: 'baseline' },
    { case: row({ id: 'unscored', expected: [] }), result: result(), config_id: 'candidate' },
  ];
  const summary = summarizeResults(records);
  assert.equal(summary.overall.requests, 5);
  assert.equal(summary.overall.suggestions, 2);
  assert.equal(summary.overall.abstentions, 1);
  assert.equal(summary.overall.deadlines, 1);
  assert.equal(summary.overall.errors, 1);
  assert.equal(summary.overall.unscored_requests, 1);
  assert.equal(summary.overall.first_word.per_request, .25);
  assert.equal(summary.overall.first_word.per_shown, 1);
  assert.equal(summary.overall.latency_ms.all.p95, 600);
  assert.equal(summary.overall.latency_ms.suggestions.p95, 100);
  assert.equal(summary.by_language.de.first_word.per_request, 0);
  assert.equal(summary.by_language.de.first_word.per_shown, null);
  assert.equal(summary.by_config.candidate.first_word.per_request, null);
  assert.equal(summary.overall.reviewed_usefulness, null);
  assert.equal(summarizeResults([]).overall.first_word.per_request, null);
  for (const invalid of [result({ outcome: 'invalid' }), result({ outcome: 'error' }), result({ text: '' }), result({ text: '   ' }),
    result({ latency_ms: NaN }), result({ latency_ms: -1 }), result({ word_complete: 'yes' })]) {
    assert.throws(() => scorePrediction(row(), invalid));
  }
});

test('suite validates unique cases and ordered trace steps without conflating a trace with independent cases', () => {
  const trace = [row({ id: 'a', trace_id: 'typing', step: 0, at_ms: 0 }), row({ id: 'b', trace_id: 'typing', step: 1, at_ms: 130 })];
  assert.equal(validateSuite(suite(trace)).cases.length, 2);
  for (const invalid of [suite([]), suite([row(), row()]), suite([trace[1]]),
    suite([trace[0], { ...trace[1], step: 2 }]), suite([trace[0], { ...trace[1], at_ms: -1 }]),
    suite([{ ...trace[0], at_ms: 100 }, { ...trace[1], at_ms: 50 }]), suite([row({ trace_id: 'typing' })]),
    suite([row({ step: 1 })]), suite([row({ trace_id: 10, step: 0, at_ms: 0 })]), { ...suite([row()]), schema: 'future-version' }]) {
    assert.throws(() => validateSuite(invalid));
  }
});

test('comparison order is reproducible, balanced by position, and keeps warm traces contiguous per configuration', () => {
  const cases = Array.from({ length: 11 }, (_, i) => row({ id: `independent-${i}` }));
  cases.push(row({ id: 'trace-0', trace_id: 'typing', step: 0, at_ms: 0 }), row({ id: 'trace-1', trace_id: 'typing', step: 1, at_ms: 120 }));
  const configs = [{ id: 'baseline' }, { id: 'candidate' }];
  const plan = planComparison(cases, configs, 72);
  assert.deepEqual(plan, planComparison(cases, configs, 72));
  assert.notDeepEqual(plan, planComparison(cases, configs, 73));
  assert.equal(plan.length, cases.length * configs.length);
  const groupOrders = new Map();
  for (const entry of plan) {
    if (!groupOrders.has(entry.group_id)) groupOrders.set(entry.group_id, []);
    const order = groupOrders.get(entry.group_id);
    if (!order.includes(entry.config_id)) order.push(entry.config_id);
    assert.deepEqual(Object.keys(entry).sort(), ['at_ms', 'case_id', 'config_id', 'group_id', 'step']);
  }
  assert.equal([...groupOrders.values()].filter(order => order[0] === 'baseline').length, 6);
  for (const config of configs) {
    const index = plan.findIndex(entry => entry.case_id === 'trace-0' && entry.config_id === config.id);
    assert.equal(plan[index + 1].case_id, 'trace-1');
    assert.equal(plan[index + 1].config_id, config.id);
    assert.equal(plan[index + 1].at_ms, 120);
  }
  for (const invalid of [-1, 2 ** 32, 1.5, '42']) assert.throws(() => planComparison(cases, configs, invalid));
  assert.throws(() => planComparison(cases, [{ id: 'same' }, { id: 'same' }], 1));
});

test('three-way counterbalancing gives each configuration every position across complete cycles', () => {
  const cases = Array.from({ length: 6 }, (_, i) => row({ id: `case-${i}` }));
  const configs = [{ id: 'a' }, { id: 'b' }, { id: 'c' }];
  const plan = planComparison(cases, configs, 10);
  for (const config of configs) {
    const positions = [0, 1, 2].map(position => plan.filter((entry, i) => i % 3 === position && entry.config_id === config.id).length);
    assert.deepEqual(positions, [2, 2, 2]);
  }
});

test('a single configuration produces an ordinary run plan with every input exactly once', () => {
  const cases = [row({ id: 'a' }), row({ id: 'b' })];
  const plan = planComparison(cases, [{ id: 'baseline' }], 42);
  assert.deepEqual(plan.map(entry => entry.case_id).sort(), ['a', 'b']);
  assert.ok(plan.every(entry => entry.config_id === 'baseline'));
});
