import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

export const ROOT = resolve(import.meta.dirname, '../..');
export const HERE = import.meta.dirname;
export const sha256 = value => createHash('sha256').update(value).digest('hex');
export const textHash = value => sha256(value ?? '');

export async function loadCorpus(split, directory = HERE, manifestName = 'manifest.json') {
  const allowed = ['development', 'heldout', 'confirmation'];
  if (!allowed.includes(split)) throw new Error('Choose development, heldout or confirmation');
  if (typeof manifestName !== 'string' || !/^[a-z0-9-]+\.json$/u.test(manifestName)) throw new Error('Use a local corpus manifest filename');
  const manifestBytes = await readFile(resolve(directory, manifestName));
  const manifest = JSON.parse(manifestBytes);
  const names = manifest.files && Object.keys(manifest.files);
  if (manifest.schema !== 'badi.writing-corpus.v1' || !names || !names.includes(split) ||
      !names.includes('development') || !names.includes('heldout') || names.some(name => !allowed.includes(name))) {
    throw new Error('Invalid corpus manifest');
  }
  const all = [];
  for (const name of names) {
    if (!/^[a-f0-9]{64}$/u.test(manifest.files[name].sha256) ||
        !Number.isSafeInteger(manifest.files[name].samples) || manifest.files[name].samples < 1) {
      throw new Error('Invalid corpus manifest entry');
    }
    const bytes = await readFile(resolve(directory, `${name}.jsonl`));
    if (sha256(bytes) !== manifest.files[name].sha256) throw new Error(`Frozen ${name} corpus changed`);
    const rows = bytes.toString('utf8').trim().split('\n').map(line => JSON.parse(line));
    if (rows.length !== manifest.files[name].samples) throw new Error('Corpus count mismatch');
    for (const row of rows) {
      if (row.split !== name || !/^[a-z0-9-]+$/u.test(row.id) ||
          typeof row.language !== 'string' || !/^[a-z]{2}(?:-[a-z0-9]+)*$/u.test(row.language) ||
          typeof row.prefix !== 'string' || !row.prefix.trim() || [...row.prefix].length > 512 ||
          typeof row.reference !== 'string' || !row.reference.trim() ||
          typeof row.category !== 'string' || !row.category || row.origin !== 'authored_synthetic') {
        throw new Error(`Invalid corpus row ${row.id}`);
      }
    }
    all.push(...rows);
  }
  if (new Set(all.map(row => row.id)).size !== all.length ||
      new Set(all.map(row => row.prefix.normalize('NFC').trim())).size !== all.length) {
    throw new Error('Duplicate corpus ID or prefix crosses evaluation splits');
  }
  return { manifest, manifestName, manifestSha256: sha256(manifestBytes), rows: all.filter(row => row.split === split) };
}

export async function validateRunCorpus(run, samples) {
  const { manifest, manifestSha256, rows } = await loadCorpus(run.corpus.split, HERE, run.corpus.manifest_name ?? 'manifest.json');
  if (run.schema !== 'badi.writing-evaluation.v1' ||
      (run.corpus.manifest_sha256 !== undefined && run.corpus.manifest_sha256 !== manifestSha256) ||
      run.corpus.manifest.files[run.corpus.split].sha256 !== manifest.files[run.corpus.split].sha256) {
    throw new Error('Run does not match the frozen corpus');
  }
  let expected = run.corpus.language_filter ? rows.filter(row => row.language === run.corpus.language_filter) : rows;
  if (run.corpus.sample_limit !== null) {
    if (run.corpus.split !== 'development' || !Number.isSafeInteger(run.corpus.sample_limit) || run.corpus.sample_limit < 1) {
      throw new Error('Invalid recorded sample limit');
    }
    expected = expected.slice(0, run.corpus.sample_limit);
  }
  const input = row => [row.id, row.split, row.language, row.category, row.prefix, row.reference, row.origin];
  if (!expected.length || JSON.stringify(expected.map(input)) !== JSON.stringify(samples.map(input))) {
    throw new Error('Evaluation samples differ from the recorded frozen inputs');
  }
}

export function words(text, language = 'en') {
  return [...new Intl.Segmenter(language, { granularity: 'word' }).segment(text ?? '')]
    .filter(part => part.isWordLike);
}

const divide = (n, d) => d ? n / d : null;
const quantile = (values, q) => values.length ? [...values].sort((a, b) => a - b)[Math.ceil(q * values.length) - 1] : null;
const latency = rows => ({ count: rows.length, p50: quantile(rows.map(row => row.latency_ms), .5),
  p95: quantile(rows.map(row => row.latency_ms), .95), max: rows.length ? Math.max(...rows.map(row => row.latency_ms)) : null });

function validateSamples(samples) {
  if (!Array.isArray(samples) || new Set(samples.map(row => row.id)).size !== samples.length || samples.some(row =>
    typeof row.id !== 'string' || typeof row.language !== 'string' || typeof row.category !== 'string' ||
    typeof row.prefix !== 'string' || typeof row.reference !== 'string' || typeof row.reason !== 'string' ||
    !Number.isFinite(row.latency_ms) || row.latency_ms < 0 ||
    !['suggestion', 'abstention', 'error'].includes(row.outcome) ||
    (row.outcome === 'suggestion' ? typeof row.text !== 'string' || !row.text.trim() : row.text !== null))) {
    throw new Error('Invalid evaluation samples');
  }
}

export function referenceMatch(row) {
  if (row.outcome !== 'suggestion') return null;
  const normalize = value => value?.normalize('NFC').toLocaleLowerCase(row.language);
  return normalize(words(row.text, row.language)[0]?.segment) ===
    normalize(words(row.reference, row.language)[0]?.segment);
}

export function validateReviews(samples, document) {
  validateSamples(samples);
  if (document.schema !== 'badi.writing-review.v1' ||
      !['human', 'agent'].includes(document.reviewer_kind) ||
      typeof document.reviewer !== 'string' || !document.reviewer.trim() ||
      typeof document.independent_of_implementation !== 'boolean' || !Array.isArray(document.labels)) {
    throw new Error('Review must identify reviewer, kind, independence and labels');
  }
  const byId = new Map(samples.map(sample => [sample.id, sample]));
  const seen = new Set();
  for (const label of document.labels) {
    const row = byId.get(label.id);
    if (!row || row.outcome !== 'suggestion' || seen.has(label.id) || label.text_sha256 !== textHash(row.text) ||
        typeof label.useful_next_word !== 'boolean' || !Number.isSafeInteger(label.useful_words) ||
        label.useful_words < 0 || label.useful_words > words(row.text, row.language).length ||
        label.useful_next_word !== (label.useful_words > 0) ||
        !['none', 'seam', 'grammar', 'irrelevant', 'language', 'unsupported_fact', 'other'].includes(label.issue) ||
        typeof label.note !== 'string') {
      throw new Error(`Invalid or stale review label ${label.id}`);
    }
    seen.add(label.id);
  }
  return new Map(document.labels.map(label => [label.id, label]));
}

export function aggregate(samples, review) {
  validateSamples(samples);
  const labels = review ? validateReviews(samples, review) : new Map();
  function group(rows) {
    const shown = rows.filter(row => row.outcome === 'suggestion');
    const abstentions = rows.filter(row => row.outcome === 'abstention');
    const errors = rows.filter(row => row.outcome === 'error');
    const reviewed = shown.filter(row => labels.has(row.id));
    const useful = reviewed.filter(row => labels.get(row.id).useful_next_word);
    const usefulWords = reviewed.reduce((sum, row) => sum + labels.get(row.id).useful_words, 0);
    const counts = Object.fromEntries([...new Set(rows.map(row => row.reason))].sort()
      .map(reason => [reason, rows.filter(row => row.reason === reason).length]));
    return { requests: rows.length, suggestions: shown.length, abstentions: abstentions.length, errors: errors.length,
      suggestion_rate: divide(shown.length, rows.length), abstention_rate: divide(abstentions.length, rows.length),
      reasons: counts,
      latency_ms: { all: latency(rows), suggestions: latency(shown), abstentions: latency(abstentions), errors: latency(errors),
        over_500_ms: rows.filter(row => row.latency_ms > 500).length },
      reference_next_word_match: { matches: shown.filter(referenceMatch).length, denominator: shown.length,
        rate: divide(shown.filter(referenceMatch).length, shown.length),
        meaning: 'Exact authored-reference agreement only; alternatives can also be useful.' },
      quality_review: { reviewed: reviewed.length, coverage: divide(reviewed.length, shown.length),
        useful_next_words: useful.length, useful_next_word_rate: divide(useful.length, reviewed.length),
        useful_words: usefulWords, useful_words_per_reviewed_suggestion: divide(usefulWords, reviewed.length),
        useful_next_word_yield_per_request: review && reviewed.length === shown.length ? divide(useful.length, rows.length) : null,
        useful_accepted_words: null, observed_keystrokes_saved: null,
        meaning: 'Rubric judgments of hypothetical usefulness; no user acceptance or typing benefit was measured.' } };
  }
  return { overall: group(samples), by_language: Object.fromEntries([...new Set(samples.map(row => row.language))].sort()
    .map(language => [language, group(samples.filter(row => row.language === language))])),
  by_category: Object.fromEntries([...new Set(samples.map(row => row.category))].sort()
    .map(category => [category, group(samples.filter(row => row.category === category))])) };
}

export function reviewTemplate(samples) {
  return { schema: 'badi.writing-review.v1', reviewer_kind: null, reviewer: null,
    independent_of_implementation: null,
    rubric: 'Judge prefix plus suggestion in its stated language. Count only consecutive useful words from the start. A malformed seam, unsupported invented fact, wrong language, irrelevant or ungrammatical first word scores zero. References are one possible future, not mandatory answers. State uncertainty in the note. Do not label missing suggestions as wrong predictions.',
    labels: samples.filter(row => row.outcome === 'suggestion').map(row => ({ id: row.id,
      text_sha256: textHash(row.text), language: row.language, category: row.category,
      prefix: row.prefix, suggestion: row.text, useful_next_word: null, useful_words: null, issue: null, note: '' })) };
}
