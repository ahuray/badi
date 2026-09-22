import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { constants } from 'node:fs';
import { open, realpath } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { isDeepStrictEqual } from 'node:util';
import { processIdentity } from './worker.mjs';

const BINARY = fileURLToPath(new URL('../../../target/release/badi-writing-lab', import.meta.url));
const FRAME_LIMIT = 64 * 1024;
const HASH = /^[a-f0-9]{64}$/u;
const ARABIC_LETTER = /^[\u0620-\u063f\u0641-\u064a\u066e-\u066f\u0671-\u06d3\u06d5\u06e5-\u06e6\u06ee-\u06ef\u06fa-\u06fc\u06ff]$/u;
const error = code => Object.assign(new Error(code), { code });
const requireValue = (condition, code) => { if (!condition) throw error(code); };
const keys = (value, expected) => value && typeof value === 'object' && !Array.isArray(value)
  && isDeepStrictEqual(Object.keys(value).sort(), [...expected].sort());
const REQUEST_KEYS = ['schema', 'id', 'before', 'language', 'context', 'style_examples'];
const separator = scalar => /^[\p{White_Space}.,;:!?()[\]{}"\u201c\u201d\u201e\u00ab\u00bb\u2026\u060c\u061b\u061f]$/u.test(scalar ?? '');

function validText(value, limit) {
  if (typeof value !== 'string' || !value.isWellFormed() || [...value].length > limit || value.includes('<|')
    || /[\u0000-\u0008\u000b-\u001f\u007f-\u009f\u202a-\u202e\u2066-\u2069\u200b\ufeff]/u.test(value)) return false;
  const chars = [...value];
  return chars.every((c, i) => c !== '\u200c' || (ARABIC_LETTER.test(chars[i - 1] ?? '') && ARABIC_LETTER.test(chars[i + 1] ?? '')));
}

function languageWord(word, language) {
  let sawLetter = false; let base = false;
  for (const scalar of word) {
    if ((language === 'fa' ? /\p{Script=Arabic}/u : /\p{Script=Latin}/u).test(scalar) && /\p{Alphabetic}/u.test(scalar)) {
      sawLetter = true; base = true;
    } else if (base && (language === 'fa' ? /^[\u064b-\u065f\u0670]$/u : /^[\u0300-\u036f]$/u).test(scalar)) {
      // A combining mark needs an actual preceding base in the selected script.
    } else if (language === 'fa' && scalar === '\u200c') base = false;
    else return false;
  }
  return sawLetter;
}

export function validateContextLookupRequest(value) {
  requireValue(keys(value, REQUEST_KEYS) && value.schema === 'badi.context-lookup.request.v1'
    && typeof value.id === 'string' && value.id.isWellFormed() && Buffer.byteLength(value.id) >= 1
    && Buffer.byteLength(value.id) <= 128 && !/\p{Cc}/u.test(value.id)
    && validText(value.before, 2048) && value.before.trim().length > 0
    && validText(value.context, 4096) && Array.isArray(value.style_examples) && value.style_examples.length <= 8
    && value.style_examples.every(text => validText(text, 1024) && text.trim().length > 0)
    && typeof value.language === 'string' && value.language.length <= 35
    && /^(en|de|fa)(?:-[a-z0-9]+)*$/iu.test(value.language), 'context_lookup_invalid_input');
  const request = Object.fromEntries(REQUEST_KEYS.map(key => [key, value[key]]));
  request.style_examples = [...value.style_examples];
  requireValue(Buffer.byteLength(JSON.stringify(request) + '\n') <= FRAME_LIMIT, 'context_lookup_invalid_input');
  return request;
}

export function validateContextLookupResult(value, request) {
  requireValue(keys(value, ['schema', 'id', 'contract_id', 'outcome', 'reason', 'text', 'matched_word',
    'matched_candidate_count', 'sources', 'latency_ms', 'warnings'])
    && value.schema === 'badi.context-lookup.result.v1' && value.id === request.id
    && value.contract_id === 'badi.context-lookup.exact.v1' && ['suggestion', 'abstention'].includes(value.outcome)
    && Number.isInteger(value.matched_candidate_count) && value.matched_candidate_count >= 0 && value.matched_candidate_count <= 14336
    && Number.isFinite(value.latency_ms) && value.latency_ms >= 0
    && isDeepStrictEqual(value.warnings, ['context_match_does_not_prove_intent'])
    && Array.isArray(value.sources) && isDeepStrictEqual(value.sources, ['context', 'style', 'draft'].filter(source => value.sources.includes(source))),
  'context_lookup_invalid_result');
  if (value.outcome === 'abstention') {
    requireValue(value.text === null && value.matched_word === null && value.sources.length === 0
      && ['no_eligible_stem', 'no_matching_word', 'ambiguous_matches', 'unsupported_suffix'].includes(value.reason), 'context_lookup_invalid_result');
    requireValue(['no_eligible_stem', 'no_matching_word'].includes(value.reason) ? value.matched_candidate_count === 0
      : value.reason === 'ambiguous_matches' ? value.matched_candidate_count > 1 : value.matched_candidate_count === 1,
    'context_lookup_invalid_result');
  } else {
    requireValue(value.reason === 'unique_source_word' && value.matched_candidate_count === 1 && value.sources.length > 0
      && typeof value.text === 'string' && value.text.length > 0 && [...value.text].length <= 64 && !/\s/u.test(value.text)
      && typeof value.matched_word === 'string' && value.matched_word.isWellFormed()
      && value.matched_word.endsWith(value.text) && value.matched_word !== value.text
      && validText(value.matched_word, 2048), 'context_lookup_invalid_result');
    const stem = value.matched_word.slice(0, -value.text.length);
    const language = request.language.split('-')[0].toLowerCase();
    requireValue(request.before.endsWith(stem) && [...stem].length >= 3 && [...stem].length <= 24
      && separator([...request.before.slice(0, -stem.length)].at(-1))
      && /\p{Alphabetic}/u.test([...stem].at(-1)) && /\p{Alphabetic}/u.test([...value.text][0])
      && /^[\p{Alphabetic}\u200c\u0300-\u036f\u064b-\u065f\u0670]+$/u.test(value.matched_word)
      && validText(value.text, 64) && languageWord(value.matched_word, language) && languageWord(value.text, language),
    'context_lookup_invalid_result');
    const closed = (source, allowStart) => {
      for (let start = source.indexOf(value.matched_word); start >= 0; start = source.indexOf(value.matched_word, start + 1)) {
        if ((start === 0 ? allowStart : separator([...source.slice(0, start)].at(-1)))
          && separator([...source.slice(start + value.matched_word.length)][0])) return true;
      }
      return false;
    };
    requireValue(isDeepStrictEqual(value.sources, [closed(request.context, true) && 'context',
      request.style_examples.some(source => closed(source, true)) && 'style', closed(request.before, false) && 'draft'].filter(Boolean)),
    'context_lookup_invalid_result');
  }
  return value;
}

const fileKey = info => ['dev', 'ino', 'size', 'mtimeNs', 'ctimeNs'].map(key => String(info[key])).join(':');
async function executableIdentity(path, noFollow = false) {
  const file = await open(path, constants.O_RDONLY | constants.O_NONBLOCK | (noFollow ? constants.O_NOFOLLOW : 0));
  try {
    const before = await file.stat({ bigint: true });
    requireValue(before.isFile() && before.size > 0 && before.size <= 64n * 1024n * 1024n, 'context_lookup_worker_unavailable');
    const hash = createHash('sha256'); let bytes = 0;
    for await (const chunk of file.createReadStream({ autoClose: false })) {
      bytes += chunk.length;
      requireValue(bytes <= 64 * 1024 * 1024, 'context_lookup_worker_unavailable');
      hash.update(chunk);
    }
    requireValue(BigInt(bytes) === before.size && fileKey(before) === fileKey(await file.stat({ bigint: true })), 'context_lookup_worker_changed');
    return { bytes, sha256: hash.digest('hex') };
  } finally { await file.close(); }
}

// A request owns one direct child. Only --context-lookup is dispatched, and the
// verified executable receives the bounded frame only after ownership checks.
export class ContextLookupWorker {
  constructor({ executable = BINARY, expectedWorker, timeoutMs = 3000, killMs = 1000, spawnProcess = spawn } = {}) {
    requireValue(Number.isInteger(timeoutMs) && timeoutMs > 0 && timeoutMs <= 10000
      && Number.isInteger(killMs) && killMs > 0 && killMs <= 4000, 'context_lookup_invalid_configuration');
    if (expectedWorker) requireValue(keys(expectedWorker, ['sha256', 'bytes']) && HASH.test(expectedWorker.sha256)
      && Number.isInteger(expectedWorker.bytes) && expectedWorker.bytes > 0, 'context_lookup_invalid_configuration');
    this.executable = resolve(executable); this.expectedWorker = expectedWorker; this.timeoutMs = timeoutMs;
    this.killMs = killMs; this.spawnProcess = spawnProcess; this.active = null; this.lastCleanup = null; this.cleanupError = null;
  }

  async check(value, signal) {
    const request = validateContextLookupRequest(value);
    requireValue(!this.active, 'context_lookup_busy');
    if (this.cleanupError) throw this.cleanupError;
    signal?.throwIfAborted();
    const controller = new AbortController();
    const operation = { controller, child: null, closed: false, forced: false, failure: null, written: false };
    operation.done = new Promise(resolveDone => { operation.finish = resolveDone; });
    this.active = operation; this.lastCleanup = null;
    const abort = code => {
      operation.failure ??= error(code); controller.abort();
      if (operation.child && !operation.closed) operation.child.kill('SIGTERM');
    };
    operation.abort = abort;
    const externalAbort = () => abort('context_lookup_cancelled');
    signal?.addEventListener('abort', externalAbort, { once: true });
    const timer = setTimeout(() => abort('context_lookup_deadline'), this.timeoutMs);
    const fresh = () => { if (controller.signal.aborted) throw operation.failure; };
    let result; let worker; let failure;
    try {
      const path = await realpath(this.executable);
      worker = await executableIdentity(path, true);
      if (this.expectedWorker) requireValue(isDeepStrictEqual(worker, this.expectedWorker), 'context_lookup_worker_changed');
      fresh();
      const child = this.spawnProcess(path, ['--context-lookup'], { stdio: ['pipe', 'pipe', 'ignore'] });
      operation.child = child;
      const chunks = []; let bytes = 0;
      const spawned = new Promise((resolveSpawn, rejectSpawn) => {
        child.once('spawn', resolveSpawn);
        child.once('error', () => { abort('context_lookup_worker_unavailable'); rejectSpawn(operation.failure); });
      });
      operation.exit = new Promise(resolveExit => child.once('close', (code, exitSignal) => {
        operation.closed = true; operation.exitCode = code; operation.exitSignal = exitSignal; resolveExit();
      }));
      child.stdout.on('data', chunk => {
        bytes += chunk.length;
        if (!operation.written || bytes > FRAME_LIMIT) { abort('context_lookup_invalid_result'); return; }
        chunks.push(chunk);
      });
      child.stdout.on('error', () => abort('context_lookup_invalid_result'));
      child.stdin.on('error', () => abort('context_lookup_input_closed'));
      await spawned; fresh();
      const owned = await processIdentity(child.pid);
      requireValue(owned && owned.parent === process.pid && owned.executable === path, 'context_lookup_invalid_identity');
      const executed = await executableIdentity(`/proc/${child.pid}/exe`);
      requireValue(isDeepStrictEqual(worker, executed) && isDeepStrictEqual(owned, await processIdentity(child.pid)), 'context_lookup_invalid_identity');
      fresh();
      operation.written = true;
      child.stdin.end(JSON.stringify(request) + '\n');
      const aborted = new Promise((_, reject) => controller.signal.addEventListener('abort', () => reject(operation.failure), { once: true }));
      await Promise.race([operation.exit, aborted]); fresh();
      const text = new TextDecoder('utf-8', { fatal: true }).decode(Buffer.concat(chunks));
      requireValue(text.endsWith('\n') && text.indexOf('\n') === text.length - 1, 'context_lookup_invalid_result');
      const response = JSON.parse(text);
      if (response?.schema === 'badi.context-lookup.error.v1') {
        requireValue(keys(response, ['schema', 'error']) && operation.exitCode === 1
          && ['invalid_arguments', 'invalid_frame', 'invalid_request'].includes(response.error), 'context_lookup_invalid_result');
        throw error('context_lookup_invalid_input');
      }
      requireValue(operation.exitCode === 0 && operation.exitSignal === null, 'context_lookup_worker_failed');
      result = validateContextLookupResult(response, request);
    } catch (caught) {
      failure = operation.failure ?? (caught.code?.startsWith('context_lookup_') ? caught : error('context_lookup_worker_unavailable'));
    } finally {
      clearTimeout(timer); signal?.removeEventListener('abort', externalAbort);
      try {
        if (operation.child) {
          let force;
          if (!operation.closed) {
            operation.child.kill('SIGTERM');
            force = setTimeout(() => { if (!operation.closed) { operation.forced = true; operation.child.kill('SIGKILL'); } }, this.killMs);
          }
          await operation.exit; clearTimeout(force);
          requireValue(!operation.child.pid || await processIdentity(operation.child.pid) === null, 'context_lookup_cleanup_unverified');
          if (operation.child.pid) this.lastCleanup = { process_id: operation.child.pid, reaped: true,
            exit_code: operation.exitCode, forced: operation.forced };
        }
      } catch { this.cleanupError = error('context_lookup_cleanup_unverified'); failure = this.cleanupError; }
      if (this.active === operation) this.active = null;
      operation.finish();
    }
    if (failure || operation.failure) throw failure ?? operation.failure;
    return { result, worker, cleanup: this.lastCleanup };
  }

  async stop() {
    const operation = this.active;
    if (operation) { operation.abort('context_lookup_cancelled'); await operation.done; }
    if (this.cleanupError) throw this.cleanupError;
    return this.lastCleanup;
  }
}
