import test from 'node:test';
import assert from 'node:assert/strict';
import { createLabServer } from './server.mjs';

const deferred = () => { let resolve; let reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };

async function setup(t, inspect) {
  const worker = { async stop() {}, async selectArtifact() {}, async predict() { throw new Error('This fixture must not start inference.'); } };
  const lab = await createLabServer({ worker, qualification: { inspect, async assess() { throw new Error('Unexpected assessment.'); } } });
  let closing;
  const close = () => { closing ??= lab.close(); return closing; };
  t.after(close);
  const page = await (await fetch(lab.origin)).text();
  const token = page.match(/name="badi-lab-token" content="([a-f0-9]{64})"/u)?.[1]; assert.ok(token);
  const headers = { 'Content-Type': 'application/json', 'X-Badi-Lab-Token': token };
  return { ...lab, close, headers, inspect: options => fetch(`${lab.origin}/api/device`, { headers, ...options }) };
}

test('device inspection failures return an actionable JSON error instead of empty HTTP 200', async t => {
  const lab = await setup(t, async () => { throw new Error('Device inspector unavailable.'); });
  const response = await lab.inspect();
  assert.ok(response.status >= 400, 'A failed device inspection must not report HTTP success.');
  assert.match(response.headers.get('content-type'), /application\/json/u);
  const body = await response.json(); assert.equal(typeof body.error, 'string'); assert.ok(body.error.length > 0);
});

test('concurrent device refreshes do not launch unbounded inspector work', async t => {
  const release = deferred(); let calls = 0;
  const lab = await setup(t, async () => { calls++; return release.promise; });
  const arrived = deferred(); let received = 0;
  lab.server.on('request', request => { if (request.url === '/api/device' && ++received === 3) arrived.resolve(); });
  const requests = [lab.inspect(), lab.inspect(), lab.inspect()];
  try {
    await arrived.promise;
    assert.equal(calls, 1, 'Device refresh should coalesce or reject concurrent inspectors.');
  } finally {
    release.resolve({ schema: 'device-fixture', observed: true });
    const replies = await Promise.all(requests); await Promise.all(replies.map(response => response.arrayBuffer()));
  }
});

test('disconnect cancels an owned device inspection rather than leaving its child running', async t => {
  const entered = deferred(), release = deferred(); let suppliedSignal;
  const lab = await setup(t, async signal => {
    suppliedSignal = signal; entered.resolve();
    if (signal) signal.addEventListener('abort', () => release.reject(new Error('Device inspection cancelled.')), { once: true });
    return release.promise;
  });
  const closed = deferred();
  lab.server.on('request', (request, response) => { if (request.url === '/api/device') response.once('close', closed.resolve); });
  const cancellation = new AbortController(); const pending = lab.inspect({ signal: cancellation.signal });
  try {
    await entered.promise; cancellation.abort(); await assert.rejects(pending); await closed.promise;
    assert.ok(suppliedSignal instanceof AbortSignal, 'The inspector needs a lifecycle cancellation signal.');
    assert.equal(suppliedSignal.aborted, true);
  } finally { release.resolve({ schema: 'device-fixture' }); }
});

test('device inspection cannot overlap a measured prediction comparison', async t => {
  const entered = deferred(), release = deferred();
  const lab = await setup(t, async () => { entered.resolve(); return release.promise; });
  const inspection = lab.inspect(); await entered.promise;
  const body = { suite: { schema: 'badi.prediction-suite.v1', name: 'fixture', cases: [
    { id: 'sample', language: 'en', prefix: 'Please review the docum', expected: ['ent'], context: '', style: '' }] },
  configs: [{ id: 'baseline', mode: 'production_baseline', budget_ms: 550, max_tokens: 8, cache_prompt: true, temperature: 0, seed: 42 }], seed: 42 };
  try {
    const response = await fetch(`${lab.origin}/api/run`, { method: 'POST', headers: lab.headers, body: JSON.stringify(body) });
    await response.arrayBuffer();
    assert.equal(response.status, 409, 'Device work and measured inference must share the ownership gate.');
  } finally { release.resolve({ schema: 'device-fixture' }); await (await inspection).arrayBuffer(); }
});

test('reset holds all work until a cancelled inspector has finished its cleanup', async t => {
  const entered = deferred(), aborted = deferred(), cleanup = deferred(); let calls = 0;
  const lab = await setup(t, async signal => {
    calls++; entered.resolve(); signal.addEventListener('abort', aborted.resolve, { once: true });
    await cleanup.promise; return { schema: 'late-device-fixture' };
  });
  const inspection = lab.inspect(); await entered.promise;
  let resetFinished = false;
  const resetting = fetch(`${lab.origin}/api/reset`, { method: 'POST', headers: lab.headers, body: '{}' }).then(response => { resetFinished = true; return response; });
  try {
    await aborted.promise;
    const blocked = await lab.inspect(); await blocked.arrayBuffer(); assert.equal(blocked.status, 409);
    const modelChange = await fetch(`${lab.origin}/api/model/baseline`, { method: 'POST', headers: lab.headers, body: '{}' });
    await modelChange.arrayBuffer(); assert.equal(modelChange.status, 409);
    assert.equal(calls, 1); assert.equal(resetFinished, false, 'Reset must wait until the inspector child is reaped.');
  } finally { cleanup.resolve(); }
  assert.equal((await resetting).status, 200);
  const stopped = await inspection; assert.equal(stopped.status, 409); assert.match((await stopped.json()).error, /cancelled/u);
  const retry = await lab.inspect(); assert.equal(retry.status, 200); await retry.arrayBuffer();
});

test('server close cancels and awaits inspector cleanup before completing', async t => {
  const entered = deferred(), aborted = deferred(), cleanup = deferred();
  const lab = await setup(t, async signal => {
    entered.resolve(); signal.addEventListener('abort', aborted.resolve, { once: true });
    await cleanup.promise; return { schema: 'late-device-fixture' };
  });
  const inspection = lab.inspect(); await entered.promise;
  let finished = false; const closing = lab.close().then(() => { finished = true; });
  try { await aborted.promise; assert.equal(finished, false, 'Server close must await reaping.'); }
  finally { cleanup.resolve(); }
  const response = await inspection; assert.equal(response.status, 409); await response.arrayBuffer();
  await closing; assert.equal(finished, true);
});
