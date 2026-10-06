import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { constants } from 'node:fs';
import { open, realpath, stat } from 'node:fs/promises';
import { createInterface } from 'node:readline';
import { isAbsolute, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { isDeepStrictEqual } from 'node:util';
import { setTimeout as delay } from 'node:timers/promises';
import { processIdentity } from './worker.mjs';

const BINARY = fileURLToPath(new URL('../../../target/release/badi-writing-lab', import.meta.url));
const HASH = /^[a-f0-9]{64}$/u;
const ARABIC_JOINER_LETTER = /^[\u0620-\u063f\u0641-\u064a\u066e-\u066f\u0671-\u06d3\u06d5\u06e5-\u06e6\u06ee-\u06ef\u06fa-\u06fc\u06ff]$/u;
const identityKeys = ['contract_id', 'manifest_sha256', 'engine_sha256', 'engine_size', 'library_sha256',
  'library_size', 'language', 'dictionary_id', 'aff_sha256', 'aff_size', 'dic_sha256', 'dic_size'];
const digest = value => createHash('sha256').update(value).digest('hex');
const keys = (v, expected) => v && typeof v === 'object' && !Array.isArray(v)
  && JSON.stringify(Object.keys(v).sort()) === JSON.stringify([...expected].sort());
const same = isDeepStrictEqual;
const number = n => Number.isFinite(n) && n >= 0;
const fail = code => { const error = new Error(code); error.code = code; throw error; };
const check = (condition, code = 'spelling_invalid_input') => { if (!condition) fail(code); };

function validText(value, limit) {
  if (typeof value !== 'string' || !value.isWellFormed() || [...value].length > limit
    || value.includes('<|') || /[\u0000-\u0008\u000b-\u001f\u007f-\u009f\u202a-\u202e\u2066-\u2069\u200b\ufeff]/u.test(value)) return false;
  const chars = [...value];
  return chars.every((c, i) => c !== '\u200c' || (ARABIC_JOINER_LETTER.test(chars[i - 1] ?? '')
    && ARABIC_JOINER_LETTER.test(chars[i + 1] ?? '')));
}

export function spellingLanguage(language) {
  check(typeof language === 'string' && language.length <= 35 && /^(de|fa)(?:-[a-zA-Z0-9]+)*$/iu.test(language));
  return language.split('-')[0].toLowerCase();
}

function validWord(word, language) {
  if (!validText(word, 24) || !word.length) return false;
  let sawLetter = false; let base = false;
  for (const scalar of word) {
    if (!/[\p{Alphabetic}\u200c\u0300-\u036f\u064b-\u065f\u0670]/u.test(scalar)) return false;
    if (language === 'de' && /\p{Script=Latin}/u.test(scalar)) { sawLetter = true; base = true; }
    else if (language === 'fa' && /\p{Script=Arabic}/u.test(scalar) && /\p{Alphabetic}/u.test(scalar)) {
      sawLetter = true; base = true;
    } else if (base && (language === 'de' ? /^[\u0300-\u036f]$/u : /^[\u064b-\u065f\u0670]$/u).test(scalar)) {
      // Combining marks retain the preceding script base, as in the native policy.
    } else if (language === 'fa' && scalar === '\u200c') base = false;
    else return false;
  }
  return sawLetter;
}

function wordCase(word) {
  const letters = [...word].filter(c => /[\p{Lowercase}\p{Uppercase}]/u.test(c));
  if (!letters.length) return 'uncased';
  if (letters.every(c => /\p{Lowercase}/u.test(c))) return 'lower';
  if (letters.every(c => /\p{Uppercase}/u.test(c))) return 'upper';
  if (/\p{Uppercase}/u.test(letters[0]) && letters.slice(1).every(c => /\p{Lowercase}/u.test(c))) return 'title';
  return null;
}

function singleEdit(original, corrected) {
  const a = [...original]; const b = [...corrected];
  if (original === corrected || a[0] !== b[0]) return false;
  let at = 0;
  while (at < Math.min(a.length, b.length) && a[at] === b[at]) at++;
  const tail = (left, right) => a.slice(left).join('') === b.slice(right).join('');
  if (a.length === b.length) return (a[at] !== '\u200c' && b[at] !== '\u200c' && tail(at + 1, at + 1))
    || (at + 1 < a.length && a[at] !== '\u200c' && a[at + 1] !== '\u200c'
      && a[at] === b[at + 1] && a[at + 1] === b[at] && tail(at + 2, at + 2));
  if (a.length === b.length + 1) return a[at] !== '\u200c' && tail(at + 1, at);
  return b.length === a.length + 1 && b[at] !== '\u200c' && tail(at, at + 1);
}

export function validateSpellingRequest(value) {
  check(keys(value, ['schema', 'id', 'before', 'language', ...(Object.hasOwn(value ?? {}, 'protected_words') ? ['protected_words'] : [])]));
  check(value.schema === 'badi.spelling-lab.request.v1' && typeof value.id === 'string'
    && Buffer.byteLength(value.id) >= 1 && Buffer.byteLength(value.id) <= 128 && !/\p{Cc}/u.test(value.id));
  const language = spellingLanguage(value.language);
  check(validText(value.before, 2048));
  const protectedWords = value.protected_words ?? [];
  check(Array.isArray(protectedWords) && protectedWords.length <= 32
    && protectedWords.every(word => typeof word === 'string' && validWord(word, language)));
  return { schema: value.schema, id: value.id, before: value.before, language: value.language,
    protected_words: [...protectedWords] };
}

function artifactPath(path) {
  check(typeof path === 'string' && path.isWellFormed() && isAbsolute(path) && resolve(path) === path
    && Buffer.byteLength(path) <= 4096 && !/\p{Cc}/u.test(path), 'spelling_invalid_manifest');
}
function artifactFile(value) {
  check(keys(value, ['path', 'sha256', 'bytes']) && HASH.test(value.sha256)
    && Number.isSafeInteger(value.bytes) && value.bytes >= 1 && value.bytes <= 64 * 1024 * 1024,
  'spelling_invalid_manifest');
  artifactPath(value.path);
}
export function validateSpellingManifest(value) {
  check(keys(value, ['schema', 'binary', 'library', 'dictionaries']) && value.schema === 'badi.spelling-artifact.v1'
    && keys(value.dictionaries, ['de', 'fa']), 'spelling_invalid_manifest');
  artifactFile(value.binary); artifactFile(value.library);
  for (const dictionary of Object.values(value.dictionaries)) {
    check(keys(dictionary, ['id', 'aff', 'dic']) && typeof dictionary.id === 'string'
      && /^[a-zA-Z0-9][a-zA-Z0-9._-]{0,95}$/u.test(dictionary.id), 'spelling_invalid_manifest');
    artifactFile(dictionary.aff); artifactFile(dictionary.dic);
    check(!dictionary.aff.path.includes(',') && !dictionary.dic.path.includes(','), 'spelling_invalid_manifest');
  }
  return value;
}

export async function readSpellingManifest(path) {
  try {
    artifactPath(path);
    check(await realpath(path) === path, 'spelling_invalid_manifest');
    const file = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
    try {
      const before = await file.stat({ bigint: true });
      check(before.isFile() && before.size <= 16n * 1024n, 'spelling_invalid_manifest');
      const buffer = Buffer.alloc(16 * 1024 + 1);
      let length = 0;
      while (length < buffer.length) {
        const { bytesRead } = await file.read(buffer, length, buffer.length - length, length);
        if (!bytesRead) break;
        length += bytesRead;
      }
      const bytes = buffer.subarray(0, length);
      const after = await file.stat({ bigint: true });
      check(bytes.length <= 16 * 1024 && ['dev', 'ino', 'size', 'mtimeNs', 'ctimeNs'].every(k => before[k] === after[k]),
        'spelling_invalid_manifest');
      const descriptor = validateSpellingManifest(JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes)));
      for (const item of [descriptor.binary, descriptor.library, ...Object.values(descriptor.dictionaries).flatMap(d => [d.aff, d.dic])]) {
        check(await realpath(item.path) === item.path && (await stat(item.path)).isFile(), 'spelling_invalid_manifest');
      }
      return { path, sha256: digest(bytes), descriptor };
    } finally { await file.close(); }
  } catch { fail('spelling_invalid_manifest'); }
}

export function expectedSpellingIdentity(manifest, language) {
  const m = manifest.descriptor; const d = m.dictionaries[language];
  return { contract_id: 'badi.spelling-lab.hunspell.v1', manifest_sha256: manifest.sha256,
    engine_sha256: m.binary.sha256, engine_size: m.binary.bytes,
    library_sha256: m.library.sha256, library_size: m.library.bytes, language, dictionary_id: d.id,
    aff_sha256: d.aff.sha256, aff_size: d.aff.bytes, dic_sha256: d.dic.sha256, dic_size: d.dic.bytes };
}
export function spellingIdentityHash(identity) {
  check(keys(identity, identityKeys), 'spelling_invalid_identity');
  return digest(JSON.stringify(Object.fromEntries(identityKeys.map(k => [k, identity[k]]))));
}

export function validateSpellingResult(result, request, identity) {
  const required = ['type', 'schema', 'id', 'outcome', 'reason', 'text', 'replace_before',
    'filtered_candidate_count', 'query_count', 'query_ms', 'latency_ms', 'identity', 'identity_sha256', 'warnings'];
  check(keys(result, required) && result.type === 'result' && result.schema === 'badi.spelling-lab.result.v1'
    && result.id === request.id && same(result.identity, identity)
    && result.identity_sha256 === spellingIdentityHash(identity), 'spelling_invalid_result');
  check(['suggestion', 'abstention', 'deadline'].includes(result.outcome)
    && typeof result.reason === 'string' && /^[a-z0-9_]{1,96}$/u.test(result.reason)
    && Number.isInteger(result.filtered_candidate_count) && result.filtered_candidate_count >= 0 && result.filtered_candidate_count <= 128
    && Number.isInteger(result.query_count) && result.query_count >= 0 && result.query_count <= 2
    && number(result.query_ms) && number(result.latency_ms)
    && same(result.warnings, ['returned_candidates_not_exhaustive', 'unknown_proper_names_not_detected']),
  'spelling_invalid_result');
  if (result.outcome === 'suggestion') {
    const language = spellingLanguage(request.language);
    const bounded = value => typeof value === 'string' && value.endsWith(' ')
      && validWord(value.slice(0, -1), language)
      && [...value.slice(0, -1)].length >= 3 && [...value.slice(0, -1)].length <= 24;
    check(bounded(result.text) && bounded(result.replace_before)
      && request.before.endsWith(result.replace_before)
      && (request.before === result.replace_before || /\p{White_Space}$/u.test(request.before.slice(0, -result.replace_before.length))),
    'spelling_invalid_result');
    const original = result.replace_before.slice(0, -1); const corrected = result.text.slice(0, -1);
    check(!(request.protected_words ?? []).includes(original) && wordCase(original) !== null
      && wordCase(original) === wordCase(corrected) && singleEdit(original, corrected)
      && result.filtered_candidate_count === 1 && result.query_count === 2 && result.latency_ms <= 550
      && result.reason === 'single_admissible_returned_candidate', 'spelling_invalid_result');
  } else check(result.text === null && result.replace_before === null, 'spelling_invalid_result');
  return result;
}

export function validateSpellingCleanup(value, engine, identity) {
  check(keys(value, ['process_id', 'identity_sha256', 'reaped', 'exit_code', 'forced'])
    && value.process_id === engine.pid && value.identity_sha256 === spellingIdentityHash(identity)
    && value.reaped === true && typeof value.forced === 'boolean'
    && (value.exit_code === null || Number.isInteger(value.exit_code)), 'spelling_cleanup_unverified');
  return value;
}

async function verifyEngine(ready, workerPid, manifest, language, capture) {
  const expected = expectedSpellingIdentity(manifest, language);
  check(keys(ready, ['type', 'schema', 'engine_pid', 'identity', 'identity_sha256'])
    && ready.type === 'ready' && ready.schema === 'badi.spelling-lab.worker.v1'
    && same(ready.identity, expected) && ready.identity_sha256 === spellingIdentityHash(expected), 'spelling_invalid_identity');
  const engine = await processIdentity(ready.engine_pid);
  check(engine && engine.parent === workerPid && engine.group === engine.pid
    && engine.executable === manifest.descriptor.binary.path, 'spelling_invalid_ownership');
  capture(engine);
  const file = await open(`/proc/${engine.pid}/exe`, 'r');
  try {
    const metadata = await file.stat();
    check(metadata.size === expected.engine_size, 'spelling_invalid_identity');
    const hash = createHash('sha256');
    for await (const chunk of file.createReadStream({ autoClose: false })) hash.update(chunk);
    check(hash.digest('hex') === expected.engine_sha256, 'spelling_invalid_identity');
  } finally { await file.close(); }
  const current = await processIdentity(engine.pid);
  check(current && same(current, engine), 'spelling_invalid_ownership');
  return engine;
}

async function retireEngine(engine) {
  if (!engine) return;
  const owned = async () => {
    const current = await processIdentity(engine.pid);
    if (!current || current.start !== engine.start) return false;
    check(current.group === engine.pid && current.executable === engine.executable
      && current.executable_key === engine.executable_key, 'spelling_cleanup_unverified');
    return true;
  };
  for (const [signal, attempts] of [['SIGTERM', 20], ['SIGKILL', 10]]) {
    if (!await owned()) return;
    try { process.kill(-engine.pid, signal); } catch (error) { if (error.code !== 'ESRCH') fail('spelling_cleanup_unverified'); }
    for (let i = 0; i < attempts && await owned(); i++) await delay(50);
  }
  check(!await owned(), 'spelling_cleanup_unverified');
}

// This transport starts only the distinct native dispatch. No model arguments,
// personal dictionaries, typed-text log, or browser-selected paths are accepted.
export class SpellingWorker {
  constructor({ manifest, language, executable = BINARY, startupMs = 5000, requestMs = 1500,
    spawnProcess = spawn } = {}) {
    check(language === 'de' || language === 'fa');
    check(manifest?.path && manifest?.descriptor, 'spelling_not_configured');
    validateSpellingManifest(manifest.descriptor);
    this.manifest = manifest; this.language = language; this.executable = resolve(executable);
    this.startupMs = startupMs; this.requestMs = requestMs; this.spawnProcess = spawnProcess;
    this.session = null; this.cleanupError = null; this.lastCleanup = null; this.generation = 0;
  }
  get ready() { return this.session?.readyMessage ?? null; }
  get identity() { return this.ready?.identity ?? null; }

  async start() {
    if (this.cleanupError) throw this.cleanupError;
    if (this.session?.closed) { await this.session.exit; return this.start(); }
    if (this.session) { check(!this.session.stopping, 'spelling_busy'); return this.session.ready; }
    const generation = this.generation;
    const current = await readSpellingManifest(this.manifest.path);
    check(current.sha256 === this.manifest.sha256, 'spelling_manifest_changed');
    check(generation === this.generation, 'spelling_cancelled');
    // Recheck after the asynchronous manifest read so concurrent starts share one worker.
    if (this.session) return this.session.ready;
    const child = this.spawnProcess(this.executable, ['--spelling-lab', '--spelling-manifest', this.manifest.path,
      '--language', this.language], { stdio: ['pipe', 'pipe', 'ignore'] });
    const session = { child, pending: null, closed: false, engine: null, readyMessage: null };
    // Cancellation revokes the caller immediately, while this separate bounded
    // settlement lets a healthy startup identify the child before shutdown.
    session.startupSettled = new Promise(resolveStartup => { session.settleStartup = resolveStartup; });
    this.session = session; this.lastCleanup = null;
    session.exit = new Promise(resolveExit => child.once('close', async () => {
      session.closed = true; clearTimeout(session.timer);
      session.fail?.('spelling_worker_stopped');
      try {
        await session.capture;
        await retireEngine(session.engine);
        if (session.readyMessage) {
          validateSpellingCleanup(session.cleanup, session.engine, session.readyMessage.identity);
          this.lastCleanup = session.cleanup;
        } else if (child.pid) fail('spelling_cleanup_unverified');
      } catch {
        this.cleanupError = new Error('spelling_cleanup_unverified');
      } finally { if (this.session === session) this.session = null; resolveExit(); }
    }));
    session.ready = new Promise((resolveReady, rejectReady) => {
      session.cancel = code => {
        const error = new Error(code); error.code = code;
        rejectReady(error); session.pending?.reject(error); session.pending = null;
      };
      session.fail = code => {
        if (!session.readyMessage) session.startupFailed = true;
        session.settleStartup();
        session.cancel(code);
      };
      const fatal = code => { session.fail(code); void this.stop().catch(() => {}); };
      child.once('error', () => fatal('spelling_worker_unavailable'));
      child.stdin.on('error', () => fatal('spelling_input_closed'));
      let bytes = 0;
      child.stdout.on('data', chunk => {
        bytes += chunk.length;
        if (bytes > 64 * 1024) fatal('spelling_response_too_large');
      });
      const lines = createInterface({ input: child.stdout });
      lines.once('close', () => { if (!session.closed && !session.shutdownSignalled && !session.retiring) fatal('spelling_output_closed'); });
      lines.on('line', line => {
        bytes = 0;
        if (Buffer.byteLength(line) > 64 * 1024) { fatal('spelling_response_too_large'); return; }
        let message;
        try { message = JSON.parse(line); check(message && typeof message === 'object' && !Array.isArray(message), 'spelling_invalid_response'); }
        catch { fatal('spelling_invalid_response'); return; }
        if (message.type === 'ready' && !session.announced) {
          session.announced = true;
          session.capture = (async () => {
            try {
              await verifyEngine(message, child.pid, this.manifest, this.language, engine => { session.engine = engine; });
              if (session.closed || session.startupFailed) return;
              session.readyMessage = message; clearTimeout(session.timer);
              session.settleStartup(); resolveReady(message.identity);
            } catch { session.identityFailed = true; fatal('spelling_invalid_identity'); }
          })();
        } else if (message.type === 'stopped' && (session.stopping || session.retiring)) {
          session.cleanup = message.cleanup;
        } else if (message.type === 'error' && !session.readyMessage) {
          fatal('spelling_startup_failed');
        } else if (session.pending && message.id === session.pending.request.id) {
          const pending = session.pending;
          try {
            if (message.type === 'error') fail('spelling_engine_error');
            validateSpellingResult(message, pending.request, session.readyMessage.identity);
            session.pending = null;
            if (message.outcome === 'deadline') session.retiring = true;
            pending.resolve(message);
          } catch { fatal('spelling_invalid_result'); }
        } else fatal('spelling_unexpected_response');
      });
      session.timer = setTimeout(() => fatal('spelling_startup_timeout'), this.startupMs);
    });
    return session.ready;
  }

  async check(value, signal) {
    const request = validateSpellingRequest(value);
    check(spellingLanguage(request.language) === this.language);
    signal?.throwIfAborted();
    check(!this.checking, 'spelling_busy');
    const turn = Symbol('spelling-request');
    this.checking = turn;
    const abort = () => { void this.stop().catch(() => {}); };
    signal?.addEventListener('abort', abort, { once: true });
    try {
      await this.start(); signal?.throwIfAborted();
      const session = this.session;
      check(session && !session.stopping && !session.retiring && !session.pending, 'spelling_busy');
      const result = await new Promise((resolveResult, rejectResult) => {
        const timer = setTimeout(() => { session.fail('spelling_request_timeout'); void this.stop().catch(() => {}); }, this.requestMs);
        session.pending = { request,
          resolve: value => { clearTimeout(timer); resolveResult(value); },
          reject: error => { clearTimeout(timer); rejectResult(error); } };
        session.child.stdin.write(JSON.stringify(request) + '\n', error => {
          if (error) { session.fail('spelling_input_closed'); void this.stop().catch(() => {}); }
        });
      });
      signal?.throwIfAborted();
      if (result.outcome === 'deadline') await this.stop();
      return result;
    } catch (error) {
      await this.stop();
      throw error;
    } finally {
      signal?.removeEventListener('abort', abort);
      if (this.checking === turn) this.checking = null;
    }
  }

  async stop() {
    this.generation++;
    const session = this.session;
    if (!session) { if (this.cleanupError) throw this.cleanupError; return this.lastCleanup; }
    if (session.stopping) return session.stopping;
    session.stopping = (async () => {
      session.cancel?.('spelling_cancelled');
      // The existing startup watchdog also settles failures. Fatal readiness
      // errors never become successful cleanup merely because stop was requested.
      if (!session.readyMessage && !session.closed) await session.startupSettled;
      session.shutdownSignalled = true;
      if (!session.closed) session.child.kill('SIGTERM');
      const timer = setTimeout(() => { if (!session.closed) session.child.kill('SIGKILL'); }, 4000);
      await session.exit; clearTimeout(timer);
      if (this.cleanupError) throw this.cleanupError;
      return this.lastCleanup;
    })();
    return session.stopping;
  }
}
