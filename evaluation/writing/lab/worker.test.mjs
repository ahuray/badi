import assert from 'node:assert/strict';
import { test } from 'node:test';
import { LabWorker, parseProcessStat, processIdentity } from './worker.mjs';
import { copyFile, mkdtemp, chmod, readFile, rm } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

// Each worker owns a real detached runtime process. A disposable copy of the
// system sleep executable keeps fixtures small while exercising the production
// PID, parent, process-group, executable-path and executable-digest checks.
const fixture = `
import { createInterface } from 'node:readline';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
const [mode, runtimePath, runtimeDigest] = process.argv.slice(1);
const batchFlag = process.argv.indexOf('--prefill-batch');
const batch = batchFlag < 0 ? 16 : Number(process.argv[batchFlag + 1]);
const emit = value => process.stdout.write(JSON.stringify(value) + '\\n');
let runtime = null;
let runtimeExit = null;
let quitting = false;
process.stdout.on('error', () => {});
const shutdown = async () => {
  if (quitting) return;
  quitting = true;
  if (runtime) {
    runtime.kill('SIGTERM');
    await runtimeExit;
    const cleanup = {process_id:runtime.pid, runtime_identity_sha256:runtimeDigest,
      challenge_completed:true, reaped:true, exit_code:0};
    if (!process.stdout.destroyed && !process.stdout.writableEnded) {
      await new Promise(resolve => process.stdout.write(JSON.stringify({type:'stopped', cleanup}) + '\\n', resolve));
    }
  }
  process.exit(0);
};
process.on('SIGTERM', shutdown);
process.on('SIGINT', shutdown);
if (mode === 'startup-error') emit({type:'error', error:'fixture unavailable'});
else if (mode === 'startup-malformed') process.stdout.write('not json\\n');
else if (mode === 'startup-null') emit(null);
else if (mode === 'startup-array') emit([]);
else if (mode === 'startup-eof') { process.stdout.end(); setInterval(() => {}, 1000); }
else if (mode === 'startup-wait') setInterval(() => {}, 1000);
else {
  runtime = spawn(mode === 'runtime-wrong-executable' ? '/usr/bin/sleep' : runtimePath, ['60'],
    {detached:mode !== 'runtime-wrong-group', stdio:'ignore'});
  runtimeExit = once(runtime, 'exit');
  await once(runtime, 'spawn');
  if (mode === 'runtime-dead') { runtime.kill('SIGTERM'); await runtimeExit; }
  const identity = {fixture:true, pid:process.pid, runtime_pid:runtime.pid,
    batch_size:mode === 'runtime-wrong-batch' ? 64 : batch, ubatch_size:batch,
    binary_sha256:mode === 'runtime-wrong-digest' ? '0'.repeat(64) : runtimeDigest};
  emit({type:'ready', schema:'badi.prediction-lab.worker.v1',
    runtime_pid:mode === 'runtime-missing' ? undefined : mode === 'runtime-wrong-parent' ? process.pid : runtime.pid,
    identity:mode === 'startup-bad-ready' ? null : identity});
}
createInterface({input:process.stdin}).on('line', line => {
  const request = JSON.parse(line);
  if (mode === 'exit') process.exit(0);
  else if (mode === 'eof') { process.stdout.end(); setInterval(() => {}, 1000); }
  else if (mode === 'hang') return;
  else if (mode === 'wrong-id') emit({type:'result', id:'different-request', outcome:'suggestion', text:' word'});
  else if (mode === 'malformed') process.stdout.write('{invalid\\n');
  else if (mode === 'null') emit(null);
  else if (mode === 'array') emit([]);
  else if (mode === 'oversized') process.stdout.write('x'.repeat(1024 * 1024 + 65536));
  else if (mode === 'error') emit({type:'error', id:request.id, error:'fixture rejected request'});
  else emit({type:'result', id:request.id, outcome:'suggestion', text:' word', latency_ms:1, word_complete:true});
});
`;

async function makeWorker(t, mode = 'normal', options = {}) {
  const directory = await mkdtemp(join(tmpdir(), 'badi-lab-worker-'));
  const runtimePath = join(directory, 'llama-server');
  await copyFile('/usr/bin/sleep', runtimePath);
  await chmod(runtimePath, 0o700);
  const runtimeDigest = createHash('sha256').update(await readFile(runtimePath)).digest('hex');
  const worker = new LabWorker({ executable: process.execPath,
    args: ['--input-type=module', '-e', fixture, mode, runtimePath, runtimeDigest], startupMs: 1000, ...options });
  worker.fixtureArgs = kind => ['--input-type=module', '-e', fixture, kind, runtimePath, runtimeDigest];
  const uncertain = mode === 'startup-bad-ready' || mode.startsWith('runtime-');
  t.after(async () => {
    try {
      if (uncertain) await assert.rejects(worker.stop(), /ownership and cleanup/iu);
      else await worker.stop();
    } finally { await rm(directory, {recursive:true, force:true}); }
  });
  return worker;
}

const request = (id = 'fixture-request') => ({ schema: 'badi.prediction-lab.request.v1', id,
  before: 'Synthetic text', language: 'en', context: '', style_examples: [], config: { mode: 'context', budget_ms: 550 } });
function reaped(pid) { assert.throws(() => process.kill(pid, 0), error => error.code === 'ESRCH'); }

test('worker negotiates readiness, matches response IDs and reaps its owned process on stop', async t => {
  const worker = await makeWorker(t);
  const identity = await worker.start();
  assert.equal(identity.fixture, true);
  assert.equal(identity.pid, worker.session.child.pid);
  assert.equal(worker.session.runtime.pid, identity.runtime_pid);
  assert.equal(worker.session.runtime.parent, identity.pid);
  assert.equal(worker.session.runtime.group, identity.runtime_pid);
  assert.equal(worker.session.runtime.executable_sha256, identity.binary_sha256);
  const response = await worker.predict(request());
  assert.equal(response.id, 'fixture-request');
  assert.equal(response.text, ' word');
  assert.equal(response.word_complete, true);
  await worker.stop();
  assert.equal(worker.session, null);
  reaped(identity.pid);
  assert.equal(await processIdentity(identity.runtime_pid), null);
  const newIdentity = await worker.start();
  assert.notEqual(newIdentity.pid, identity.pid);
});

test('concurrent startup callers share one process and one ready identity', async t => {
  const worker = await makeWorker(t);
  const [a, b] = await Promise.all([worker.start(), worker.start()]);
  assert.equal(a.pid, b.pid);
  await worker.stop();
  reaped(a.pid);
});

test('startup errors, malformed readiness and unavailable executable reject without leaving a process', async t => {
  for (const mode of ['startup-error', 'startup-malformed', 'startup-null', 'startup-array']) {
    const worker = await makeWorker(t, mode);
    await assert.rejects(worker.start(), /startup failed|Invalid model response/iu);
    const pid = worker.session?.child.pid;
    await worker.stop();
    if (pid) reaped(pid);
  }
  const missing = await makeWorker(t, 'normal', { executable: '/nonexistent/badi-test-worker' });
  await assert.rejects(missing.start(), /Cannot start/iu);
  await missing.stop();
  assert.equal(missing.session, null);
});

test('readiness without a model identity fails the worker protocol', async t => {
  const worker = await makeWorker(t, 'startup-bad-ready');
  await assert.rejects(worker.start(), /ownership and cleanup/iu);
  await assert.rejects(worker.stop(), /ownership and cleanup/iu);
  await assert.rejects(worker.start(), /ownership and cleanup/iu);
});

test('startup timeout terminates and reaps the unresponsive child', async t => {
  const worker = await makeWorker(t, 'startup-wait', { startupMs: 150 });
  const promise = worker.start();
  const pid = worker.session.child.pid;
  await assert.rejects(promise, /startup timed out/iu);
  await worker.stop();
  reaped(pid);
});

test('worker preserves explicit per-request errors as outcomes the server can classify', async t => {
  const worker = await makeWorker(t, 'error');
  const response = await worker.predict(request());
  assert.equal(response.type, 'error');
  assert.equal(response.error, 'fixture rejected request');
  assert.equal(response.id, 'fixture-request');
});

test('wrong response identity, invalid JSON and oversized unfinished lines retire their processes', async t => {
  for (const mode of ['wrong-id', 'malformed', 'null', 'array', 'oversized']) {
    const worker = await makeWorker(t, mode);
    const identity = await worker.start();
    await assert.rejects(worker.predict(request()), /identity|Invalid model response|exceeded/iu);
    await worker.stop();
    reaped(identity.pid);
  }
});

test('process exit rejects the active request and a later request starts a fresh process', async t => {
  const worker = await makeWorker(t, 'exit');
  const identity = await worker.start();
  await assert.rejects(worker.predict(request()), /stopped|closed|EOF/iu);
  await worker.stop();
  reaped(identity.pid);
  worker.args = worker.fixtureArgs('normal');
  const response = await worker.predict(request('recovery'));
  assert.equal(response.id, 'recovery');
  assert.notEqual(worker.session.child.pid, identity.pid);
});

test('stdout EOF during startup or inference rejects promptly even when the process remains alive', { timeout: 4000 }, async t => {
  const starting = await makeWorker(t, 'startup-eof');
  await assert.rejects(starting.start(), /EOF|closed|ended|stopped/iu);
  await starting.stop();
  const worker = await makeWorker(t, 'eof');
  const identity = await worker.start();
  await assert.rejects(worker.predict(request()), /EOF|closed|ended|stopped/iu);
  await worker.stop();
  reaped(identity.pid);
});

test('aborting an active request cancels and reaps its child; a second concurrent request is rejected', async t => {
  const worker = await makeWorker(t, 'hang');
  const identity = await worker.start();
  const controller = new AbortController();
  const pending = worker.predict(request(), controller.signal);
  // predict resumes after readiness in the next microtask, without a timer race.
  await Promise.resolve();
  await assert.rejects(worker.predict(request('concurrent')), /already running/iu);
  controller.abort();
  await assert.rejects(pending, /cancelled|stopped|abort/iu);
  await worker.stop();
  reaped(identity.pid);
});

test('already-aborted requests never start a process and cancellation during startup also cleans up', async t => {
  const cancelled = new AbortController(); cancelled.abort();
  const worker = await makeWorker(t);
  await assert.rejects(worker.predict(request(), cancelled.signal), /abort/iu);
  assert.equal(worker.session, null);
  const starting = await makeWorker(t, 'startup-wait');
  const controller = new AbortController();
  const pending = starting.predict(request(), controller.signal);
  const pid = starting.session.child.pid;
  controller.abort();
  await assert.rejects(pending, /cancelled|abort/iu);
  await starting.stop();
  reaped(pid);
});


test('malformed process metadata is an inspection error rather than evidence of disappearance', async () => {
  const actual = await readFile(`/proc/${process.pid}/stat`, 'utf8');
  const parsed = parseProcessStat(process.pid, actual);
  assert.equal(parsed.pid, process.pid);
  assert.equal(parsed.parent, process.ppid);
  assert.match(parsed.start, /^[1-9]\d*$/u);
  for (const broken of ['', 'malformed', `${process.pid} (name) S wrong fields`, actual.replace(/^\d+/, '1')]) {
    assert.throws(() => parseProcessStat(process.pid, broken), /Malformed/iu);
  }
  await assert.rejects(processIdentity(-1), /Invalid/iu);
  assert.equal(await processIdentity(0xffffffff), null);
});

test('missing, stale and mismatched ready ownership never resolves and permanently latches uncertainty', async t => {
  for (const mode of ['runtime-missing', 'runtime-dead', 'runtime-wrong-parent', 'runtime-wrong-group',
    'runtime-wrong-executable', 'runtime-wrong-digest', 'runtime-wrong-batch']) {
    const worker = await makeWorker(t, mode);
    await assert.rejects(worker.start(), /ownership and cleanup/iu);
    assert.equal(worker.session?.identity ?? null, null);
    await assert.rejects(worker.stop(), /ownership and cleanup/iu);
    await assert.rejects(worker.start(), /ownership and cleanup/iu);
    assert.equal(process.kill(process.pid, 0), true, 'an unrelated parent process is never signalled');
  }
});

test('prompt batch is a bounded launch choice and readiness must confirm that choice', async t => {
  for (const prefillBatch of [0, 32, '64', null]) assert.throws(() => new LabWorker({ prefillBatch }));
  assert.throws(() => new LabWorker({ args: ['--prefill-batch', '64'] }));
  const standard = new LabWorker(); assert.deepEqual(standard.args, []);
  const candidate = await makeWorker(t, 'normal', { prefillBatch: 64 });
  assert.deepEqual(candidate.args.slice(-2), ['--prefill-batch', '64']);
  const identity = await candidate.start();
  assert.equal(identity.batch_size, 64); assert.equal(identity.ubatch_size, 64);
  await candidate.stop(); reaped(identity.pid);
});
