import { timingSafeEqual } from 'node:crypto';
import { ContextLookupWorker, validateContextLookupRequest, validateContextLookupResult } from './context-lookup-worker.mjs';

const ROUTES = new Set(['/api/context-lookup', '/api/context-lookup/reset', '/api/context-lookup/status']);
export const isContextLookupRoute = path => ROUTES.has(path);
const error = code => Object.assign(new Error(code), { code });
const CSP = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

async function bodyJSON(request) {
  const chunks = []; let bytes = 0;
  for await (const chunk of request) {
    bytes += chunk.length;
    if (bytes > 64 * 1024) throw error('context_lookup_invalid_input');
    chunks.push(chunk);
  }
  try { return JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(Buffer.concat(chunks))); }
  catch { throw error('context_lookup_invalid_input'); }
}

async function abortable(operation, signal) {
  let rejectAbort;
  const cancelled = new Promise((_, reject) => { rejectAbort = reject; });
  const abort = () => rejectAbort(error('context_lookup_cancelled'));
  signal.addEventListener('abort', abort, { once: true });
  if (signal.aborted) abort();
  try { return await Promise.race([operation, cancelled]); }
  finally { signal.removeEventListener('abort', abort); }
}

export function createContextLookupHandler({ origin, capability, isOtherBusy = () => false,
  workerFactory = () => new ContextLookupWorker() } = {}) {
  if (typeof origin !== 'function' || !/^[a-f0-9]{64}$/u.test(capability ?? '')) throw error('context_lookup_invalid_configuration');
  let active = null; let generation = 0; let resets = 0; let closed = false; let fault = null;
  const stop = async job => {
    if (!job?.worker) return null;
    job.stopping ??= job.worker.stop().catch(() => { fault = 'context_lookup_cleanup_unverified'; throw error(fault); });
    return job.stopping;
  };
  const reset = async () => {
    generation++; resets++;
    const job = active;
    job?.controller.abort();
    try { const cleanup = await stop(job); await job?.done; return cleanup; }
    finally { resets--; }
  };
  return {
    get busy() { return Boolean(active || resets || fault); },
    async handle(request, response) {
      if (!isContextLookupRoute(request.url)) return false;
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
      try {
        const expectedOrigin = origin();
        const token = Buffer.from(String(request.headers['x-badi-lab-token'] ?? ''));
        if (request.headers.host !== new URL(expectedOrigin).host
          || (request.headers.origin && request.headers.origin !== expectedOrigin)
          || token.length !== capability.length || !timingSafeEqual(token, Buffer.from(capability))) {
          send(403, { error: 'context_lookup_forbidden' }); return true;
        }
        if (request.method === 'GET' && request.url === '/api/context-lookup/status') {
          send(200, { schema: 'badi.context-lookup.status.v1', busy: Boolean(active || resets || fault) }); return true;
        }
        if (request.method !== 'POST' || request.headers['content-type'] !== 'application/json') throw error('context_lookup_invalid_input');
        if (request.url === '/api/context-lookup/reset') {
          const cleanup = await reset(); send(200, { schema: 'badi.context-lookup.reset.v1', cleanup }); return true;
        }
        if (request.url !== '/api/context-lookup') { send(404, { error: 'context_lookup_unknown_operation' }); return true; }
        if (closed || fault) { send(503, { error: fault ?? 'context_lookup_closed' }); return true; }
        if (active || resets || isOtherBusy()) { send(409, { error: 'context_lookup_busy' }); return true; }
        const uploadGeneration = generation;
        const input = validateContextLookupRequest(await bodyJSON(request));
        if (uploadGeneration !== generation || resets) { send(409, { error: 'context_lookup_cancelled' }); return true; }
        if (closed || fault || active || isOtherBusy()) { send(409, { error: 'context_lookup_busy' }); return true; }
        const controller = new AbortController(); let finish;
        const job = { controller, worker: null, done: new Promise(resolveDone => { finish = resolveDone; }) };
        active = job;
        const disconnect = () => { if (!response.writableEnded) controller.abort(); };
        response.once('close', disconnect);
        const started = performance.now();
        try {
          job.worker = workerFactory();
          const output = await abortable(job.worker.check(input, controller.signal), controller.signal);
          validateContextLookupResult(output.result, input);
          if (!output.cleanup || output.cleanup.reaped !== true || output.cleanup.exit_code !== 0
            || output.cleanup.forced !== false || !Number.isInteger(output.cleanup.process_id) || output.cleanup.process_id <= 1) {
            throw error('context_lookup_cleanup_unverified');
          }
          if (!/^[a-f0-9]{64}$/u.test(output.worker?.sha256) || !Number.isInteger(output.worker?.bytes) || output.worker.bytes <= 0) {
            throw error('context_lookup_invalid_identity');
          }
          if (controller.signal.aborted || active !== job || generation !== uploadGeneration) throw error('context_lookup_cancelled');
          send(200, { schema: 'badi.context-lookup.response.v1', result: output.result,
            elapsed_ms: performance.now() - started, worker: { sha256: output.worker.sha256, bytes: output.worker.bytes },
            cleanup: { process_id: output.cleanup.process_id, reaped: true, exit_code: 0, forced: false } });
        } catch (failure) {
          try { await stop(job); } catch { failure = error('context_lookup_cleanup_unverified'); }
          if (failure.code === 'context_lookup_cleanup_unverified') fault = failure.code;
          const code = fault ?? (controller.signal.aborted ? 'context_lookup_cancelled'
            : ['context_lookup_invalid_input', 'context_lookup_deadline'].includes(failure.code) ? failure.code : 'context_lookup_worker_unavailable');
          send(code === 'context_lookup_cancelled' ? 409 : code === 'context_lookup_invalid_input' ? 400 : 503, { error: code });
        } finally {
          response.off('close', disconnect); if (active === job) active = null; finish();
        }
      } catch (failure) {
        const code = failure.code === 'context_lookup_cleanup_unverified' ? failure.code : 'context_lookup_invalid_input';
        send(code === 'context_lookup_invalid_input' ? 400 : 503, { error: code });
      }
      return true;
    },
    async close() { closed = true; return reset(); },
  };
}
