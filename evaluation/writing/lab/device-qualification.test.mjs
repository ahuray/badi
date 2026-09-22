import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { DeviceQualification, candidateMetadata, digest } from './device-qualification.mjs';
import { summarizeHubModel } from './discovery.mjs';
import { LabWorker } from './worker.mjs';

const candidate = () => ({ candidate_id: 'a'.repeat(64), revision: 'b'.repeat(40), sha256: 'c'.repeat(64), bytes: 380000000,
  quantization: 'Q8_0', architecture: 'llama', context_length: 8192, languages: ['en'], private: false, gated: false, license: 'apache-2.0',
  config: { model_type: 'llama', num_hidden_layers: 16, hidden_size: 1024, num_attention_heads: 16, num_key_value_heads: 8, max_position_embeddings: 8192 } });
const gguf = (architecture = 'lfm2') => ({ architecture, tokenizer_sha256: 'd'.repeat(64), metadata: {
  [`${architecture}.block_count`]: 16, [`${architecture}.embedding_length`]: 1024, [`${architecture}.context_length`]: 128000,
  [`${architecture}.attention.head_count`]: 16,
  [`${architecture}.attention.head_count_kv`]: [0, 0, 8, 0, 0, 8, 0, 0, 8, 0, 8, 0, 8, 0, 8, 0],
  [`${architecture}.shortconv.l_cache`]: 3,
} });
const config = { id: 'fixture-arm', mode: 'healed', budget_ms: 550, max_tokens: 8, cache_prompt: true, temperature: 0, seed: 42 };
const report = () => ({ stages: [{ state: 'estimated_fit', passed: true, reasons: [] }], recommended: false });

test('actual LFM per-layer KV metadata maps six attention and ten convolution layers', () => {
  const result = candidateMetadata(candidate(), gguf());
  assert.equal(result.architecture, 'lfm2');
  assert.deepEqual(result.state, { kind: 'lfm2', attention_layers: 6, convolution_layers: 10,
    kv_heads: 8, key_length: 64, value_length: 64, embedding_length: 1024, convolution_cache_length: 3 });
  assert.equal(result.context_length, 128000, 'Actual GGUF context is authoritative over old Hub/config claims.');
  assert.equal(result.tokenizer_identity, `sha256:${'d'.repeat(64)}`);
  const granite = gguf('granite');
  granite.metadata['granite.block_count'] = 28; granite.metadata['granite.attention.head_count_kv'] = Array(28).fill(4);
  assert.deepEqual(candidateMetadata(candidate(), granite).state, { kind: 'transformer', layers: 28, kv_heads: 4, key_length: 64, value_length: 64 });
});

test('missing actual GGUF fields never fall back to richer upstream config or Hub claims', () => {
  for (const field of ['block_count', 'embedding_length', 'attention.head_count', 'attention.head_count_kv', 'shortconv.l_cache']) {
    const data = gguf(); delete data.metadata[`lfm2.${field}`]; assert.equal(candidateMetadata(candidate(), data).state.kind, 'unknown', field);
  }
  const missing = gguf(); delete missing.metadata['lfm2.context_length']; delete missing.tokenizer_sha256;
  const result = candidateMetadata(candidate(), missing); assert.equal(result.context_length, null); assert.equal(result.tokenizer_identity, null);
  assert.equal(candidateMetadata(candidate(), { ...gguf(), architecture: null }).architecture, null);
});

test('zero, contradictory, nonintegral and malformed attention dimensions fail closed', () => {
  for (const change of [data => data.metadata['lfm2.attention.key_length'] = 0,
    data => data.metadata['lfm2.attention.value_length'] = -1,
    data => data.metadata['lfm2.attention.key_length'] = '64',
    data => data.metadata['lfm2.attention.head_count'] = 0,
    data => data.metadata['lfm2.attention.head_count'] = 15,
    data => data.metadata['lfm2.attention.head_count_kv'] = [8],
    data => data.metadata['lfm2.attention.head_count_kv'] = Array(16).fill(0),
    data => data.metadata['lfm2.attention.head_count_kv'][2] = 32,
    data => data.metadata['lfm2.attention.head_count_kv'][2] = 1.5,
    data => data.metadata['lfm2.attention.head_count_kv'][2] = -1,
    data => data.metadata['lfm2.attention.head_count_kv'] = 8,
    data => data.metadata['lfm2.shortconv.l_cache'] = 0]) {
    const data = gguf(); change(data); assert.equal(candidateMetadata(candidate(), data).state.kind, 'unknown');
  }
  const input = candidate(); input.config.head_dim = 0; assert.equal(candidateMetadata(input).state.kind, 'unknown');
  const data = gguf('llama'); data.metadata['llama.attention.head_count_kv'] = Array(16).fill(0);
  assert.equal(candidateMetadata(candidate(), data).state.kind, 'unknown');
});

test('variable query-head dimensions use the largest state cost instead of underestimating it', () => {
  const data = gguf('llama'); Object.assign(data.metadata, {
    'llama.block_count': 2, 'llama.attention.head_count': [8, 16], 'llama.attention.head_count_kv': [4, 8],
  });
  assert.deepEqual(candidateMetadata(candidate(), data).state, { kind: 'transformer', layers: 2, kv_heads: 8, key_length: 128, value_length: 128 });
  data.metadata['llama.attention.key_length'] = 192; data.metadata['llama.attention.value_length'] = 256;
  assert.equal(candidateMetadata(candidate(), data).state.key_length, 192, 'Explicit valid model dimensions override inference.');
});

test('upstream LFM masks require bounded distinct indices and consistent declared layer types', () => {
  const input = candidate(); input.architecture = 'lfm2'; Object.assign(input.config, {
    layer_types: ['conv', 'conv', 'full_attention', 'conv', 'conv', 'full_attention', 'conv', 'conv', 'full_attention', 'conv', 'full_attention', 'conv', 'full_attention', 'conv', 'full_attention', 'conv'],
    full_attn_idxs: [2, 5, 8, 10, 12, 14], conv_L_cache: 3,
  });
  assert.equal(candidateMetadata(input).state.attention_layers, 6);
  for (const change of [c => c.full_attn_idxs = [2, 2, 8, 10, 12, 14], c => c.full_attn_idxs = [0, 5, 8, 10, 12, 14],
    c => c.full_attn_idxs = [2, 5, 8, 10, 12, 16], c => c.full_attn_idxs = [2.5], c => c.layer_types[0] = 'unknown',
    c => c.layer_types.pop(), c => c.conv_L_cache = 0]) {
    const changed = structuredClone(input); change(changed.config); assert.equal(candidateMetadata(changed).state.kind, 'unknown');
  }
});

test('zero experts survives Hub normalization and only the exact all-attention Granite hybrid maps to granite', () => {
  const input = candidate(); input.architecture = 'granitemoehybrid'; Object.assign(input.config, { model_type: 'granitemoehybrid', num_local_experts: 0, layer_types: Array(16).fill('attention') });
  const normalized = summarizeHubModel({ id: 'publisher/granite', config: input.config });
  assert.equal(normalized.config.num_local_experts, 0);
  assert.equal(candidateMetadata({ ...input, config: normalized.config }).architecture, 'granite');
  for (const change of [c => c.num_local_experts = 1, c => delete c.num_local_experts, c => c.layer_types[0] = 'mamba']) {
    const changed = structuredClone(input); change(changed.config); assert.equal(candidateMetadata(changed).state.kind, 'unknown');
  }
  const invalidDimension = summarizeHubModel({ id: 'publisher/small', config: { ...candidate().config, head_dim: 0 } });
  assert.equal(candidateMetadata({ ...candidate(), config: invalidDimension.config }).state.kind, 'unknown');
});

async function bridge(t, command = async () => report()) {
  const directory = await mkdtemp(join(tmpdir(), 'badi-device-bridge-')); t.after(() => rm(directory, { recursive: true, force: true }));
  const executable = join(directory, 'worker-identity.fixture'); await writeFile(executable, 'Only hashed. Never executed.');
  return { directory, executable, qualification: new DeviceQualification({ executable, command }) };
}

test('settings bind the exact config JSON, requested languages, worker bytes and batch choice', async t => {
  const { qualification, executable } = await bridge(t);
  const initial = await qualification.settings(config, ['de', 'en']);
  assert.equal(initial.generation_settings_sha256, digest(config)); assert.deepEqual(initial.languages, ['de', 'en']);
  assert.equal(digest(await qualification.settings(config, ['en', 'de'])), digest(initial));
  for (const change of [{ id: 'different-arm' }, { mode: 'native_instructed' }, { budget_ms: 1000 }, { max_tokens: 16 },
    { cache_prompt: false }, { temperature: 0.2 }, { seed: 7 }]) {
    assert.notEqual((await qualification.settings({ ...config, ...change }, ['de', 'en'])).generation_settings_sha256, initial.generation_settings_sha256);
  }
  assert.notEqual(digest(await qualification.settings(config, ['en'])), digest(initial));
  assert.notEqual(digest(await new DeviceQualification({ executable, prefillBatch: 64 }).settings(config, ['de', 'en'])), digest(initial));
  await writeFile(executable, 'Changed worker implementation.');
  assert.notEqual((await qualification.settings(config, ['de', 'en'])).runtime_identity, initial.runtime_identity);
  for (const languages of [[], ['en', 'en'], ['fr'], 'en']) await assert.rejects(qualification.settings(config, languages));
  assert.throws(() => new DeviceQualification({ prefillBatch: 32 }));
});

test('assessment and ranking pass exact model, settings, evidence and cancellation through the fixed command boundary', async t => {
  const calls = []; const { qualification } = await bridge(t, async (flag, input, options) => { calls.push({ flag, input, options }); return flag === '--rank-models' ? { recommendation: null } : report(); });
  const signal = new AbortController().signal; const evidence = { identity: { fixture: 'evidence-only' } };
  const assessed = await qualification.assess(candidate(), signal, { config, languages: ['en'], evidence });
  assert.equal(assessed.download_allowed, true); assert.equal(assessed.load_allowed, false);
  assert.deepEqual(calls[0].input.evidence, evidence); assert.equal(calls[0].input.settings.generation_settings_sha256, digest(config));
  assert.deepEqual(calls[0].input.settings.languages, ['en']); assert.equal(calls[0].options.signal, signal);
  await qualification.rank([{ assessment: assessed, evidence }], signal);
  assert.equal(calls[1].flag, '--rank-models');
  assert.deepEqual(calls[1].input, [{ candidate: assessed.metadata, settings: assessed.settings, evidence }]);
  assert.equal(calls[1].options.signal, signal);
  await assert.rejects(qualification.assess(candidate(), signal, { artifact: { descriptor: { sha256: 'f'.repeat(64), bytes: candidate().bytes, weights_path: '/must-not-open' } } }), /differs/u);
  assert.equal(calls.length, 2, 'Mismatched artifacts must not reach inspection or qualification.');
});

test('aborted assessment cannot save a late successful response or authorize another load', async t => {
  let finish, began; const entered = new Promise(resolve => { began = resolve; });
  const { qualification } = await bridge(t, async () => { began(); return new Promise(resolve => { finish = resolve; }); });
  const controller = new AbortController(); const pending = qualification.assess(candidate(), controller.signal, { config });
  await entered; controller.abort(); finish(report()); await assert.rejects(pending);
  assert.equal(qualification.assessments.size, 0);
});

test('an empty embedded chat template is not advertised as an instruction prompt profile', async t => {
  const { qualification, directory } = await bridge(t);
  const u32 = n => { const data = Buffer.alloc(4); data.writeUInt32LE(n); return data; };
  const u64 = n => { const data = Buffer.alloc(8); data.writeBigUInt64LE(BigInt(n)); return data; };
  const string = text => Buffer.concat([u64(Buffer.byteLength(text)), Buffer.from(text)]);
  const fields = [['general.architecture', 'llama'], ['tokenizer.ggml.model', 'gpt2'], ['tokenizer.chat_template', '  ']];
  const bytes = Buffer.concat([Buffer.from('GGUF'), u32(3), u64(0), u64(fields.length),
    ...fields.map(([key, value]) => Buffer.concat([string(key), u32(8), string(value)]))]);
  const path = join(directory, 'header-only.gguf'); await writeFile(path, bytes);
  const input = { ...candidate(), bytes: bytes.length, sha256: digest(bytes) };
  const artifact = { descriptor: { sha256: input.sha256, bytes: input.bytes, weights_path: path } };
  const assessed = await qualification.assess(input, undefined, { artifact, config });
  assert.equal(assessed.prompt_profile, 'base_continuation');
  assert.equal(assessed.metadata.state.kind, 'unknown'); assert.equal(assessed.metadata.context_length, null);
  // The injected inspector is a bridge fixture, not runtime model qualification.
});

test('worker reset during asynchronous preload prevents a late resource check from launching', async () => {
  let release, began; const entered = new Promise(resolve => { began = resolve; }); let starts = 0;
  const worker = new LabWorker({ executable: '/must-not-launch' });
  await worker.selectArtifact(null, async () => { began(); await new Promise(resolve => { release = resolve; }); });
  worker.start = async () => { starts++; throw new Error('A cancelled preflight must not start a model.'); };
  const pending = worker.predict({ id: 'preload-fixture', config }, new AbortController().signal);
  await entered; await worker.stop(); release();
  await assert.rejects(pending, /preparation was cancelled/u);
  assert.equal(starts, 0); assert.equal(worker.session, null); assert.equal(worker.preparing, false);
});
