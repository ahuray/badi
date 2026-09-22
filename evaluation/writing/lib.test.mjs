import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, copyFile, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { HERE, loadCorpus, validateRunCorpus, aggregate, textHash, validateReviews, reviewTemplate, sha256 } from './lib.mjs';

const sample = (id, overrides = {}) => ({ id, language: 'en', category: 'email', prefix: 'Please open the',
  reference: ' window', text: ' door now', outcome: 'suggestion', reason: 'suggestion', latency_ms: 250, ...overrides });
const review = labels => ({ schema: 'badi.writing-review.v1', reviewer_kind: 'agent', reviewer: 'independent test reviewer',
  independent_of_implementation: true, labels });
const label = (row, usefulWords) => ({ id: row.id, text_sha256: textHash(row.text),
  useful_next_word: usefulWords > 0, useful_words: usefulWords, issue: usefulWords ? 'none' : 'irrelevant', note: 'Rubric example' });

test('frozen corpus has 100+ untouched probes across three languages, with disjoint development', async () => {
  const dev = await loadCorpus('development');
  const heldout = await loadCorpus('heldout');
  assert.equal(dev.rows.length, 24);
  assert.equal(heldout.rows.length, 132);
  assert.deepEqual([...new Set(heldout.rows.map(row => row.language))].sort(), ['de', 'en', 'fa']);
  for (const language of ['en', 'de', 'fa']) assert.equal(heldout.rows.filter(row => row.language === language).length, 44);
  assert.ok(heldout.rows.some(row => row.category === 'partial_word'));
  assert.ok(heldout.rows.some(row => row.category === 'mixed_language'));
});

test('corpus tampering fails, including duplicate prefixes even with regenerated hashes', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'badi-writing-corpus-'));
  try {
    for (const name of ['manifest.json', 'development.jsonl', 'heldout.jsonl']) await copyFile(join(HERE, name), join(directory, name));
    await writeFile(join(directory, 'heldout.jsonl'), '\n', { flag: 'a' });
    await assert.rejects(loadCorpus('heldout', directory), /Frozen heldout/);
    await copyFile(join(HERE, 'heldout.jsonl'), join(directory, 'heldout.jsonl'));
    const development = (await readFile(join(directory, 'development.jsonl'), 'utf8')).trim().split('\n').map(JSON.parse);
    const heldout = (await readFile(join(directory, 'heldout.jsonl'), 'utf8')).trim().split('\n').map(JSON.parse);
    heldout[0].prefix = development[0].prefix;
    const bytes = heldout.map(row => JSON.stringify(row)).join('\n') + '\n';
    await writeFile(join(directory, 'heldout.jsonl'), bytes);
    const manifest = JSON.parse(await readFile(join(directory, 'manifest.json'), 'utf8'));
    manifest.files.heldout.sha256 = sha256(bytes);
    await writeFile(join(directory, 'manifest.json'), JSON.stringify(manifest));
    await assert.rejects(loadCorpus('heldout', directory), /Duplicate corpus/);
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test('coverage, errors, abstentions and latency retain separate denominators', () => {
  const rows = [sample('a'), sample('b', { outcome: 'abstention', text: null, latency_ms: 1, reason: 'no_suggestion' }),
    sample('c', { outcome: 'error', text: null, latency_ms: 2000, reason: 'timeout', language: 'de' })];
  const result = aggregate(rows);
  assert.equal(result.overall.suggestion_rate, 1 / 3);
  assert.equal(result.overall.abstention_rate, 1 / 3);
  assert.equal(result.overall.errors, 1);
  assert.equal(result.overall.latency_ms.all.p95, 2000);
  assert.equal(result.overall.latency_ms.suggestions.p95, 250);
  assert.equal(result.by_language.de.latency_ms.suggestions.p50, null);
  assert.equal(result.overall.quality_review.useful_next_word_rate, null);
  assert.equal(result.overall.quality_review.observed_keystrokes_saved, null);
});

test('a separate confirmation manifest preserves old splits and rejects reused or altered cases', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'badi-writing-confirmation-'));
  try {
    for (const name of ['manifest.json', 'development.jsonl', 'heldout.jsonl']) await copyFile(join(HERE, name), join(directory, name));
    const original = await readFile(join(directory, 'manifest.json'), 'utf8');
    const manifest = JSON.parse(original);
    const row = { id: 'new-confirmation-probe', split: 'confirmation', language: 'en', category: 'email',
      prefix: 'The fixture is a new independent', reference: ' sample', origin: 'authored_synthetic' };
    const freeze = async value => {
      const bytes = JSON.stringify(value) + '\n';
      await writeFile(join(directory, 'confirmation.jsonl'), bytes);
      manifest.files.confirmation = { sha256: sha256(bytes), samples: 1 };
      await writeFile(join(directory, 'confirmation-manifest.json'), JSON.stringify(manifest));
    };
    await freeze(row);
    const loaded = await loadCorpus('confirmation', directory, 'confirmation-manifest.json');
    assert.deepEqual(loaded.rows, [row]);
    assert.equal(await readFile(join(directory, 'manifest.json'), 'utf8'), original);
    await assert.rejects(loadCorpus('confirmation', directory, '../manifest.json'), /local corpus/);
    await assert.rejects(loadCorpus('confirmation', directory), /Invalid corpus manifest/);
    await writeFile(join(directory, 'confirmation.jsonl'), '\n', { flag: 'a' });
    await assert.rejects(loadCorpus('confirmation', directory, 'confirmation-manifest.json'), /Frozen confirmation/);
    const previous = (await loadCorpus('development', directory)).rows[0];
    await freeze({ ...row, prefix: previous.prefix });
    await assert.rejects(loadCorpus('confirmation', directory, 'confirmation-manifest.json'), /Duplicate corpus/);
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test('reference disagreement is not semantic failure and partial review cannot imply full coverage', () => {
  const rows = [sample('a'), sample('b', { text: ' window' })];
  const result = aggregate(rows, review([label(rows[0], 2)]));
  assert.equal(result.overall.reference_next_word_match.rate, .5);
  assert.equal(result.overall.quality_review.useful_next_word_rate, 1);
  assert.equal(result.overall.quality_review.coverage, .5);
  assert.equal(result.overall.quality_review.useful_words_per_reviewed_suggestion, 2);
  assert.equal(result.overall.quality_review.useful_next_word_yield_per_request, null);
  const complete = aggregate(rows, review([label(rows[0], 2), label(rows[1], 0)]));
  assert.equal(complete.overall.quality_review.useful_next_word_yield_per_request, .5);
});

test('stale, duplicated, impossible and unlabeled reviews fail closed', () => {
  const row = sample('a');
  const good = label(row, 1);
  for (const labels of [[{ ...good, text_sha256: '0'.repeat(64) }], [good, good],
    [{ ...good, useful_words: 20 }], [{ ...good, useful_next_word: false }], [{ ...good, id: 'absent' }]]) {
    assert.throws(() => validateReviews([row], review(labels)), /Invalid or stale/);
  }
  assert.throws(() => validateReviews([row], reviewTemplate([row])), /Review must identify/);
  assert.throws(() => aggregate([sample('a', { latency_ms: NaN })]), /Invalid evaluation/);
  assert.throws(() => aggregate([row, row]), /Invalid evaluation/);
});

test('Persian and German reference matches use Unicode words without claiming cross-language quality', () => {
  const result = aggregate([sample('fa', { language: 'fa', text: ' کتاب تازه', reference: ' کتاب دیگری' }),
    sample('de', { language: 'de', text: ' ÜBERPRÜFUNG', reference: ' Überprüfung der Daten' })]);
  assert.equal(result.overall.reference_next_word_match.matches, 2);
  assert.equal(result.overall.quality_review.useful_next_word_rate, null);
});

test('run scoring cannot silently omit failed cases or relabel language/category after inference', async () => {
  const { manifest, rows } = await loadCorpus('heldout');
  const run = { schema: 'badi.writing-evaluation.v1', corpus: { split: 'heldout', manifest, language_filter: null, sample_limit: null } };
  await validateRunCorpus(run, rows);
  await assert.rejects(validateRunCorpus(run, rows.slice(1)), /differ from/);
  await assert.rejects(validateRunCorpus(run, [{ ...rows[0], language: 'de' }, ...rows.slice(1)]), /differ from/);
  await assert.rejects(validateRunCorpus({ ...run, corpus: { ...run.corpus, sample_limit: 10 } }, rows.slice(0, 10)), /Invalid recorded/);
});
