import { fromLabPage, readJSON, sendJSON, setSecurityHeaders } from './http.mjs';
import { DiscoveryService } from './discovery.mjs';

const OPERATIONS = { search: ['query', 'cursor'], inspect: ['model_id'], assess: ['candidate_id'], download: ['candidate_id'], select: ['artifact_id'], reset: [] };
const ROUTES = new Set(['status', ...Object.keys(OPERATIONS)].map(value => `/api/discovery/${value}`));
export const isDiscoveryRoute = path => ROUTES.has(path);
const error = (code, message = code) => Object.assign(new Error(message), { code });

async function inputJSON(request, operation) {
  const value = await readJSON(request, 2048, {
    tooLarge: () => error('discovery_invalid_input', 'Discovery accepts only bounded model search and selection fields.'),
    malformed: () => error('discovery_invalid_input', 'Use a JSON object.') });
  if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).some(key => !OPERATIONS[operation].includes(key))
    || Object.values(value).some(field => typeof field !== 'string')
    || operation !== 'reset' && Object.keys(value).length < 1
    || Object.entries(value).some(([key, field]) => key !== 'query' && !/^[a-f0-9]{64}$/u.test(field))) {
    throw error('discovery_invalid_input', 'Use only the server-issued selection ID or model search query.');
  }
  return value;
}

export function createDiscoveryHandler({ origin, capability, isOtherBusy = () => false, assessCandidate,
  selectArtifact = async () => { throw error('discovery_selection_unavailable', 'Lab model selection is unavailable.'); },
  service = new DiscoveryService({ assessCandidate }) } = {}) {
  if (typeof origin !== 'function' || !/^[a-f0-9]{64}$/u.test(capability ?? '')) throw error('discovery_invalid_configuration');
  let active = null; let generation = 0; let resets = 0; let closed = false; let selected = null;
  const reset = async () => {
    generation++; resets++;
    const job = active; job?.controller.abort();
    try { await job?.done; return { cancelled: Boolean(job), partial_download_retained: true }; }
    finally { resets--; }
  };
  return {
    service,
    get busy() { return Boolean(active || resets); },
    clearSelection() { selected = null; },
    reset,
    async handle(request, response) {
      if (!isDiscoveryRoute(request.url)) return false;
      setSecurityHeaders(response);
      const send = (status, value) => sendJSON(response, status, value);
      try {
        if (!fromLabPage(request, origin(), capability)) {
          send(403, { error: 'discovery_forbidden', message: 'Refresh the local Lab page to reconnect.' }); return true;
        }
        if (request.method === 'GET' && request.url === '/api/discovery/status') {
          send(200, { schema: 'badi.discovery.status.v1', busy: Boolean(active || resets), operation: active?.operation ?? null,
            progress: service.progress, selected, recommendation: 'No qualified model. Discovery, fit and download alone do not demonstrate prediction quality.' }); return true;
        }
        const operation = request.url.split('/').at(-1);
        if (request.method !== 'POST' || request.headers['content-type'] !== 'application/json' || !OPERATIONS[operation]) throw error('discovery_invalid_input', 'Use a JSON discovery request.');
        const uploadGeneration = generation;
        const input = await inputJSON(request, operation);
        if (uploadGeneration !== generation || resets) throw error('discovery_cancelled', 'This request began before the Lab reset. Submit it again.');
        if (operation === 'reset') { send(200, { schema: 'badi.discovery.reset.v1', cleanup: await reset() }); return true; }
        if (closed) throw error('discovery_closed', 'Discovery is closed.');
        if (active || isOtherBusy()) throw error('discovery_busy', 'Another comparison, model operation or reset is running.');
        const controller = new AbortController(); let finish;
        const job = { controller, operation, done: new Promise(resolve => { finish = resolve; }) }; active = job;
        const disconnect = () => { if (!response.writableEnded) controller.abort(); };
        response.once('close', disconnect);
        try {
          let result;
          if (operation === 'select') {
            const selection = await service.selection(input.artifact_id, controller.signal);
            controller.signal.throwIfAborted();
            const callbackResult = await selectArtifact(selection, controller.signal);
            controller.signal.throwIfAborted();
            selected = { artifact_id: selection.artifact_id, candidate: selection.candidate,
              state: 'selected_for_experiment', qualification: 'unqualified', ...(callbackResult ? { details: callbackResult } : {}) };
            result = { schema: 'badi.discovery.selection.v1', selected };
          } else result = await service[operation](input, controller.signal);
          if (controller.signal.aborted || generation !== uploadGeneration) throw error('discovery_cancelled', 'Operation cancelled.');
          send(200, result);
        } catch (failure) {
          if (controller.signal.aborted) throw error('discovery_cancelled', 'Operation cancelled. Retry a download to resume its pinned bytes.');
          throw failure;
        } finally { response.off('close', disconnect); if (active === job) active = null; finish(); }
      } catch (failure) {
        const code = failure.code?.startsWith('discovery_') ? failure.code : 'discovery_unavailable';
        const status = code.includes('busy') || code === 'discovery_cancelled' ? 409
          : code.includes('invalid_input') || code.includes('unknown_') ? 400 : code === 'discovery_fit_rejected' ? 422 : 503;
        send(status, { error: code, message: code === failure.code ? failure.message : 'Discovery could not complete. Retry or inspect local prerequisites.' });
      }
      return true;
    },
    async close() { closed = true; return reset(); },
  };
}
