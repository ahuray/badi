import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { test } from 'node:test';
import { runDiagnostics } from './qualification-diagnostics.mjs';

class Clock {
  time = 0;
  serial = 0;
  timers = new Map();
  now = () => this.time;
  unix = () => 1800000000 + Math.floor(this.time / 1000);
  schedule = (callback, ms) => {
    const id = ++this.serial; this.timers.set(id, { at: this.time + Math.max(0, ms), callback }); return id;
  };
  clear = id => this.timers.delete(id);
  sleep = (ms, signal) => new Promise((resolve, reject) => {
    if (signal?.aborted) { reject(new Error('fixture abort')); return; }
    const cancel = () => { this.clear(timer); reject(new Error('fixture abort')); };
    const timer = this.schedule(() => { signal?.removeEventListener('abort', cancel); resolve(); }, ms);
    signal?.addEventListener('abort', cancel, { once: true });
  });
  async finish(promise) {
    let done = false, result, error;
    promise.then(value => { result = value; done = true; }, value => { error = value; done = true; });
    for (let ticks = 0; !done && ticks < 100000; ticks++) {
      for (let i = 0; i < 50; i++) await Promise.resolve();
      if (done) break;
      const next = [...this.timers].sort((a, b) => a[1].at - b[1].at || a[0] - b[0])[0];
      assert.ok(next, 'Diagnostic left no scheduled work but did not settle.');
      this.time = next[1].at; this.timers.delete(next[0]); next[1].callback();
    }
    assert.ok(done, 'Diagnostic exceeded the bounded scheduler iterations.');
    if (error) throw error;
    return result;
  }
}

const identity = { launch_contract_id: 'fixture-only', binary_sha256: '1'.repeat(64),
  model_sha256: '2'.repeat(64), model_size: 1000, gpu_layers: 0 };
const wireIdentity = () => Object.fromEntries(Object.entries(identity).sort(([a], [b]) => a.localeCompare(b)));
const identitySha = createHash('sha256').update(JSON.stringify(identity)).digest('hex');
class Worker {
  session = null;
  startupMs = 30000;
  starts = 0;
  checks = 0;
  active = 0;
  maximumActive = 0;
  calls = [];
  constructor(clock, options = {}) { this.clock = clock; this.options = options; }
  beforeStart = async signal => { this.checks++; await this.clock.sleep(7, signal); };
  async predict(request, signal) {
    if (signal?.aborted) throw new Error('fixture abort');
    this.active++; this.maximumActive = Math.max(this.maximumActive, this.active);
    this.calls.push(structuredClone(request));
    try {
      if (!this.session) {
        await this.beforeStart(signal); this.starts++;
        this.session = { identity: wireIdentity(), runtime: { pid: 100 + this.starts }, closed: false,
          exit: Promise.resolve(), pending: null };
        await this.clock.sleep(20, signal);
      }
      const session = this.session;
      session.pending = { id: request.id };
      const abort = () => { void this.stop(); };
      signal.addEventListener('abort', abort, { once: true });
      try { await this.clock.sleep(this.options.requestMs ?? 60, signal); }
      finally { signal.removeEventListener('abort', abort); session.pending = null; }
      return { id: request.id, outcome: this.options.outcome ?? 'suggestion', text: 'PRIVATE GENERATED ADDITION',
        word_complete: true, shape_valid: true, terminal_received: this.options.terminal !== false,
        latency_ms: this.options.requestMs ?? 60, replace_before: null, preflight_ms: 3,
        reused_prompt_tokens: this.options.noReuse ? null : 23, newly_evaluated_prompt_tokens: 1,
        identity: wireIdentity(), local_measurement: { worker_startup_ms: 20,
          request_roundtrip_ms: this.options.requestMs ?? 60,
          peak_runtime_rss_bytes: this.options.noRss ? null : 1000000, gpu_memory_bytes: 0 } };
    } finally { this.active--; }
  }
  async stop() {
    const session = this.session;
    if (!session) return;
    if (session.stopping) return session.stopping;
    session.closed = true; this.session = null;
    const receipt = { process_id: session.runtime.pid, runtime_identity_sha256: this.options.badReceipt ? 'f'.repeat(64) : identitySha,
      challenge_completed: true, reaped: true, exit_code: 0 };
    session.cleanup = receipt; session.stopping = Promise.resolve(receipt);
    return receipt;
  }
}

const cases = [
  { id: 'en', language: 'en', prefix: 'PRIVATE ENGLISH DRAFT', context: '', style: '', expected: ['SECRET EXPECTED ANSWER'] },
  { id: 'de', language: 'de', prefix: 'PRIVATE GERMAN DRAFT', context: '', style: '', expected: [] },
  { id: 'fa', language: 'fa', prefix: 'من دارم می‌نویسم', context: '', style: '', expected: [] },
];
const config = { id: 'fixture', mode: 'healed', budget_ms: 550, cache_prompt: true, max_tokens: 8, temperature: 0, seed: 42 };
const makeRequest = (row, config) => {
  const settings = { ...config }; delete settings.id;
  return { schema: 'badi.prediction-lab.request.v1', id: crypto.randomUUID(), before: row.prefix,
    context: row.context, language: row.language, style_examples: [], config: settings };
};
const pressure = async () => ({ some_avg10: 0, full_avg10: 0, some_total_usec: 5, full_total_usec: 0 });
function setup(options = {}) {
  const clock = new Clock(), worker = new Worker(clock, options);
  return { clock, worker, cases, config, makeRequest, readPressure: pressure };
}

test('diagnostics measure actual phases, reuse, scheduled typing, cancellation and fresh recovery', async () => {
  const input = setup(); const events = [];
  const original = input.worker.beforeStart;
  const report = await input.clock.finish(runDiagnostics({ ...input, onEvent: value => events.push(value) }));
  assert.equal(report.complete, true, JSON.stringify(report.measurements.cancellation));
  assert.equal(report.evidenceMetrics.cold_start_ms, 20);
  assert.equal(report.evidenceMetrics.preparation_ms, 7);
  assert.equal(report.evidenceMetrics.prompt_reuse_measured, true);
  assert.equal(report.evidenceMetrics.cancellation_recovery_passed, true);
  assert.equal(report.evidenceMetrics.paced_typing_measured, true);
  assert.equal(report.evidenceMetrics.sustained_seconds, 30);
  assert.equal(report.evidenceMetrics.peak_vram_bytes, 0);
  assert.equal(report.measurements.paced.observed_events, 9);
  assert.equal(input.worker.maximumActive, 1);
  assert.equal(input.worker.checks, input.worker.starts);
  assert.equal(input.worker.starts, 2);
  assert.equal(input.worker.beforeStart, original);
  assert.equal(input.worker.session, null);
  assert.ok(report.measurements.cleanup.every(value => value.verified));
  for (const forbidden of ['PRIVATE ENGLISH DRAFT', 'PRIVATE GENERATED ADDITION', 'SECRET EXPECTED ANSWER']) {
    assert.equal(JSON.stringify(report).includes(forbidden), false);
    assert.equal(JSON.stringify(events).includes(forbidden), false);
  }
  assert.equal(JSON.stringify(input.worker.calls).includes('SECRET EXPECTED ANSWER'), false);
  assert.ok(events.some(event => event.phase === 'sustained'));
  assert.equal(report.measurements.languages.reduce((sum, row) => sum + row.total, 0), report.records.length);
});

test('missing cache counters, RSS or terminal confirmation remain unmeasured', async () => {
  const input = setup({ noReuse: true, noRss: true, terminal: false });
  const report = await input.clock.finish(runDiagnostics(input));
  assert.equal(report.evidenceMetrics.prompt_reuse_measured, false);
  assert.equal(report.evidenceMetrics.loaded_and_exercised, false);
  assert.equal(report.evidenceMetrics.paced_typing_measured, false);
  assert.equal(report.evidenceMetrics.cancellation_recovery_passed, false);
  assert.equal(report.measurements.peak_rss_bytes, null);
  assert.equal(report.evidenceMetrics.peak_rss_bytes, 0);
  assert.ok(report.measurements.languages.every(row => row.complete_on_time === 0 && row.p95_complete_word_ms_lower_bound >= 551));
});

test('timer-driven traces retain coalesced and superseded events rather than serially claiming typing', async () => {
  const input = setup({ requestMs: 500 });
  const report = await input.clock.finish(runDiagnostics(input));
  const paced = report.records.filter(row => row.phase === 'paced');
  assert.equal(paced.length, 9);
  assert.equal(paced.filter(row => row.outcome === 'coalesced').length, 3);
  assert.equal(paced.filter(row => row.reason === 'superseded').length, 3);
  assert.ok(paced.every(row => row.delivered_at_ms === null));
  for (const language of ['en', 'de', 'fa']) {
    const rows = paced.filter(row => row.language === language);
    assert.deepEqual(rows.map(row => row.observed_at_ms), [0, 100, 250]);
    assert.deepEqual(rows.map(row => row.scheduled_at_ms), [0, 100, 250]);
  }
  assert.equal(input.worker.maximumActive, 1);
});

test('already-cancelled runs keep unexecuted opportunities and perform no model loads', async () => {
  const input = setup(); const controller = new AbortController(); controller.abort();
  const report = await input.clock.finish(runDiagnostics({ ...input, signal: controller.signal }));
  assert.equal(report.cancelled, true);
  assert.equal(report.complete, false);
  assert.equal(input.worker.starts, 0);
  assert.ok(report.records.length > 0 && report.records.every(row => row.outcome === 'not_run'));
  assert.equal(report.evidenceMetrics.cleanup_verified, false);
});

test('external cancellation captures the owned session receipt even when abort clears worker.session', async () => {
  const input = setup(); const controller = new AbortController();
  input.clock.schedule(() => controller.abort(), 2000);
  const report = await input.clock.finish(runDiagnostics({ ...input, signal: controller.signal }));
  assert.equal(report.cancelled, true);
  assert.equal(report.complete, false);
  assert.equal(input.worker.session, null);
  assert.ok(report.measurements.cleanup.length && report.measurements.cleanup.every(row => row.verified));
  assert.ok(report.records.some(row => row.outcome === 'cancelled'));
});

test('wrong cleanup identity blocks recovery and cannot confer a passing cancellation result', async () => {
  const input = setup({ badReceipt: true });
  const report = await input.clock.finish(runDiagnostics(input));
  assert.equal(report.complete, false);
  assert.equal(report.error, 'cleanup_unverified');
  assert.equal(report.evidenceMetrics.cleanup_verified, false);
  assert.equal(report.evidenceMetrics.cancellation_recovery_passed, false);
  assert.equal(input.worker.starts, 1);
  assert.equal(report.records.find(row => row.phase === 'recovery').outcome, 'not_run');
});

test('duration, resource hook and worker exclusivity are checked before work', async () => {
  for (const durationSeconds of [0, 1, 60, 1801, '30']) {
    const input = setup(); await assert.rejects(runDiagnostics({ ...input, durationSeconds }), /duration/u);
    assert.equal(input.worker.starts, 0);
  }
  const noHook = setup(); noHook.worker.beforeStart = null;
  await assert.rejects(runDiagnostics(noHook), /resource recheck/u);
  const busy = setup(); busy.worker.session = { pending: {} };
  await assert.rejects(runDiagnostics(busy), /busy/u);
  const concurrent = setup();
  const first = runDiagnostics(concurrent);
  await assert.rejects(runDiagnostics(concurrent), /already running/u);
  await concurrent.clock.finish(first);
});

test('30-minute mode is genuinely timed, serial, resource checked, and request bounded', async () => {
  const input = setup();
  const report = await input.clock.finish(runDiagnostics({ ...input, durationSeconds: 1800 }));
  assert.equal(report.complete, true);
  assert.equal(report.evidenceMetrics.sustained_seconds, 1800);
  assert.ok(input.worker.calls.length <= 20000);
  assert.equal(input.worker.maximumActive, 1);
  assert.ok(report.measurements.memory_pressure.length <= 1801);
});

test('a request completed before cancellation cannot earn cancellation or recovery credit', async () => {
  const input = setup({ requestMs: 10 });
  const report = await input.clock.finish(runDiagnostics(input));
  assert.equal(report.complete, true);
  assert.equal(report.measurements.cancellation.reason, 'request_finished_before_cancellation');
  assert.equal(report.evidenceMetrics.cancellation_recovery_passed, false);
  assert.equal(report.records.find(row => row.phase === 'recovery').outcome, 'not_run');
  assert.equal(input.worker.starts, 1);
});

test('malformed request factories fail without unhandled paced work or leaking errors', async () => {
  const input = setup();
  const report = await input.clock.finish(runDiagnostics({ ...input, makeRequest: () => { throw new Error('PRIVATE ERROR TEXT'); } }));
  assert.equal(report.complete, false);
  assert.equal(report.error, 'cold_runtime_unavailable');
  assert.equal(report.records[0].reason, 'invalid_request_boundary');
  assert.equal(JSON.stringify(report).includes('PRIVATE ERROR TEXT'), false);
  assert.equal(input.worker.starts, 0);
});
