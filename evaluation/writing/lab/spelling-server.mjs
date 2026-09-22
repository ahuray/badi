import { randomUUID, timingSafeEqual } from 'node:crypto';
import { SpellingWorker, readSpellingManifest, spellingLanguage, validateSpellingRequest,
  validateSpellingResult } from './spelling-worker.mjs';

const ROUTES = new Set(['/api/spelling', '/api/spelling/reset', '/api/spelling/status']);
export const isSpellingRoute = path => ROUTES.has(path);
const CSP = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";
const errorCode = code => Object.assign(new Error(code), { code });
const abortable = async (operation, signal) => {
  let rejectAbort;
  const aborted = new Promise((_, reject) => { rejectAbort = reject; });
  const abort = () => rejectAbort(errorCode('spelling_cancelled'));
  signal.addEventListener('abort', abort, { once: true });
  if (signal.aborted) abort();
  try { return await Promise.race([operation, aborted]); }
  finally { signal.removeEventListener('abort', abort); }
};

async function bodyJSON(request) {
  const chunks = []; let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    if (size > 64 * 1024) throw errorCode('spelling_invalid_input');
    chunks.push(chunk);
  }
  try { return JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(Buffer.concat(chunks))); }
  catch { throw errorCode('spelling_invalid_input'); }
}

export function spellingWorkerRequest(body) {
  if (!body || typeof body !== 'object' || Array.isArray(body)
    || Object.keys(body).some(k => !['before', 'language', 'protected_words'].includes(k))) {
    throw errorCode('spelling_invalid_input');
  }
  return validateSpellingRequest({ schema: 'badi.spelling-lab.request.v1', id: randomUUID(),
    before: body.before, language: body.language, protected_words: body.protected_words ?? [] });
}

// Host, Origin, capability, and response headers are enforced here as well as
// by the containing Lab server. Configuration is supplied only by local startup.
export async function createSpellingHandler({ origin, capability, manifests = {},
  isPredictionBusy = () => false, workerFactory = options => new SpellingWorker(options),
  loadManifest = readSpellingManifest } = {}) {
  if (typeof origin !== 'function' || !/^[a-f0-9]{64}$/u.test(capability ?? '')
    || Object.keys(manifests).some(language => !['de', 'fa'].includes(language))) {
    throw errorCode('spelling_invalid_configuration');
  }
  const configured = new Map();
  for (const language of ['de', 'fa']) {
    if (manifests[language] !== undefined) configured.set(language, await loadManifest(manifests[language]));
  }
  let selected = null; let active = null; let generation = 0; let resets = 0;
  let closed = false; let fault = null;

  async function retire(slot = selected) {
    if (!slot) return null;
    if (!slot.stopping) slot.stopping = (async () => {
      try { return await slot.worker.stop(); }
      catch { fault = 'spelling_cleanup_unverified'; throw errorCode(fault); }
    })();
    const receipt = await slot.stopping;
    if (selected === slot) selected = null;
    return receipt;
  }
  async function reset() {
    generation++; resets++;
    const pending = active;
    pending?.controller.abort();
    try { const cleanup = await retire(); await pending?.done; return cleanup; }
    finally { resets--; }
  }

  return {
    get busy() { return Boolean(active || resets || fault); },
    async handle(request, response) {
      if (!ROUTES.has(request.url)) return false;
      response.setHeader('Cache-Control', 'no-store');
      response.setHeader('X-Content-Type-Options', 'nosniff');
      response.setHeader('Cross-Origin-Resource-Policy', 'same-origin');
      response.setHeader('Referrer-Policy', 'no-referrer');
      response.setHeader('Content-Security-Policy', CSP);
      const send = (status, value) => {
        if (response.destroyed || response.writableEnded) return;
        response.writeHead(status, { 'Content-Type': 'application/json; charset=utf-8' });
        response.end(JSON.stringify(value));
      };
      const reject = (status, error) => send(status, { error });
      try {
        const localOrigin = origin();
        if (request.headers.host !== new URL(localOrigin).host
          || (request.headers.origin && request.headers.origin !== localOrigin)) {
          reject(403, 'spelling_forbidden'); return true;
        }
        const token = Buffer.from(String(request.headers['x-badi-lab-token'] ?? ''));
        if (token.length !== capability.length || !timingSafeEqual(token, Buffer.from(capability))) {
          reject(403, 'spelling_forbidden'); return true;
        }
        if (request.method === 'GET' && request.url === '/api/spelling/status') {
          send(200, { schema: 'badi.spelling-lab.status.v1', configured_languages: [...configured.keys()],
            busy: Boolean(active || resets || fault), ready: selected?.worker.ready ?? null }); return true;
        }
        if (request.method !== 'POST' || request.headers['content-type'] !== 'application/json') {
          reject(400, 'spelling_invalid_input'); return true;
        }
        if (request.url === '/api/spelling/reset') {
          const cleanup = await reset();
          send(200, { schema: 'badi.spelling-lab.reset.v1', cleanup }); return true;
        }
        if (request.url !== '/api/spelling') { reject(404, 'spelling_unknown_operation'); return true; }
        if (closed || fault) { reject(503, fault ?? 'spelling_closed'); return true; }
        if (active || resets || isPredictionBusy()) { reject(409, 'spelling_busy'); return true; }
        // An upload started before reset cannot acquire the new generation.
        const uploadGeneration = generation;
        const requestBody = spellingWorkerRequest(await bodyJSON(request));
        const language = spellingLanguage(requestBody.language);
        if (uploadGeneration !== generation || resets) { reject(409, 'spelling_cancelled'); return true; }
        if (!configured.has(language)) { reject(503, 'spelling_not_configured'); return true; }
        // Body acquisition yielded; claim capacity synchronously after rechecking.
        if (closed || fault || active || isPredictionBusy()) { reject(409, 'spelling_busy'); return true; }
        const controller = new AbortController();
        let finish;
        const job = { controller, done: new Promise(resolve => { finish = resolve; }) };
        active = job;
        const disconnected = () => { if (!response.writableEnded) controller.abort(); };
        response.once('close', disconnected);
        const started = performance.now();
        try {
          if (selected && selected.language !== language) await retire();
          controller.signal.throwIfAborted();
          if (!selected) selected = { language,
            worker: workerFactory({ language, manifest: configured.get(language) }), stopping: null };
          const slot = selected;
          await abortable(slot.worker.start(), controller.signal);
          const ready = structuredClone(slot.worker.ready);
          const result = await abortable(slot.worker.check(requestBody, controller.signal), controller.signal);
          validateSpellingResult(result, requestBody, ready?.identity);
          if (active !== job || controller.signal.aborted || generation !== uploadGeneration) {
            throw errorCode('spelling_cancelled');
          }
          send(200, { schema: 'badi.spelling-lab.response.v1', ready, result,
            elapsed_ms: performance.now() - started, cleanup: result.outcome === 'deadline' ? slot.worker.lastCleanup : null });
        } catch (error) {
          try { await retire(); }
          catch { error = errorCode('spelling_cleanup_unverified'); }
          const code = error.code === 'spelling_cleanup_unverified' ? error.code
            : controller.signal.aborted ? 'spelling_cancelled'
            : ['spelling_cleanup_unverified', 'spelling_not_configured', 'spelling_invalid_input'].includes(error.code)
              ? error.code : 'spelling_engine_unavailable';
          reject(code === 'spelling_cancelled' ? 409 : 503, code);
        } finally {
          response.off('close', disconnected);
          if (active === job) active = null;
          finish();
        }
      } catch (error) {
        reject(error.code === 'spelling_cleanup_unverified' ? 503 : 400,
          error.code === 'spelling_cleanup_unverified' ? error.code : 'spelling_invalid_input');
      }
      return true;
    },
    async close() { closed = true; return reset(); },
  };
}
