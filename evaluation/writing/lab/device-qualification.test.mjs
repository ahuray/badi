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
const qwenText = () => ({ model_type: 'qwen3_5_text', hidden_size: 1024, num_hidden_layers: 24,
  num_attention_heads: 8, num_key_value_heads: 2, head_dim: 256, max_position_embeddings: 262144,
  full_attention_interval: 4, layer_types: Array.from({ length: 24 }, (_, index) => (index + 1) % 4 ? 'linear_attention' : 'full_attention'),
  linear_conv_kernel_dim: 4, linear_key_head_dim: 128, linear_value_head_dim: 128,
  linear_num_key_heads: 16, linear_num_value_heads: 16 });
const qwenCandidate = () => ({ ...candidate(), architecture: 'qwen35', bytes: 532517120,
  config: summarizeHubModel({ id: 'publisher/qwen', config: { model_type: 'qwen3_5', text_config: qwenText(),
    vision_config: { hidden_size: 768, num_hidden_layers: 12 }, auto_map: { AutoModel: 'untrusted.py' } } }).config });
const qwenGguf = () => ({ architecture: 'qwen35', tokenizer_sha256: 'd'.repeat(64), metadata: {
  'qwen35.block_count': 25, 'qwen35.nextn_predict_layers': 1, 'qwen35.context_length': 262144,
  'qwen35.attention.recurrent_layers': [...qwenText().layer_types.map(type => type === 'linear_attention'), false],
  'qwen35.full_attention_interval': 4, 'qwen35.attention.head_count': 8, 'qwen35.attention.head_count_kv': 2,
  'qwen35.attention.key_length': 256, 'qwen35.attention.value_length': 256,
  'qwen35.ssm.conv_kernel': 4, 'qwen35.ssm.state_size': 128, 'qwen35.ssm.inner_size': 2048,
  'qwen35.ssm.group_count': 16, 'qwen35.ssm.time_step_rank': 16,
} });
const qwenState = { kind: 'qwen35', attention_layers: 6, recurrent_layers: 18, kv_heads: 2,
  key_length: 256, value_length: 256, convolution_kernel: 4, ssm_inner_size: 2048,
  ssm_state_size: 128, ssm_group_count: 16, ssm_time_step_rank: 16 };

test('Qwen3.5 nested text and actual GGUF independently describe the normal-context hybrid state', () => {
  const input = qwenCandidate();
  assert.deepEqual(candidateMetadata(input).state, qwenState);
  assert.equal(candidateMetadata(input).context_length, 8192, 'The smaller published context claim remains conservative before download.');
  assert.equal(input.config.vision_config, undefined); assert.equal(input.config.auto_map, undefined);
  const data = qwenGguf();
  assert.deepEqual(candidateMetadata(input, data).state, qwenState, 'One MTP head contributes no normal-context cache.');
  assert.equal(candidateMetadata(input, data).context_length, 262144);
  input.config.text_config.head_dim = 64;
  assert.deepEqual(candidateMetadata(input, data).state, qwenState, 'Downloaded GGUF dimensions replace upstream estimates.');
  data.metadata['qwen35.block_count'] = 24; delete data.metadata['qwen35.nextn_predict_layers'];
  data.metadata['qwen35.attention.recurrent_layers'].pop();
  assert.deepEqual(candidateMetadata(input, data).state, qwenState, 'Absent nextn metadata means no MTP layers.');
  for (const architecture of ['qwen3_5', 'qwen3_5_text']) {
    const changed = qwenCandidate(); changed.architecture = architecture;
    if (architecture === 'qwen3_5_text') changed.config = qwenText();
    assert.deepEqual(candidateMetadata(changed).state, qwenState);
    assert.equal(candidateMetadata(changed).architecture, 'qwen35');
  }
  assert.equal(candidateMetadata({ ...input, architecture: 'qwen2', config: qwenText() }).state.kind, 'unknown',
    'Conflicting upstream architecture cannot route hybrid dimensions through the transformer estimate.');
});

test('Qwen3.5 requires complete explicit dimensions and rejects malformed or contradictory layouts', () => {
  for (const name of ['num_hidden_layers', 'num_attention_heads', 'num_key_value_heads', 'head_dim',
    'linear_conv_kernel_dim', 'linear_key_head_dim', 'linear_value_head_dim', 'linear_num_key_heads', 'linear_num_value_heads']) {
    const input = qwenCandidate(); delete input.config.text_config[name];
    assert.equal(candidateMetadata(input).state.kind, 'unknown', name);
  }
  const mutations = [c => c.layer_types.pop(), c => c.layer_types[0] = 'full_attention', c => c.layer_types = null,
    c => c.layer_types = Array(257).fill('linear_attention'), c => c.full_attention_interval = 0,
    c => c.full_attention_interval = 3, c => c.head_dim = '256', c => c.head_dim = 0,
    c => c.num_attention_heads = 7, c => c.num_key_value_heads = 16, c => c.linear_num_value_heads = 17,
    c => c.linear_key_head_dim = 64, c => c.linear_value_head_dim = 128.5, c => c.linear_conv_kernel_dim = 1,
    c => c.linear_num_key_heads = 0, c => c.linear_num_value_heads = Number.MAX_SAFE_INTEGER,
    c => { c.linear_key_head_dim = 65536; c.linear_value_head_dim = 65536; c.linear_num_key_heads = 1; c.linear_num_value_heads = 1; },
    c => c.linear_conv_kernel_dim = 4097,
    c => { delete c.layer_types; delete c.full_attention_interval; }];
  for (const mutate of mutations) {
    const input = qwenCandidate(); mutate(input.config.text_config);
    input.config = summarizeHubModel({ id: 'publisher/qwen', config: input.config }).config;
    assert.equal(candidateMetadata(input).state.kind, 'unknown');
  }
  for (const remove of ['layer_types', 'full_attention_interval']) {
    const input = qwenCandidate(); delete input.config.text_config[remove];
    assert.deepEqual(candidateMetadata(input).state, qwenState, 'One complete explicit layer-layout declaration suffices.');
  }
  for (const change of [c => c.hidden_size = 768, c => c.text_config.model_type = 'unreviewed', c => c.text_config = []]) {
    const input = qwenCandidate(); change(input.config);
    input.config = summarizeHubModel({ id: 'publisher/qwen', config: input.config }).config;
    assert.equal(candidateMetadata(input).state.kind, 'unknown');
  }
});

test('Qwen3.5 GGUF refuses absent or invalid metadata without falling back to the complete upstream config', () => {
  for (const name of ['block_count', 'attention.head_count', 'attention.head_count_kv', 'attention.key_length',
    'attention.value_length', 'ssm.conv_kernel', 'ssm.state_size', 'ssm.inner_size', 'ssm.group_count', 'ssm.time_step_rank']) {
    const data = qwenGguf(); delete data.metadata[`qwen35.${name}`];
    assert.equal(candidateMetadata(qwenCandidate(), data).state.kind, 'unknown', name);
  }
  for (const change of [m => m['qwen35.nextn_predict_layers'] = 25, m => m['qwen35.nextn_predict_layers'] = -1,
    m => m['qwen35.nextn_predict_layers'] = 0.5, m => m['qwen35.attention.recurrent_layers'][24] = true,
    m => m['qwen35.attention.recurrent_layers'][0] = 1, m => m['qwen35.attention.recurrent_layers'].pop(),
    m => m['qwen35.full_attention_interval'] = 2, m => m['qwen35.ssm.inner_size'] = 1024,
    m => m['qwen35.attention.value_length'] = 128,
    m => m['qwen35.attention.head_count_kv'] = Array(25).fill(2),
    m => { delete m['qwen35.attention.recurrent_layers']; delete m['qwen35.full_attention_interval']; }]) {
    const data = qwenGguf(); change(data.metadata);
    assert.equal(candidateMetadata(qwenCandidate(), data).state.kind, 'unknown');
  }
  for (const remove of ['attention.recurrent_layers', 'full_attention_interval']) {
    const data = qwenGguf(); delete data.metadata[`qwen35.${remove}`];
    assert.deepEqual(candidateMetadata(qwenCandidate(), data).state, qwenState);
  }
  const data = qwenGguf(); data.architecture = 'qwen35moe';
  assert.equal(candidateMetadata(qwenCandidate(), data).state.kind, 'unknown');
});

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

test('Qwen3.5 local GGUF is parsed and reassessed before it can authorize loading', async t => {
  const { qualification, directory } = await bridge(t, async (flag, input) => ({ stages: [{ state: 'estimated_fit',
    passed: input.candidate.state.kind === 'qwen35', reasons: [] }], recommended: false }));
  const u32 = n => { const b = Buffer.alloc(4); b.writeUInt32LE(n); return b; };
  const u64 = n => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(n)); return b; };
  const string = value => Buffer.concat([u64(Buffer.byteLength(value)), Buffer.from(value)]);
  const header = metadata => Buffer.concat([Buffer.from('GGUF'), u32(3), u64(0), u64(Object.keys(metadata).length),
    ...Object.entries(metadata).map(([key, value]) => Buffer.concat([string(key),
      ...(typeof value === 'string' ? [u32(8), string(value)] : Array.isArray(value)
        ? [u32(9), u32(7), u64(value.length), Buffer.from(value.map(Number))] : [u32(4), u32(value)])]))]);
  const input = qwenCandidate();
  assert.equal((await qualification.assess(input)).download_allowed, true);
  const metadata = { 'general.architecture': 'qwen35', 'tokenizer.ggml.model': 'gpt2', ...qwenGguf().metadata };
  for (const complete of [true, false]) {
    if (!complete) delete metadata['qwen35.attention.key_length'];
    const bytes = header(metadata), path = join(directory, 'qwen-header.gguf'); await writeFile(path, bytes);
    const local = { ...input, bytes: bytes.length, sha256: digest(bytes) };
    const assessed = await qualification.assess(local, undefined, { artifact: { descriptor: { weights_path: path,
      sha256: local.sha256, bytes: local.bytes } } });
    assert.equal(assessed.metadata_source, 'local_gguf'); assert.equal(assessed.load_allowed, complete);
    assert.equal(assessed.metadata.state.kind, complete ? 'qwen35' : 'unknown');
    assert.equal(assessed.metadata.context_length, 262144);
  }
});

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
  assert.notEqual((await new DeviceQualification({ executable, memoryMap: false }).settings(config, ['de', 'en'])).runtime_identity, initial.runtime_identity);
  await writeFile(executable, 'Changed worker implementation.');
  assert.notEqual((await qualification.settings(config, ['de', 'en'])).runtime_identity, initial.runtime_identity);
  assert.deepEqual((await qualification.settings(config, ['fr', 'es'])).languages, ['es', 'fr']);
  for (const languages of [[], ['en', 'en'], ['it'], 'en']) await assert.rejects(qualification.settings(config, languages));
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
