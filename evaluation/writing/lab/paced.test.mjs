import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdtemp, writeFile, readFile, rm, stat, copyFile, chmod } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { parsePacedOptions, validateDecision, planPacedTrials, trialInput, validatePacedReport,
  runPacedChild, runPacedSuite } from './paced.mjs';
import { readModelArtifact } from './model-artifact.mjs';

const hash = value => createHash('sha256').update(value).digest('hex');
const identity = { launch_contract_id: 'fixture-no-model', binary_sha256: 'b'.repeat(64), model_sha256: 'c'.repeat(64), context_size: 2048 };
const identityHash = 'a'.repeat(64);
const cleanup = { process_id: 1234, runtime_identity_sha256: identityHash, challenge_completed: true, reaped: true, exit_code: 0 };
const times = [0, 100, 250, 500];
async function artifactFixture(directory) {
  const weights_path = join(directory, 'fixture.gguf'), path = join(directory, 'artifact.json');
  await writeFile(weights_path, 'fixture');
  await writeFile(path, JSON.stringify({ schema: 'badi.lab-model-artifact.v1', weights_path,
    sha256: hash('fixture'), bytes: 7, alias: 'fixture-artifact' }) + '\n');
  return readModelArtifact(path);
}
const artifactIdentity = artifact => ({ ...identity, model_origin: 'explicit_lab_artifact',
  model_sha256: artifact.descriptor.sha256, model_size: artifact.descriptor.bytes, model_alias: artifact.descriptor.alias });
function suite(count = 2) {
  return { schema: 'badi.prediction-suite.v1', name: 'Paced fixture', cases: Array.from({ length: count }, (_, trace) =>
    times.map((at_ms, step) => ({ id: `PRIVATE_CASE_${trace}_${step}`, trace_id: `PRIVATE_TRACE_${trace}`, step, at_ms,
      language: 'en', context: 'PRIVATE_CONTEXT: a blue door beside the kitchen.', style: 'PRIVATE_STYLE: keep directions short.',
      prefix: `The guide has a blue ${'door'.slice(0, step)}`, expected: [`${'door'.slice(step)} near the kitchen`, 'UNFORWARDED_REFERENCE'] }))).flat() };
}
function decision(bytes) {
  return { schema: 'badi.paced-context.decision.v1', suite_sha256: hash(bytes), arms: ['cold', 'primed'], order_seed: 71,
    generation: { mode: 'healed', budget_ms: 550, max_tokens: 8, cache_prompt: true, temperature: 0, seed: 42 },
    prelude: { duration_ms: 1500, primed_n_predict: 1, both_arms_wait_the_same_fixed_interval: true },
    scheduling: { events_ms: times, active_jobs: 1, pending_jobs: 1 } };
}
function reportFor(input, runtimeIdentity = identity, runtimePid = 1234) {
  return { schema: 'badi.paced-probe.report.v1', arm: input.arm, complete: true, cancelled: false, error: null,
    startup_ms: 10, prelude: { duration_ms: 1500, elapsed_ms: 1500, preflight_ms: input.arm === 'primed' ? 2 : null,
      priming_ms: input.arm === 'primed' ? 30 : null, metrics: input.arm === 'primed' ? {
        resolved_n_predict: 1, tokens_predicted: 1, predicted_n: 1, returned_token_count: 1, truncated: false } : null },
    latency_ms: 2211, identity: runtimeIdentity, runtime_identity_sha256: identityHash, cleanup: { ...cleanup, process_id: runtimePid }, cleanup_ms: 1,
    events: input.events.map((event, i) => ({ request_id: event.request.id, at_ms: event.at_ms, deadline_ms: event.at_ms + 550,
      observed_at_ms: event.at_ms, started_at_ms: i === 3 ? 520 : null, finished_at_ms: i === 3 ? 700 : null,
      queue_ms: i === 3 ? 20 : null, scheduling_lateness_ms: 0, delivered_at_ms: i === 3 ? 700 : null,
      disposition: i === 3 ? 'eligible' : 'coalesced', reason: i === 3 ? 'on_time' : 'replaced_pending',
      result: i === 3 ? { type: 'result', id: event.request.id, config: Object.fromEntries(Object.entries(event.request.config).reverse()),
        identity: Object.fromEntries(Object.entries(runtimeIdentity).reverse()), outcome: 'suggestion', text: 'r near the kitchen',
        word_complete: true, shape_valid: true, terminal_received: true, replace_before: null, latency_ms: 180 } : null })) };
}
const childFor = input => ({ report: reportFor(input), ready: null, error: null, cleanup_confirmed: true,
  execution: { pid: 42, start_ticks: '123', executable: { path: '/fixture/only', sha256: 'f'.repeat(64), bytes: 1 } } });
async function fixture(t, document = suite()) {
  const directory = await mkdtemp(join(tmpdir(), 'badi-paced-test-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const bytes = JSON.stringify(document) + '\n';
  const suitePath = join(directory, 'suite.json'), decisionPath = join(directory, 'decision.json');
  await writeFile(suitePath, bytes); await writeFile(decisionPath, JSON.stringify(decision(bytes)));
  const outputRoot = join(directory, 'output/writing');
  return { directory, bytes, outputRoot, options: { suitePath, decisionPath, outputPath: join(outputRoot, 'result') },
    dependencies: { outputRoot, captureSources: async () => ({ 'fixture-source': '0'.repeat(64) }) } };
}

test('paced CLI requires explicit decision and supports only the frozen settings', () => {
  assert.deepEqual(parsePacedOptions(['--suite', 's.json', '--decision', 'd.json', '--output', 'out'], '/repo'),
    { suitePath: '/repo/s.json', decisionPath: '/repo/d.json', outputPath: '/repo/out' });
  assert.throws(() => parsePacedOptions(['--suite', 's', '--output', 'o']));
  assert.throws(() => parsePacedOptions(['--suite', 's', '--decision', 'd', '--output', 'o', '--budget-ms', '5000']));
  const args = ['--suite', 's', '--decision', 'd', '--output', 'o'];
  assert.equal(parsePacedOptions([...args, '--model-artifact', '/private/artifact.json']).modelArtifactPath, '/private/artifact.json');
  for (const extra of [['--model-artifact', 'relative.json'], ['--model-artifact', '/private/../artifact.json'],
    ['--model-artifact', ''], ['--runtime-args', '--threads=16']]) assert.throws(() => parsePacedOptions([...args, ...extra]));
  const value = decision('fixture');
  assert.equal(validateDecision(value), value);
  for (const change of [v => v.order_seed++, v => v.prelude.duration_ms++, v => v.prelude.primed_n_predict = 0,
    v => v.generation.max_tokens = 32, v => v.scheduling.events_ms[1] = 101, v => v.scheduling.active_jobs = 2]) {
    const changed = structuredClone(value); change(changed); assert.throws(() => validateDecision(changed));
  }
});

test('paired plan counterbalances whole traces and wire requests exclude references and identifiers', () => {
  const plan = planPacedTrials(suite(4));
  assert.deepEqual(planPacedTrials(suite(4)), plan);
  assert.equal(plan.trials.length, 8);
  const firstArms = [];
  for (let i = 0; i < plan.trials.length; i += 2) {
    assert.equal(plan.trials[i].trace_id, plan.trials[i + 1].trace_id);
    assert.notEqual(plan.trials[i].arm, plan.trials[i + 1].arm);
    firstArms.push(plan.trials[i].arm);
    const first = trialInput(plan.trials[i]), second = trialInput(plan.trials[i + 1]);
    assert.deepEqual(first.events.map(row => row.at_ms), times);
    for (const text of ['expected', 'PRIVATE_CASE', 'PRIVATE_TRACE', 'UNFORWARDED_REFERENCE']) assert.equal(JSON.stringify(first).includes(text), false);
    assert.ok(first.events.every(row => row.request.context.includes('PRIVATE_CONTEXT')));
    assert.deepEqual(first.events.map(row => ({ ...row.request, id: '' })), second.events.map(row => ({ ...row.request, id: '' })));
  }
  assert.equal(firstArms.filter(arm => arm === 'cold').length, 2);
  for (const change of [v => v.cases.pop(), v => v.cases[1].context += ' changed', v => v.cases[1].at_ms = 101,
    v => v.cases[1].prefix += 'two', v => v.cases[0].trace_id = null]) {
    const changed = suite(); change(changed); assert.throws(() => planPacedTrials(changed));
  }
});

test('strict reports preserve structural key-order independence and reject stale, late or unbound outputs', () => {
  const input = trialInput(planPacedTrials(suite(1)).trials[0]);
  const valid = reportFor(input);
  assert.equal(validatePacedReport(valid, input), valid);
  for (const change of [v => v.extra = true, v => v.events.pop(), v => v.events[3].request_id = 'wrong',
    v => v.events[3].deadline_ms++, v => v.events[3].delivered_at_ms = 1051,
    v => v.events[0] = { ...v.events[3], request_id: input.events[0].request.id, at_ms: 0, deadline_ms: 550,
      observed_at_ms: 0, started_at_ms: 0, finished_at_ms: 100, delivered_at_ms: 100 },
    v => v.events[3].result.config.seed++, v => v.cleanup.runtime_identity_sha256 = 'd'.repeat(64),
    v => v.cleanup.reaped = false, v => v.events[0].delivered_at_ms = 1,
    v => v.prelude.elapsed_ms = 0, v => v.events[3].result.terminal_received = false,
    v => v.events[3].result.shape_valid = false, v => delete v.events[3].result.shape_valid,
    v => v.events[3].queue_ms = 0]) {
    const changed = structuredClone(valid); change(changed); assert.throws(() => validatePacedReport(changed, input));
  }
});

test('transient Persian joiner stays exact and unavailable while superseding old work and retaining all opportunities', async t => {
  const fragments = ['م', 'می', 'می\u200c', 'می\u200cر'];
  const continuation = 'می\u200cروم کنار خانه';
  const document = suite(1);
  document.cases = document.cases.map((row, i) => ({ ...row, language: 'fa',
    prefix: 'من برای سفر ' + fragments[i], expected: [continuation.slice(fragments[i].length)] }));
  const plan = planPacedTrials(document);
  assert.equal(plan.trials.length, 2);
  const input = trialInput(plan.trials[0]);
  assert.deepEqual(input.events.map(row => row.request.before), document.cases.map(row => row.prefix));
  assert.equal(input.events[2].request.before.at(-1), '\u200c');
  assert.equal(JSON.stringify(input).includes('کنار خانه'), false);
  const transientReport = wire => {
    const value = reportFor(wire);
    value.events[3].result.text = 'وم کنار خانه';
    Object.assign(value.events[0], { disposition: 'superseded', reason: 'newer_observed_snapshot',
      started_at_ms: 0, finished_at_ms: 300, queue_ms: 0,
      result: { ...value.events[3].result, id: wire.events[0].request.id, text: continuation.slice(1), latency_ms: 300 } });
    Object.assign(value.events[2], { disposition: 'unavailable', reason: 'transient_joiner_no_request' });
    return value;
  };
  const valid = transientReport(input);
  assert.equal(validatePacedReport(valid, input), valid);
  for (const change of [v => v.events[2].reason = 'model_abstention',
    v => v.events[2].disposition = 'coalesced',
    v => Object.assign(v.events[2], { observed_at_ms: null, scheduling_lateness_ms: null }),
    v => Object.assign(v.events[2], { started_at_ms: 250, finished_at_ms: 260, queue_ms: 0 }),
    v => v.events[2].result = { ...v.events[3].result, id: input.events[2].request.id },
    v => v.events[2].delivered_at_ms = 260,
    v => Object.assign(v.events[2], { disposition: 'eligible', reason: 'on_time',
      started_at_ms: 300, finished_at_ms: 350, queue_ms: 50, delivered_at_ms: 350,
      result: { ...v.events[3].result, id: input.events[2].request.id, latency_ms: 50 } }),
    v => Object.assign(v.events[0], { disposition: 'eligible', reason: 'on_time', delivered_at_ms: 300 }),
    v => Object.assign(v.events[1], { disposition: 'unavailable', reason: 'transient_joiner_no_request' })]) {
    const changed = structuredClone(valid); change(changed);
    assert.throws(() => validatePacedReport(changed, input));
  }
  // A runtime stopped before the transient's scheduled time never observes it.
  const stopped = transientReport(input); stopped.error = 'active_deadline';
  stopped.events = stopped.events.map((row, i) => ({ ...row,
    disposition: i === 0 ? 'deadline' : 'unavailable', reason: i === 0 ? 'active_deadline' : 'prior_deadline',
    observed_at_ms: i === 0 ? 0 : null, scheduling_lateness_ms: i === 0 ? 0 : null,
    started_at_ms: i === 0 ? 0 : null, finished_at_ms: i === 0 ? 80 : null, queue_ms: i === 0 ? 0 : null,
    delivered_at_ms: null, result: null }));
  assert.doesNotThrow(() => validatePacedReport(stopped, input));
  const f = await fixture(t, document); let calls = 0;
  const result = await runPacedSuite(f.options, { ...f.dependencies, runTrial: async wire => {
    calls++; return { ...childFor(wire), report: transientReport(wire) };
  } });
  assert.equal(calls, 2); assert.equal(result.report.complete, true);
  for (const arm of ['cold', 'primed']) {
    const summary = result.report.measurements[arm];
    assert.equal(summary.scheduled_events, 4); assert.equal(summary.scheduled_traces, 1);
    assert.equal(summary.dispositions.unavailable, 1); assert.equal(summary.dispositions.superseded, 1);
    assert.equal(summary.dispositions.coalesced, 1); assert.equal(summary.dispositions.eligible, 1);
    assert.equal(summary.eligible_final_traces, 1);
    assert.equal(summary.multiword_reference_agreement[2].scheduled_events, 4);
    assert.equal(summary.multiword_reference_agreement[2].scorable_events, 4);
  }
  for (const trial of result.report.trials) {
    assert.equal(trial.records[2].reason, 'transient_joiner_no_request');
    assert.equal(trial.records[2].score, null);
    assert.equal(trial.records[2].word_scores[2].match, false);
  }
  assert.equal(await readFile(join(f.options.outputPath, 'suite-input.json'), 'utf8'), f.bytes);
});

test('prelude and single-active-job claims require arm-specific evidence and nonoverlapping dispatch', () => {
  const base = trialInput(planPacedTrials(suite(1)).trials[0]);
  for (const arm of ['cold', 'primed']) {
    const input = { ...base, arm };
    const value = reportFor(input);
    assert.doesNotThrow(() => validatePacedReport(value, input));
    const missing = structuredClone(value);
    if (arm === 'cold') missing.prelude.priming_ms = 1;
    else missing.prelude.metrics = null;
    assert.throws(() => validatePacedReport(missing, input), /priming/iu);
  }
  const overlap = reportFor(base);
  Object.assign(overlap.events[0], { disposition: 'superseded', reason: 'superseded', started_at_ms: 0, finished_at_ms: 540, queue_ms: 0 });
  assert.throws(() => validatePacedReport(overlap, base), /overlap/iu);
  const coalesced = reportFor(base);
  Object.assign(coalesced.events[0], { started_at_ms: 0, finished_at_ms: 20, queue_ms: 0 });
  assert.throws(() => validatePacedReport(coalesced, base), /coalesced/iu);
});

test('prelude and active timeouts cannot be followed by eligible runtime reuse', () => {
  const input = trialInput(planPacedTrials(suite(1)).trials[0]);
  const prelude = reportFor(input); prelude.error = 'prelude_deadline';
  assert.throws(() => validatePacedReport(prelude, input), /failed prelude/iu);
  const active = reportFor(input); active.error = 'active_deadline';
  Object.assign(active.events[0], { disposition: 'deadline', reason: 'active_deadline', started_at_ms: 0, finished_at_ms: 510, queue_ms: 0 });
  assert.throws(() => validatePacedReport(active, input), /hard deadline/iu);
  prelude.events = prelude.events.map(row => ({ ...row, disposition: 'unavailable', reason: 'prelude_deadline',
    observed_at_ms: null, started_at_ms: null, finished_at_ms: null, queue_ms: null, scheduling_lateness_ms: null,
    result: null, delivered_at_ms: null }));
  assert.doesNotThrow(() => validatePacedReport(prelude, input));
});

test('suite runner serializes fresh trial calls, saves private artifacts and includes coalesced denominators', async t => {
  const f = await fixture(t); const seen = []; let active = 0; const progress = [];
  const result = await runPacedSuite(f.options, { ...f.dependencies, onProgress: value => progress.push(value), runTrial: async input => {
    assert.equal(active++, 0); seen.push(input); await new Promise(resolve => setTimeout(resolve, 2)); active--; return childFor(input);
  } });
  assert.equal(result.report.complete, true);
  assert.equal(result.report.provenance.valid_executed_identity, false);
  assert.equal(result.report.provenance.execution_kind, 'injected_fixture');
  assert.equal(seen.length, 4);
  for (const arm of ['cold', 'primed']) {
    const summary = result.report.measurements[arm];
    assert.equal(summary.scheduled_events, 8); assert.equal(summary.scheduled_traces, 2);
    assert.equal(summary.dispositions.coalesced, 6); assert.equal(summary.eligible_final_traces, 2);
    assert.deepEqual(summary.eligible_event_latencies_ms, [200, 200]);
    assert.equal(summary.multiword_reference_agreement[4].matches, 2);
    assert.equal(summary.multiword_reference_agreement[4].scorable_events, 8);
    assert.equal(summary.reviewed_useful_final_traces, null);
  }
  assert.equal(JSON.stringify(progress).includes('PRIVATE_CONTEXT'), false);
  assert.equal((await stat(f.options.outputPath)).mode & 0o777, 0o700);
  for (const name of ['suite-input.json', 'decision-input.json', 'run.json', 'report.json']) {
    assert.equal((await stat(join(f.options.outputPath, name))).mode & 0o777, 0o600);
  }
  assert.equal(await readFile(join(f.options.outputPath, 'suite-input.json'), 'utf8'), f.bytes);
  await assert.rejects(runPacedSuite(f.options, { ...f.dependencies, runTrial: async () => { throw Error('must not run'); } }), /EEXIST/u);
});

test('measured hard deadlines retain four opportunities and continue after confirmed cleanup', async t => {
  const f = await fixture(t); let calls = 0;
  const result = await runPacedSuite(f.options, { ...f.dependencies, runTrial: async input => {
    const child = childFor(input);
    if (calls++ === 0) {
      child.report.error = 'active_deadline';
      child.report.events = child.report.events.map((row, i) => ({ ...row, disposition: i === 0 ? 'deadline' : 'unavailable',
        observed_at_ms: i === 0 ? 0 : null, scheduling_lateness_ms: i === 0 ? 0 : null,
        started_at_ms: i === 0 ? 0 : null, finished_at_ms: i === 0 ? 550 : null, queue_ms: i === 0 ? 0 : null,
        delivered_at_ms: null, result: null, reason: i === 0 ? 'active_deadline' : 'prior_deadline' }));
    }
    return child;
  } });
  assert.equal(calls, 4);
  assert.equal(result.report.complete, true);
  assert.equal(result.report.trials[0].records.length, 4);
  assert.equal(result.report.trials[1].records[3].disposition, 'eligible');
  assert.equal(Object.values(result.report.measurements).reduce((sum, value) => sum + value.scheduled_events, 0), 16);
});

test('paced artifact runs preserve private descriptor provenance and reject mismatched model identities', async t => {
  const f = await fixture(t, suite(1)), artifact = await artifactFixture(f.directory); let calls = 0;
  const result = await runPacedSuite({ ...f.options, modelArtifactPath: artifact.path }, { ...f.dependencies,
    runTrial: async (input, options) => {
      calls++; assert.equal(options.artifact.path, artifact.path);
      assert.equal(options.artifact.sha256, artifact.sha256);
      assert.equal(JSON.stringify(input).includes('weights_path'), false);
      return { ...childFor(input), report: reportFor(input, artifactIdentity(artifact)) };
    } });
  assert.equal(calls, 2); assert.equal(result.report.complete, true);
  assert.equal(result.report.provenance.valid_executed_identity, false);
  const before = result.report.provenance.before.model_artifact;
  assert.equal(before.sha256, artifact.sha256);
  assert.deepEqual(result.report.provenance.after.model_artifact, before);
  const saved = join(result.output, 'model-artifact-input.json');
  assert.equal(await readFile(saved, 'utf8'), artifact.source.toString());
  assert.equal((await stat(saved)).mode & 0o777, 0o600);
  const input = trialInput(planPacedTrials(suite(1)).trials[0]);
  for (const change of [v => v.model_size++, v => v.model_alias = 'wrong',
    v => v.model_sha256 = 'a'.repeat(64), v => delete v.model_origin]) {
    const changed = artifactIdentity(artifact); change(changed);
    assert.throws(() => validatePacedReport(reportFor(input, changed), input, null, artifact), /model identity/u);
  }
  for (const mode of ['mismatch', 'changed']) {
    const next = await fixture(t, suite(1)), expected = await artifactFixture(next.directory); let attempts = 0;
    const rejected = await runPacedSuite({ ...next.options, modelArtifactPath: expected.path }, { ...next.dependencies,
      runTrial: async wire => {
        attempts++;
        if (mode === 'changed') await writeFile(expected.path, expected.source.toString() + ' ');
        const model = artifactIdentity(expected);
        if (mode === 'mismatch') model.model_alias = 'wrong';
        return { ...childFor(wire), report: reportFor(wire, model) };
      } });
    assert.equal(attempts, 1); assert.equal(rejected.report.complete, false);
    assert.equal(rejected.report.measurements.cold.scheduled_events + rejected.report.measurements.primed.scheduled_events, 8);
    assert.ok(rejected.report.trials[1].records.every(row => row.disposition === 'unavailable' && row.score === null));
    if (mode === 'mismatch') assert.ok(rejected.report.trials[0].records.every(row => row.score === null));
    else assert.equal(rejected.report.provenance.stable, false);
  }
});

test('unverified cleanup halts inference and leaves all remaining opportunities unavailable', async t => {
  const f = await fixture(t); let calls = 0;
  const result = await runPacedSuite(f.options, { ...f.dependencies, runTrial: async input => {
    calls++; return { ...childFor(input), cleanup_confirmed: false };
  } });
  assert.equal(calls, 1); assert.equal(result.report.complete, false);
  assert.equal(result.report.trials.flatMap(trial => trial.records).filter(row => row.disposition === 'unavailable').length, 16);
  assert.ok(result.report.trials.flatMap(trial => trial.records).every(row => row.score === null));
});

test('runtime failures halt after preserving a valid partial report and cancellation starts no later child', async t => {
  const f = await fixture(t); let calls = 0;
  const result = await runPacedSuite(f.options, { ...f.dependencies, runTrial: async input => {
    calls++; const child = childFor(input); child.report.complete = false; child.report.error = 'runtime_unavailable'; return child;
  } });
  assert.equal(calls, 1); assert.equal(result.report.complete, false);
  assert.equal(result.report.trials[0].child.report.error, 'runtime_unavailable');
  assert.ok(result.report.trials.slice(1).flatMap(trial => trial.records).every(row => row.disposition === 'unavailable'));
  const aborted = new AbortController(); aborted.abort();
  await assert.rejects(runPacedSuite(f.options, { ...f.dependencies, signal: aborted.signal,
    runTrial: () => { throw Error('never start'); } }), /abort/iu);
});

test('frozen suite mismatch and changed provenance cannot produce successful evidence', async t => {
  const f = await fixture(t); let calls = 0;
  await writeFile(f.options.suitePath, f.bytes + ' ');
  await assert.rejects(runPacedSuite(f.options, { ...f.dependencies, runTrial: async () => calls++ }), /frozen decision/u);
  assert.equal(calls, 0);
  await writeFile(f.options.suitePath, f.bytes);
  let captures = 0;
  const result = await runPacedSuite(f.options, { ...f.dependencies,
    captureSources: async () => ({ source: `${captures++}` }), runTrial: async input => childFor(input) });
  assert.equal(result.report.complete, false); assert.equal(result.report.provenance.stable, false);
});

const processFixture = `
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { readFile } from 'node:fs/promises';
const [runtimePath, digest, mode, reportText, artifactFlag, artifactPath] = process.argv.slice(1);
if(mode.startsWith('artifact-') && (artifactFlag!=='--model-artifact' || !artifactPath)) throw Error('missing artifact fixture argument');
const descriptor = artifactPath ? JSON.parse(await readFile(artifactPath,'utf8')) : null;
let input = ''; for await (const part of process.stdin) input += part;
const wire = JSON.parse(input);
const runtime = spawn(runtimePath, ['60'], {detached:true,stdio:'ignore'});
const exited = once(runtime, 'exit'); await once(runtime, 'spawn');
let closing = false;
async function stop() { if(closing)return; closing=true; runtime.kill('SIGTERM'); await exited; process.exit(0); }
process.on('SIGTERM',stop);
if(mode==='no-ready') { await new Promise(resolve=>setTimeout(resolve,100)); await stop(); }
const identity = {launch_contract_id:'fixture-no-model',binary_sha256:digest,model_sha256:'c'.repeat(64),context_size:2048,
  ...(descriptor?{model_origin:'explicit_lab_artifact',model_sha256:descriptor.sha256,model_size:descriptor.bytes,
    model_alias:mode==='artifact-mismatch'?'wrong':descriptor.alias}:{})};
const emit = value => process.stdout.write(JSON.stringify(value)+'\\n');
emit({type:'ready',schema:'badi.prediction-lab.worker.v1',runtime_pid:runtime.pid,identity,runtime_identity_sha256:'a'.repeat(64)});
await new Promise(resolve=>setTimeout(resolve,100));
if(mode==='hang') { await new Promise(()=>{}); }
const report = JSON.parse(reportText);
report.identity=identity; report.cleanup.process_id=runtime.pid;
for(const row of report.events) if(row.result) row.result.identity=identity;
runtime.kill('SIGTERM'); await exited;
if(mode==='malformed') process.stdout.write('PRIVATE_INVALID_JSON\\n');
else if(mode==='oversized') process.stdout.write('x'.repeat(3*1024*1024));
else { emit({type:'paced_report',report}); if(mode==='extra') emit({type:'extra'}); }
`;

test('disposable subprocesses verify actual executable ownership, reject malformed streams and reap runtime children', async t => {
  const f = await fixture(t, suite(1)); const runtimePath = join(f.directory, 'llama-server');
  await copyFile('/usr/bin/sleep', runtimePath); await chmod(runtimePath, 0o700);
  const digest = hash(await readFile(runtimePath));
  const artifact = await artifactFixture(f.directory);
  for (const mode of ['normal', 'malformed', 'extra', 'hang', 'no-ready', 'oversized', 'artifact-normal', 'artifact-mismatch']) {
    const input = trialInput(planPacedTrials(suite(1)).trials[0]);
    const child = await runPacedChild(input, { executable: process.execPath,
      args: ['--input-type=module', '-e', processFixture, runtimePath, digest, mode, JSON.stringify(reportFor(input))],
      timeoutMs: mode === 'hang' ? 200 : 2000, terminateMs: 300, artifact: mode.startsWith('artifact-') ? artifact : null });
    assert.equal(child.cleanup_confirmed, mode !== 'no-ready');
    assert.equal(child.execution.executable.path, await import('node:fs/promises').then(fs => fs.realpath(process.execPath)));
    if (mode !== 'no-ready') assert.equal(child.runtime.verified_executable.sha256, digest);
    assert.throws(() => process.kill(child.execution.pid, 0), error => error.code === 'ESRCH');
    const success = ['normal', 'artifact-normal'].includes(mode);
    assert.equal(success ? child.error : Boolean(child.error), success ? null : true);
    if (mode === 'artifact-mismatch') assert.match(child.error, /model identity/u);
  }
});
