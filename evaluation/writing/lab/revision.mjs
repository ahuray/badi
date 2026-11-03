// Revision lane (ADR 0012) cases: one selection with the text around it,
// and, for scoring only, its expected corrections or the facts a refinement
// must keep. revisionRequest is the inference boundary: it copies only what
// an adapter sends the broker.
import { randomUUID } from 'node:crypto';
import { randomFrom, shuffle } from './cases.mjs';

export const REVISION_SUITE_SCHEMA = 'badi.revision-suite.v1';
export const REVISION_REQUEST_SCHEMA = 'badi.prediction-lab.revision-request.v1';
/** Lab modes and the broker revision mode each one runs. */
export const REVISION_MODES = Object.freeze({ revision_grammar: 'grammar', revision_refine: 'refine', revision_synonyms: 'synonyms' });
/** The broker's revision languages and the ISO 639-3 code its detector reports for each. */
export const REVISION_LANGUAGES = Object.freeze({ en: 'eng', de: 'deu', fr: 'fra', es: 'spa', pt: 'por' });
// The protocol's bounds (MAX_SELECTED_SCALARS, MAX_BEFORE_CHARS, MAX_AFTER_CHARS);
// the worker checks them again.
const LIMITS = { selected: 600, before: 512, after: 128 };
const OUTCOMES = new Set(['shown', 'unchanged', 'shape', 'language', 'facts', 'length', 'edits',
  'truncated', 'timeout', 'failed', 'unavailable', 'error']);
const caseKeys = new Set(['id', 'language', 'mode', 'kind', 'before', 'selected', 'after', 'expected', 'facts']);
const identifier = /^[a-zA-Z0-9][a-zA-Z0-9._-]{0,95}$/u;

function object(value, label) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(`${label} must be an object`);
}

function text(value, label, limit, { line = false, allowEmpty = true } = {}) {
  if (typeof value !== 'string' || !value.isWellFormed() || [...value].length > limit || (!allowEmpty && !value.trim())
    || /[\u0000-\u0009\u000b-\u001f\u007f]/u.test(value)
    // One line, as `safe_revision_text`: no other space than U+0020 and the
    // no-break spaces French sets before its marks, and no format character.
    || (line && /[^\S   ]|\p{Cf}/u.test(value))) {
    throw new Error(`${label} must be ${line ? 'one line of ' : ''}valid text of at most ${limit} Unicode characters`);
  }
  return value;
}

function strings(value, label, check) {
  if (!Array.isArray(value) || !value.length || value.length > 16) throw new Error(`${label} must list 1 to 16 strings`);
  const list = value.map(item => check(text(item, label, LIMITS.selected * 2, { line: true, allowEmpty: false })));
  if (new Set(list.map(normalized)).size !== list.length) throw new Error(`Duplicate ${label}`);
  return list;
}

/** Whitespace runs as one space, trimmed, in NFC: how a correction is compared. */
export const normalized = value => value.normalize('NFC').replace(/\s+/gu, ' ').trim();

export function validateRevisionCase(raw) {
  object(raw, 'Revision case');
  if (Object.keys(raw).some(key => !caseKeys.has(key))) throw new Error('Unknown revision case field');
  if (typeof raw.id !== 'string' || !identifier.test(raw.id)) throw new Error('Invalid revision case id');
  if (!Object.hasOwn(REVISION_LANGUAGES, raw.language)) throw new Error(`Revision languages are ${Object.keys(REVISION_LANGUAGES).join(', ')}`);
  if (!['grammar', 'refine', 'synonyms'].includes(raw.mode)) throw new Error('A revision case mode is grammar, refine or synonyms');
  const selected = text(raw.selected, 'selected', LIMITS.selected, { line: true, allowEmpty: false });
  const before = text(raw.before ?? '', 'before', LIMITS.before);
  const after = text(raw.after ?? '', 'after', LIMITS.after);
  // Absent, or as a validated case writes it, so a validated case validates.
  const absent = value => value === undefined || value === null || (Array.isArray(value) && !value.length);
  let kind = null; let expected = []; let facts = [];
  if (raw.mode === 'grammar') {
    if (!absent(raw.facts)) throw new Error('Grammar cases score expected corrections, not facts');
    if (!['error', 'correct'].includes(raw.kind)) throw new Error('A grammar case kind is error or correct');
    kind = raw.kind;
    if (kind === 'correct' && !absent(raw.expected)) throw new Error('A correct case must come back unchanged; give no expected text');
    if (kind === 'error') {
      expected = strings(raw.expected, 'expected correction', value => {
        if (normalized(value) === normalized(selected)) throw new Error('An expected correction must differ from the selection');
        return value;
      });
    }
  } else if (raw.mode === 'synonyms') {
    if (!absent(raw.kind) || !absent(raw.facts)) throw new Error('Synonym cases score expected synonyms only');
    // The broker's synonym_selection bound: a word or short phrase only.
    const words = selected.trim().split(/\s+/u).filter(Boolean);
    if (!words.length || words.length > 3 || words.some(word => !/\p{L}/u.test(word)) || /[0-9@.!?;:]/u.test(selected)) {
      throw new Error('A synonym selection is one to three words with a letter each and no digit, address or sentence mark');
    }
    expected = strings(raw.expected, 'expected synonym', value => {
      if (normalized(value) === normalized(selected)) throw new Error('An expected synonym must differ from the selection');
      return value;
    });
  } else {
    if (!absent(raw.kind) || !absent(raw.expected)) throw new Error('Refine cases score kept facts only');
    facts = strings(raw.facts, 'fact', value => {
      if (!normalized(selected).includes(normalized(value))) throw new Error('Each fact must appear in the selection');
      return value;
    });
  }
  return { id: raw.id, language: raw.language, mode: raw.mode, kind, before, selected, after, expected, facts };
}

export function validateRevisionSuite(raw) {
  object(raw, 'Revision suite');
  if (raw.schema !== REVISION_SUITE_SCHEMA || Object.keys(raw).some(key => !['schema', 'name', 'cases'].includes(key))) {
    throw new Error(`A revision suite uses ${REVISION_SUITE_SCHEMA}`);
  }
  if (!Array.isArray(raw.cases) || !raw.cases.length || raw.cases.length > 1000) throw new Error('A revision suite has 1 to 1000 cases');
  const cases = raw.cases.map(validateRevisionCase);
  if (new Set(cases.map(row => row.id)).size !== cases.length) throw new Error('Duplicate revision case id');
  return { schema: REVISION_SUITE_SCHEMA, name: text(raw.name, 'Suite name', 160, { allowEmpty: false }), cases };
}

/** The worker request: the selection and its surroundings, never the answer. */
export function revisionRequest(raw) {
  const row = validateRevisionCase(raw);
  return { schema: REVISION_REQUEST_SCHEMA, id: randomUUID(), language: row.language, mode: row.mode,
    before: row.before, selected: row.selected, after: row.after };
}

/**
 * One runtime serves the whole run, as the broker keeps one: modes, then
 * languages within a mode, then cases within a language, each in seeded
 * order. Language blocks let the prompt prefix stay cached as it does for a
 * writer who stays in one language; each block's first case reads it anew.
 */
export function planRevision(rawCases, modes, seed) {
  const { cases } = validateRevisionSuite({ schema: REVISION_SUITE_SCHEMA, name: 'Plan', cases: rawCases });
  if (!Array.isArray(modes) || !modes.length || new Set(modes).size !== modes.length
    || modes.some(mode => !['grammar', 'refine', 'synonyms'].includes(mode))) throw new Error('Plan distinct revision modes');
  const random = randomFrom(seed);
  const plan = [];
  for (const mode of shuffle(modes, random)) {
    const rows = cases.filter(row => row.mode === mode);
    for (const language of shuffle([...new Set(rows.map(row => row.language))].sort(), random)) {
      for (const row of shuffle(rows.filter(value => value.language === language), random)) plan.push({ case_id: row.id, mode });
    }
  }
  return plan;
}

export function validateRevisionResult(raw) {
  object(raw, 'Revision result');
  const optional = (value, check) => value === undefined || value === null || check(value);
  if (!OUTCOMES.has(raw.outcome) || !Number.isFinite(raw.latency_ms) || raw.latency_ms < 0
    || !optional(raw.output, value => typeof value === 'string')
    || !optional(raw.changed_word_share, value => Number.isFinite(value) && value >= 0 && value <= 1)
    || !optional(raw.generated_tokens, value => Number.isSafeInteger(value) && value >= 0)
    || !optional(raw.output_language, value => typeof value === 'string')
    || !optional(raw.field_language, value => typeof value === 'string')) {
    throw new Error('Invalid revision outcome, latency or measurement');
  }
  if (raw.outcome === 'shown' ? typeof raw.text !== 'string' || !raw.text.trim() : !optional(raw.text, () => false)) {
    throw new Error('Only a shown revision has text');
  }
  return raw;
}

export function scoreRevision(rawCase, rawResult) {
  const row = validateRevisionCase(rawCase);
  const result = validateRevisionResult(rawResult);
  const shown = result.outcome === 'shown';
  // Facts are read in what would be shown, else in the model's answer, so a
  // facts abstention can be checked against the declared facts.
  const answer = shown ? result.text : result.output ?? null;
  const missing = answer === null ? [...row.facts] : row.facts.filter(fact => !normalized(answer).includes(normalized(fact)));
  return { shown, outcome: result.outcome,
    gold_match: row.kind === 'error' || row.mode === 'synonyms' ? shown && row.expected.some(value => normalized(value) === normalized(result.text)) : null,
    left_unchanged: row.kind === 'correct' ? result.outcome === 'unchanged' : null,
    facts_kept: row.mode === 'refine' && answer !== null ? missing.length === 0 : null,
    missing_facts: row.mode === 'refine' ? missing : [],
    other_language: typeof result.output_language === 'string' && result.output_language !== REVISION_LANGUAGES[row.language],
    field_language_differs: typeof result.field_language === 'string' && result.field_language !== row.language };
}

function quantiles(values) {
  const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
  const rank = share => sorted.length ? sorted[Math.ceil(share * sorted.length) - 1] : null;
  return { count: sorted.length, p50: rank(0.5), p90: rank(0.9), max: sorted.at(-1) ?? null };
}

const count = (rows, test) => rows.filter(test).length;
const tally = rows => Object.fromEntries([...rows.reduce((counts, row) =>
  counts.set(row.result.outcome, (counts.get(row.result.outcome) ?? 0) + 1), new Map())].sort(([a], [b]) => a.localeCompare(b)));

function shared(rows) {
  const measured = rows.filter(row => row.result.outcome !== 'error');
  return { other_language: count(rows, row => row.score.other_language),
    field_language_differs: count(rows, row => row.score.field_language_differs),
    latency_ms: quantiles(measured.map(row => row.result.latency_ms)),
    generated_tokens: quantiles(measured.map(row => row.result.generated_tokens ?? NaN)) };
}

function grammar(rows) {
  const errors = rows.filter(row => row.case.kind === 'error');
  const correct = rows.filter(row => row.case.kind === 'correct');
  return { cases: rows.length,
    errors: { cases: errors.length, fixed: count(errors, row => row.score.gold_match),
      shown_wrong: count(errors, row => row.score.shown && !row.score.gold_match),
      not_shown: tally(errors.filter(row => !row.score.shown)) },
    correct: { cases: correct.length, unchanged: count(correct, row => row.score.left_unchanged),
      changed: count(correct, row => row.score.shown),
      other: tally(correct.filter(row => !row.score.shown && !row.score.left_unchanged)) },
    ...shared(rows) };
}

function refine(rows) {
  const shown = rows.filter(row => row.score.shown);
  return { cases: rows.length, shown: shown.length, shown_rate: rows.length ? shown.length / rows.length : null,
    not_shown: tally(rows.filter(row => !row.score.shown)),
    facts_kept: count(shown, row => row.score.facts_kept),
    facts_abstentions_keeping_declared_facts: count(rows, row => row.result.outcome === 'facts' && row.score.facts_kept),
    ...shared(rows) };
}

function synonyms(rows) {
  return { cases: rows.length, matched: count(rows, row => row.score.gold_match),
    shown_other: count(rows, row => row.score.shown && !row.score.gold_match),
    not_shown: tally(rows.filter(row => !row.score.shown)),
    ...shared(rows) };
}

export function summarizeRevision(records) {
  if (!Array.isArray(records)) throw new Error('Revision results must be an array');
  const rows = records.map(record => {
    object(record, 'Revision record');
    return { case: validateRevisionCase(record.case), result: validateRevisionResult(record.result),
      score: scoreRevision(record.case, record.result) };
  });
  const byMode = {};
  for (const [mode, summarize] of [['grammar', grammar], ['refine', refine], ['synonyms', synonyms]]) {
    const selected = rows.filter(row => row.case.mode === mode);
    if (!selected.length) continue;
    const languages = [...new Set(selected.map(row => row.case.language))].sort();
    byMode[mode] = { overall: summarize(selected),
      by_language: Object.fromEntries(languages.map(language => [language, summarize(selected.filter(row => row.case.language === language))])) };
  }
  return { by_mode: byMode,
    meaning: 'Development cases. Grammar: fixed means shown and equal to an expected correction after whitespace normalization; a correct input should come back unchanged. Refine: facts kept means every declared fact appears in the shown text. Synonyms: matched means shown and equal to an expected synonym after whitespace normalization. Latency is the model call on a warm runtime, startup excluded; nearest-rank percentiles.' };
}
