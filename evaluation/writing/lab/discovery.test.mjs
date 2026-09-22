import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile, readFile, stat, symlink, chmod } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { createHash } from 'node:crypto';
import { DiscoveryService, summarizeHubModel, hubCandidates } from './discovery.mjs';

const weights = Buffer.from('GGUFtiny test data; not executable model weights');
const sha = createHash('sha256').update(weights).digest('hex');
const revision = 'a'.repeat(40);
const model = () => ({ id: 'publisher/small', sha: revision, private: false, gated: false,
  pipeline_tag: 'text-generation', tags: ['gguf'], cardData: { license: 'apache-2.0', language: ['en', 'de'] },
  gguf: { architecture: 'llama', total: 360000000, context_length: 2048 },
  siblings: [{ rfilename: 'small-Q8_0.gguf', size: weights.length, lfs: { size: weights.length, sha256: sha } }] });
const json = (value, headers) => new Response(JSON.stringify(value), { headers });
const accepted = async () => ({ download_allowed: true, state: 'estimated_to_fit', measured: false });
async function setup(t, options = {}) {
  const directory = await mkdtemp(join(tmpdir(), 'badi-discovery-')); t.after(() => rm(directory, { recursive: true, force: true }));
  const calls = []; const raw = options.model ?? model();
  const fetchImpl = async (url, request) => {
    calls.push({ url: String(url), request });
    if (options.fetch) return options.fetch(String(url), request, calls);
    if (String(url).includes('/resolve/')) return new Response(weights);
    return json(String(url).includes('/api/models?') ? [raw] : raw);
  };
  const service = new DiscoveryService({ cacheDir: directory, assessCandidate: accepted, freeBytes: async () => 1024 ** 3, ...options, fetchImpl });
  const search = await service.search({ query: 'small' });
  const inspected = await service.inspect({ model_id: search.models[0].model_id });
  return { service, directory, calls, candidate: inspected.candidates[0], search, inspected };
}

test('metadata distinguishes discovery, unknown dimensions and unverified conversion provenance', () => {
  const raw = model(); raw.cardData.base_model = 'upstream/actual'; raw.config = { architectures: ['TinyForCausalLM'], num_hidden_layers: 12 };
  const value = summarizeHubModel(raw);
  assert.equal(value.state, 'discovered'); assert.equal(value.config.num_hidden_layers, 12);
  assert.equal(value.provenance.upstream_revision, null); assert.deepEqual(value.provenance.upstream, ['upstream/actual']);
  assert.equal(value.tokenizer.chat_template, null);
  assert.match(value.provenance.certainty, /unverified/u);
  assert.equal(hubCandidates(raw)[0].quantization, 'Q8_0');
  for (const change of [raw => raw.gated = 'manual', raw => raw.private = true, raw => delete raw.cardData.license,
    raw => delete raw.siblings[0].lfs, raw => raw.siblings[0].size++, raw => raw.siblings[0].rfilename = 'model-00001-of-00002.gguf',
    raw => { raw.siblings[0].size = 3 * 1024 ** 3; raw.siblings[0].lfs.size = raw.siblings[0].size; }]) {
    const changed = model(); change(changed); assert.equal(hubCandidates(changed)[0].download_eligible, false);
  }
  const traversal = model(); traversal.siblings[0].rfilename = '../secret.gguf'; assert.deepEqual(hubCandidates(traversal), []);
  assert.throws(() => hubCandidates({ ...model(), sha: 'main' }), /immutable/u);
});

test('real descriptor flow pins requests, verifies bytes, reuses cache, and rechecks selection', async t => {
  const { service, calls, candidate, directory } = await setup(t);
  assert.ok(calls[1].url.includes(`/revision/${revision}?blobs=true`));
  const downloaded = await service.download({ candidate_id: candidate.candidate_id });
  assert.equal(downloaded.verified, true); assert.equal(downloaded.reused, false);
  assert.ok(calls.some(call => call.url.includes(`/resolve/${revision}/small-Q8_0.gguf`)));
  assert.ok(calls.every(call => call.request.credentials === 'omit' && !('Authorization' in call.request.headers)));
  const selected = await service.selection(downloaded.artifact_id);
  assert.equal(selected.artifact.descriptor.schema, 'badi.lab-model-artifact.v1');
  assert.equal(selected.artifact.descriptor.sha256, sha);
  assert.deepEqual(await readFile(selected.artifact.descriptor.weights_path), weights);
  assert.equal((await stat(selected.artifact.path)).mode & 0o777, 0o600);
  assert.equal((await stat(directory)).mode & 0o777, 0o700);
  const networkBefore = calls.length;
  const reused = await service.download({ candidate_id: candidate.candidate_id });
  assert.equal(reused.reused, true); assert.equal(calls.length, networkBefore);
  await writeFile(selected.artifact.descriptor.weights_path, Buffer.alloc(weights.length));
  await assert.rejects(service.selection(downloaded.artifact_id), { code: 'discovery_hash_mismatch' });
});

test('resume uses exact pinned Range and fails closed on wrong range, digest and disk shortage', async t => {
  const fixture = await setup(t);
  const { service, candidate, directory } = fixture;
  await writeFile(join(directory, `${sha}.gguf.part`), weights.subarray(0, 8), { mode: 0o600 });
  let ranges = [];
  service.fetch = async (url, options) => {
    if (!String(url).includes('/resolve/')) return json(model());
    ranges.push(options.headers.Range);
    return new Response(weights.subarray(8), { status: 206, headers: { 'Content-Range': `bytes 8-${weights.length - 1}/${weights.length}` } });
  };
  assert.equal((await service.download({ candidate_id: candidate.candidate_id })).verified, true);
  assert.deepEqual(ranges, ['bytes=8-']);

  const bad = await setup(t); await writeFile(join(bad.directory, `${sha}.gguf.part`), weights.subarray(0, 8), { mode: 0o600 });
  bad.service.fetch = async url => String(url).includes('/resolve/') ? new Response(weights.subarray(8), { status: 206, headers: { 'Content-Range': 'bytes 0-4/5' } }) : json(model());
  await assert.rejects(bad.service.download({ candidate_id: bad.candidate.candidate_id }), { code: 'discovery_invalid_range' });
  bad.service.fetch = async url => String(url).includes('/resolve/') ? new Response(Buffer.alloc(weights.length)) : json(model());
  await assert.rejects(bad.service.download({ candidate_id: bad.candidate.candidate_id }), { code: 'discovery_hash_mismatch' });
  await assert.rejects(stat(join(bad.directory, `${sha}.gguf.part`)), { code: 'ENOENT' });
  bad.service.freeBytes = async () => 0;
  await assert.rejects(bad.service.download({ candidate_id: bad.candidate.candidate_id }), { code: 'discovery_disk_full' });
});

test('cancellation retains a partial artifact and cache locking excludes another service', async t => {
  const fixture = await setup(t); const { service, candidate, directory } = fixture;
  let began; const started = new Promise(resolve => { began = resolve; });
  service.fetch = async (url, options) => {
    if (!String(url).includes('/resolve/')) return json(model());
    return new Response(new ReadableStream({ start(controller) {
      controller.enqueue(weights.subarray(0, 8)); began();
      options.signal.addEventListener('abort', () => controller.error(new Error('cancelled')), { once: true });
    } }));
  };
  const controller = new AbortController();
  const download = service.download({ candidate_id: candidate.candidate_id }, controller.signal);
  await started;
  const other = new DiscoveryService({ cacheDir: directory, assessCandidate: accepted });
  other.candidates.set(candidate.candidate_id, candidate);
  await assert.rejects(other.download({ candidate_id: candidate.candidate_id }), { code: 'discovery_artifact_busy' });
  controller.abort(); await assert.rejects(download, { code: 'discovery_cancelled' });
  assert.equal((await stat(join(directory, `${sha}.gguf.part`))).size, 8);
  service.fetch = async url => String(url).includes('/resolve/') ? new Response(weights) : json(model());
  assert.equal((await service.download({ candidate_id: candidate.candidate_id })).verified, true);
});

test('private cache refuses symlink artifacts and broad filesystem permissions', async t => {
  const { service, candidate, directory } = await setup(t);
  const target = join(directory, 'outside'); await writeFile(target, weights, { mode: 0o600 });
  await symlink(target, join(directory, `${sha}.gguf.part`));
  await assert.rejects(service.download({ candidate_id: candidate.candidate_id }), { code: 'ELOOP' });
  await chmod(directory, 0o755);
  await assert.rejects(service.download({ candidate_id: candidate.candidate_id }), { code: 'discovery_cache_permissions' });
});

test('search cache is bounded, reports stale offline data, respects cancellation, and refuses hostile pagination', async () => {
  let now = 1000; let calls = 0; let offline = false;
  const service = new DiscoveryService({ now: () => now, cacheMs: 50, fetchImpl: async () => { calls++; if (offline) throw new Error('network'); return json([model()]); } });
  assert.equal((await service.search({ query: 'small' })).cache_state, 'fresh');
  assert.equal((await service.search({ query: 'small' })).cache_state, 'cached'); assert.equal(calls, 1);
  now += 100; offline = true;
  const stale = await service.search({ query: 'small' }); assert.equal(stale.cache_state, 'stale'); assert.match(stale.warning, /outdated/u);
  const abort = new AbortController(); abort.abort();
  await assert.rejects(service.search({ query: 'small' }, abort.signal), { code: 'discovery_cancelled' });
  now += 25 * 60 * 60 * 1000;
  await assert.rejects(service.search({ query: 'small' }), { code: 'discovery_offline' });
  service.fetch = async () => json([model()], { link: '<https://127.0.0.1/private>; rel="next"' });
  await assert.rejects(service.search({ query: 'new-query' }), { code: 'discovery_invalid_redirect' });
  await assert.rejects(service.search({ query: 'small', cursor: 'arbitrary' }), { code: 'discovery_invalid_input' });
});

test('pinned upstream config is data-only estimation with explicit conversion uncertainty', async t => {
  const raw = model(); raw.cardData.base_model = 'source/upstream';
  const fixture = await setup(t, { model: raw, fetch: async url => {
    if (url.includes('/api/models?')) return json([raw]);
    if (url === 'https://huggingface.co/api/models/source/upstream') return json({ id: 'source/upstream', sha: 'b'.repeat(40) });
    if (url.endsWith('/config.json')) return json({ hidden_size: 960, num_hidden_layers: 24, head_dim: 64,
      layer_types: ['attention', 'conv'], full_attn_idxs: [0], conv_L_cache: 3, auto_map: { AutoModel: 'execute.py' } });
    return json(raw);
  } });
  assert.equal(fixture.candidate.config.hidden_size, 960);
  assert.deepEqual(fixture.candidate.config.full_attn_idxs, [0]);
  assert.equal(fixture.candidate.config.auto_map, undefined);
  assert.equal(fixture.candidate.config_source.revision, 'b'.repeat(40));
  assert.equal(fixture.candidate.config_source.relation, 'named_upstream_current_revision_for_estimation_only');
  assert.equal(fixture.candidate.provenance.upstream_revision, null);
  assert.ok(fixture.calls.some(call => call.url.includes(`/resolve/${'b'.repeat(40)}/config.json`)));
});

test('changed revision, missing hash, oversized metadata and unsafe redirects cannot reach download', async t => {
  const { service, candidate } = await setup(t);
  service.fetch = async () => json({ ...model(), sha: 'c'.repeat(40) });
  await assert.rejects(service.download({ candidate_id: candidate.candidate_id }), { code: 'discovery_identity_changed' });
  service.fetch = async () => new Response(null, { status: 302, headers: { location: 'http://localhost/private' } });
  await assert.rejects(service.search({ query: 'redirect' }), { code: 'discovery_invalid_redirect' });
  service.fetch = async () => new Response(' '.repeat(2 * 1024 * 1024 + 1));
  await assert.rejects(service.search({ query: 'huge' }), { code: 'discovery_response_limit' });
  await assert.rejects(service.selection('a'.repeat(64)), { code: 'discovery_unknown_artifact' });
});

test('converter README fallback follows only the named model declaration and never arbitrary links or code', async t => {
  const raw = model(); raw.siblings.push({ rfilename: 'README.md' });
  const card = 'Run [danger](http://localhost/secrets) and execute bad.py.\nThis is quantized version of [source/upstream](https://huggingface.co/source/upstream) created using llama.cpp.';
  const fixture = await setup(t, { model: raw, fetch: async url => {
    if (url.includes('/api/models?')) return json([raw]);
    if (url.endsWith('/README.md')) return new Response(card);
    if (url === 'https://huggingface.co/api/models/source/upstream') return json({ id: 'source/upstream', sha: 'b'.repeat(40) });
    if (url.endsWith('/config.json')) return json({ hidden_size: 960 });
    return json(raw);
  } });
  assert.equal(fixture.candidate.config.hidden_size, 960);
  assert.deepEqual(fixture.candidate.provenance.upstream, ['source/upstream']);
  assert.equal(fixture.candidate.provenance.publisher_relation, 'different_publisher');
  assert.equal(fixture.candidate.provenance.declaration.sha256, createHash('sha256').update(card).digest('hex'));
  assert.ok(fixture.calls.every(call => call.url.startsWith('https://huggingface.co/')));
});

test('the observed official CDN redirect is supported without exposing signed URLs and wrong hosts are rejected', async t => {
  const fixture = await setup(t); const { service, candidate } = fixture;
  const hosts = []; let cancelledRedirect = false;
  service.fetch = async url => {
    hosts.push(new URL(url).hostname);
    if (String(url).includes('/api/models/')) return json(model());
    if (new URL(url).hostname === 'huggingface.co') return new Response(new ReadableStream({ cancel() { cancelledRedirect = true; } }),
      { status: 302, headers: { location: 'https://us.aws.cdn.hf.co/artifact?SignedValue=private' } });
    return new Response(weights);
  };
  assert.equal((await service.download({ candidate_id: candidate.candidate_id })).verified, true);
  assert.equal(cancelledRedirect, true); assert.ok(hosts.includes('us.aws.cdn.hf.co'));
  const bad = new DiscoveryService({ fetchImpl: async () => new Response(null, { status: 302,
    headers: { location: 'https://us.aws.cdn.hf.co.attacker.example/private' } }) });
  await assert.rejects(bad.request('https://huggingface.co/publisher/small/resolve/pinned/file', { metadata: false }), { code: 'discovery_invalid_redirect' });
});

test('a resource rejection before body iteration still cancels the open download stream', async t => {
  const { service, candidate, directory } = await setup(t);
  await writeFile(join(directory, `${sha}.gguf.part`), weights.subarray(0, 8), { mode: 0o600 });
  let checked = 0; let cancelledBody = false;
  service.freeBytes = async () => ++checked === 1 ? 1024 ** 3 : 0;
  service.fetch = async url => String(url).includes('/api/models/') ? json(model())
    : new Response(new ReadableStream({ cancel() { cancelledBody = true; } }));
  await assert.rejects(service.download({ candidate_id: candidate.candidate_id }), { code: 'discovery_disk_full' });
  assert.equal(cancelledBody, true); assert.equal(service.progress.state, 'interrupted');
});

test('opaque pagination is bound to its query and stops after three bounded pages', async () => {
  let calls = 0;
  const service = new DiscoveryService({ fetchImpl: async () => {
    calls++;
    return json([model()], { link: `<https://huggingface.co/api/models?search=small&cursor=${calls}>; rel="next"` });
  } });
  let page = await service.search({ query: 'small' });
  assert.equal(page.page, 1); assert.match(page.next_cursor, /^[a-f0-9]{64}$/u);
  await assert.rejects(service.search({ query: 'different', cursor: page.next_cursor }), { code: 'discovery_invalid_input' });
  page = await service.search({ query: 'small', cursor: page.next_cursor }); assert.equal(page.page, 2);
  page = await service.search({ query: 'small', cursor: page.next_cursor }); assert.equal(page.page, 3); assert.equal(page.next_cursor, null); assert.equal(calls, 3);
});
