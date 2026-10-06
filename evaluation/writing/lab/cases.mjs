// Prediction Lab cases are development inputs. Expectations are scoring data,
// never model context; keep modelInputForCase as the inference boundary.
const SCHEMA = 'badi.prediction-suite.v1';
const identifier = /^[a-zA-Z0-9][a-zA-Z0-9._-]{0,95}$/u;
const caseKeys = new Set(['id', 'language', 'prefix', 'expected', 'context', 'style', 'trace_id', 'step', 'at_ms']);
const outcomes = new Set(['suggestion', 'abstention', 'deadline', 'error']);

function object(value, label) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(`${label} must be an object`);
}

function text(value, label, limit, allowEmpty = true) {
  if (typeof value !== 'string' || !value.isWellFormed() || [...value].length > limit ||
      (!allowEmpty && !value.length) || /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/u.test(value)) {
    throw new Error(`${label} must be valid text of at most ${limit} Unicode characters`);
  }
  return value;
}

function id(value, label) {
  if (typeof value !== 'string' || !identifier.test(value)) throw new Error(`Invalid ${label}`);
  return value;
}

export function validateCase(raw) {
  object(raw, 'Case');
  if (Object.keys(raw).some(key => !caseKeys.has(key))) throw new Error('Unknown case field');
  const language = text(raw.language, 'language', 40, false);
  try { Intl.getCanonicalLocales(language); } catch { throw new Error('Invalid case language'); }
  const expected = raw.expected ?? [];
  if (!Array.isArray(expected) || expected.length > 32) throw new Error('Expected outputs must be an array of at most 32 append strings');
  const alternatives = expected.map(value => text(value, 'Expected output', 2048, false));
  if (new Set(alternatives).size !== alternatives.length) throw new Error('Duplicate expected output');
  const style = text(raw.style ?? '', 'style', 8192);
  const examples = style.split(/\n\s*\n/u).filter(value => value.trim());
  if (examples.length > 8 || examples.some(value => [...value].length > 1024)) throw new Error('Style supports at most eight examples of 1024 Unicode characters each, separated by blank lines');
  const traceId = raw.trace_id ?? null;
  const step = raw.step ?? null;
  const at = raw.at_ms ?? null;
  if (traceId === null ? step !== null || at !== null :
    typeof traceId !== 'string' || !identifier.test(traceId) || !Number.isSafeInteger(step) || step < 0 || !Number.isFinite(at) || at < 0) {
    throw new Error('Trace cases require trace_id, a nonnegative integer step and nonnegative at_ms together');
  }
  return { id: id(raw.id, 'case id'), language, prefix: text(raw.prefix, 'prefix', 2048, false),
    context: text(raw.context ?? '', 'context', 4096), style,
    expected: alternatives, trace_id: traceId, step, at_ms: at };
}

export function validateSuite(raw) {
  object(raw, 'Suite');
  if (raw.schema !== SCHEMA || Object.keys(raw).some(key => !['schema', 'name', 'cases'].includes(key))) {
    throw new Error(`Suite must use ${SCHEMA}`);
  }
  if (!Array.isArray(raw.cases) || !raw.cases.length || raw.cases.length > 10000) throw new Error('Suite must contain 1 to 10000 cases');
  const cases = raw.cases.map(validateCase);
  if (new Set(cases.map(row => row.id)).size !== cases.length) throw new Error('Duplicate case id');
  const traces = new Map();
  for (const row of cases.filter(row => row.trace_id !== null)) {
    const previous = traces.get(row.trace_id);
    if ((!previous && row.step !== 0) || (previous && (row.step !== previous.step + 1 || row.at_ms < previous.at_ms))) {
      throw new Error('Trace steps must start at zero, be contiguous and have nondecreasing event times');
    }
    traces.set(row.trace_id, row);
  }
  return { schema: SCHEMA, name: text(raw.name, 'Suite name', 160, false), cases };
}

export function modelInputForCase(raw) {
  const row = validateCase(raw);
  return { language: row.language, prefix: row.prefix, context: row.context, style: row.style };
}

function validateResult(raw) {
  object(raw, 'Prediction result');
  if (!outcomes.has(raw.outcome) || !Number.isFinite(raw.latency_ms) || raw.latency_ms < 0 ||
      (raw.word_complete !== undefined && typeof raw.word_complete !== 'boolean')) {
    throw new Error('Invalid prediction outcome, latency or complete-word certification');
  }
  if (raw.outcome === 'suggestion') {
    text(raw.text, 'Prediction text', 8192, false);
    if (!raw.text.trim()) throw new Error('Whitespace-only output is not a suggestion');
  }
  else if (raw.text !== null) throw new Error('Non-suggestions must have null text; preserve raw diagnostic output separately');
  return raw;
}

const canonical = value => value.normalize('NFC');
const graphemes = value => [...new Intl.Segmenter(undefined, { granularity: 'grapheme' }).segment(value)].map(part => part.segment);

function affectedWords(prefix, addition, language, certified = false) {
  const full = prefix + addition;
  const parts = [...new Intl.Segmenter(language, { granularity: 'word' }).segment(full)]
    .filter(segment => segment.isWordLike && segment.index + segment.segment.length > prefix.length);
  // Appending a token fragment does not establish a complete word. A following
  // lexical boundary or explicit provider certification does; references are
  // authored expected continuations and may certify their own final word.
  return parts.map(part => {
    const following = full.slice(part.index + part.segment.length);
    const boundary = following.length > 0 && !/^[\p{L}\p{M}\p{N}\p{Pc}\u200c\u200d'’ʼ\-‐‑]/u.test(following);
    return { word: canonical(part.segment), complete: certified || boundary };
  });
}

function firstAffectedWord(prefix, addition, language, certified = false) {
  return affectedWords(prefix, addition, language, certified)[0] ?? { word: null, complete: false };
}

export function scoreCompleteWordPrefixes(rawCase, rawResult) {
  const row = validateCase(rawCase);
  const result = validateResult(rawResult);
  const correction = result.replace_before !== null && result.replace_before !== undefined;
  const observed = result.outcome === 'suggestion' && !correction
    ? affectedWords(row.prefix, result.text, row.language, result.word_complete === true) : [];
  const alternatives = row.expected.map(value => affectedWords(row.prefix, value, row.language, true));
  return Object.fromEntries([2, 3, 4].map(count => {
    const references = alternatives.filter(words => words.length >= count);
    const scorable = !correction && references.length > 0;
    const complete = observed.length >= count && observed.slice(0, count).every(word => word.complete);
    return [count, { scorable, match: scorable ? complete && references.some(words =>
      words.slice(0, count).every((word, index) => word.word === observed[index].word)) : null }];
  }));
}

function commonGraphemes(left, right) {
  const a = graphemes(canonical(left));
  const b = graphemes(canonical(right));
  let count = 0;
  while (count < a.length && count < b.length && a[count] === b[count]) count++;
  return count;
}

export function scorePrediction(rawCase, rawResult) {
  const row = validateCase(rawCase);
  const result = validateResult(rawResult);
  const shown = result.outcome === 'suggestion';
  const correction = result.replace_before !== null && result.replace_before !== undefined;
  const scorable = row.expected.length > 0 && !correction;
  const first = shown && !correction ? firstAffectedWord(row.prefix, result.text, row.language, result.word_complete === true) :
    { word: null, complete: false };
  const expectedWords = row.expected.map(value => firstAffectedWord(row.prefix, value, row.language, true).word);
  return { scorable, unscored_reason: correction ? 'correction_requires_replacement_reference' : row.expected.length ? null : 'no_reference',
    expected_alternatives: row.expected.length, outcome: result.outcome, shown,
    raw_exact_match: scorable ? shown && row.expected.includes(result.text) : null,
    canonical_exact_match: scorable ? shown && row.expected.some(value => canonical(value) === canonical(result.text)) : null,
    first_word_match: scorable ? shown && first.complete && first.word !== null && expectedWords.includes(first.word) : null,
    first_word_complete: first.complete, predicted_first_word: first.word,
    matching_reference_graphemes: scorable ? shown ? Math.max(...row.expected.map(value => commonGraphemes(result.text, value))) : 0 : null,
    observed_keystrokes_saved: null,
    meaning: 'Case-sensitive reference agreement only. Alternatives may also be useful; matching graphemes are potential reference text, not observed typing savings.' };
}

const rate = (n, d) => d ? n / d : null;
function latency(rows) {
  const values = rows.map(row => row.result.latency_ms).sort((a, b) => a - b);
  const q = value => values.length ? values[Math.ceil(value * values.length) - 1] : null;
  return { count: values.length, p50: q(.5), p95: q(.95), max: values.at(-1) ?? null };
}

export function summarizeResults(records) {
  if (!Array.isArray(records)) throw new Error('Results must be an array');
  const rows = records.map(record => {
    object(record, 'Result record');
    return { ...record, case: validateCase(record.case), result: validateResult(record.result), score: scorePrediction(record.case, record.result) };
  });
  const group = selected => {
    const shown = selected.filter(row => row.score.shown);
    const scored = selected.filter(row => row.score.scorable);
    const scoredShown = scored.filter(row => row.score.shown);
    const agreement = field => {
      const matches = scored.filter(row => row.score[field]).length;
      return { matches, requests: scored.length, shown: scoredShown.length,
        per_request: rate(matches, scored.length), per_shown: rate(matches, scoredShown.length) };
    };
    return { requests: selected.length, suggestions: shown.length,
      abstentions: selected.filter(row => row.result.outcome === 'abstention').length,
      deadlines: selected.filter(row => row.result.outcome === 'deadline').length,
      errors: selected.filter(row => row.result.outcome === 'error').length,
      unscored_requests: selected.length - scored.length, suggestion_rate: rate(shown.length, selected.length),
      raw_exact: agreement('raw_exact_match'), canonical_exact: agreement('canonical_exact_match'), first_word: agreement('first_word_match'),
      matching_reference_graphemes: scored.reduce((sum, row) => sum + row.score.matching_reference_graphemes, 0),
      latency_ms: { all: latency(selected), suggestions: latency(shown) },
      observed_keystrokes_saved: null, reviewed_usefulness: null };
  };
  const languages = [...new Set(rows.map(row => row.case.language))].sort();
  const configs = [...new Set(rows.map(row => row.config_id).filter(value => typeof value === 'string'))].sort();
  return { overall: group(rows), by_language: Object.fromEntries(languages.map(language => [language, group(rows.filter(row => row.case.language === language))])),
    by_config: Object.fromEntries(configs.map(config => [config, group(rows.filter(row => row.config_id === config))])),
    meaning: 'Development reference agreement, with errors and deadlines in request denominators. No semantic quality, user acceptance or real keystroke-saving claim.' };
}

function randomFrom(seed) {
  if (!Number.isSafeInteger(seed) || seed < 0 || seed > 0xffffffff) throw new Error('Comparison seed must be an unsigned 32-bit integer');
  let state = seed;
  return () => {
    state += 0x6d2b79f5;
    let value = Math.imul(state ^ state >>> 15, state | 1);
    value ^= value + Math.imul(value ^ value >>> 7, value | 61);
    return ((value ^ value >>> 14) >>> 0) / 4294967296;
  };
}

function shuffle(values, random) {
  const copy = [...values];
  for (let i = copy.length - 1; i > 0; i--) {
    const j = Math.floor(random() * (i + 1));
    [copy[i], copy[j]] = [copy[j], copy[i]];
  }
  return copy;
}

export function planComparison(rawCases, configs, seed) {
  const cases = validateSuite({ schema: SCHEMA, name: 'Comparison', cases: rawCases }).cases;
  if (!Array.isArray(configs) || configs.length < 1 || configs.length > 16) throw new Error('Comparison requires 1 to 16 configurations');
  const ids = configs.map(config => { object(config, 'Configuration'); return id(config.id, 'configuration id'); });
  if (new Set(ids).size !== ids.length) throw new Error('Duplicate configuration id');
  const random = randomFrom(seed);
  const groups = new Map();
  for (const row of cases) {
    const key = row.trace_id === null ? `case:${row.id}` : `trace:${row.trace_id}`;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(row);
  }
  const order = shuffle(ids, random);
  // A randomized Latin-square cycle balances position. Alternating its direction
  // across cycles also counterbalances carryover; keep each trace indivisible.
  const schedule = [];
  for (const [index, [groupId, group]] of shuffle([...groups], random).entries()) {
    const shift = index % order.length;
    const cycle = Math.floor(index / order.length);
    const rotated = [...order.slice(shift), ...order.slice(0, shift)];
    const arms = cycle % 2 ? rotated.reverse() : rotated;
    for (const configId of arms) {
      for (const row of group) schedule.push({ case_id: row.id, config_id: configId, group_id: groupId, step: row.step, at_ms: row.at_ms });
    }
  }
  return schedule;
}
