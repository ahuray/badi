import { createServer } from 'node:http';
import { randomBytes, randomUUID, timingSafeEqual } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import { parseArgs } from 'node:util';
import { LabWorker } from './worker.mjs';
import { createSpellingHandler, isSpellingRoute } from './spelling-server.mjs';
import { createContextLookupHandler, isContextLookupRoute } from './context-lookup-server.mjs';
import { createDiscoveryHandler, isDiscoveryRoute } from './discovery-server.mjs';
import { DeviceQualification } from './device-qualification.mjs';
import { createQualificationHandler, isQualificationRoute } from './qualification-server.mjs';
import { validateSuite, modelInputForCase, planComparison, scorePrediction, summarizeResults } from './cases.mjs';

const MODES = ['production_baseline', 'production_boundary', 'context', 'instructed', 'healed', 'instructed_healed', 'instructed_word', 'healed_attested', 'native_instructed'];

export function labOptions(args) {
  const { values } = parseArgs({ args, strict: true, options: {
    port: { type: 'string' }, 'prefill-batch': { type: 'string' },
    'spelling-de': { type: 'string' }, 'spelling-fa': { type: 'string' },
  } });
  const port = values.port ?? '0';
  const batch = values['prefill-batch'] ?? '16';
  if (!/^(?:0|[1-9][0-9]{0,4})$/u.test(port) || Number(port) > 65535) throw new Error('Use a port from 0 to 65535; 0 selects an available port.');
  if (!['16', '64'].includes(batch)) throw new Error('Use --prefill-batch 16 or 64.');
  return { port: Number(port), prefillBatch: Number(batch),
    spellingManifests: Object.fromEntries(['de', 'fa'].filter(language => values[`spelling-${language}`] !== undefined)
      .map(language => [language, values[`spelling-${language}`]])) };
}

export function renderLabPage(source, capability) {
  if (!/^[a-f0-9]{64}$/u.test(capability) || !source.includes('id="lab-ui" hidden')
    || !source.includes('id="launch-help"') || !source.includes('__LAB_CAPABILITY__')) {
    throw new Error('Invalid workbench page or session capability.');
  }
  return source.replace('__LAB_CAPABILITY__', capability)
    .replace('id="launch-help"', 'id="launch-help" hidden')
    .replace('id="lab-ui" hidden', 'id="lab-ui"');
}
export function validateConfigs(configs) {
  if (!Array.isArray(configs) || !configs.length || configs.length > MODES.length) throw new Error(`Select between one and ${MODES.length} configurations.`);
  const ids = new Set();
  return configs.map(config => {
    if (!config || typeof config !== 'object' || Object.keys(config).some(key => !['id', 'mode', 'budget_ms', 'max_tokens', 'cache_prompt', 'temperature', 'seed'].includes(key))) throw new Error('Invalid configuration.');
    if (typeof config.id !== 'string' || !/^[a-z0-9_-]{1,48}$/u.test(config.id) || ids.has(config.id)) throw new Error('Configuration IDs must be unique.');
    ids.add(config.id);
    if (!MODES.includes(config.mode) || !Number.isInteger(config.budget_ms) || config.budget_ms < 550 || config.budget_ms > 10000
      || !Number.isInteger(config.max_tokens) || config.max_tokens < 8 || config.max_tokens > 64
      || typeof config.cache_prompt !== 'boolean' || !Number.isFinite(config.temperature) || config.temperature < 0 || config.temperature > 1
      || !Number.isInteger(config.seed) || config.seed < 0 || config.seed > 2147483647) throw new Error('Configuration is outside the supported limits.');
    if (['production_baseline', 'production_boundary'].includes(config.mode)
      && (config.budget_ms !== 550 || config.max_tokens !== 8 || !config.cache_prompt || config.temperature !== 0 || config.seed !== 42)) throw new Error('Production comparison modes use fixed production settings.');
    return { ...config };
  });
}

// Explicit allowlist: answer keys and human judgments cannot enter model input.
export function workerRequest(testCase, config) {
  const parameters = { ...config }; delete parameters.id;
  const input = modelInputForCase(testCase);
  return { schema: 'badi.prediction-lab.request.v1', id: randomUUID(), before: input.prefix,
    language: input.language, context: input.context,
    style_examples: input.style.split(/\n\s*\n/u).filter(value => value.trim()), config: parameters };
}

async function readJSON(request) {
  const chunks = [];
  let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    if (size > 512 * 1024) throw new Error('Request exceeds 512 KiB.');
    chunks.push(chunk);
  }
  return JSON.parse(Buffer.concat(chunks).toString('utf8'));
}

export async function createLabServer({ worker = new LabWorker(), port = 0, spellingManifests = {},
  spellingWorkerFactory, spellingManifestLoader, contextLookupWorkerFactory, discoveryService, evidenceStore, qualificationDiagnostics,
  qualification = new DeviceQualification({ prefillBatch: worker.prefillBatch ?? 16 }) } = {}) {
  const capability = randomBytes(32).toString('hex');
  let origin;
  let active = null;
  let resetGeneration = 0;
  let resetsInFlight = 0;
  let contextLookup, discovery, qualificationReview, selectedModel = null;
  let deviceInspection = null;
  const stopDeviceInspection = async () => {
    const job = deviceInspection;
    job?.controller.abort();
    // The inspector promise settles only after its fixed child has closed.
    // Keep the shared busy slot until that cleanup has completed.
    await job?.done;
  };
  const spelling = await createSpellingHandler({ origin: () => origin, capability, manifests: spellingManifests,
    isPredictionBusy: () => Boolean(active || resetsInFlight || contextLookup?.busy || discovery?.busy || deviceInspection || qualificationReview?.busy),
    ...(spellingWorkerFactory ? { workerFactory: spellingWorkerFactory } : {}),
    ...(spellingManifestLoader ? { loadManifest: spellingManifestLoader } : {}) });
  contextLookup = createContextLookupHandler({ origin: () => origin, capability,
    isOtherBusy: () => Boolean(active || resetsInFlight || spelling.busy || discovery?.busy || deviceInspection || qualificationReview?.busy),
    ...(contextLookupWorkerFactory ? { workerFactory: contextLookupWorkerFactory } : {}) });
  discovery = createDiscoveryHandler({ origin: () => origin, capability,
    isOtherBusy: () => Boolean(active || resetsInFlight || spelling.busy || contextLookup.busy || deviceInspection || qualificationReview?.busy),
    assessCandidate: (candidate, signal) => qualification.assess(candidate, signal),
    ...(discoveryService ? { service: discoveryService } : {}),
    selectArtifact: async (selection, signal) => {
      const assessment = await qualification.assess(selection.candidate, signal, { artifact: selection.artifact });
      if (!assessment.load_allowed) throw new Error(`The downloaded model cannot load: ${assessment.rejection_reasons.join(' ')}`);
      signal.throwIfAborted();
      const previousArtifact = worker.modelArtifact, previousHook = worker.beforeStart;
      try {
        await worker.selectArtifact(selection.artifact, async signal => {
          const current = await qualification.assess(selection.candidate, signal, { artifact: selection.artifact });
          if (!current.load_allowed) throw new Error(`Current resources do not allow this model: ${current.rejection_reasons.join(' ')}`);
        });
        signal.throwIfAborted();
      } catch (error) {
        if (signal.aborted) await worker.selectArtifact(previousArtifact, previousHook);
        throw error;
      }
      await qualificationReview.reset();
      selectedModel = { ...selection, assessment };
      return { assessment, prompt_profile: assessment.prompt_profile };
    } });
  qualificationReview = createQualificationHandler({ origin: () => origin, capability, qualification, worker, makeRequest: workerRequest,
    isOtherBusy: () => Boolean(active || resetsInFlight || spelling.busy || contextLookup.busy || discovery.busy || deviceInspection),
    ...(evidenceStore ? { evidenceStore } : {}), ...(qualificationDiagnostics ? { diagnostics: qualificationDiagnostics } : {}) });
  const assets = new Map([['/', ['index.html', 'text/html; charset=utf-8']], ['/app.mjs', ['app.mjs', 'text/javascript; charset=utf-8']], ['/style.css', ['style.css', 'text/css; charset=utf-8']],
    ['/spelling.mjs', ['spelling.mjs', 'text/javascript; charset=utf-8']], ['/spelling.css', ['spelling.css', 'text/css; charset=utf-8']],
    ['/context-lookup.mjs', ['context-lookup.mjs', 'text/javascript; charset=utf-8']],
    ['/context-lookup.css', ['context-lookup.css', 'text/css; charset=utf-8']], ['/tabs.mjs', ['tabs.mjs', 'text/javascript; charset=utf-8']],
    ['/qualification.mjs', ['qualification.mjs', 'text/javascript; charset=utf-8']], ['/discovery.mjs', ['discovery.mjs', 'text/javascript; charset=utf-8']], ['/discovery.css', ['discovery.css', 'text/css; charset=utf-8']]]);
  const server = createServer(async (request, response) => {
    response.setHeader('Cache-Control', 'no-store');
    response.setHeader('X-Content-Type-Options', 'nosniff');
    response.setHeader('Cross-Origin-Resource-Policy', 'same-origin');
    response.setHeader('Referrer-Policy', 'no-referrer');
    response.setHeader('Content-Security-Policy', "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'");
    const fail = (status, error) => {
      if (response.destroyed || response.writableEnded) return;
      response.writeHead(status, { 'Content-Type': 'application/json' }); response.end(JSON.stringify({ error }));
    };
    try {
      if (isSpellingRoute(request.url)) { await spelling.handle(request, response); return; }
      if (isContextLookupRoute(request.url)) { await contextLookup.handle(request, response); return; }
      if (isDiscoveryRoute(request.url)) { await discovery.handle(request, response); return; }
      if (isQualificationRoute(request.url)) { await qualificationReview.handle(request, response); return; }
      if (request.headers.host !== new URL(origin).host || (request.headers.origin && request.headers.origin !== origin)) return fail(403, 'This lab accepts requests only from its own local page.');
      if (request.method === 'GET' && request.url === '/favicon.ico') { response.writeHead(204); response.end(); return; }
      if (request.method === 'GET' && assets.has(request.url)) {
        const [name, type] = assets.get(request.url);
        let body = await readFile(new URL(`public/${name}`, import.meta.url));
        if (name === 'index.html') body = renderLabPage(body.toString(), capability);
        response.writeHead(200, { 'Content-Type': type }); response.end(body); return;
      }
      const token = Buffer.from(String(request.headers['x-badi-lab-token'] ?? ''));
      if (token.length !== capability.length || !timingSafeEqual(token, Buffer.from(capability))) return fail(403, 'Refresh the lab page to reconnect.');
      if (request.method === 'GET' && request.url === '/api/status') {
        response.writeHead(200, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ schema: 'badi.prediction-lab.status.v1', active: Boolean(active), identity: worker.session?.identity ?? null,
          selected_model: selectedModel?.candidate ?? null, assessment: selectedModel?.assessment ?? null })); return;
      }
      if (request.method === 'GET' && request.url === '/api/device') {
        if (active || resetsInFlight || spelling.busy || contextLookup.busy || discovery.busy || deviceInspection || qualificationReview?.busy) return fail(409, 'Finish or cancel the active test or device inspection first.');
        const controller = new AbortController(); let finish;
        const job = { controller, done: new Promise(resolve => { finish = resolve; }) };
        const generation = resetGeneration; deviceInspection = job;
        const disconnect = () => { if (!response.writableEnded) controller.abort(); };
        response.once('close', disconnect);
        try {
          const result = await qualification.inspect(controller.signal);
          if (controller.signal.aborted || generation !== resetGeneration) return fail(409, 'Device inspection cancelled.');
          if (!response.destroyed && !response.writableEnded) {
            response.writeHead(200, { 'Content-Type': 'application/json' }); response.end(JSON.stringify(result));
          }
        } catch (error) {
          fail(controller.signal.aborted ? 409 : 503, controller.signal.aborted ? 'Device inspection cancelled.' : error.message);
        } finally { response.off('close', disconnect); if (deviceInspection === job) deviceInspection = null; finish(); }
        return;
      }
      if (request.method !== 'POST' || request.headers['content-type'] !== 'application/json') return fail(400, 'Use a JSON request.');
      if (request.url === '/api/reset') {
        resetGeneration++;
        resetsInFlight++;
        active?.abort();
        deviceInspection?.controller.abort();
        try { await qualificationReview.reset(); await discovery.reset(); await stopDeviceInspection(); await worker.stop(); } finally { resetsInFlight--; }
        response.writeHead(200, { 'Content-Type': 'application/json' }); response.end('{}'); return;
      }
      if (request.url === '/api/model/baseline') {
        if (active || resetsInFlight || spelling.busy || contextLookup.busy || discovery.busy || deviceInspection || qualificationReview?.busy) return fail(409, 'Finish or cancel the active test first.');
        const input = await readJSON(request);
        if (!input || typeof input !== 'object' || Array.isArray(input) || Object.keys(input).length) return fail(400, 'Baseline selection takes no options.');
        if (active || resetsInFlight || spelling.busy || contextLookup.busy || discovery.busy || deviceInspection || qualificationReview?.busy) return fail(409, 'Finish or cancel the active test first.');
        resetGeneration++; resetsInFlight++;
        try { await qualificationReview.reset(); await worker.selectArtifact(null, null); selectedModel = null; discovery.clearSelection(); }
        finally { resetsInFlight--; }
        response.writeHead(200, { 'Content-Type': 'application/json' }); response.end('{}'); return;
      }
      if (request.url !== '/api/run') return fail(404, 'Unknown operation.');
      if (active || resetsInFlight || spelling.busy || contextLookup.busy || discovery.busy || deviceInspection || qualificationReview?.busy) return fail(409, 'A comparison or reset is already running.');
      const uploadGeneration = resetGeneration;
      const uploadedModel = selectedModel;
      const body = await readJSON(request);
      if (uploadGeneration !== resetGeneration || resetsInFlight || uploadedModel !== selectedModel) return fail(409, 'This upload began before the lab was cleared or its model changed. Submit it again.');
      const suite = validateSuite(body.suite);
      if (suite.cases.length > 300) throw new Error('Run up to 300 cases at a time.');
      const configs = validateConfigs(body.configs);
      if (selectedModel) {
        const legacyQwenModes = ['instructed', 'instructed_healed', 'instructed_word'];
        if (selectedModel.candidate.sha256 !== 'd2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5'
          && configs.some(config => legacyQwenModes.includes(config.mode))) throw new Error('These legacy instruction experiments use a pinned Qwen template. Choose Selected model instructions for this model.');
        if (selectedModel.assessment.prompt_profile === 'base_continuation' && configs.some(config => config.mode === 'native_instructed')) throw new Error('This base model has no embedded instruction template. Choose full context or word-boundary continuation.');
      }
      const seed = body.seed ?? 42;
      if (!Number.isInteger(seed) || seed < 0 || seed > 2147483647) throw new Error('Use a nonnegative integer order seed.');
      const plan = planComparison(suite.cases, configs, seed);
      // readJSON yields: another request may have claimed the worker meanwhile.
      if (active || resetsInFlight || spelling.busy || contextLookup.busy || discovery.busy || deviceInspection || qualificationReview?.busy) return fail(409, 'A comparison is already running.');
      const cancellation = new AbortController();
      active = cancellation;
      const records = [];
      const runId = randomUUID();
      const previousBeforeStart = worker.beforeStart;
      let cleanupVerified = true;
      let completedSummaries = null;
      let terminalFailure = null;
      response.writeHead(200, { 'Content-Type': 'application/x-ndjson; charset=utf-8' });
      const send = event => { if (!response.destroyed) response.write(JSON.stringify(event) + '\n'); };
      response.once('close', () => { if (!response.writableEnded) cancellation.abort(); });
      send({ type: 'plan', schema: 'badi.prediction-lab.run.v1', run_id: runId, created_at: new Date().toISOString(), suite, configs, seed, plan,
        selected_model: selectedModel?.candidate ?? null, assessment: selectedModel?.assessment ?? null,
        measurement: 'Sequential snapshot replay. Each case/trace/config begins in an empty owned runtime; only steps within a trace reuse context. Trace event times are metadata, not paced keystrokes or cancellation replay. Model startup is excluded from result latency. Runtime size may differ from installed Badi; inspect identity.' });
      try {
        const qualificationPlan = await qualificationReview.beginRun({ runId, suite, configs, selection: selectedModel, signal: cancellation.signal, confirmation: body.confirmation });
        send({ type: 'qualification_plan', ...qualificationPlan });
        let previousGroup = null;
        for (const item of plan) {
          cancellation.signal.throwIfAborted();
          const testCase = suite.cases.find(value => value.id === item.case_id);
          const config = configs.find(value => value.id === item.config_id);
          // Each case/trace/config starts from an empty owned slot. Only successive
          // steps of the same explicit typing trace share cached context.
          const group = `${item.group_id}:${item.config_id}`;
          if (previousGroup !== null && previousGroup !== group) {
            let cleanup;
            try { cleanup = await worker.stop(); } catch (error) { cleanupVerified = false; throw error; }
            if (cleanup) send({ type: 'cleanup', group: previousGroup, cleanup });
          }
          previousGroup = group;
          send({ type: 'progress', case_id: testCase.id, config_id: config.id, completed: records.length, total: plan.length });
          const requestBody = workerRequest(testCase, config);
          if (selectedModel) worker.beforeStart = signal => qualificationReview.preflight(runId, config.id, signal);
          const started = performance.now();
          let result = await worker.predict(requestBody, cancellation.signal);
          if (result.type === 'error') result = { ...result, outcome: 'error', text: null, reason: result.error, identity: worker.session?.identity ?? null,
            latency_ms: performance.now() - started, timing_includes_worker_startup: true };
          if (!['suggestion', 'abstention', 'deadline', 'error'].includes(result.outcome)
            || !Number.isFinite(result.latency_ms) || result.latency_ms < 0
            || (result.outcome === 'suggestion' && (typeof result.text !== 'string' || !result.text.trim()))) {
            result = { ...result, outcome: 'error', text: null, reason: 'invalid_model_output', latency_ms: performance.now() - started, timing_includes_worker_startup: true };
          }
          if (result.outcome !== 'suggestion') result.text = null;
          const record = { case: testCase, config, config_id: config.id, result, score: scorePrediction(testCase, result) };
          records.push(record);
          const reviewTarget = qualificationReview.addRecord(runId, record);
          send({ type: 'record', record, review_target: reviewTarget });
        }
        completedSummaries = configs.map(config => ({ config, summary: summarizeResults(records.filter(record => record.config.id === config.id)) }));
      } catch (error) {
        terminalFailure = { type: cancellation.signal.aborted ? 'cancelled' : 'error', error: cancellation.signal.aborted ? 'Comparison cancelled.' : error.message };
      } finally {
        // No typed prompt remains in the runtime after a completed comparison.
        try {
          const cleanup = await worker.stop();
          if (cleanup) send({ type: 'cleanup', cleanup });
          const qualificationResult = qualificationReview.finishRun(runId, { cleanupVerified });
          if (qualificationResult) send(qualificationResult);
          if (terminalFailure) send(terminalFailure);
          else if (cancellation.signal.aborted) send({ type: 'cancelled', error: 'Comparison cancelled.' });
          else if (completedSummaries) send({ type: 'complete', summaries: completedSummaries });
        } catch (error) {
          cleanupVerified = false;
          const qualificationResult = qualificationReview.finishRun(runId, { cleanupVerified });
          if (qualificationResult) send(qualificationResult);
          send({ type: 'error', error: error.message });
        }
        finally {
          worker.beforeStart = previousBeforeStart;
          if (active === cancellation) active = null; response.end();
        }
      }
    } catch (error) { if (!response.headersSent) fail(400, error.message); else response.end(); }
  });
  server.requestTimeout = 15000;
  server.headersTimeout = 10000;
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(port, '127.0.0.1', resolve); });
  origin = `http://127.0.0.1:${server.address().port}`;
  return { server, origin, spelling, contextLookup, discovery, qualificationReview, async close() {
    resetGeneration++;
    resetsInFlight++;
    active?.abort();
    deviceInspection?.controller.abort();
    let failure;
    try { await stopDeviceInspection(); } catch (error) { failure = error; }
    try { await qualificationReview.close(); } catch (error) { failure = error; }
    try { await discovery.close(); } catch (error) { failure = error; }
    try { await worker.stop(); } catch (error) { failure = error; }
    try { await spelling.close(); } catch (error) { failure ??= error; }
    try { await contextLookup.close(); } catch (error) { failure ??= error; }
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
    if (failure) throw failure;
  } };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const { port, prefillBatch, spellingManifests } = labOptions(process.argv.slice(2));
  const lab = await createLabServer({ port, spellingManifests, worker: new LabWorker({ prefillBatch }) });
  console.log(`Badi Prediction Lab: ${lab.origin}\nOpen this page in your browser. Text stays in this session unless you export it. Ctrl+C closes the lab.`);
  let closing = false;
  const close = async () => { if (closing) return; closing = true; await lab.close(); };
  process.on('SIGINT', close);
  process.on('SIGTERM', close);
}
