import { timingSafeEqual } from 'node:crypto';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';
import { createEvidence, reviewTargets, registerConfirmation, evidenceDigest, EvidenceStore } from './qualification-evidence.mjs';
import { runDiagnostics } from './qualification-diagnostics.mjs';

const PREFIX = '/api/qualification/';
const OPERATIONS = new Set(['status', 'review', 'diagnose', 'load']);
const INPUTS = { review: ['run_id', 'config_id', 'reviews', 'save'], diagnose: ['run_id', 'config_id', 'duration_seconds'], load: ['run_id', 'config_id', 'evidence_id'] };
export const isQualificationRoute = path => typeof path === 'string' && path.startsWith(PREFIX) && OPERATIONS.has(path.slice(PREFIX.length));
export const CONFIRMATION_REVIEW_PROTOCOL = 'badi.prediction-quality.v1: Reserve an untouched confirmation set before inference. Review every assigned outcome and each complete displayed addition, including unwanted tails and alternatives. Generic function words and reference agreement alone are not substantive usefulness. Each requested language needs at least 40 cases, at least 60 percent substantive useful terminal additions within 550 ms, and zero harmful additions. Keep errors, deadlines, abstentions and missing responses in denominators. Require 550 ms complete-word p95, separate cold/preparation timings, measured prompt reuse, paced typing, cancellation/recovery, verified cleanup and at least 1800 seconds sustained resource measurement.';
const emptyRecommendation = () => ({ schema: 'badi.model-recommendation.v1', status: 'no_qualified_model', candidate_id: null, ranking: [], reasons: ['No current reviewed evidence has passed every device, performance and quality gate.'] });
const failure = (message, status = 400) => Object.assign(new Error(message), { status });
const languagesOf = suite => [...new Set(suite.cases.map(row => row.language.toLowerCase().split('-')[0]))].sort();
const sameIdentity = (left, right) => evidenceDigest(left) === evidenceDigest(right);

async function inputJSON(request, operation) {
  const chunks = []; let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    if (size > 256 * 1024) throw failure('Qualification input exceeds 256 KiB.');
    chunks.push(chunk);
  }
  let value;
  try { value = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(Buffer.concat(chunks))); }
  catch { throw failure('Use a JSON qualification object.'); }
  if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).some(key => !INPUTS[operation].includes(key))
    || typeof value.run_id !== 'string' || typeof value.config_id !== 'string') throw failure('Use only server-issued run/configuration IDs and explicit review fields; measurements are server-owned.');
  if (operation === 'review' && (!Array.isArray(value.reviews) || value.reviews.length > 300 || value.save !== undefined && typeof value.save !== 'boolean')) throw failure('Supply up to 300 explicit reviews and an optional save boolean.');
  if (operation === 'diagnose' && ![30, 300, 1800].includes(value.duration_seconds)) throw failure('Choose a diagnostic duration of 30, 300 or 1800 seconds.');
  if (operation === 'load' && (typeof value.evidence_id !== 'string' || !/^[a-f0-9]{64}$/u.test(value.evidence_id))) throw failure('Use the saved evidence SHA-256 ID.');
  return value;
}

export function createQualificationHandler({ origin, capability, qualification, worker, makeRequest,
  isOtherBusy = () => false, diagnostics = runDiagnostics,
  evidenceStore = new EvidenceStore({ directory: resolve(process.env.XDG_CACHE_HOME || join(homedir(), '.cache'), 'badi', 'prediction-lab', 'evidence') }),
  now = () => Math.floor(Date.now() / 1000) } = {}) {
  if (typeof origin !== 'function' || !/^[a-f0-9]{64}$/u.test(capability ?? '') || typeof qualification?.assess !== 'function') throw failure('Invalid qualification handler configuration.');
  let latest = null, generation = 0, active = null, closed = false;
  let recommendation = emptyRecommendation();
  const seenInputs = new Set(); let historyFull = false;
  const snapshot = ({ recommendationFresh = false } = {}) => ({ schema: 'badi.qualification.status.v1', busy: Boolean(active), progress: active?.progress ?? null,
    run: latest ? { run_id: latest.id, finished: latest.finished, confirmation_untouched: latest.confirmationRequested,
      unavailable_reason: latest.selection ? null : 'Select a verified discovered artifact to bind empirical evidence to candidate metadata.',
      configurations: latest.configs.map(config => ({ config, review_targets: targets(latest, config.id),
        reviewed: latest.reviews.get(config.id)?.length ?? 0, assessment: visibleAssessment(latest.reports.get(config.id) ?? latest.assessments.get(config.id) ?? null, recommendationFresh),
        evidence: latest.evidence.get(config.id) ?? null, diagnostics: latest.diagnosticReports.get(config.id) ?? null, saved_evidence_id: latest.saved.get(config.id) ?? null })) } : null,
    recommendation: recommendation.status === 'recommended' && !recommendationFresh
      ? { ...emptyRecommendation(), status: 'pending_recheck', reasons: ['The previous recommendation is historical until current device, runtime and resource checks finish.'] } : recommendation, limits: ['This status shows the last assessment; review, load and diagnostics recheck the current model/device/settings before using evidence.',
      'Complete-word percentiles retain undelivered outcomes as censored lower bounds of at least 551 ms, not measured delivery times.',
      'Untouched registration rejects inputs already used in this server session; prior exposure outside this session remains the explicit reviewer responsibility.'] });
  const targets = (run, configId) => reviewTargets({ records: run.records, plannedCases: run.suite.cases, configId, runId: run.id });
  const visibleAssessment = (assessment, fresh) => assessment?.recommended && !fresh
    ? { ...assessment, recommended: false, requires_recheck: true } : assessment;
  const invalidate = (run, reason) => {
    if (latest === run) recommendation = { ...emptyRecommendation(), reasons: [reason] };
    for (const [id, report] of run.reports) run.reports.set(id, { ...report, recommended: false, evidence_valid: false, stale: true, requires_recheck: true });
  };
  const reset = async () => {
    generation++; latest = null; recommendation = emptyRecommendation();
    const job = active; job?.controller.abort(); await job?.done;
  };
  const requireRun = (runId, configId) => {
    if (!latest || latest.id !== runId || !latest.configs.some(config => config.id === configId)) throw failure('This run/configuration is no longer held by the server. Run the comparison again.', 409);
    if (!latest.finished) throw failure('Finish or cancel this comparison before reviewing it.', 409);
    if (!latest.selection || !latest.assessments.has(configId)) throw failure('This run has no verified discovery artifact assessment. Select a model in discovery and run again.', 422);
    return latest;
  };
  const freshAssessment = async (run, configId, signal, evidence) => qualification.assess(run.selection.candidate, signal,
    { artifact: run.selection.artifact, config: run.configs.find(config => config.id === configId), languages: languagesOf(run.suite), ...(evidence ? { evidence } : {}) });
  const freshBound = async (run, configId, signal) => {
    let fresh;
    try { fresh = await freshAssessment(run, configId, signal); }
    catch (error) { invalidate(run, 'The previous recommendation could not be rechecked against the current runtime/device.'); throw error; }
    if (!sameIdentity(fresh.identity, run.assessments.get(configId).identity)) {
      const reason = 'The model, runtime, device or settings changed after this run. Repeat the comparison before using its evidence.';
      invalidate(run, reason); throw failure(reason, 409);
    }
    if (!fresh.load_allowed) invalidate(run, 'Current resource checks reject the previously assessed configuration.');
    return fresh;
  };
  const aggregate = (run, configId, reviews = run.reviews.get(configId) ?? []) => {
    const initial = run.assessments.get(configId);
    const records = run.records.filter(record => record.config_id === configId);
    const identities = records.map(record => record.result.identity).filter(Boolean);
    const backend = identities.length && identities.every(identity => identity.gpu_layers === 0) ? 'cpu' : 'unknown';
    const metrics = { identity: initial.identity, measured_at_unix_s: run.finishedAt,
      loaded_and_exercised: identities.length > 0, artifact_verified: identities.length > 0,
      actual_backend: backend, peak_vram_bytes: backend === 'cpu' ? 0 : null,
      cleanup_verified: run.cleanupVerified,
      prompt_reuse_measured: records.some(record => record.result.terminal_received === true && Number.isSafeInteger(record.result.reused_prompt_tokens) && record.result.reused_prompt_tokens >= 0),
      ...(run.diagnostics.get(configId)?.evidenceMetrics ?? {}) };
    metrics.identity = initial.identity;
    metrics.measured_at_unix_s = run.finishedAt; // New diagnostics cannot renew old quality evidence.
    metrics.cleanup_verified = run.cleanupVerified && metrics.cleanup_verified;
    return createEvidence({ assessment: initial, records: run.records, plannedCases: run.suite.cases, configId, runId: run.id,
      reviews, runtimeMeasurements: metrics, confirmation: run.confirmations.get(configId) ?? null, runStartedAtUnixS: run.startedAt });
  };
  const assessAndRank = async (run, configId, evidence, signal) => {
    const assessment = await freshAssessment(run, configId, signal, evidence);
    signal.throwIfAborted();
    const entries = [...run.evidence].filter(([id]) => id !== configId).map(([id, receipt]) => ({ assessment: run.reports.get(id), evidence: receipt }));
    entries.push({ assessment, evidence });
    const ranked = typeof qualification.rank === 'function' ? await qualification.rank(entries, signal) : emptyRecommendation();
    signal.throwIfAborted();
    if (latest !== run) throw failure('Qualification run was reset.', 409);
    run.reports.set(configId, assessment); run.evidence.set(configId, evidence); recommendation = ranked;
    return assessment;
  };
  const resultFor = (run, configId, assessment, evidence, extra = {}) => ({ schema: 'badi.qualification.result.v1', run_id: run.id, config_id: configId,
    review_targets: targets(run, configId), assessment, evidence, saved_evidence_id: run.saved.get(configId) ?? null, recommendation, ...extra });
  return {
    get busy() { return Boolean(active); },
    snapshot,
    reset,
    async beginRun({ runId, suite, configs, selection, signal, confirmation }) {
      if (active || closed) throw failure('Qualification is busy or closed.', 409);
      const epoch = ++generation;
      const inputHashes = suite.cases.map(row => evidenceDigest({ language: row.language, prefix: row.prefix, context: row.context, style: row.style }));
      const confirmationRequested = confirmation !== undefined;
      if (confirmationRequested && (!confirmation || typeof confirmation !== 'object' || Array.isArray(confirmation)
        || Object.keys(confirmation).some(key => !['untouched', 'review_protocol'].includes(key)) || confirmation.untouched !== true
        || confirmation.review_protocol !== undefined && (typeof confirmation.review_protocol !== 'string'
          || !confirmation.review_protocol.isWellFormed() || confirmation.review_protocol.trim().length < 32
          || Buffer.byteLength(confirmation.review_protocol) > 16384 || /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/u.test(confirmation.review_protocol))
        || inputHashes.some(hash => seenInputs.has(hash)) || new Set(inputHashes).size !== inputHashes.length || historyFull || !selection)) throw failure('Confirmation requires an unused set, a selected artifact and the frozen full-addition protocol before inference.');
      for (const hash of inputHashes) { if (seenInputs.size < 10000) seenInputs.add(hash); else historyFull = true; }
      const protocol = CONFIRMATION_REVIEW_PROTOCOL + (confirmation?.review_protocol && confirmation.review_protocol !== CONFIRMATION_REVIEW_PROTOCOL
        ? `\nAdditional frozen review notes (the hard gates above remain mandatory):\n${confirmation.review_protocol}` : '');
      const run = { id: runId, suite: structuredClone(suite), configs: structuredClone(configs), selection,
        records: [], assessments: new Map(), reports: new Map(), evidence: new Map(), reviews: new Map(), saved: new Map(), diagnostics: new Map(), diagnosticReports: new Map(), confirmations: new Map(),
        finished: false, cleanupVerified: false, finishedAt: 0, startedAt: now(), confirmationRequested };
      latest = run; recommendation = emptyRecommendation();
      if (selection) for (const config of configs) {
        const assessment = await freshAssessment(run, config.id, signal);
        signal.throwIfAborted();
        if (epoch !== generation) throw failure('Run assessment was reset.', 409);
        if (!assessment.load_allowed) throw failure(`Current resources do not permit this run: ${assessment.rejection_reasons.join(' ')}`, 422);
        run.assessments.set(config.id, assessment);
        if (confirmationRequested) run.confirmations.set(config.id, registerConfirmation({ assessment, plannedCases: run.suite.cases, configId: config.id,
          runId, reviewProtocol: protocol, registeredAtUnixS: now() }));
      }
      run.startedAt = now();
      return { run_id: runId, confirmation_untouched: confirmationRequested, review_protocol: confirmationRequested ? protocol : null };
    },
    async preflight(runId, configId, signal) {
      if (!latest || latest.id !== runId || !latest.selection) throw failure('This model run was cleared.', 409);
      const fresh = await freshBound(latest, configId, signal);
      if (!fresh.load_allowed) throw failure('Current resources no longer permit this model.', 422);
      return fresh;
    },
    addRecord(runId, record) {
      if (!latest || latest.id !== runId || latest.finished) return null;
      const copy = structuredClone(record); latest.records.push(copy);
      return reviewTargets({ records: [copy], plannedCases: [copy.case], configId: copy.config_id, runId })[0];
    },
    finishRun(runId, { cleanupVerified = false } = {}) {
      if (!latest || latest.id !== runId) return null;
      latest.finished = true; latest.cleanupVerified = cleanupVerified; latest.finishedAt = now();
      return { type: 'qualification', ...snapshot() };
    },
    async handle(request, response) {
      if (!isQualificationRoute(request.url)) return false;
      response.setHeader('Cache-Control', 'no-store'); response.setHeader('X-Content-Type-Options', 'nosniff');
      response.setHeader('Cross-Origin-Resource-Policy', 'same-origin'); response.setHeader('Referrer-Policy', 'no-referrer');
      const send = (status, value) => { if (!response.destroyed && !response.writableEnded) { response.writeHead(status, { 'Content-Type': 'application/json; charset=utf-8' }); response.end(JSON.stringify(value)); } };
      try {
        const expected = origin(); const token = Buffer.from(String(request.headers['x-badi-lab-token'] ?? ''));
        if (request.headers.host !== new URL(expected).host || request.headers.origin && request.headers.origin !== expected
          || token.length !== capability.length || !timingSafeEqual(token, Buffer.from(capability))) throw failure('Refresh the local Lab page to reconnect.', 403);
        const operation = request.url.slice(PREFIX.length);
        if (operation === 'status' && request.method === 'GET') {
          if (recommendation.status !== 'recommended' || !latest?.finished || closed || active || isOtherBusy()) {
            send(200, snapshot()); return true;
          }
          const run = latest, epoch = generation, controller = new AbortController(); let finish;
          const job = { controller, done: new Promise(resolve => { finish = resolve; }), progress: { phase: 'rechecking_recommendation' } }; active = job;
          const disconnect = () => { if (!response.writableEnded) controller.abort(); };
          response.once('close', disconnect);
          try {
            const entries = [];
            for (const [id, evidence] of run.evidence) {
              await freshBound(run, id, controller.signal);
              const assessment = await freshAssessment(run, id, controller.signal, evidence);
              controller.signal.throwIfAborted();
              entries.push({ assessment, evidence }); run.reports.set(id, assessment);
            }
            const ranked = typeof qualification.rank === 'function' ? await qualification.rank(entries, controller.signal) : emptyRecommendation();
            controller.signal.throwIfAborted();
            if (epoch !== generation || latest !== run) throw failure('Recommendation refresh was reset.', 409);
            recommendation = ranked; send(200, { ...snapshot({ recommendationFresh: true }), busy: false, progress: null });
          } catch (error) {
            if (latest === run) invalidate(run, error.name === 'AbortError' ? 'Recommendation refresh was cancelled; current qualification is unverified.' : error.message);
            send(200, { ...snapshot(), busy: false, progress: null, refresh_error: error.message });
          } finally { response.off('close', disconnect); if (active === job) active = null; finish(); }
          return true;
        }
        if (request.method !== 'POST' || request.headers['content-type'] !== 'application/json' || !INPUTS[operation]) throw failure('Use a JSON qualification request.');
        const uploadGeneration = generation;
        const input = await inputJSON(request, operation);
        if (uploadGeneration !== generation) throw failure('This upload began before the run was reset or replaced.', 409);
        if (closed || active || isOtherBusy()) throw failure('Finish or cancel the active Lab operation first.', 409);
        const run = requireRun(input.run_id, input.config_id);
        const controller = new AbortController(); let finish;
        const job = { controller, done: new Promise(resolve => { finish = resolve; }), progress: null }; active = job;
        const disconnect = () => { if (!response.writableEnded) controller.abort(); };
        response.once('close', disconnect);
        try {
          const fresh = await freshBound(run, input.config_id, controller.signal);
          let evidence, extra = {};
          if (operation === 'load') {
            if (!run.cleanupVerified) throw failure('This run observed unverified runtime cleanup. Repeat the comparison before loading saved evidence.', 409);
            evidence = await evidenceStore.load(input.evidence_id, { assessment: fresh, nowUnixS: now() });
            run.saved.set(input.config_id, input.evidence_id);
            extra.loaded_evidence = true;
          } else if (operation === 'diagnose') {
            if (!fresh.load_allowed) throw failure('Current resources do not permit diagnostics.', 422);
            // A new lifecycle trial supersedes prior passing runtime measurements.
            // Keep the full-addition reviews so quality can be aggregated again.
            run.diagnostics.clear(); run.evidence.clear(); run.saved.clear(); run.diagnosticReports.clear();
            invalidate(run, 'New runtime diagnostics must finish with verified cleanup before this run can be recommended again.');
            const config = run.configs.find(value => value.id === input.config_id);
            const previousHook = worker.beforeStart;
            worker.beforeStart = async signal => {
              const current = await freshBound(run, input.config_id, signal);
              if (!current.load_allowed) throw failure('Current resources no longer permit diagnostics.', 422);
            };
            let diagnosticsResult;
            try { diagnosticsResult = await diagnostics({ worker, cases: run.suite.cases, config, makeRequest, signal: controller.signal,
              durationSeconds: input.duration_seconds, onEvent: progress => { job.progress = progress; } }); }
            finally {
              worker.beforeStart = previousHook;
              if (diagnosticsResult) run.diagnosticReports.set(input.config_id, diagnosticsResult);
              // Consume cleanup even when the client has cancelled or diagnostics
              // threw: abort propagation must not preserve an older passing receipt.
              if (diagnosticsResult?.evidenceMetrics?.cleanup_verified !== true) {
                run.cleanupVerified = false;
                invalidate(run, 'Runtime cleanup was not verified; repeat the comparison before using this run as qualification evidence.');
              }
            }
            controller.signal.throwIfAborted();
            if (diagnosticsResult.identity?.model_sha256 !== fresh.metadata.sha256 || diagnosticsResult.identity?.model_size !== fresh.metadata.artifact_bytes) throw failure('Diagnostic model identity differs from the selected artifact.', 409);
            const observedIdentities = run.records.filter(record => record.config_id === input.config_id).map(record => record.result.identity).filter(Boolean);
            if (observedIdentities.some(identity => !sameIdentity(identity, diagnosticsResult.identity))) throw failure('Diagnostic runtime configuration differs from the measured run.', 409);
            if (diagnosticsResult.complete === true) run.diagnostics.set(input.config_id, diagnosticsResult);
            else run.diagnostics.delete(input.config_id);
            evidence = aggregate(run, input.config_id); extra.diagnostics = diagnosticsResult;
          } else {
            evidence = aggregate(run, input.config_id, input.reviews);
          }
          controller.signal.throwIfAborted();
          const assessment = await assessAndRank(run, input.config_id, evidence, controller.signal);
          if (operation === 'review') {
            run.reviews.set(input.config_id, structuredClone(input.reviews)); run.saved.delete(input.config_id);
            if (input.save === true) {
              controller.signal.throwIfAborted();
              run.saved.set(input.config_id, await evidenceStore.save(evidence));
            }
          }
          if (generation !== uploadGeneration) throw failure('Qualification results were reset.', 409);
          send(200, resultFor(run, input.config_id, assessment, evidence, extra));
        } finally { response.off('close', disconnect); if (active === job) active = null; finish(); }
      } catch (error) { send(error.status ?? (error.name === 'AbortError' ? 409 : 400), { error: error.message }); }
      return true;
    },
    async close() { closed = true; await reset(); },
  };
}
