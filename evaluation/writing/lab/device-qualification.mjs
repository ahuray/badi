import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { inspectGguf } from './gguf.mjs';

const binary = fileURLToPath(new URL('../../../target/release/badi-writing-lab', import.meta.url));
const evaluatorSources = ['device-qualification.mjs', 'gguf.mjs', 'model-artifact.mjs', 'worker.mjs', 'cases.mjs', 'server.mjs',
  'qualification-evidence.mjs', 'qualification-diagnostics.mjs', 'qualification-server.mjs', 'public/qualification.mjs', 'public/app.mjs'];
export const digest = value => createHash('sha256').update(typeof value === 'string' || Buffer.isBuffer(value) ? value : JSON.stringify(value)).digest('hex');
const positive = value => Number.isSafeInteger(value) && value > 0 ? value : null;

// The child is a fixed local read-only inspector, never a repository executable.
export function qualificationCommand(flag, input, { executable = binary, cacheDirectory, signal } = {}) {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) { reject(new Error('Device assessment cancelled.')); return; }
    const child = spawn(executable, [flag, ...(cacheDirectory ? ['--cache-directory', cacheDirectory] : [])], { stdio: ['pipe', 'pipe', 'ignore'] });
    const chunks = []; let size = 0, failure;
    const stop = message => { failure ??= new Error(message); child.kill('SIGKILL'); };
    const timer = setTimeout(() => stop('Device assessment timed out.'), 8000);
    const abort = () => stop('Device assessment cancelled.'); signal?.addEventListener('abort', abort, { once: true });
    child.stdout.on('data', chunk => { size += chunk.length; if (size > 256 * 1024) stop('Device assessment exceeded its output limit.'); else chunks.push(chunk); });
    child.stdin.on('error', () => stop('Device assessment input failed.'));
    child.on('error', () => { failure = new Error('Build the Prediction Lab worker before inspecting models.'); });
    child.on('close', code => {
      clearTimeout(timer); signal?.removeEventListener('abort', abort);
      if (failure) { reject(failure); return; }
      try { const result = JSON.parse(Buffer.concat(chunks)); if (code !== 0 || result.type === 'error') throw new Error(); resolve(result); }
      catch { reject(new Error('Device assessment failed. Rebuild the Prediction Lab worker and inspect the model metadata.')); }
    });
    child.stdin.end(input === undefined ? '' : JSON.stringify(input));
  });
}

export function candidateMetadata(candidate, gguf = null) {
  const c = candidate.config ?? {}, m = gguf?.metadata ?? {};
  let architecture = gguf ? gguf.architecture ?? null : candidate.architecture ?? c.model_type ?? null;
  if (!gguf && architecture === 'granitemoehybrid' && Array.isArray(c.layer_types)
    && c.layer_types.length === c.num_hidden_layers && c.num_hidden_layers > 0
    && c.layer_types.every(value => value === 'attention') && c.num_local_experts === 0) architecture = 'granite';
  const field = (key, other) => gguf ? positive(m[`${architecture}.${key}`]) : positive(other);
  const layers = field('block_count', c.num_hidden_layers);
  const hidden = field('embedding_length', c.hidden_size);
  const dimension = (key, other, zeros = false) => {
    const values = gguf ? m[`${architecture}.${key}`] : other;
    if (Array.isArray(values)) return values.length === layers && values.length <= 1024
      && values.every(value => Number.isSafeInteger(value) && (zeros ? value >= 0 : value > 0)) ? values : null;
    return positive(values);
  };
  const hybrid = architecture === 'lfm2';
  const heads = dimension('attention.head_count', c.num_attention_heads, hybrid);
  const kvHeads = dimension('attention.head_count_kv', c.num_key_value_heads, hybrid);
  const values = value => Array.isArray(value) ? value : value === null ? [] : [value];
  const at = (value, index) => Array.isArray(value) ? value[index] : value;
  const validHeads = heads !== null && kvHeads !== null && values(heads).some(n => n > 0)
    && values(kvHeads).some(n => n > 0)
    && Array.from({ length: Array.isArray(heads) ? heads.length : Array.isArray(kvHeads) ? kvHeads.length : 1 }, (_, index) => index)
      .every(index => at(kvHeads, index) === 0 && hybrid || at(heads, index) > 0 && at(kvHeads, index) <= at(heads, index) && at(heads, index) % at(kvHeads, index) === 0);
  const kv = validHeads ? Math.max(...values(kvHeads)) : null;
  // Variable per-layer query heads imply variable head dimensions when no
  // explicit dimensions are provided. Charge the largest dimension, not the
  // smaller hidden/max(heads) value that can underestimate attention state.
  const inferred = validHeads && hidden ? values(heads).filter(n => n > 0).map(n => positive(hidden / n)) : [];
  const derivedHead = inferred.length && inferred.every(Boolean) ? Math.max(...inferred) : null;
  const headDimension = key => {
    const source = gguf ? m : c; const name = gguf ? `${architecture}.${key}` : 'head_dim';
    return Object.hasOwn(source, name) ? positive(source[name]) : derivedHead;
  };
  const key = headDimension('attention.key_length');
  const value = headDimension('attention.value_length');
  let state = { kind: 'unknown' };
  if (layers && kv && key && value && ['llama', 'qwen2', 'qwen3', 'granite', 'gemma', 'gemma2', 'gemma3', 'smollm3'].includes(architecture)) {
    state = { kind: 'transformer', layers, kv_heads: kv, key_length: key, value_length: value };
  } else if (architecture === 'lfm2' && layers && hidden && kv && key && value) {
    let attention = null;
    if (gguf && Array.isArray(kvHeads)) attention = kvHeads.filter(n => n > 0).length;
    if (!gguf) {
      const types = c.layer_types; const indices = c.full_attn_idxs;
      const validTypes = Array.isArray(types) && types.length === layers
        && types.every(type => ['conv', 'full_attention'].includes(type));
      const validIndices = Array.isArray(indices) && indices.length < layers && new Set(indices).size === indices.length
        && indices.every(index => Number.isSafeInteger(index) && index >= 0 && index < layers);
      const declaredTypes = Object.hasOwn(c, 'layer_types'), declaredIndices = Object.hasOwn(c, 'full_attn_idxs');
      const consistent = !declaredTypes || !declaredIndices || validTypes && validIndices
        && types.every((type, index) => (type === 'full_attention') === indices.includes(index));
      if ((!declaredTypes || validTypes) && (!declaredIndices || validIndices) && consistent) {
        attention = validTypes ? types.filter(kind => kind === 'full_attention').length : validIndices ? indices.length : null;
      }
    }
    const cache = field('shortconv.l_cache', c.conv_L_cache);
    if (attention && attention < layers && cache) state = { kind: 'lfm2', attention_layers: attention,
      convolution_layers: layers - attention, kv_heads: kv, key_length: key, value_length: value,
      embedding_length: hidden, convolution_cache_length: cache };
  }
  const limits = (gguf ? [field('context_length')] : [positive(c.max_position_embeddings), positive(candidate.context_length)]).filter(Boolean);
  return { id: candidate.candidate_id, revision: candidate.revision, sha256: candidate.sha256,
    artifact_bytes: candidate.bytes, tokenizer_identity: gguf ? gguf.tokenizer_sha256 ? `sha256:${gguf.tokenizer_sha256}` : null : candidate.sha256 ? `gguf:${candidate.sha256}` : null,
    quantization: candidate.quantization, architecture, context_length: limits.length ? Math.min(...limits) : null,
    languages: candidate.languages ?? [], format: 'gguf', access: candidate.private === false && candidate.gated === false ? 'public' : 'unknown',
    license: candidate.license, state };
}

export class DeviceQualification {
  constructor({ executable = binary, cacheDirectory, prefillBatch = 16, command = qualificationCommand } = {}) {
    if (![16, 64].includes(prefillBatch) || typeof command !== 'function') throw new Error('Use the verified Lab prefill batch of 16 or 64.');
    this.executable = executable; this.cacheDirectory = cacheDirectory; this.prefillBatch = prefillBatch;
    this.command = command;
    this.evidence = new Map(); this.assessments = new Map();
  }
  async inspect(signal) { return this.command('--inspect-device', undefined, { executable: this.executable, cacheDirectory: this.cacheDirectory, signal }); }
  async settings(config = { mode: 'healed', budget_ms: 550, max_tokens: 8, cache_prompt: true, temperature: 0, seed: 42 }, languages = ['en', 'de', 'fa']) {
    if (!Array.isArray(languages) || !languages.length || languages.length > 3 || new Set(languages).size !== languages.length || languages.some(value => !['en', 'de', 'fa'].includes(value))) throw new Error('Select distinct supported evaluation languages.');
    const workerSha = digest(await readFile(this.executable));
    // Bind the worker (which embeds its pinned llama.cpp archive) and the local
    // evaluator/HTTP/review implementation. A JS-only change invalidates saved
    // measurements just as a changed model worker does.
    const evaluator = await Promise.all(evaluatorSources.map(async path => [path, digest(await readFile(new URL(path, import.meta.url)))]));
    return { runtime_identity: digest({ worker_sha256: workerSha, evaluator }), runtime_version: 'b10726', runtime_architecture: 'x86_64', required_cpu_features: ['avx2'], backend: 'cpu', context_tokens: 2048,
      batch_tokens: this.prefillBatch, threads: 4, parallel_sequences: 1, cache_element_bytes: 2,
      recurrent_snapshots: 0, context_checkpoints: 0, prompt_format: config.mode,
      generation_settings_sha256: digest(config), languages: [...languages].sort(), evaluation_version: 'badi.prediction-quality.v1' };
  }
  async assess(candidate, signal, { artifact, config, evidence, languages } = {}) {
    signal?.throwIfAborted();
    if (artifact && (artifact.descriptor?.sha256 !== candidate.sha256 || artifact.descriptor?.bytes !== candidate.bytes)) {
      throw new Error('The local artifact descriptor differs from the discovered candidate identity.');
    }
    const gguf = artifact ? await inspectGguf(artifact.descriptor.weights_path) : null;
    signal?.throwIfAborted();
    const metadata = candidateMetadata(candidate, gguf), settings = await this.settings(config, languages);
    signal?.throwIfAborted();
    const report = await this.command('--assess-model', { candidate: metadata, settings, evidence: evidence ?? null },
      { executable: this.executable, cacheDirectory: this.cacheDirectory, signal });
    signal?.throwIfAborted();
    const fit = report.stages.find(stage => stage.state === 'estimated_fit');
    const result = { ...report, download_allowed: fit?.passed === true, load_allowed: Boolean(artifact) && fit?.passed === true,
      metadata, settings, metadata_source: gguf ? 'local_gguf' : 'hub_metadata_estimate',
      rejection_reasons: fit?.reasons ?? ['Fit assessment unavailable.'],
      prompt_profile: gguf ? (typeof gguf.metadata['tokenizer.chat_template'] === 'string' && gguf.metadata['tokenizer.chat_template'].trim() ? 'model_template_available' : 'base_continuation') : 'inspect_after_download' };
    if (!this.assessments.has(candidate.candidate_id) && this.assessments.size >= 128) this.assessments.delete(this.assessments.keys().next().value);
    this.assessments.set(candidate.candidate_id, result);
    return result;
  }
  async rank(entries, signal) {
    signal?.throwIfAborted();
    return this.command('--rank-models', entries.map(({ assessment, evidence }) => ({ candidate: assessment.metadata, settings: assessment.settings, evidence: evidence ?? null })),
      { executable: this.executable, cacheDirectory: this.cacheDirectory, signal });
  }
}
