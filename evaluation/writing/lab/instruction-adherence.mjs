import { createHash } from 'node:crypto';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { validateCase, validateSuite } from './cases.mjs';
import { validateConfigs } from './server.mjs';
import { buildWorker, parseOptions, runSuite } from './run.mjs';

const schema = 'badi.instruction-adherence-suite.v1';
const categories = ['terminology', 'language', 'tone', 'formality'];
const fields = ['id', 'category', 'language', 'prefix', 'after', 'context', 'instruction', 'checks'];
const checkFields = ['all', 'any', 'none', 'case_sensitive', 'exclamation_marks'];
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const bytes = value => JSON.stringify(value, null, 2) + '\n';
const object = value => value && typeof value === 'object' && !Array.isArray(value);
const words = (value, sensitive) => [...new Intl.Segmenter(undefined, { granularity: 'word' })
  .segment(value.normalize('NFC').replaceAll('’', "'"))].filter(part => part.isWordLike)
  .map(part => sensitive ? part.segment : part.segment.toLocaleLowerCase('und'));

function runtimeSignature(identity) {
  if (!object(identity) || !['binary_sha256', 'model_sha256'].every(key => /^[a-f0-9]{64}$/u.test(identity[key]))
    || !['model_size', 'threads', 'context_size', 'batch_size', 'ubatch_size'].every(key => Number.isSafeInteger(identity[key]) && identity[key] > 0)) return null;
  // The launch can select a different thread count for each cold request.
  // Compare every recorded identity field; requested configs alone do not
  // establish that an instruction and its control ran the same experiment.
  return bytes(Object.fromEntries(Object.entries(identity).sort(([left], [right]) => left.localeCompare(right))));
}

export function validateInstructionSuite(value) {
  if (!object(value) || value.schema !== schema || Object.keys(value).some(key => !['schema', 'name', 'cases'].includes(key))
    || !Array.isArray(value.cases) || !value.cases.length || value.cases.length > 100) throw new Error('Invalid instruction-adherence suite.');
  const cases = value.cases.map(row => {
    if (!object(row) || Object.keys(row).some(key => !fields.includes(key)) || !categories.includes(row.category)
      || typeof row.instruction !== 'string' || !row.instruction.trim() || row.instruction.includes('<|')
      || [...row.instruction].length > 512 || !object(row.checks)
      || Object.keys(row.checks).some(key => !checkFields.includes(key))) throw new Error('Invalid instruction probe.');
    const checks = row.checks;
    for (const key of ['all', 'any', 'none']) {
      if (!Array.isArray(checks[key]) || checks[key].length > 8 || checks[key].some(term => typeof term !== 'string'
        || !term.isWellFormed() || [...term].length > 80 || !words(term, true).length || /[\p{Cc}]/u.test(term))) {
        throw new Error('Instruction checks require bounded lexical phrases.');
      }
    }
    if (!(checks.all.length + checks.any.length) || (checks.case_sensitive !== undefined && typeof checks.case_sensitive !== 'boolean')
      || (checks.exclamation_marks !== undefined && ![0, 1].includes(checks.exclamation_marks))) throw new Error('Instruction checks need a positive marker and valid options.');
    const input = validateCase({ id: row.id, language: row.language, prefix: row.prefix, after: row.after,
      context: row.context, style: '', expected: [] });
    // The same boundary validates instructions as ordinary explicit context.
    validateCase({ ...input, context: [input.context, row.instruction].filter(Boolean).join('\n\n') });
    return { id: input.id, category: row.category, language: input.language, prefix: input.prefix,
      after: input.after, context: input.context, instruction: row.instruction, checks: { ...checks } };
  });
  const normalized = { schema, name: value.name, cases };
  predictionSuite(normalized, false); // Enforce ordinary suite/name and unique-ID bounds too.
  return normalized;
}

export function predictionSuite(document, validate = true) {
  const suite = validate ? validateInstructionSuite(document) : document;
  return validateSuite({ schema: 'badi.prediction-suite.v1', name: suite.name,
    cases: suite.cases.flatMap(row => ['control', 'instruction'].map(variant => ({
      id: `${row.id}-${variant}`, language: row.language, prefix: row.prefix, after: row.after,
      context: [row.context, variant === 'instruction' ? row.instruction : ''].filter(Boolean).join('\n\n'),
      style: '', expected: [],
    }))) });
}

export function checkAddition(addition, checks) {
  const tokens = words(addition, checks.case_sensitive);
  const contains = phrase => {
    const wanted = words(phrase, checks.case_sensitive);
    return tokens.some((_, start) => wanted.every((token, offset) => tokens[start + offset] === token));
  };
  const missing = checks.all.filter(term => !contains(term));
  const anyMissing = checks.any.length > 0 && !checks.any.some(contains);
  const forbidden = checks.none.filter(contains);
  const punctuation = checks.exclamation_marks === undefined || [...addition].filter(char => char === '!').length === checks.exclamation_marks;
  return { pass: missing.length === 0 && !anyMissing && forbidden.length === 0 && punctuation,
    missing, any_missing: anyMissing, forbidden, punctuation_matches: punctuation };
}

export function scoreInstructions(document, session) {
  const suite = validateInstructionSuite(document), expected = predictionSuite(suite);
  if (!object(session) || session.schema !== 'badi.prediction-lab.session.v1'
    || JSON.stringify(session.suite) !== JSON.stringify(expected) || !Array.isArray(session.records)) {
    throw new Error('Session does not match the frozen instruction suite.');
  }
  const configs = validateConfigs(session.configs), seen = new Map();
  for (const record of session.records) {
    const config = configs.find(value => value.id === record.config_id);
    const testCase = expected.cases.find(value => value.id === record.case?.id);
    const key = `${record.config_id}:${record.case?.id}`;
    if (!config || !testCase || JSON.stringify(testCase) !== JSON.stringify(record.case)
      || JSON.stringify(config) !== JSON.stringify(record.config) || seen.has(key)) throw new Error('Foreign or duplicate instruction record.');
    seen.set(key, record.result);
  }
  const records = [], pairs = [];
  for (const config of configs) for (const probe of suite.cases) {
    const pair = { config_id: config.id, probe_id: probe.id, category: probe.category };
    const identities = {};
    for (const variant of ['control', 'instruction']) {
      const result = seen.get(`${config.id}:${probe.id}-${variant}`);
      identities[variant] = runtimeSignature(result?.identity);
      const eligible = result?.outcome === 'suggestion' && typeof result.text === 'string' && result.text.trim()
        && Number.isFinite(result.latency_ms) && result.latency_ms >= 0
        && result.word_complete === true && result.shape_valid === true;
      const checks = eligible ? checkAddition(result.text, probe.checks) : null;
      const pass = checks?.pass === true;
      const withinBudget = Number.isFinite(result?.latency_ms) && result.latency_ms >= 0 && result.latency_ms <= 550;
      records.push({ config_id: config.id, probe_id: probe.id, category: probe.category,
        variant, outcome: result?.outcome ?? 'missing', reason: result?.reason ?? null,
        addition: result?.text ?? null, latency_ms: result?.latency_ms ?? null, lexical_pass: pass,
        lexical_pass_within_550_ms: pass && withinBudget, checks,
        runtime_identity_sha256: identities[variant] === null ? null : hash(identities[variant]),
        runtime_threads: result?.identity?.threads ?? null, full_addition_review: 'unreviewed' });
      pair[variant] = pass;
      pair[`${variant}_within_550_ms`] = pass && withinBudget;
    }
    pair.matched_runtime_identity = identities.control !== null && identities.control === identities.instruction;
    pair.effect = !pair.matched_runtime_identity ? 'unpaired'
      : pair.instruction ? (pair.control ? 'both_pass' : 'instruction_only') : (pair.control ? 'control_only' : 'neither_pass');
    pairs.push(pair);
  }
  const summaries = configs.map(config => {
    const rows = pairs.filter(pair => pair.config_id === config.id);
    return { config_id: config.id, probes: rows.length, matched_pairs: rows.filter(row => row.matched_runtime_identity).length,
      control_pass: rows.filter(row => row.control).length,
      instruction_pass: rows.filter(row => row.instruction).length,
      control_pass_within_550_ms: rows.filter(row => row.control_within_550_ms).length,
      instruction_pass_within_550_ms: rows.filter(row => row.instruction_within_550_ms).length,
      effects: Object.fromEntries(['both_pass', 'instruction_only', 'control_only', 'neither_pass', 'unpaired'].map(effect => [effect, rows.filter(row => row.effect === effect).length])) };
  });
  return { schema: 'badi.instruction-adherence-report.v1', complete: session.complete === true && seen.size === records.length,
    not_qualification: true, limitation: 'Synthetic lexical markers only. Passing does not establish language, tone, usefulness or safety. Review every full addition; late outputs earn no production-usefulness credit.',
    records, pairs, summaries };
}

export async function runInstructions(options, { signal, onProgress, run = runSuite } = {}) {
  if (options.revision || options.warmPrefix || !options.configs.some(config => config.mode === 'production_boundary')
    || !options.configs.some(config => config.mode !== 'production_boundary')
    || options.configs.some(config => !['production_boundary', 'healed', 'instructed_healed', 'native_instructed'].includes(config.mode))) {
    throw new Error('Compare production_boundary with healed, instructed_healed and/or native_instructed, using cold requests.');
  }
  const input = await readFile(options.suitePath);
  if (input.length > 512 * 1024) throw new Error('Instruction suite exceeds 512 KiB.');
  const document = validateInstructionSuite(JSON.parse(input.toString('utf8')));
  const temporary = await mkdtemp(join(tmpdir(), 'badi-instruction-suite-'));
  try {
    const suitePath = join(temporary, 'suite.json');
    await writeFile(suitePath, bytes(predictionSuite(document)), { flag: 'wx', mode: 0o600 });
    const result = await run({ ...options, suitePath }, { signal, onProgress });
    const adherence = { ...scoreInstructions(document, result.session),
      input_sha256: hash(input), run_report_sha256: hash(bytes(result.report)) };
    await writeFile(join(result.output, 'instruction-suite.json'), input, { flag: 'wx', mode: 0o600 });
    await writeFile(join(result.output, 'instruction-adherence.json'), bytes(adherence), { flag: 'wx', mode: 0o600 });
    return { ...result, adherence };
  } finally { await rm(temporary, { recursive: true, force: true }); }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const controller = new AbortController(), cancel = () => controller.abort();
  process.once('SIGINT', cancel); process.once('SIGTERM', cancel);
  try {
    const options = parseOptions(process.argv.slice(2));
    await buildWorker();
    const result = await runInstructions(options, { signal: controller.signal,
      onProgress: value => process.stdout.write(`${value.completed}/${value.total} ${value.case_id} ${value.config_id}\n`) });
    process.stdout.write(bytes({ output: result.output, complete: result.adherence.complete, summaries: result.adherence.summaries }));
    if (!result.adherence.complete || !result.report.provenance.valid_executed_identity) process.exitCode = 1;
  } catch (error) { process.stderr.write(`Instruction comparison failed: ${error.message}\n`); process.exitCode = 1; }
  finally { process.off('SIGINT', cancel); process.off('SIGTERM', cancel); }
}
