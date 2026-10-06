import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { setTimeout as wait } from 'node:timers/promises';
import { validateCase } from './cases.mjs';

const activeWorkers = new WeakSet();
const MAX_REQUESTS = 20000;
const TIMING_TARGET_MS = 550;
const HASH = /^[a-f0-9]{64}$/u;
const finite = value => Number.isFinite(value) && value >= 0;
const integer = value => Number.isSafeInteger(value) && value >= 0;
const languageOf = row => row.language.toLowerCase().split('-')[0];
// Rust hashes StableRuntimeIdentity in declaration order, while ready messages
// pass through serde_json::Value and may have alphabetically sorted object keys.
const RUNTIME_KEYS = ['launch_contract_id', 'binary_sha256', 'runtime_bundle_manifest_sha256',
  'model_sha256', 'model_size', 'model_alias', 'model_origin', 'threads', 'context_size',
  'gpu_layers', 'batch_size', 'ubatch_size'];
const hash = value => createHash('sha256').update(JSON.stringify(Object.fromEntries(
  RUNTIME_KEYS.filter(key => Object.hasOwn(value, key)).map(key => [key, value[key]])))).digest('hex');
const percentile = (values, p) => values.length ? [...values].sort((a, b) => a - b)[Math.ceil(values.length * p) - 1] : null;
const realClock = {
  now: () => performance.now(),
  unix: () => Math.floor(Date.now() / 1000),
  sleep: (ms, signal) => wait(ms, undefined, signal ? { signal } : undefined),
  schedule: (callback, ms) => setTimeout(callback, ms),
  clear: timer => clearTimeout(timer),
};

async function memoryPressure() {
  try {
    const text = await readFile('/proc/pressure/memory', 'utf8');
    const result = {};
    for (const kind of ['some', 'full']) {
      const match = text.match(new RegExp(`^${kind} avg10=([0-9.]+) avg60=([0-9.]+) avg300=([0-9.]+) total=(\\d+)$`, 'm'));
      if (!match) return null;
      result[`${kind}_avg10`] = Number(match[1]);
      result[`${kind}_total_usec`] = Number(match[4]);
    }
    return Object.values(result).every(finite) ? result : null;
  } catch { return null; }
}

function ownedSession(worker) {
  const session = worker.session;
  return session && !session.closed && session.identity && session.runtime?.pid > 1 ? session : null;
}

function cleanupMatches(receipt, session) {
  return Boolean(receipt && session?.identity && receipt.process_id === session.runtime?.pid
    && HASH.test(receipt.runtime_identity_sha256 ?? '')
    && receipt.runtime_identity_sha256 === hash(session.identity)
    && receipt.challenge_completed === true && receipt.reaped === true);
}

/** Only an HTTP owner with exclusive use of its selected LabWorker may call this.
 * clock/readPressure are dependency seams for deterministic tests, never HTTP input.
 * No results, prompts, draft text or expected answers are returned or persisted.
 */
export async function runDiagnostics({ worker, cases, config, makeRequest, signal, durationSeconds = 30,
  onEvent = () => {}, clock = realClock, readPressure = memoryPressure } = {}) {
  if (!worker || typeof worker.predict !== 'function' || typeof worker.stop !== 'function'
    || typeof worker.beforeStart !== 'function') throw new Error('Diagnostics require a selected artifact with a resource recheck before each load.');
  if (activeWorkers.has(worker) || worker.preparing || worker.session?.pending) throw new Error('Diagnostics are already running or this worker is busy.');
  if (![30, 300, 1800].includes(durationSeconds) || !Array.isArray(cases) || !cases.length || cases.length > 256
    || typeof makeRequest !== 'function' || !config || !integer(config.budget_ms) || config.budget_ms < 1 || config.budget_ms > 5000) {
    throw new Error('Use 1–256 explicit cases and a diagnostic duration of 30, 300 or 1800 seconds.');
  }
  config = structuredClone(config);
  const inputs = cases.map(validateCase);
  if (new Set(inputs.map(row => row.id)).size !== inputs.length
    || inputs.some(row => !['en', 'de', 'fa'].includes(languageOf(row)))) throw new Error('Diagnostics need unique English, German or Persian cases.');
  const representatives = [...new Map(inputs.map(row => [languageOf(row), row])).values()];
  const controller = new AbortController(), sessions = new Set(), cleanupBySession = new Map();
  const capture = () => { if (worker.session) sessions.add(worker.session); };
  const abort = () => { capture(); controller.abort(); };
  signal?.addEventListener('abort', abort, { once: true });
  if (signal?.aborted) abort();
  activeWorkers.add(worker);
  const records = [], cleanups = [], pressure = [], limitations = [];
  const started = clock.now();
  let failure = null, externalCancelled = false, calls = 0, identity = null;
  let peakRss = null, peakVram = null, coldStart = null, preparation = null;
  let beforeHookMs = 0, resourceChecks = 0, successfulChecks = 0;
  let sustainedMs = 0, scheduledEvents = 0, observedEvents = 0, pacedComplete = false;
  let cancellation = { attempted: false, pending_at_abort: false, rejected_after_abort: false,
    cleanup_verified: false, recovery_terminal: false, fresh_runtime: false, reason: 'not_run' };
  const originalHook = worker.beforeStart;
  const wrapper = async hookSignal => {
    const beginning = clock.now(); resourceChecks++;
    try { await originalHook.call(worker, hookSignal); successfulChecks++; }
    finally { beforeHookMs += Math.max(0, clock.now() - beginning); }
  };
  worker.beforeStart = wrapper;
  const timeout = clock.schedule(() => { failure ??= 'diagnostic_wall_limit'; abort(); }, durationSeconds * 1000 + 180000);
  const notify = phase => {
    try { onEvent({ phase, completed_requests: records.filter(row => row.outcome !== 'not_run').length,
      assigned_opportunities: records.length, elapsed_seconds: Math.floor((clock.now() - started) / 1000), duration_requested_seconds: durationSeconds }); }
    catch { failure ??= 'progress_delivery_failed'; controller.abort(); }
  };
  const assign = (row, phase) => {
    const record = { ordinal: records.length, case_id: row.id, language: languageOf(row), phase,
      outcome: 'not_run', reason: 'not_run', budget_ms: config.budget_ms, elapsed_ms: null, complete_word_ms: null,
      terminal_received: false, reused_prompt_tokens: null, newly_evaluated_prompt_tokens: null,
      preflight_ms: null, scheduled_at_ms: null, observed_at_ms: null, started_at_ms: null,
      finished_at_ms: null, delivered_at_ms: null, session_startup_ms: null };
    records.push(record); return record;
  };
  const plans = {
    cold: [inputs[0], assign(inputs[0], 'cold')],
    warm: representatives.flatMap(row => [[row, assign(row, 'warm')], [row, assign(row, 'repeat')]]),
    paced: representatives.map(row => {
      const parts = [...new Intl.Segmenter(row.language, { granularity: 'grapheme' }).segment(row.prefix)].map(part => part.segment);
      const variants = parts.length >= 3 ? [parts.slice(0, -2).join(''), parts.slice(0, -1).join(''), row.prefix] : [];
      return { source: row, events: variants.map((prefix, index) => {
        const changed = { ...row, prefix };
        return { row: changed, record: assign(changed, 'paced'), at: [0, 100, 250][index] };
      }) };
    }),
  };
  const cancellationCase = [...inputs].sort((a, b) => b.context.length + b.prefix.length - a.context.length - a.prefix.length)[0];
  const cancelRecord = assign(cancellationCase, 'cancellation');
  const recoveryCase = inputs.find(row => row.id !== cancellationCase.id) ?? inputs[0];
  const recoveryRecord = assign(recoveryCase, 'recovery');

  const retainResult = (record, result, elapsed) => {
    record.elapsed_ms = Math.ceil(elapsed);
    if (!result || !['suggestion', 'abstention', 'deadline', 'error'].includes(result.outcome)) {
      record.outcome = 'error'; record.reason = 'invalid_worker_result'; return;
    }
    record.outcome = result.outcome;
    record.reason = result.outcome === 'error' ? 'worker_error' : result.outcome;
    record.terminal_received = result.terminal_received === true;
    if (record.terminal_received && integer(result.reused_prompt_tokens)) record.reused_prompt_tokens = result.reused_prompt_tokens;
    if (record.terminal_received && integer(result.newly_evaluated_prompt_tokens)) record.newly_evaluated_prompt_tokens = result.newly_evaluated_prompt_tokens;
    if (finite(result.preflight_ms)) record.preflight_ms = result.preflight_ms;
    const local = result.local_measurement;
    if (finite(local?.peak_runtime_rss_bytes) && local.peak_runtime_rss_bytes > 0) peakRss = Math.max(peakRss ?? 0, local.peak_runtime_rss_bytes);
    if (result.identity?.gpu_layers === 0 && local?.gpu_memory_bytes === 0) peakVram = 0;
    if (finite(local?.worker_startup_ms)) record.session_startup_ms = local.worker_startup_ms;
    const runtime = result.identity;
    if (runtime && HASH.test(runtime.model_sha256 ?? '') && HASH.test(runtime.binary_sha256 ?? '')) {
      if (identity && hash(identity) !== hash(runtime)) {
        record.outcome = 'error'; record.reason = 'runtime_identity_changed'; failure ??= record.reason; controller.abort();
      } else identity ??= structuredClone(runtime);
    }
    if (record.outcome === 'suggestion' && result.word_complete === true && result.shape_valid === true
      && record.terminal_received && result.replace_before == null && typeof result.text === 'string' && result.text.trim()) {
      record.complete_word_ms = Math.ceil(Math.max(elapsed, result.latency_ms ?? 0, local?.request_roundtrip_ms ?? 0));
    }
  };
  const run = async (row, record, requestSignal = controller.signal) => {
    if (requestSignal.aborted || calls >= MAX_REQUESTS) return null;
    let request;
    try {
      request = makeRequest(row, config);
      const keys = ['schema', 'id', 'before', 'language', 'context', 'style_examples', 'config'];
      if (!request || typeof request.id !== 'string' || Object.keys(request).some(key => !keys.includes(key))) throw new Error();
    } catch { record.outcome = 'error'; record.reason = 'invalid_request_boundary'; return null; }
    const local = new AbortController();
    const forward = () => local.abort(); requestSignal.addEventListener('abort', forward, { once: true });
    if (requestSignal.aborted) forward();
    const wasCold = !worker.session, beginning = clock.now();
    const deadline = clock.schedule(() => { capture(); local.abort(); }, config.budget_ms + 1000 + (wasCold ? (worker.startupMs ?? 30000) + 60000 : 0));
    calls++; record.reason = 'request_started'; capture();
    try {
      const result = await worker.predict(request, local.signal);
      capture();
      retainResult(record, result, clock.now() - beginning);
      if (record.phase === 'cold') {
        const measured = result.local_measurement;
        if (finite(measured?.worker_startup_ms)) coldStart = measured.worker_startup_ms;
        if (finite(coldStart) && finite(measured?.request_roundtrip_ms)) {
          preparation = Math.max(0, clock.now() - beginning - coldStart - measured.request_roundtrip_ms);
        }
      }
      return result;
    } catch {
      record.outcome = requestSignal.aborted ? 'cancelled' : local.signal.aborted ? 'deadline' : 'error';
      record.reason = record.outcome === 'error' ? 'worker_request_failed' : record.outcome;
      record.elapsed_ms = Math.ceil(clock.now() - beginning);
      return null;
    } finally { clock.clear(deadline); requestSignal.removeEventListener('abort', forward); }
  };
  const stop = async (phase, observed = worker.session) => {
    if (!observed) return null;
    if (cleanupBySession.has(observed)) return cleanupBySession.get(observed);
    sessions.add(observed);
    let receipt = null, error = null;
    try {
      if (observed.stopping) receipt = await observed.stopping;
      else if (worker.session === observed) receipt = await worker.stop();
      else { await observed.exit; receipt = observed.cleanup ?? null; }
    }
    catch { error = 'cleanup_failed'; failure ??= error; }
    const verified = !error && cleanupMatches(receipt, observed) && worker.session !== observed;
    cleanups.push({ phase, runtime_pid: observed.runtime?.pid ?? null, verified, receipt: receipt ?? null, error });
    cleanupBySession.set(observed, verified);
    if (!verified) { limitations.push(`${phase}: runtime cleanup lacks a matching challenged/reaped receipt.`); failure ??= 'cleanup_unverified'; abort(); }
    return verified;
  };

  try {
    controller.signal.throwIfAborted();
    await stop('before_cold');
    if (failure || worker.session) throw new Error('Prior runtime cleanup failed.');
    notify('cold');
    await run(...plans.cold);
    if (!ownedSession(worker) || !identity) { failure ??= 'cold_runtime_unavailable'; controller.abort(); }
    for (const plan of plans.warm) {
      if (controller.signal.aborted) break;
      notify(plan[1].phase); await run(...plan);
    }
    for (const trace of plans.paced) {
      if (controller.signal.aborted) break;
      if (trace.events.length !== 3) { limitations.push(`${languageOf(trace.source)}: too few graphemes for the three-event typing diagnostic.`); continue; }
      notify('paced');
      await pacedTrace(trace, { clock, signal: controller.signal, run, observe: () => { observedEvents++; } });
      scheduledEvents += 3;
    }
    pacedComplete = plans.paced.every(trace => trace.events.length === 3
      && trace.events.every(event => event.record.observed_at_ms !== null)
      && trace.events.some(event => event.record.terminal_received));

    if (!controller.signal.aborted) {
      notify('cancellation');
      const cancel = new AbortController();
      const forward = () => cancel.abort(); controller.signal.addEventListener('abort', forward, { once: true });
      let settled = false;
      const pending = run(cancellationCase, cancelRecord, cancel.signal).finally(() => { settled = true; });
      const deadline = clock.now() + 2000;
      while (!settled && !worker.session?.pending && clock.now() < deadline && !controller.signal.aborted) await clock.sleep(1, controller.signal);
      if (!settled && worker.session?.pending && !controller.signal.aborted) await clock.sleep(25, controller.signal);
      const session = ownedSession(worker);
      const oldPid = session?.runtime.pid;
      cancellation.attempted = true;
      cancellation.pending_at_abort = Boolean(!settled && session?.pending);
      if (cancellation.pending_at_abort) {
        sessions.add(session); cancel.abort();
        await pending;
        cancellation.rejected_after_abort = cancelRecord.outcome === 'cancelled';
        cancellation.cleanup_verified = await stop('cancellation', session) === true;
        if (!controller.signal.aborted && cancellation.cleanup_verified) {
          const recovered = await run(recoveryCase, recoveryRecord);
          cancellation.recovery_terminal = recovered?.terminal_received === true && recovered.outcome !== 'error';
          cancellation.fresh_runtime = Boolean(ownedSession(worker)?.runtime.pid !== oldPid && ownedSession(worker));
        }
        cancellation.reason = cancellation.rejected_after_abort && cancellation.cleanup_verified
          && cancellation.recovery_terminal && cancellation.fresh_runtime ? 'passed' : 'cancellation_or_recovery_unverified';
      } else { await pending; cancellation.reason = 'request_finished_before_cancellation'; }
      controller.signal.removeEventListener('abort', forward);
    }

    if (!controller.signal.aborted && ownedSession(worker)) {
      notify('sustained');
      const beginning = clock.now();
      let continuousStart = beginning, continuousSession = ownedSession(worker), nextPressure = beginning, index = 0;
      while (clock.now() - beginning < durationSeconds * 1000 && calls < MAX_REQUESTS && !controller.signal.aborted) {
        const dispatch = clock.now();
        if (dispatch >= nextPressure) {
          const sample = await readPressure();
          pressure.push({ at_ms: Math.ceil(dispatch - beginning), memory: sample }); nextPressure = dispatch + 1000;
          notify('sustained');
        }
        const row = inputs[index++ % inputs.length];
        const record = assign(row, 'sustained');
        await run(row, record);
        const pause = Math.min(Math.max(0, 100 - (clock.now() - dispatch)), Math.max(0, durationSeconds * 1000 - (clock.now() - beginning)));
        if (pause && !controller.signal.aborted) await clock.sleep(pause, controller.signal);
        const current = ownedSession(worker);
        if (!current || current !== continuousSession) { continuousStart = clock.now(); continuousSession = current; }
        else sustainedMs = Math.max(sustainedMs, clock.now() - continuousStart);
      }
      if (calls >= MAX_REQUESTS) { failure ??= 'request_limit'; limitations.push('Sustained phase reached the 20,000-request cap.'); }
    }
  } catch {
    if (!controller.signal.aborted) failure ??= 'diagnostic_failed';
  } finally {
    externalCancelled = signal?.aborted === true;
    abort();
    await stop('final');
    for (const session of sessions) await stop('recovered_session', session);
    clock.clear(timeout); signal?.removeEventListener('abort', abort);
    if (worker.beforeStart === wrapper) worker.beforeStart = originalHook;
    else { failure ??= 'selection_changed'; limitations.push('Selected model preparation hook changed during diagnostics.'); }
    activeWorkers.delete(worker);
  }

  if (!externalCancelled && !failure && sustainedMs < durationSeconds * 1000) failure = 'sustained_interval_not_completed';
  const cleanupVerified = cleanups.length > 0 && cleanups.every(row => row.verified);
  const exercised = records.some(row => row.terminal_received && !['error', 'not_run'].includes(row.outcome));
  const reuseRows = records.filter(row => ['repeat', 'warm', 'sustained'].includes(row.phase) && row.terminal_received && row.reused_prompt_tokens !== null);
  const metrics = {
    measured_at_unix_s: clock.unix(), loaded_and_exercised: Boolean(identity && exercised),
    actual_backend: identity?.gpu_layers === 0 ? 'cpu' : 'unknown',
    artifact_verified: Boolean(identity && successfulChecks > 0 && successfulChecks === resourceChecks),
    cold_start_ms: Math.ceil(coldStart ?? 0), preparation_ms: Math.ceil(preparation ?? 0),
    peak_rss_bytes: Math.ceil(peakRss ?? 0), peak_vram_bytes: peakVram,
    sustained_seconds: Math.floor(sustainedMs / 1000), cancellation_recovery_passed: cancellation.reason === 'passed',
    cleanup_verified: cleanupVerified, prompt_reuse_measured: reuseRows.length > 0,
    paced_typing_measured: pacedComplete,
  };
  if (preparation === null) limitations.push('Preload verification/preparation timing was unavailable; zero in evidence denotes unmeasured.');
  if (!reuseRows.length) limitations.push('No terminal cache-reuse counters were available. cache_prompt=true alone does not prove reuse.');
  if (!pressure.some(sample => sample.memory !== null)) limitations.push('Linux memory-pressure samples were unavailable.');
  if (!pacedComplete) limitations.push('The scheduled three-event typing trace was not completely measured for every requested language.');
  return {
    schema: 'badi.qualification-diagnostics.v1', complete: !failure && !externalCancelled && cleanupVerified,
    cancelled: externalCancelled, error: failure, duration_requested_seconds: durationSeconds,
    elapsed_ms: Math.ceil(clock.now() - started), identity, evidenceMetrics: metrics,
    measurements: { cold_start_ms: coldStart, preparation_ms: preparation, resource_recheck_ms: beforeHookMs,
      resource_rechecks: resourceChecks, successful_resource_rechecks: successfulChecks,
      request_preflight_ms: records.map(row => row.preflight_ms).filter(finite),
      peak_rss_bytes: peakRss, peak_vram_bytes: peakVram, sustained_seconds: sustainedMs / 1000,
      languages: summarize(records), warm_repeat: { terminal_counters: reuseRows.length,
        requests_with_actual_reuse: reuseRows.filter(row => row.reused_prompt_tokens > 0).length,
        reused_tokens: reuseRows.reduce((total, row) => total + row.reused_prompt_tokens, 0) },
      paced: { schedule_ms: [0, 100, 250], scheduled_events: scheduledEvents, observed_events: observedEvents, measured: pacedComplete },
      cancellation, cleanup: cleanups, memory_pressure: pressure },
    records, limitations: [...new Set([...limitations,
      'All preassigned opportunities remain in denominators, including coalesced, cancelled, missing and failed outcomes. Timed sustained requests are assigned only at dispatch.',
      'Complete-word latency without a complete result is right-censored at at least 551 ms or budget+1; this is a lower bound, not fabricated delivery.',
      'Preparation is measured call wall minus worker startup and request roundtrip; it includes pre-load checks and supervisor transition overhead.',
      'Synthetic timer-driven typing and process cancellation do not prove actual OS input, browser rendering, editing authority or native undo.',
      'This performance diagnostic assigns no usefulness labels and cannot replace untouched full-addition quality confirmation.',
    ])],
  };
}

async function pacedTrace(trace, { clock, signal, run, observe }) {
  const origin = clock.now();
  let active = false, pending = null, completed = 0;
  return new Promise(resolve => {
    const finish = () => { if (completed === trace.events.length && !active && !pending) { signal.removeEventListener('abort', abort); resolve(); } };
    const dispatch = async event => {
      if (clock.now() - origin >= event.at + TIMING_TARGET_MS) {
        event.record.outcome = 'deadline'; event.record.reason = 'expired_before_dispatch'; finish(); return;
      }
      active = true;
      event.record.started_at_ms = clock.now() - origin;
      await run(event.row, event.record);
      event.record.finished_at_ms = clock.now() - origin;
      const newer = trace.events.some(other => other.at > event.at && other.at <= event.record.finished_at_ms);
      if (newer) { event.record.reason = 'superseded'; event.record.complete_word_ms = null; }
      else if (event.record.complete_word_ms !== null && event.record.finished_at_ms <= event.at + TIMING_TARGET_MS) {
        event.record.delivered_at_ms = event.record.finished_at_ms;
        event.record.complete_word_ms = Math.ceil(event.record.finished_at_ms - event.at);
      } else event.record.complete_word_ms = null;
      active = false;
      if (pending && !signal.aborted) { const next = pending; pending = null; void dispatch(next); }
      else { if (pending) { pending.record.outcome = 'not_run'; pending.record.reason = 'cancelled_before_dispatch'; pending = null; } finish(); }
    };
    const timers = trace.events.map(event => {
      event.record.scheduled_at_ms = event.at;
      return clock.schedule(() => {
        completed++; observe(); event.record.observed_at_ms = clock.now() - origin;
        if (signal.aborted) { event.record.reason = 'cancelled_before_event'; finish(); return; }
        if (active) {
          if (pending) { pending.record.outcome = 'coalesced'; pending.record.reason = 'newer_pending_event'; }
          pending = event;
        } else void dispatch(event);
        finish();
      }, event.at);
    });
    const abort = () => {
      for (const timer of timers) clock.clear(timer);
      completed = trace.events.length;
      if (pending) { pending.record.reason = 'cancelled_before_dispatch'; pending = null; }
      finish();
    };
    signal.addEventListener('abort', abort, { once: true });
    if (signal.aborted) abort();
  });
}

function summarize(records) {
  return [...new Set(records.map(row => row.language))].map(language => {
    const rows = records.filter(row => row.language === language);
    const latencies = rows.map(row => row.complete_word_ms ?? Math.max(TIMING_TARGET_MS + 1, row.budget_ms + 1, row.elapsed_ms ?? 0));
    return { language, total: rows.length,
      outcomes: Object.fromEntries(['suggestion', 'abstention', 'deadline', 'error', 'cancelled', 'coalesced', 'not_run'].map(outcome => [outcome, rows.filter(row => row.outcome === outcome).length])),
      complete_on_time: rows.filter(row => row.complete_word_ms !== null && row.complete_word_ms <= TIMING_TARGET_MS).length,
      p50_complete_word_ms_lower_bound: percentile(latencies, .5), p95_complete_word_ms_lower_bound: percentile(latencies, .95),
      all_request_elapsed_ms: rows.map(row => row.elapsed_ms),
    };
  });
}
