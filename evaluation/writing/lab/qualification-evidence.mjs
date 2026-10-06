import { createHash, randomBytes } from 'node:crypto';
import { constants } from 'node:fs';
import { open, mkdir, opendir, link, unlink, rmdir } from 'node:fs/promises';
import { isAbsolute, resolve } from 'node:path';

const MAX_CASES = 10000;
const MAX_FILE_BYTES = 64 * 1024;
const MAX_FILES = 256;
const MAX_AGE_SECONDS = 30 * 24 * 3600;
const HASH = /^[a-f0-9]{64}$/u;
const ID = /^[a-zA-Z0-9][a-zA-Z0-9._-]{0,95}$/u;
const isHash = value => typeof value === 'string' && HASH.test(value);
const isId = value => typeof value === 'string' && ID.test(value);
const LANGUAGES = ['en', 'de', 'fa'];
const IDENTITY_KEYS = ['candidate_sha256', 'device_fingerprint', 'settings_sha256'];
const COUNT_KEYS = ['total', 'reviewed', 'useful_on_time', 'harmful', 'generic_or_incorrect', 'abstained_or_failed', 'p50_complete_word_ms', 'p95_complete_word_ms'];
const METRIC_KEYS = ['measured_at_unix_s', 'cold_start_ms', 'preparation_ms', 'peak_rss_bytes', 'sustained_seconds'];
const FLAG_KEYS = ['loaded_and_exercised', 'artifact_verified', 'cancellation_recovery_passed', 'cleanup_verified', 'prompt_reuse_measured', 'paced_typing_measured'];
const EVIDENCE_KEYS = ['identity', ...METRIC_KEYS, ...FLAG_KEYS, 'actual_backend', 'peak_vram_bytes', 'confirmation_set_sha256', 'confirmation_untouched', 'review_protocol_sha256', 'languages'];
const registrations = new WeakMap();
const fail = message => { throw new Error(`Qualification evidence: ${message}`); };
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const integer = value => Number.isSafeInteger(value) && value >= 0;
const exact = (value, keys) => object(value) && Object.keys(value).length === keys.length && Object.keys(value).every(key => keys.includes(key));
const canonical = value => {
  if (Array.isArray(value)) return value.map(canonical);
  if (object(value)) return Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])]));
  return value;
};
const serialized = value => JSON.stringify(canonical(value));
export const evidenceDigest = value => createHash('sha256').update(serialized(value)).digest('hex');
const same = (left, right) => serialized(left) === serialized(right);
const identityValid = value => exact(value, IDENTITY_KEYS) && IDENTITY_KEYS.every(key => isHash(value[key]));
const baseLanguage = value => typeof value === 'string' ? value.toLowerCase().split('-')[0] : '';

function assignments({ records, plannedCases, configId, runId }) {
  if (!isId(runId) || !isId(configId)) fail('use the server-held run and configuration IDs.');
  if (!Array.isArray(plannedCases) || !plannedCases.length || plannedCases.length > MAX_CASES
    || !Array.isArray(records) || records.length > MAX_CASES * 9) fail('invalid or oversized planned outcomes.');
  const cases = new Map();
  for (const row of plannedCases) {
    if (!object(row) || !isId(row.id) || !LANGUAGES.includes(baseLanguage(row.language)) || cases.has(row.id)) fail('invalid, duplicate or unsupported planned case.');
    cases.set(row.id, { case: row, record: null });
  }
  for (const record of records) {
    if (!object(record)) fail('invalid result record.');
    const selected = record.config_id ?? record.config?.id;
    if (selected !== configId) continue;
    if (record.run_id !== undefined && record.run_id !== runId) fail('result belongs to another run.');
    const slot = cases.get(record.case?.id);
    if (!slot || slot.record || !same(slot.case, record.case)
      || record.config?.id !== configId) fail('result does not match its unique planned case/configuration.');
    if (!object(record.result)) fail('invalid result.');
    const result = record.result;
    if (!['suggestion', 'abstention', 'deadline', 'error'].includes(result.outcome)
      || !Number.isFinite(result.latency_ms) || result.latency_ms < 0 || result.latency_ms > Number.MAX_SAFE_INTEGER
      || result.outcome === 'suggestion' && (typeof result.text !== 'string' || !result.text.trim() || Buffer.byteLength(result.text) > 32768)
      || result.outcome !== 'suggestion' && result.text != null) fail('invalid outcome; retain failed attempts as explicit error records.');
    slot.record = record;
  }
  return [...cases.values()].map(slot => {
    const result = slot.record?.result ?? null;
    return { ...slot, target: { run_id: runId, case_id: slot.case.id, config_id: configId,
      result_sha256: evidenceDigest({ run_id: runId, case: slot.case, config_id: configId, config: slot.record?.config ?? null, result }),
      outcome: result?.outcome ?? 'missing' } };
  });
}

// These hashes bind the *entire* displayed addition, settings and outcome to the
// server's run. Expected-answer agreement never assigns review credit.
export function reviewTargets(input) { return assignments(input).map(row => row.target); }

function checkedReviews(rows, reviews) {
  if (!Array.isArray(reviews) || reviews.length > rows.length) fail('invalid or excessive reviews.');
  const targets = new Map(rows.map(row => [row.case.id, row]));
  const checked = new Map();
  const keys = ['run_id', 'case_id', 'config_id', 'result_sha256', 'outcome', 'judgment', 'full_addition_reviewed', 'substantive'];
  for (const review of reviews) {
    if (!object(review) || Object.keys(review).some(key => !keys.includes(key))) fail('unknown review field; measurements cannot be supplied by reviews.');
    const row = targets.get(review.case_id);
    if (!row || checked.has(review.case_id)
      || ['run_id', 'case_id', 'config_id', 'result_sha256'].some(key => review[key] !== row.target[key])
      || review.outcome !== undefined && review.outcome !== row.target.outcome) fail('review does not match the exact run/case/configuration/full result.');
    if (!['useful', 'neutral', 'unhelpful', 'harmful'].includes(review.judgment)
      || review.full_addition_reviewed !== true || review.substantive !== undefined && typeof review.substantive !== 'boolean') fail('review the full displayed addition and choose an explicit judgment.');
    if (['useful', 'harmful'].includes(review.judgment) && row.target.outcome !== 'suggestion') fail('absent additions cannot be useful or harmful suggestions.');
    if (review.judgment === 'useful' && review.substantive !== true) fail('usefulness requires explicit substantive value; generic function words are insufficient.');
    checked.set(review.case_id, review);
  }
  return checked;
}

// Registration is an in-memory server capability, not an importable boolean.
// The caller must reserve an unused set and freeze its review protocol before
// starting the run. It must never register client-supplied post-run assertions.
export function registerConfirmation({ assessment, plannedCases, configId, runId, reviewProtocol, registeredAtUnixS }) {
  assignments({ records: [], plannedCases, configId, runId });
  if (!identityValid(assessment?.identity) || typeof reviewProtocol !== 'string'
    || reviewProtocol.trim().length < 32 || Buffer.byteLength(reviewProtocol) > 32768 || !integer(registeredAtUnixS)) fail('invalid confirmation registration.');
  const token = Object.freeze({});
  registrations.set(token, { identity: structuredClone(assessment.identity), set: evidenceDigest(plannedCases),
    protocol: evidenceDigest(reviewProtocol), runId, configId, at: registeredAtUnixS });
  return token;
}

/** Pure aggregation over trusted, server-held run records and explicit reviews.
 * runtimeMeasurements is populated by owned workers/diagnostics, NEVER HTTP
 * input. It must carry this run's preflight assessment identity. This module
 * cannot authenticate a caller; the HTTP owner enforces that trust boundary.
 */
export function createEvidence({ assessment, records, plannedCases, configId, runId, reviews = [], runtimeMeasurements = {}, confirmation = null, runStartedAtUnixS }) {
  if (!identityValid(assessment?.identity) || !Array.isArray(assessment.settings?.languages)
    || !assessment.settings.languages.length || new Set(assessment.settings.languages).size !== assessment.settings.languages.length
    || assessment.settings.languages.some(language => !LANGUAGES.includes(language))) fail('a current assessment with exact language/settings identity is required.');
  const rows = assignments({ records, plannedCases, configId, runId });
  if (rows.some(row => !assessment.settings.languages.includes(baseLanguage(row.case.language)))) fail('planned language is outside assessed settings.');
  if (assessment.settings.languages.some(language => !rows.some(row => baseLanguage(row.case.language) === language))) fail('every assessed language needs planned outcomes, including missing ones.');
  const reviewed = checkedReviews(rows, reviews);
  const metrics = runtimeMeasurements;
  const metricKeys = ['identity', ...METRIC_KEYS, ...FLAG_KEYS, 'actual_backend', 'peak_vram_bytes'];
  if (!object(metrics) || Object.keys(metrics).some(key => !metricKeys.includes(key))) fail('unknown runtime measurement field.');
  if (Object.keys(metrics).length && (!identityValid(metrics.identity) || !same(metrics.identity, assessment.identity))) fail('runtime measurements have stale identity.');
  for (const key of METRIC_KEYS) if (metrics[key] !== undefined && !integer(metrics[key])) fail('invalid measured integer.');
  for (const key of FLAG_KEYS) if (metrics[key] !== undefined && typeof metrics[key] !== 'boolean') fail('invalid measured status.');
  if (metrics.actual_backend !== undefined && !['cpu', 'unknown'].includes(metrics.actual_backend)
    || metrics.peak_vram_bytes !== undefined && metrics.peak_vram_bytes !== null && !integer(metrics.peak_vram_bytes)) fail('invalid backend measurement.');
  let startup = 0, rss = 0;
  for (const { record } of rows) {
    if (!record) continue;
    const result = record.result;
    if (result.identity && (result.identity.model_sha256 !== assessment.metadata?.sha256
      || result.identity.model_size !== assessment.metadata?.artifact_bytes)) fail('observed model identity differs from the assessment.');
    if (result.identity && (assessment.settings.backend === 'cpu' && result.identity.gpu_layers !== 0
      || assessment.settings.threads !== undefined && result.identity.threads !== assessment.settings.threads
      || assessment.settings.context_tokens !== undefined && result.identity.context_size !== assessment.settings.context_tokens
      || assessment.settings.batch_tokens !== undefined && (result.identity.batch_size !== assessment.settings.batch_tokens
        || result.identity.ubatch_size !== assessment.settings.batch_tokens))) fail('observed runtime configuration differs from the assessment.');
    if (record.config.mode !== assessment.settings.prompt_format
      || createHash('sha256').update(JSON.stringify(record.config)).digest('hex') !== assessment.settings.generation_settings_sha256) fail('observed generation settings differ from the assessment.');
    const local = result.local_measurement;
    if (local) {
      for (const key of ['worker_startup_ms', 'request_roundtrip_ms', 'peak_runtime_rss_bytes', 'gpu_memory_bytes']) {
        if (local[key] != null && (!Number.isFinite(local[key]) || local[key] < 0 || local[key] > Number.MAX_SAFE_INTEGER)) fail('invalid worker measurement.');
      }
      startup = Math.max(startup, Math.ceil(local.worker_startup_ms ?? 0));
      rss = Math.max(rss, Math.ceil(local.peak_runtime_rss_bytes ?? 0));
    }
  }
  const percentile = (values, fraction) => [...values].sort((a, b) => a - b)[Math.ceil(values.length * fraction) - 1];
  const languages = assessment.settings.languages.map(language => {
    const selected = rows.filter(row => baseLanguage(row.case.language) === language);
    const counts = { language, total: selected.length, reviewed: 0, useful_on_time: 0, harmful: 0, generic_or_incorrect: 0, abstained_or_failed: 0 };
    const latencies = [];
    for (const row of selected) {
      const result = row.record?.result;
      const review = reviewed.get(row.case.id);
      if (review) counts.reviewed++;
      const shown = result?.outcome === 'suggestion';
      const complete = shown && result.word_complete === true && result.terminal_received === true
        && result.replace_before == null && result.shape_valid !== false;
      const elapsed = Math.ceil(Math.max(result?.latency_ms ?? 0, result?.local_measurement?.request_roundtrip_ms ?? 0));
      // No complete word is a right-censored delivery, never a zero-ms success.
      // The 551/budget+1 value is a conservative lower bound, not measured time.
      latencies.push(complete ? elapsed : Math.max(551, (row.record?.config.budget_ms ?? 550) + 1, elapsed));
      if (!shown) counts.abstained_or_failed++;
      else if (review?.judgment === 'harmful') counts.harmful++;
      else if (review?.judgment === 'useful' && complete && elapsed <= 550 && result.timing_includes_worker_startup !== true) counts.useful_on_time++;
      else counts.generic_or_incorrect++;
    }
    return { ...counts, p50_complete_word_ms: percentile(latencies, .5), p95_complete_word_ms: percentile(latencies, .95) };
  });
  const registration = object(confirmation) ? registrations.get(confirmation) : null;
  const untouched = Boolean(registration && registration.runId === runId && registration.configId === configId
    && same(registration.identity, assessment.identity) && registration.set === evidenceDigest(plannedCases)
    && integer(runStartedAtUnixS) && registration.at <= runStartedAtUnixS);
  if (confirmation !== null && !untouched) fail('confirmation registration is missing, late or bound to a different run/configuration/set.');
  const evidence = {
    identity: structuredClone(assessment.identity), measured_at_unix_s: metrics.measured_at_unix_s ?? 0,
    loaded_and_exercised: metrics.loaded_and_exercised === true && rows.some(row => row.record?.result.identity
      && row.record.result.outcome !== 'error' && row.record.result.terminal_received === true),
    actual_backend: metrics.actual_backend ?? 'unknown', artifact_verified: metrics.artifact_verified === true,
    cold_start_ms: Math.max(metrics.cold_start_ms ?? 0, startup), preparation_ms: metrics.preparation_ms ?? 0,
    peak_rss_bytes: Math.max(metrics.peak_rss_bytes ?? 0, rss), peak_vram_bytes: metrics.peak_vram_bytes ?? null,
    sustained_seconds: metrics.sustained_seconds ?? 0, cancellation_recovery_passed: metrics.cancellation_recovery_passed === true,
    cleanup_verified: metrics.cleanup_verified === true, prompt_reuse_measured: metrics.prompt_reuse_measured === true,
    paced_typing_measured: metrics.paced_typing_measured === true,
    confirmation_set_sha256: evidenceDigest(plannedCases), confirmation_untouched: untouched,
    review_protocol_sha256: registration?.protocol ?? '', languages,
  };
  return validateEvidence(evidence);
}

// Privacy allowlist and structural gate used on both sides of persistence.
// Runtime assessment remains authoritative for recommendation thresholds.
export function validateEvidence(value) {
  if (!exact(value, EVIDENCE_KEYS) || !identityValid(value.identity)
    || METRIC_KEYS.some(key => !integer(value[key])) || FLAG_KEYS.some(key => typeof value[key] !== 'boolean')
    || !['cpu', 'unknown'].includes(value.actual_backend)
    || value.peak_vram_bytes !== null && !integer(value.peak_vram_bytes)
    || typeof value.confirmation_untouched !== 'boolean' || !isHash(value.confirmation_set_sha256)
    || !(value.review_protocol_sha256 === '' || isHash(value.review_protocol_sha256))
    || value.confirmation_untouched && !isHash(value.review_protocol_sha256)
    || !Array.isArray(value.languages) || !value.languages.length || value.languages.length > LANGUAGES.length) fail('malformed evidence or disallowed content.');
  const languages = new Set();
  for (const row of value.languages) {
    if (!exact(row, ['language', ...COUNT_KEYS]) || !LANGUAGES.includes(row.language) || languages.has(row.language)
      || COUNT_KEYS.some(key => !integer(row[key])) || row.total < 1 || row.total > MAX_CASES || row.reviewed > row.total
      || row.useful_on_time + row.harmful > row.reviewed
      || row.useful_on_time + row.harmful + row.generic_or_incorrect + row.abstained_or_failed !== row.total
      || row.p50_complete_word_ms > row.p95_complete_word_ms) fail('invalid counts, duplicate languages or omitted outcomes.');
    languages.add(row.language);
  }
  return structuredClone(value);
}

/** Explicit opt-in save only. No drafts, additions, case IDs or review prose are
 * retained. Linux directory FDs pin the traversal; no symlink is followed. */
export class EvidenceStore {
  constructor({ directory } = {}) {
    if (typeof directory !== 'string' || !isAbsolute(directory) || resolve(directory) !== directory
      || directory === '/' || Buffer.byteLength(directory) > 4096 || /[\p{Cc}]/u.test(directory)) fail('use an absolute private evidence directory.');
    this.directory = directory;
    this.pending = Promise.resolve();
  }
  async directoryHandle(create) {
    let handle = await open('/', constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW);
    try {
      const parts = this.directory.split('/').filter(Boolean);
      for (const part of parts) {
        const child = `/proc/self/fd/${handle.fd}/${part}`;
        if (create) { try { await mkdir(child, { mode: 0o700 }); } catch (error) { if (error.code !== 'EEXIST') throw error; } }
        const next = await open(child, constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
        await handle.close(); handle = next;
      }
      const stat = await handle.stat();
      if (stat.uid !== process.getuid() || (stat.mode & 0o077) !== 0) fail('evidence directory must be owned by this user with private permissions.');
      return handle;
    } catch (error) { await handle.close(); throw error; }
  }
  async save(raw) {
    const evidence = validateEvidence(raw);
    // Queue this instance's writes so the bounded store cannot race itself.
    const operation = this.pending.then(() => this.saveValidated(evidence));
    this.pending = operation.catch(() => {});
    return operation;
  }
  async saveValidated(evidence) {
    const bytes = Buffer.from(serialized(evidence));
    if (bytes.length > MAX_FILE_BYTES) fail('evidence exceeds its file limit.');
    const id = evidenceDigest(evidence);
    const directory = await this.directoryHandle(true);
    const path = `/proc/self/fd/${directory.fd}`;
    let temporary, locked = false;
    try {
      try { await mkdir(`${path}/.save-lock`, { mode: 0o700 }); locked = true; }
      catch (error) { if (error.code === 'EEXIST') fail('save lock exists; retry after completion, or inspect an interrupted save before removing its private .save-lock directory.'); throw error; }
      const existing = [];
      for await (const entry of await opendir(path)) {
        if (entry.name === '.save-lock') continue;
        existing.push(entry.name);
        if (existing.length > MAX_FILES) fail('private evidence store is full; explicitly remove obsolete evidence before saving.');
      }
      if (existing.includes(`${id}.json`)) { await this.readAt(directory, id); return id; }
      if (existing.length >= MAX_FILES) fail('private evidence store is full; explicitly remove obsolete evidence before saving.');
      temporary = `${path}/.${randomBytes(16).toString('hex')}.tmp`;
      const file = await open(temporary, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW, 0o600);
      try { await file.writeFile(bytes); await file.sync(); } finally { await file.close(); }
      // link is atomic and never replaces an existing evidence file or symlink.
      try { await link(temporary, `${path}/${id}.json`); }
      catch (error) { if (error.code !== 'EEXIST') throw error; await this.readAt(directory, id); }
      await unlink(temporary); temporary = null;
      await directory.sync();
      return id;
    } finally {
      if (temporary) await unlink(temporary).catch(() => {});
      try { if (locked) await rmdir(`${path}/.save-lock`); } finally { await directory.close(); }
    }
  }
  async readAt(directory, id) {
    const file = await open(`/proc/self/fd/${directory.fd}/${id}.json`, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
    try {
      const before = await file.stat({ bigint: true });
      if (!before.isFile() || before.uid !== BigInt(process.getuid()) || (before.mode & 0o077n) !== 0n
        || before.nlink !== 1n || before.size < 1n || before.size > BigInt(MAX_FILE_BYTES)) fail('evidence file must be private, regular, bounded and singly linked.');
      const buffer = Buffer.alloc(MAX_FILE_BYTES + 1);
      let size = 0;
      while (size < buffer.length) {
        const { bytesRead } = await file.read(buffer, size, buffer.length - size, size);
        if (!bytesRead) break;
        size += bytesRead;
      }
      const after = await file.stat({ bigint: true });
      if (size > MAX_FILE_BYTES || ['dev', 'ino', 'size', 'mtimeNs', 'ctimeNs'].some(key => before[key] !== after[key])) fail('evidence changed during bounded read.');
      let value, text;
      try { text = new TextDecoder('utf-8', { fatal: true }).decode(buffer.subarray(0, size)); value = JSON.parse(text); }
      catch { fail('malformed evidence JSON.'); }
      const validated = validateEvidence(value);
      // Requiring our canonical encoding rejects duplicate JSON keys, numeric
      // lexical ambiguity and injected fields before anything is assessed.
      if (text !== serialized(validated) || evidenceDigest(validated) !== id) fail('evidence bytes or content identity changed.');
      return validated;
    } finally { await file.close(); }
  }
  async load(id, { assessment, nowUnixS = Math.floor(Date.now() / 1000) } = {}) {
    if (!isHash(id) || !identityValid(assessment?.identity) || !integer(nowUnixS)) fail('load requires an evidence ID and freshly assessed identity.');
    const directory = await this.directoryHandle(false);
    let evidence;
    try { evidence = await this.readAt(directory, id); } finally { await directory.close(); }
    if (!same(evidence.identity, assessment.identity)) fail('saved evidence is stale for this model/device/settings identity.');
    if (evidence.measured_at_unix_s === 0 || evidence.measured_at_unix_s > nowUnixS
      || nowUnixS - evidence.measured_at_unix_s > MAX_AGE_SECONDS) fail('saved evidence is future-dated or older than 30 days; repeat local measurements.');
    if (Array.isArray(assessment.settings?.languages)
      && !same([...assessment.settings.languages].sort(), evidence.languages.map(row => row.language).sort())) fail('saved evidence language coverage changed.');
    return evidence;
  }
}
