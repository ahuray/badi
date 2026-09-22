import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { open, readFile, realpath } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { parseArgs, isDeepStrictEqual } from 'node:util';
import { StringDecoder } from 'node:string_decoder';
import { setTimeout as delay } from 'node:timers/promises';
import { validateSuite, planComparison, scorePrediction, scoreCompleteWordPrefixes } from './cases.mjs';
import { workerRequest } from './server.mjs';
import { processIdentity } from './worker.mjs';
import { OUTPUT_ROOT, WORKER_BINARY, fileIdentity, sourceHashes, prepareOutput, validateCleanup } from './run.mjs';
import { modelArtifactPath, readModelArtifact, modelArtifactProvenance, modelArtifactArgs,
  assertModelArtifactUnchanged, assertModelArtifactIdentity } from './model-artifact.mjs';

const TIMES = [0, 100, 250, 500];
const ARMS = ['cold', 'primed'];
const GENERATION = { mode: 'healed', budget_ms: 550, max_tokens: 8, cache_prompt: true, temperature: 0, seed: 42 };
const DISPOSITIONS = ['eligible', 'coalesced', 'superseded', 'deadline', 'error', 'unavailable'];
const SHA = /^[a-f0-9]{64}$/u;
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const json = value => JSON.stringify(value, null, 2) + '\n';
const same = isDeepStrictEqual;
const number = value => typeof value === 'number' && Number.isFinite(value) && value >= 0;
const optionalNumber = value => value === null || number(value);
const object = value => value && typeof value === 'object' && !Array.isArray(value);
const exactKeys = (value, keys) => object(value) && same(Object.keys(value).sort(), [...keys].sort());
const fail = message => { throw new Error(message); };
const code = value => value === null || (typeof value === 'string' && /^[a-z0-9_]{1,96}$/u.test(value));

export function parsePacedOptions(args, cwd = process.cwd()) {
  const { values } = parseArgs({ args, strict: true, options: {
    suite: { type: 'string' }, decision: { type: 'string' }, output: { type: 'string' },
    'model-artifact': { type: 'string' },
  } });
  if (!values.suite || !values.decision || !values.output) fail('Supply --suite PATH --decision PATH --output NEWDIR.');
  return { suitePath: resolve(cwd, values.suite), decisionPath: resolve(cwd, values.decision), outputPath: resolve(cwd, values.output),
    ...(values['model-artifact'] !== undefined ? { modelArtifactPath: modelArtifactPath(values['model-artifact']) } : {}) };
}

export function validateDecision(value) {
  if (!object(value) || value.schema !== 'badi.paced-context.decision.v1' || !SHA.test(value.suite_sha256)
    || !same(value.arms, ARMS) || value.order_seed !== 71 || !object(value.generation)
    || !exactKeys(value.generation, Object.keys(GENERATION))
    || Object.entries(GENERATION).some(([key, expected]) => value.generation[key] !== expected)
    || value.prelude?.duration_ms !== 1500 || value.prelude?.primed_n_predict !== 1
    || value.prelude?.both_arms_wait_the_same_fixed_interval !== true
    || !same(value.scheduling?.events_ms, TIMES) || value.scheduling?.active_jobs !== 1
    || value.scheduling?.pending_jobs !== 1) fail('Unsupported paced experiment decision.');
  return value;
}

export function planPacedTrials(rawSuite) {
  const suite = validateSuite(rawSuite);
  if (suite.cases.length > 48 || suite.cases.some(row => !row.trace_id)) fail('Use at most twelve complete four-event traces.');
  const groups = new Map();
  for (const row of suite.cases) {
    if (!groups.has(row.trace_id)) groups.set(row.trace_id, []);
    groups.get(row.trace_id).push(row);
  }
  for (const rows of groups.values()) {
    if (rows.length !== 4 || !rows[0].context.trim() || !/^(en|de|fa)(-|$)/iu.test(rows[0].language)
      || rows.some((row, i) => row.step !== i || row.at_ms !== TIMES[i] || row.language !== rows[0].language
        || row.context !== rows[0].context || row.style !== rows[0].style
        || (i && (!row.prefix.startsWith(rows[i - 1].prefix) || [...row.prefix].length !== [...rows[i - 1].prefix].length + 1)))) {
      fail('Paced traces require fixed event times, stable context/style/language and one-scalar additions.');
    }
  }
  const plan = planComparison(suite.cases, ARMS.map(id => ({ id })), 71);
  const trials = [];
  for (let i = 0; i < plan.length; i += 4) {
    const chunk = plan.slice(i, i + 4);
    if (chunk.some(row => row.group_id !== chunk[0].group_id || row.config_id !== chunk[0].config_id)) fail('Invalid grouped comparison plan.');
    trials.push({ trace_id: chunk[0].group_id.slice('trace:'.length), arm: chunk[0].config_id,
      cases: chunk.map(row => suite.cases.find(value => value.id === row.case_id)) });
  }
  return { suite, trials };
}

export function trialInput(trial) {
  // Keep every observed revision exact, including a pending orthographic joiner.
  // Rust validates that paced-only transient and records it without inference.
  return { schema: 'badi.paced-probe.request.v1', arm: trial.arm, prelude_ms: 1500,
    events: trial.cases.map(row => ({ at_ms: row.at_ms, request: workerRequest(row, { id: 'healed', ...GENERATION }) })) };
}

function validateIdentity(value) {
  if (!object(value) || !SHA.test(value.binary_sha256) || !SHA.test(value.model_sha256)
    || value.context_size !== 2048 || typeof value.launch_contract_id !== 'string') fail('Invalid paced runtime identity.');
}

export function validatePacedReport(report, input, ready = null, artifact = null) {
  const keys = ['schema', 'arm', 'complete', 'cancelled', 'error', 'startup_ms', 'prelude', 'latency_ms',
    'identity', 'runtime_identity_sha256', 'cleanup', 'cleanup_ms', 'events'];
  if (!exactKeys(report, keys) || report.schema !== 'badi.paced-probe.report.v1' || report.arm !== input.arm
    || typeof report.complete !== 'boolean' || typeof report.cancelled !== 'boolean' || !code(report.error)
    || !number(report.startup_ms) || !number(report.latency_ms) || !optionalNumber(report.cleanup_ms)
    || !exactKeys(report.prelude, ['duration_ms', 'elapsed_ms', 'preflight_ms', 'priming_ms', 'metrics'])
    || report.prelude.duration_ms !== 1500 || !number(report.prelude.elapsed_ms)
    || !optionalNumber(report.prelude.preflight_ms) || !optionalNumber(report.prelude.priming_ms)
    || !Array.isArray(report.events) || report.events.length !== 4) fail('Invalid paced report contract.');
  if (report.identity !== null) validateIdentity(report.identity);
  if (report.identity !== null) assertModelArtifactIdentity(report.identity, artifact);
  if (ready && (!same(report.identity, ready.identity) || report.runtime_identity_sha256 !== ready.runtime_identity_sha256)) fail('Paced runtime identity changed.');
  if (report.identity === null ? report.runtime_identity_sha256 !== null : !SHA.test(report.runtime_identity_sha256)) fail('Missing paced identity binding.');
  if (report.cleanup !== null) {
    validateCleanup(report.cleanup);
    if (report.cleanup.runtime_identity_sha256 !== report.runtime_identity_sha256
      || (ready && report.cleanup.process_id !== ready.runtime_pid)) fail('Paced cleanup identity differs.');
  }
  const timingFailure = ['active_deadline', 'prelude_deadline'].includes(report.error);
  if (report.complete && (report.cancelled || (report.error !== null && !timingFailure) || !report.identity
    || !report.cleanup?.reaped || !report.cleanup?.challenge_completed)) fail('Paced completion lacks verified cleanup.');
  if (input.arm === 'cold' && (report.prelude.preflight_ms !== null || report.prelude.priming_ms !== null || report.prelude.metrics !== null)) fail('Cold arm cannot claim priming.');
  if (report.complete && report.error !== 'prelude_deadline') {
    if (report.prelude.elapsed_ms < 1500) fail('Paced run shortened its fixed prelude.');
    if (input.arm === 'primed') {
      const metrics = report.prelude.metrics;
      if (!number(report.prelude.preflight_ms) || !number(report.prelude.priming_ms) || !object(metrics)
        || metrics.resolved_n_predict !== 1 || metrics.tokens_predicted !== 1 || metrics.predicted_n !== 1
        || metrics.returned_token_count !== 1 || metrics.truncated !== false) fail('Primed arm lacks its one-token priming evidence.');
    }
  }
  const rowKeys = ['request_id', 'at_ms', 'observed_at_ms', 'deadline_ms', 'started_at_ms', 'finished_at_ms',
    'queue_ms', 'scheduling_lateness_ms', 'delivered_at_ms', 'disposition', 'reason', 'result'];
  let previousDispatch = null;
  for (let i = 0; i < 4; i++) {
    const row = report.events[i], source = input.events[i];
    if (!exactKeys(row, rowKeys) || row.request_id !== source.request.id || row.at_ms !== TIMES[i]
      || row.deadline_ms !== TIMES[i] + 550 || !DISPOSITIONS.includes(row.disposition) || !code(row.reason)
      || ['observed_at_ms', 'started_at_ms', 'finished_at_ms', 'queue_ms', 'scheduling_lateness_ms', 'delivered_at_ms'].some(key => !optionalNumber(row[key]))) fail('Invalid paced event contract.');
    if (row.observed_at_ms !== null && row.observed_at_ms < row.at_ms) fail('Paced event observed before its schedule.');
    if ((row.observed_at_ms === null) !== (row.scheduling_lateness_ms === null)
      || (row.started_at_ms === null) !== (row.queue_ms === null)) fail('Paced timing availability is inconsistent.');
    if (row.started_at_ms !== null && (row.observed_at_ms === null || row.started_at_ms < row.observed_at_ms)) fail('Paced request started before its event.');
    if (row.started_at_ms !== null) {
      if (row.started_at_ms >= row.deadline_ms || (i < 3 && row.started_at_ms >= TIMES[i + 1])) fail('An expired or superseded snapshot was dispatched.');
      if (previousDispatch && (previousDispatch.finished_at_ms === null || previousDispatch.finished_at_ms > row.started_at_ms)) fail('Paced requests overlap.');
      previousDispatch = row;
    }
    if (row.disposition === 'coalesced' && (row.started_at_ms !== null || row.finished_at_ms !== null || row.result !== null)) fail('A coalesced event was dispatched.');
    if (row.finished_at_ms !== null && (row.started_at_ms === null || row.finished_at_ms < row.started_at_ms)) fail('Paced request finished before dispatch.');
    if (row.observed_at_ms !== null && (row.scheduling_lateness_ms === null
      || Math.abs(row.scheduling_lateness_ms - (row.observed_at_ms - row.at_ms)) > 0.001)) fail('Paced scheduling lateness does not match its event.');
    if (row.started_at_ms !== null && (row.queue_ms === null
      || Math.abs(row.queue_ms - (row.started_at_ms - row.observed_at_ms)) > 0.001)) fail('Paced queue time does not match dispatch.');
    const trailingJoiner = source.request.before.endsWith('\u200c');
    if (trailingJoiner || row.reason === 'transient_joiner_no_request') {
      if (!trailingJoiner || row.disposition !== 'unavailable' || row.started_at_ms !== null
        || row.finished_at_ms !== null || row.result !== null || row.delivered_at_ms !== null
        || (row.observed_at_ms !== null) !== (row.reason === 'transient_joiner_no_request')) {
        fail('A transient joiner must be observed without dispatch or retain its earlier stopping reason.');
      }
    }
    if (row.disposition === 'eligible') {
      if (!object(row.result) || row.result.outcome !== 'suggestion' || typeof row.result.text !== 'string'
        || !row.result.text.trim() || row.result.word_complete !== true || row.result.shape_valid !== true
        || row.result.terminal_received !== true || row.result.replace_before != null
        || row.delivered_at_ms === null || row.finished_at_ms === null || row.delivered_at_ms < row.finished_at_ms
        || row.delivered_at_ms > row.deadline_ms || (i < 3 && row.delivered_at_ms >= TIMES[i + 1])) fail('Stale or late paced output was marked eligible.');
    } else if (row.delivered_at_ms !== null) fail('Ineligible paced output cannot be delivered.');
    if (row.result !== null && (!object(row.result) || row.result.id !== source.request.id || !number(row.result.latency_ms)
      || row.started_at_ms === null || row.finished_at_ms === null
      || !same(row.result.config, source.request.config) || !same(row.result.identity, report.identity)
      || !['suggestion', 'abstention', 'deadline', 'error'].includes(row.result.outcome))) fail('Paced result differs from its request.');
  }
  if (report.error === 'prelude_deadline' && report.events.some(row => row.disposition !== 'unavailable'
    || row.observed_at_ms !== null || row.started_at_ms !== null || row.finished_at_ms !== null
    || row.result !== null || row.delivered_at_ms !== null)) fail('Paced events ran after a failed prelude.');
  if (report.error === 'active_deadline') {
    const expired = report.events.findIndex(row => row.disposition === 'deadline' && row.reason === 'active_deadline');
    if (expired < 0 || report.events.slice(expired + 1).some(row => row.started_at_ms !== null || row.result !== null
      || !['unavailable', 'coalesced'].includes(row.disposition))) fail('Paced runtime was reused after its hard deadline.');
  }
  return report;
}

async function captureRuntime(ready, workerPid, cache) {
  if (!exactKeys(ready, ['type', 'schema', 'runtime_pid', 'identity', 'runtime_identity_sha256'])
    || ready.type !== 'ready' || ready.schema !== 'badi.prediction-lab.worker.v1' || !SHA.test(ready.runtime_identity_sha256)) fail('Invalid paced readiness.');
  validateIdentity(ready.identity);
  const observed = await processIdentity(ready.runtime_pid);
  if (!observed || observed.parent !== workerPid || observed.group !== observed.pid || !observed.executable.endsWith('/llama-server')) fail('Paced runtime ownership mismatch.');
  const executable = await fileIdentity(`/proc/${observed.pid}/exe`, cache);
  const after = await processIdentity(observed.pid);
  if (!after || !same(observed, after) || executable.sha256 !== ready.identity.binary_sha256) fail('Paced runtime executable mismatch.');
  return { ...observed, verified_executable: executable };
}

// The diagnostic owns one process until close and verified runtime disappearance.
// A transport failure never silently turns a surviving process into another trial.
export async function runPacedChild(input, { executable = WORKER_BINARY, args = ['--paced-probe'], signal,
  timeoutMs = 65000, terminateMs = 6500, fileCache = new Map(), artifact = null } = {}) {
  const started = performance.now();
  signal?.throwIfAborted();
  const frame = JSON.stringify(input) + '\n';
  if (Buffer.byteLength(frame) > 256 * 1024) fail('Paced input exceeds 256 KiB.');
  // Warm only the metadata-bound hashing cache, before the child can exit.
  // Its actual /proc executable is independently opened and matched below.
  await fileIdentity(executable, fileCache);
  await assertModelArtifactUnchanged(artifact);
  signal?.throwIfAborted();
  const child = spawn(executable, [...args, ...modelArtifactArgs(artifact)], { stdio: ['pipe', 'pipe', 'ignore'] });
  const events = []; let ready = null; let report = null; let failure = null; let runtime = null;
  let capture = Promise.resolve(null); let captureWorker = Promise.resolve(null); let bytes = 0; let pending = ''; let force; let finished = false;
  const decoder = new StringDecoder('utf8');
  const stop = () => {
    if (finished) return;
    child.kill('SIGTERM');
    force ??= setTimeout(() => { child.kill('SIGKILL'); child.stdout.destroy(); }, terminateMs);
  };
  const reject = message => { failure ??= message; stop(); };
  const abort = () => reject('Paced child cancelled.');
  const closed = new Promise(resolveClose => child.once('close', (exitCode, exitSignal) => {
    finished = true; resolveClose({ exit_code: exitCode, exit_signal: exitSignal });
  }));
  child.once('spawn', () => {
    captureWorker = (async () => {
      const process = await processIdentity(child.pid);
      if (!process) fail('Executed paced worker disappeared before inspection.');
      return { pid: child.pid, start_ticks: process.start, executable: await fileIdentity(`/proc/${child.pid}/exe`, fileCache) };
    })().catch(() => { reject('Could not verify the executed paced worker.'); return null; });
  });
  child.once('error', () => reject('Could not start the paced worker.'));
  child.stdin.on('error', () => reject('Paced input stream closed.'));
  child.stdout.on('data', chunk => {
    bytes += chunk.length;
    if (bytes > 2 * 1024 * 1024) { reject('Paced response exceeded 2 MiB.'); return; }
    pending += decoder.write(chunk);
    for (;;) {
      const end = pending.indexOf('\n');
      if (end < 0) break;
      const line = pending.slice(0, end); pending = pending.slice(end + 1);
      if (!line) continue;
      let event;
      try { event = JSON.parse(line); } catch { reject('Invalid paced JSON response.'); continue; }
      events.push(event);
      if (report) { reject('Paced worker emitted data after its terminal report.'); continue; }
      if (event?.type === 'ready' && !ready) {
        ready = event;
        capture = captureRuntime(ready, child.pid, fileCache).then(value => {
          runtime = value; // Retain verified ownership even if the model claim fails.
          try { assertModelArtifactIdentity(ready.identity, artifact); }
          catch { reject('The runtime model identity differs from the frozen Lab artifact.'); }
        })
          .catch(() => reject('Could not verify paced runtime ownership.'));
      } else if (exactKeys(event, ['type', 'report']) && event.type === 'paced_report') report = event.report;
      else reject('Unexpected paced worker event.');
    }
  });
  signal?.addEventListener('abort', abort, { once: true });
  const timer = setTimeout(() => reject('Paced worker exceeded its process deadline.'), timeoutMs);
  child.stdin.end(frame);
  if (signal?.aborted) abort();
  const exit = await closed;
  clearTimeout(timer); clearTimeout(force); signal?.removeEventListener('abort', abort);
  await capture;
  const execution = await captureWorker;
  if ((pending + decoder.end()).trim()) failure ??= 'Paced response ended with an incomplete event.';
  let cleanupConfirmed = false;
  if (runtime) {
    try {
      const stillOwned = async () => {
        const current = await processIdentity(runtime.pid);
        if (!current || current.start !== runtime.start) return false;
        if (current.group !== runtime.group || current.executable_key !== runtime.executable_key) fail('Owned paced process identity changed.');
        return true;
      };
      for (const [signal, attempts] of [['SIGTERM', 10], ['SIGKILL', 10]]) {
        if (!await stillOwned()) break;
        try { process.kill(-runtime.pid, signal); } catch (error) { if (error.code !== 'ESRCH') throw error; }
        for (let i = 0; i < attempts && await stillOwned(); i++) await delay(100);
      }
      cleanupConfirmed = !await stillOwned();
    } catch { cleanupConfirmed = false; }
  } else if (ready) cleanupConfirmed = false;
  if (!cleanupConfirmed) failure ??= 'Paced runtime cleanup could not be confirmed.';
  try { validatePacedReport(report, input, ready, artifact); } catch { failure ??= 'Invalid or missing paced terminal report.'; }
  if (exit.exit_code !== 0 && (!report || report.complete)) failure ??= 'Paced worker exited unsuccessfully.';
  return { report, ready, events, execution, runtime, exit, error: failure, cleanup_confirmed: cleanupConfirmed,
    owner_wall_ms: performance.now() - started };
}

export async function pacedSourceHashes() {
  return { ...await sourceHashes(), ...await sourceHashes(['evaluation/writing/lab/paced.mjs',
    'broker/src/writing_lab/paced_probe.rs', 'broker/src/writing_lab/paced_probe/scheduler.rs']) };
}

export function summarizePaced(trials) {
  return Object.fromEntries(ARMS.map(arm => {
    const selected = trials.filter(trial => trial.arm === arm);
    const rows = selected.flatMap(trial => trial.records);
    const final = selected.map(trial => trial.records[3]);
    return [arm, { scheduled_events: rows.length, scheduled_traces: selected.length,
      dispositions: Object.fromEntries(DISPOSITIONS.map(value => [value, rows.filter(row => row.disposition === value).length])),
      eligible_final_traces: final.filter(row => row.disposition === 'eligible').length,
      first_word_reference_matches_all_events: rows.filter(row => row.score?.first_word_match === true).length,
      first_word_reference_matches_final_traces: final.filter(row => row.score?.first_word_match === true).length,
      reference_aligned_graphemes: rows.reduce((total, row) => total + (row.score?.matching_reference_graphemes ?? 0), 0),
      eligible_event_latencies_ms: rows.filter(row => row.disposition === 'eligible').map(row => row.delivered_at_ms - row.at_ms),
      reviewed_useful_final_traces: null, reviewed_harmful_final_traces: null,
      multiword_reference_agreement: Object.fromEntries([2, 3, 4].map(words => [words, {
        matches: rows.filter(row => row.word_scores[words].match === true).length,
        scorable_events: rows.filter(row => row.word_scores[words].scorable).length,
        scheduled_events: rows.length,
        final_matches: final.filter(row => row.word_scores[words].match === true).length,
        scorable_final_traces: final.filter(row => row.word_scores[words].scorable).length,
        scheduled_final_traces: final.length,
      }])),
      observed_keystrokes_saved: null }];
  }));
}

export async function runPacedSuite(options, { runTrial, outputRoot = OUTPUT_ROOT, signal, onProgress = () => {},
  captureSources = pacedSourceHashes, workerPath = WORKER_BINARY } = {}) {
  signal?.throwIfAborted();
  const suiteBytes = await readFile(options.suitePath), decisionBytes = await readFile(options.decisionPath);
  if (suiteBytes.length > 512 * 1024 || decisionBytes.length > 64 * 1024) fail('Paced input exceeds its file bounds.');
  let decision, document;
  try { decision = validateDecision(JSON.parse(decisionBytes)); document = JSON.parse(suiteBytes); }
  catch { fail('Invalid paced decision or suite JSON.'); }
  if (hash(suiteBytes) !== decision.suite_sha256) fail('Suite bytes differ from the frozen decision.');
  const { suite, trials: plan } = planPacedTrials(document);
  const artifact = options.modelArtifactPath ? await readModelArtifact(options.modelArtifactPath) : null;
  const output = await prepareOutput(options.outputPath, outputRoot);
  const save = async (name, value) => {
    const file = await open(resolve(output, name), 'wx', 0o600);
    try { await file.writeFile(Buffer.isBuffer(value) ? value : json(value)); } finally { await file.close(); }
  };
  const sources = await captureSources();
  const declaredWorker = runTrial ? null : await fileIdentity(workerPath);
  const before = { suite_sha256: hash(suiteBytes), decision_sha256: hash(decisionBytes), source_sha256: sources,
    declared_worker: declaredWorker, model_artifact: modelArtifactProvenance(artifact) };
  await save('suite-input.json', suiteBytes); await save('decision-input.json', decisionBytes);
  if (artifact) await save('model-artifact-input.json', artifact.source);
  await save('run.json', { schema: 'badi.paced-context.cli-run.v1', started_at: new Date().toISOString(),
    input: { suite_path: await realpath(options.suitePath), decision_path: await realpath(options.decisionPath) },
    provenance: before, plan: plan.map(trial => ({ trace_id: trial.trace_id, arm: trial.arm, case_ids: trial.cases.map(row => row.id) })) });
  const fileCache = new Map(); const trials = []; let halted = false;
  for (const trial of plan) {
    const input = trialInput(trial); let child = null; let failure = null;
    if (halted || signal?.aborted) failure = signal?.aborted ? 'cancelled' : 'prior_trial_failed';
    else {
      try {
        await assertModelArtifactUnchanged(artifact);
        child = await (runTrial ?? runPacedChild)(input, { executable: workerPath, signal, fileCache, artifact });
        if (child.error) fail(child.error);
        validatePacedReport(child.report, input, child.ready, artifact);
        if (!child.cleanup_confirmed) fail('Paced cleanup was incomplete.');
        if (!runTrial && child.execution?.executable.sha256 !== declaredWorker.sha256) fail('Executed paced worker differs from frozen binary.');
        if (!child.report.complete) { failure = 'trial_failed'; halted = true; }
      } catch { failure = signal?.aborted ? 'cancelled' : 'trial_failed'; halted = true; }
    }
    let validReport = null;
    try {
      if (child?.report && !child.error && child.cleanup_confirmed
        && (runTrial || child.execution?.executable.sha256 === declaredWorker.sha256)) validReport = validatePacedReport(child.report, input, child.ready, artifact);
    } catch { /* Preserve raw diagnostic, score no unvalidated output. */ }
    const records = trial.cases.map((row, i) => {
      const event = validReport?.events[i] ?? { request_id: input.events[i].request.id, at_ms: row.at_ms,
        deadline_ms: row.at_ms + 550, disposition: 'unavailable', reason: failure ?? 'invalid_report', result: null, delivered_at_ms: null };
      const eligible = event.disposition === 'eligible';
      return { ...event, case_id: row.id, score: eligible ? scorePrediction(row, event.result) : null,
        word_scores: scoreCompleteWordPrefixes(row, eligible ? event.result : { outcome: 'abstention', text: null, latency_ms: 0 }) };
    });
    trials.push({ trace_id: trial.trace_id, arm: trial.arm, error: failure,
      wire_sha256: hash(JSON.stringify(input) + '\n'), wire_input: input, child, records });
    onProgress({ completed: trials.length, total: plan.length, trace_id: trial.trace_id, arm: trial.arm });
  }
  let after = null;
  try { after = { suite_sha256: hash(await readFile(options.suitePath)), decision_sha256: hash(await readFile(options.decisionPath)),
    source_sha256: await captureSources(), declared_worker: runTrial ? null : await fileIdentity(workerPath),
    model_artifact: modelArtifactProvenance(artifact ? await readModelArtifact(artifact.path) : null) }; } catch { halted = true; }
  const stable = same(before, after);
  const executions = trials.map(trial => trial.child?.execution).filter(Boolean);
  const report = { schema: 'badi.paced-context.cli-report.v1', finished_at: new Date().toISOString(),
    complete: !halted && !signal?.aborted && stable && trials.every(trial => !trial.error), cancelled: Boolean(signal?.aborted),
    provenance: { execution_kind: runTrial ? 'injected_fixture' : 'local_model_worker', node: process.version, before, after, stable,
      executed_workers: executions, valid_executed_identity: !runTrial && stable && executions.length === plan.length
        && executions.every(value => value.executable.sha256 === declaredWorker.sha256)
        && trials.every(trial => trial.child?.runtime && !trial.child.error && trial.child.cleanup_confirmed) },
    measurements: summarizePaced(trials), trials,
    measurement: { delivery_semantics: 'delivered_at_ms is the Rust scheduler observation that a current terminal result is eligible. It is not Node receipt time, visible UI rendering or actual displayed latency.',
      eligible_latency: 'Scheduled event to terminal eligibility observation, including scheduling lateness, queue, preflight and inference.',
      owner_wall_time: 'Child owner_wall_ms includes executable inspection, child startup, the full diagnostic and observed cleanup; Rust startup/prelude/latency fields measure their narrower owned-runtime phases.',
      denominator: 'All scheduled events and final snapshots remain in denominators, including coalesced, stale, failed and unexecuted opportunities.',
      prelude: 'Both arms wait 1500 ms after runtime startup. Only supplied stable context/style is assumed available during priming; no draft is available then.' },
    limitation: 'Synthetic development diagnostics. Priming availability is assumed; usefulness needs independent full-continuation review. No application acceptance or typing savings measured.' };
  await save('report.json', report);
  return { output, report, suite };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const controller = new AbortController(); const cancel = () => controller.abort();
  process.once('SIGINT', cancel); process.once('SIGTERM', cancel);
  try {
    const result = await runPacedSuite(parsePacedOptions(process.argv.slice(2)), { signal: controller.signal,
      onProgress: value => process.stdout.write(`${value.completed}/${value.total} ${value.trace_id} ${value.arm}\n`) });
    process.stdout.write(JSON.stringify({ output: result.output, complete: result.report.complete,
      valid_executed_identity: result.report.provenance.valid_executed_identity }) + '\n');
    if (!result.report.complete || !result.report.provenance.valid_executed_identity) process.exitCode = 1;
  } catch { process.stderr.write('Paced diagnostic failed; inspect its private report if created.\n'); process.exitCode = 1; }
  finally { process.off('SIGINT', cancel); process.off('SIGTERM', cancel); }
}
