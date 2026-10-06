import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer, request as httpRequest } from 'node:http';
import { createDiscoveryHandler } from './discovery-server.mjs';

const deferred = () => { let resolve; let reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
const token = 'a'.repeat(64);
async function setup(t, overrides = {}) {
  let origin; const calls = []; const selections = [];
  const service = { progress: null,
    async search(value) { calls.push(value); return { schema: 'fixture', models: [] }; },
    async selection(artifact_id) { return { artifact_id, artifact: { path: '/server/private/descriptor.json' }, candidate: { repo: 'public/small' } }; },
    ...overrides.service };
  const handler = createDiscoveryHandler({ capability: token, origin: () => origin, service,
    selectArtifact: async value => { selections.push(value); }, ...overrides });
  const server = createServer(async (request, response) => {
    if (!await handler.handle(request, response)) { response.writeHead(404); response.end(); }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  origin = `http://127.0.0.1:${server.address().port}`;
  t.after(async () => { await handler.close(); server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); });
  const headers = { 'Content-Type': 'application/json', 'X-Badi-Lab-Token': token };
  return { origin, handler, server, calls, selections, headers,
    post: (operation, body, extra = {}) => fetch(`${origin}/api/discovery/${operation}`, { method: 'POST', headers, body: JSON.stringify(body), ...extra }) };
}

test('HTTP discovery rejects foreign origin, missing capability, arbitrary paths, prose and runtime fields', async t => {
  const lab = await setup(t);
  assert.equal((await lab.post('search', { query: 'small' }, { headers: { 'Content-Type': 'application/json' } })).status, 403);
  assert.equal((await lab.post('search', { query: 'small' }, { headers: { ...lab.headers, Origin: 'https://foreign.example' } })).status, 403);
  const foreignHost = await new Promise((resolve, reject) => {
    const request = httpRequest(`${lab.origin}/api/discovery/status`, { headers: { ...lab.headers, Host: 'foreign.example' } }, response => { response.resume(); response.on('end', () => resolve(response.statusCode)); });
    request.on('error', reject); request.end();
  });
  assert.equal(foreignHost, 403);
  for (const value of [{ query: 'small', draft: 'PRIVATE' }, { query: 'small', expected: 'PRIVATE' }, { query: 'small', url: 'http://localhost/private' },
    { query: 'small', runtime: '--arbitrary' }, { query: 42 }, [], null]) {
    assert.equal((await lab.post('search', value)).status, 400);
  }
  assert.equal((await lab.post('select', { artifact_id: '/tmp/arbitrary.json' })).status, 400);
  assert.equal((await lab.post('search', { query: 'x'.repeat(2200) })).status, 400);
  assert.equal(lab.calls.length, 0);
  assert.equal((await lab.post('search', { query: 'small' })).status, 200);
  assert.deepEqual(lab.calls, [{ query: 'small' }]);
  const status = await fetch(`${lab.origin}/api/discovery/status`, { headers: lab.headers });
  assert.equal(status.headers.get('cache-control'), 'no-store');
  assert.match((await status.json()).recommendation, /No qualified model/u);
});

test('global busy blocks discovery and selection preserves server-owned artifact identity', async t => {
  let busy = true; const lab = await setup(t, { isOtherBusy: () => busy });
  assert.equal((await lab.post('search', { query: 'small' })).status, 409); assert.equal(lab.calls.length, 0);
  busy = false;
  const result = await lab.post('select', { artifact_id: 'b'.repeat(64) }); assert.equal(result.status, 200);
  const value = await result.json();
  assert.equal(value.selected.state, 'selected_for_experiment'); assert.equal(value.selected.qualification, 'unqualified');
  assert.equal(JSON.stringify(value).includes('/server/private'), false);
  assert.equal(lab.selections[0].artifact.path, '/server/private/descriptor.json');
});

test('reset aborts the active operation, waits for it, and excludes stale success', async t => {
  const entered = deferred(); let signal;
  const lab = await setup(t, { service: { search: async (value, operationSignal) => {
    signal = operationSignal; entered.resolve();
    await new Promise(resolve => signal.addEventListener('abort', resolve, { once: true }));
    return { schema: 'late-success', models: ['must not appear'] };
  } } });
  const running = lab.post('search', { query: 'small' }); await entered.promise;
  assert.equal(lab.handler.busy, true);
  assert.equal((await lab.post('search', { query: 'second' })).status, 409);
  const reset = await lab.post('reset', {}); assert.equal(reset.status, 200);
  assert.equal(signal.aborted, true); assert.equal(lab.handler.busy, false);
  const stopped = await running; assert.equal(stopped.status, 409); assert.equal((await stopped.json()).error, 'discovery_cancelled');
});

test('an upload started before reset cannot later authorize network or selection', async t => {
  const lab = await setup(t); const entered = deferred(); const reply = deferred();
  lab.server.once('request', request => request.once('data', () => entered.resolve()));
  const request = httpRequest(`${lab.origin}/api/discovery/search`, { method: 'POST', headers: lab.headers }, response => {
    const chunks = []; response.on('data', chunk => chunks.push(chunk));
    response.on('end', () => reply.resolve({ status: response.statusCode, body: JSON.parse(Buffer.concat(chunks)) }));
  });
  request.on('error', reply.reject); request.write('{"query":'); await entered.promise;
  assert.equal((await lab.post('reset', {})).status, 200);
  request.end('"small"}'); const result = await reply.promise;
  assert.equal(result.status, 409); assert.equal(result.body.error, 'discovery_cancelled'); assert.equal(lab.calls.length, 0);
});

test('disconnect aborts owned work and unexpected failures do not disclose local exception contents', async t => {
  const entered = deferred(); let signal;
  const lab = await setup(t, { service: { search: async (value, suppliedSignal) => {
    signal = suppliedSignal; entered.resolve();
    await new Promise(resolve => signal.addEventListener('abort', resolve, { once: true }));
    throw new Error('PRIVATE PATH OR DRAFT');
  } } });
  const cancellation = new AbortController();
  const pending = lab.post('search', { query: 'small' }, { signal: cancellation.signal });
  await entered.promise; cancellation.abort(); await assert.rejects(pending);
  await new Promise(resolve => signal.aborted ? resolve() : signal.addEventListener('abort', resolve, { once: true }));
  assert.equal(signal.aborted, true);
});
