import { constants } from 'node:fs';
import { mkdir, open, realpath, lstat, rename, unlink, statfs } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { homedir } from 'node:os';
import { createHash, randomUUID } from 'node:crypto';
import { spawn } from 'node:child_process';
import { readModelArtifact } from './model-artifact.mjs';

// Supported public Hub APIs only. No inference endpoint, token, repository code,
// draft, expected answer, or user-selected URL is accepted by this service.
// https://huggingface.co/docs/hub/api
// https://huggingface.co/docs/huggingface_hub/package_reference/hf_api#huggingface_hub.HfApi.model_info
const HUB = 'https://huggingface.co';
const REVISION = /^[a-f0-9]{40}$/u;
const HASH = /^[a-f0-9]{64}$/u;
const REPO = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,95}\/[A-Za-z0-9][A-Za-z0-9_.-]{0,95}$/u;
const GiB = 1024 ** 3;
const fail = (code, message = code) => Object.assign(new Error(message), { code });
const bounded = (text, n = 256) => typeof text === 'string' && text.isWellFormed() && text.length <= n && !/[\p{Cc}]/u.test(text);
const strings = value => (Array.isArray(value) ? value : [value]).filter(value => bounded(value)).slice(0, 64);
const positive = value => Number.isSafeInteger(value) && value > 0 ? value : null;
const cancelled = signal => { if (signal?.aborted) throw fail('discovery_cancelled', 'Operation cancelled. Partial verified-address downloads can resume.'); };
const idFor = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
const hold = (map, key, value, max = 512) => { if (!map.has(key) && map.size >= max) map.delete(map.keys().next().value); map.set(key, value); return value; };
const safeFile = value => bounded(value, 512) && value.split('/').every(part => /^[A-Za-z0-9][A-Za-z0-9_.-]*$/u.test(part) && part !== '..');
const configFields = config => ({ ...Object.fromEntries(['model_type', 'hidden_size', 'num_hidden_layers', 'num_attention_heads', 'num_key_value_heads', 'head_dim', 'max_position_embeddings', 'vocab_size', 'torch_dtype', 'conv_L_cache', 'num_local_experts']
  .filter(key => bounded(config?.[key]) || positive(config?.[key]) || ['num_local_experts', 'head_dim'].includes(key) && Number.isSafeInteger(config?.[key])).map(key => [key, config[key]])),
...(Array.isArray(config?.layer_types) && config.layer_types.length <= 256 && config.layer_types.every(value => bounded(value, 64)) ? { layer_types: config.layer_types } : {}),
...(Array.isArray(config?.full_attn_idxs) && config.full_attn_idxs.length <= 256 && config.full_attn_idxs.every(value => Number.isSafeInteger(value) && value >= 0 && value < 256) ? { full_attn_idxs: config.full_attn_idxs } : {}),
...(typeof config?.tie_word_embeddings === 'boolean' ? { tie_word_embeddings: config.tie_word_embeddings } : {}) });

export function summarizeHubModel(raw) {
  if (!raw || !REPO.test(raw.id ?? raw.modelId ?? '')) throw fail('discovery_invalid_metadata', 'The Hub returned invalid model metadata.');
  const card = raw.cardData ?? {}; const config = raw.config ?? {}; const gguf = raw.gguf ?? {};
  const upstream = strings(card.base_model).filter(value => REPO.test(value));
  const tags = strings(raw.tags);
  for (const tag of tags) {
    const source = tag.replace(/^base_model:(?:quantized:)?/u, '');
    if (tag.startsWith('base_model:') && REPO.test(source) && !upstream.includes(source)) upstream.push(source);
  }
  const architectures = strings(config.architectures);
  return { repo: raw.id ?? raw.modelId, revision: REVISION.test(raw.sha ?? '') ? raw.sha : null,
    architecture: bounded(gguf.architecture) ? gguf.architecture : bounded(config.model_type) ? config.model_type : null,
    architectures, languages: strings(card.language), task: bounded(raw.pipeline_tag) ? raw.pipeline_tag : bounded(card.pipeline_tag) ? card.pipeline_tag : null,
    parameters: positive(gguf.total) ?? positive(raw.safetensors?.total),
    license: bounded(card.license) ? card.license : tags.find(value => value.startsWith('license:'))?.slice(8) ?? null,
    license_details: { name: bounded(card.license_name) ? card.license_name : null, link: bounded(card.license_link, 1024) ? card.license_link : null },
    private: typeof raw.private === 'boolean' ? raw.private : null, gated: raw.gated === false ? false : raw.gated === true || ['auto', 'manual'].includes(raw.gated) ? raw.gated : null,
    disabled: raw.disabled === true,
    context_length: positive(gguf.context_length) ?? positive(config.max_position_embeddings),
    tokenizer: { embedded_in_gguf: Boolean(raw.gguf), bos_token: bounded(gguf.bos_token) ? gguf.bos_token : null,
      eos_token: bounded(gguf.eos_token) ? gguf.eos_token : null,
      chat_template: typeof gguf.chat_template === 'string' && gguf.chat_template.length <= 16384 ? gguf.chat_template : null,
      requirement: 'Use and verify the downloaded GGUF tokenizer; Hub claims do not prove runtime compatibility.' },
    config: configFields(config),
    provenance: { kind: tags.includes('gguf') || raw.gguf ? 'quantized_conversion' : 'upstream_or_unclassified', upstream,
      upstream_revision: null, certainty: upstream.length ? 'Upstream model named; exact source revision and conversion reproducibility are unverified.' : 'Upstream source and conversion provenance are unknown.' },
    url: `${HUB}/${raw.id ?? raw.modelId}`, state: 'discovered' };
}

export function hubCandidates(raw) {
  const model = summarizeHubModel(raw);
  if (!model.revision) throw fail('discovery_unpinned', 'The Hub did not provide an immutable repository revision.');
  if (!Array.isArray(raw.siblings) || raw.siblings.length > 10000) throw fail('discovery_invalid_metadata');
  return raw.siblings.filter(file => safeFile(file.rfilename) && /\.gguf$/iu.test(file.rfilename)).slice(0, 128).map(file => {
    const bytes = positive(file.lfs?.size) ?? positive(file.size);
    const sha256 = HASH.test(file.lfs?.sha256 ?? '') ? file.lfs.sha256 : null;
    const reasons = [];
    if (model.private !== false || model.gated !== false || model.disabled) reasons.push('Public, ungated access is required.');
    if (!sha256 || !bytes || (file.size && file.size !== bytes)) reasons.push('Exact artifact bytes and LFS SHA-256 are unavailable or inconsistent.');
    if (/-\d{5}-of-\d{5}\.gguf$/iu.test(file.rfilename)) reasons.push('Split GGUF loading is not supported by this Lab artifact contract.');
    if (!model.license) reasons.push('The repository has no declared license.');
    if (bytes > 2 * GiB) reasons.push('This compact-model discovery lane is limited to 2 GiB per artifact.');
    const quantization = file.rfilename.match(/(?:^|[-_.])(IQ\d_[A-Z0-9_]+|Q\d(?:_[A-Z0-9]+)+|BF16|F16|F32)(?=[-.]|$)/iu)?.[1]?.toUpperCase() ?? null;
    return { ...model, candidate_id: idFor([model.repo, model.revision, file.rfilename, sha256, bytes]), file: file.rfilename, bytes, sha256,
      quantization, runtime: 'llama.cpp GGUF; actual architecture, tokenizer and load must be exercised locally',
      download_eligible: reasons.length === 0, rejection_reasons: reasons };
  });
}

function allowedURL(value, metadata = false) {
  const url = new URL(value);
  if (url.protocol !== 'https:' || url.username || url.password || url.port || url.hash
    || (metadata ? url.hostname !== 'huggingface.co' : !['huggingface.co', 'cdn-lfs.huggingface.co', 'cdn-lfs.hf.co', 'us.aws.cdn.hf.co'].includes(url.hostname)
      && !url.hostname.endsWith('.xethub.hf.co') && !url.hostname.endsWith('.huggingface.co'))) throw fail('discovery_invalid_redirect', 'The Hub returned an unsupported redirect.');
  return url;
}

async function responseBytes(response, max, signal) {
  const chunks = []; let size = 0;
  for await (const chunk of response.body) {
    cancelled(signal); size += chunk.length;
    if (size > max) throw fail('discovery_response_limit', 'Hub metadata exceeded the response limit.');
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}

async function secureDirectory(path) {
  await mkdir(path, { recursive: true, mode: 0o700 });
  const stat = await lstat(path);
  if (!stat.isDirectory() || stat.isSymbolicLink() || stat.uid !== process.getuid() || (stat.mode & 0o077) || await realpath(path) !== path) {
    throw fail('discovery_cache_permissions', 'The Lab cache must be a private owned directory without symbolic links.');
  }
}

async function privateFile(path, flags, mode = 0o600) {
  const handle = await open(path, flags | constants.O_NOFOLLOW | constants.O_NONBLOCK, mode);
  const stat = await handle.stat();
  if (!stat.isFile() || stat.uid !== process.getuid() || stat.nlink !== 1 || (stat.mode & 0o077)) {
    await handle.close(); throw fail('discovery_cache_permissions', 'Artifact cache files must be private, owned regular files.');
  }
  return handle;
}

async function verifiedFile(path, candidate, signal) {
  let handle;
  try { handle = await privateFile(path, constants.O_RDONLY); }
  catch (error) { if (error.code === 'ENOENT') return false; throw error; }
  try {
    const before = await handle.stat({ bigint: true });
    if (before.size !== BigInt(candidate.bytes)) return false;
    const digest = createHash('sha256'); const buffer = Buffer.alloc(1024 * 1024); let offset = 0;
    for (;;) {
      cancelled(signal);
      const { bytesRead } = await handle.read(buffer, 0, buffer.length, offset);
      if (!bytesRead) break;
      if (!offset && buffer.subarray(0, 4).toString() !== 'GGUF') return false;
      offset += bytesRead; digest.update(buffer.subarray(0, bytesRead));
    }
    const after = await handle.stat({ bigint: true });
    return offset === candidate.bytes && digest.digest('hex') === candidate.sha256
      && ['dev', 'ino', 'size', 'mtimeNs', 'ctimeNs'].every(key => before[key] === after[key]);
  } finally { await handle.close(); }
}

export class DiscoveryService {
  constructor({ fetchImpl = globalThis.fetch, cacheDir = resolve(process.env.XDG_CACHE_HOME || join(homedir(), '.cache'), 'badi', 'prediction-lab'),
    assessCandidate = async () => ({ download_allowed: false, rejection_reasons: ['Device assessment is unavailable.'] }),
    freeBytes = async path => { const fs = await statfs(path, { bigint: true }); return Number(fs.bavail * fs.bsize); },
    now = () => Date.now(), cacheMs = 5 * 60 * 1000, requestTimeoutMs = 15000 } = {}) {
    this.fetch = fetchImpl; this.cacheDir = resolve(cacheDir); this.assessCandidate = assessCandidate; this.freeBytes = freeBytes;
    this.now = now; this.cacheMs = cacheMs; this.requestTimeoutMs = requestTimeoutMs;
    this.cache = new Map(); this.models = new Map(); this.candidates = new Map(); this.cursors = new Map(); this.artifacts = new Map();
    this.progress = null;
  }

  async request(url, { signal, metadata = true, headers = {} } = {}) {
    let target = allowedURL(url, metadata);
    for (let redirects = 0; redirects <= 5; redirects++) {
      cancelled(signal);
      const response = await this.fetch(target, { signal, redirect: 'manual', credentials: 'omit',
        headers: { Accept: metadata ? 'application/json' : 'application/octet-stream', 'Accept-Encoding': 'identity', ...headers } });
      if (![301, 302, 303, 307, 308].includes(response.status)) return response;
      await response.body?.cancel();
      const location = response.headers.get('location');
      if (!location) throw fail('discovery_invalid_redirect');
      target = allowedURL(new URL(location, target), metadata);
    }
    throw fail('discovery_invalid_redirect', 'Too many Hub redirects.');
  }

  async metadata(url, signal, { asText = false, maxBytes = 2 * 1024 * 1024 } = {}) {
    const cached = this.cache.get(url);
    cancelled(signal);
    if (cached && this.now() - cached.at < this.cacheMs) return { ...cached, cache_state: 'cached' };
    const deadline = AbortSignal.timeout(this.requestTimeoutMs);
    const combined = signal ? AbortSignal.any([signal, deadline]) : deadline;
    try {
      const response = await this.request(url, { signal: combined });
      if (!response.ok) { await response.body?.cancel(); throw fail(response.status === 429 ? 'discovery_rate_limited' : 'discovery_hub_unavailable', `The public Hub returned HTTP ${response.status}.`); }
      const bytes = await responseBytes(response, maxBytes, combined);
      let value;
      try { const text = new TextDecoder('utf-8', { fatal: true }).decode(bytes); value = asText ? text : JSON.parse(text); }
      catch { throw fail('discovery_invalid_metadata', 'The Hub response is not valid bounded UTF-8 metadata.'); }
      const entry = { value, at: this.now(), link: response.headers.get('link'), bytes_sha256: createHash('sha256').update(bytes).digest('hex') };
      hold(this.cache, url, entry, 64); return { ...entry, cache_state: 'fresh' };
    } catch (error) {
      cancelled(signal);
      if (cached && this.now() - cached.at <= 24 * 60 * 60 * 1000) return { ...cached, cache_state: 'stale', warning: 'The Hub is unavailable. These cached results may be outdated; downloads require a fresh pinned check.' };
      if (error.code?.startsWith('discovery_')) throw error;
      throw fail('discovery_offline', 'Cannot reach the public Hugging Face Hub. Check your connection and try again.');
    }
  }

  async search({ query, cursor }, signal) {
    if (!bounded(query, 120) || !query.trim() || cursor !== undefined && !this.cursors.has(cursor)) throw fail('discovery_invalid_input', 'Enter a model name or family (up to 120 characters).');
    const previous = cursor ? this.cursors.get(cursor) : null;
    if (previous && (previous.query !== query || previous.page > 3)) throw fail('discovery_invalid_input');
    const page = previous?.page ?? 1;
    const url = previous?.url ?? `${HUB}/api/models?${new URLSearchParams({ search: query, limit: '12', full: 'true', config: 'true', cardData: 'true' })}`;
    const data = await this.metadata(url, signal);
    if (!Array.isArray(data.value) || data.value.length > 100) throw fail('discovery_invalid_metadata');
    const models = data.value.slice(0, 12).map(raw => {
      const model = summarizeHubModel(raw); const model_id = idFor([model.repo, model.revision]);
      hold(this.models, model_id, model); return { ...model, model_id };
    });
    let next_cursor = null;
    const next = data.link?.match(/<([^>]+)>;\s*rel="next"/u)?.[1];
    if (next && page < 3) {
      const nextURL = allowedURL(next, true);
      if (nextURL.pathname !== '/api/models') throw fail('discovery_invalid_redirect');
      next_cursor = idFor([query, page + 1, next]);
      hold(this.cursors, next_cursor, { url: nextURL.href, page: page + 1, query }, 64);
    }
    return { schema: 'badi.discovery.search.v1', models, next_cursor, page, page_limit: 3, cache_state: data.cache_state, fetched_at: new Date(data.at).toISOString(), warning: data.warning ?? null };
  }

  async inspect({ model_id }, signal) {
    const model = this.models.get(model_id);
    if (!model) throw fail('discovery_unknown_model', 'Search again to select a current server-issued model.');
    const url = `${HUB}/api/models/${model.repo}${model.revision ? `/revision/${model.revision}` : ''}?blobs=true`;
    const data = await this.metadata(url, signal);
    if ((data.value.id ?? data.value.modelId) !== model.repo || model.revision && data.value.sha !== model.revision) throw fail('discovery_identity_changed');
    const pinned = structuredClone(data.value); let modelMetadata = summarizeHubModel(pinned);
    let provenanceSource = null;
    if (!modelMetadata.provenance.upstream.length && pinned.siblings?.some(file => file.rfilename === 'README.md')) {
      try {
        const card = await this.metadata(`${HUB}/${model.repo}/resolve/${pinned.sha}/README.md`, signal, { asText: true, maxBytes: 64 * 1024 });
        // This common converter declaration names a model; arbitrary links and
        // commands elsewhere in a card never select a source or execute code.
        const named = card.value.match(/(?:quantized|quantised|GGUF) version of \[[^\]\r\n]{1,192}\]\(https:\/\/huggingface\.co\/([A-Za-z0-9][A-Za-z0-9_.-]{0,95}\/[A-Za-z0-9][A-Za-z0-9_.-]{0,95})\)/iu)?.[1];
        if (named && REPO.test(named) && named !== model.repo) {
          pinned.cardData = { ...pinned.cardData, base_model: named };
          modelMetadata = summarizeHubModel(pinned);
          provenanceSource = { file: 'README.md', revision: pinned.sha, sha256: card.bytes_sha256,
            evidence: 'Converter card names a quantized version of this upstream; this is an unverified repository claim.' };
        }
      } catch { cancelled(signal); }
    }
    let configSource = null; let configWarning = null;
    // Fetch JSON data only, never a tokenizer loader or repository script. A
    // converter's named upstream does not attest which source revision it used.
    try {
      let sourceRepo = model.repo; let sourceRevision = pinned.sha;
      const ownConfig = pinned.siblings?.some(file => file.rfilename === 'config.json');
      if (!ownConfig && modelMetadata.provenance.upstream.length) {
        sourceRepo = modelMetadata.provenance.upstream[0];
        const sourceInfo = await this.metadata(`${HUB}/api/models/${sourceRepo}`, signal);
        if ((sourceInfo.value.id ?? sourceInfo.value.modelId) !== sourceRepo || !REVISION.test(sourceInfo.value.sha ?? '')) throw fail('discovery_unpinned');
        sourceRevision = sourceInfo.value.sha;
      }
      if (ownConfig || sourceRepo !== model.repo) {
        const config = await this.metadata(`${HUB}/${sourceRepo}/resolve/${sourceRevision}/config.json`, signal);
        if (!config.value || typeof config.value !== 'object' || Array.isArray(config.value)) throw fail('discovery_invalid_metadata');
        pinned.config = { ...pinned.config, ...configFields(config.value) };
        configSource = { repo: sourceRepo, revision: sourceRevision, file: 'config.json', cache_state: config.cache_state,
          relation: ownConfig ? 'same_artifact_repository' : 'named_upstream_current_revision_for_estimation_only',
          sha256: config.bytes_sha256, hash_scope: 'Exact downloaded config.json response bytes' };
      }
    } catch (error) { cancelled(signal); configWarning = 'Architecture dimensions could not be read; fit must remain conservative or unavailable.'; }
    const candidates = hubCandidates(pinned).map(candidate => ({ ...candidate, config_source: configSource,
      provenance: { ...candidate.provenance, declaration: provenanceSource,
        publisher_relation: candidate.provenance.upstream.length ? candidate.provenance.upstream[0].split('/')[0] === model.repo.split('/')[0] ? 'same_publisher' : 'different_publisher' : 'unknown' },
      metadata_warnings: configWarning ? [configWarning] : [] }));
    for (const candidate of candidates) hold(this.candidates, candidate.candidate_id, candidate);
    return { schema: 'badi.discovery.inspect.v1', model: { ...summarizeHubModel(pinned), config_source: configSource }, candidates,
      cache_state: data.cache_state, fetched_at: new Date(data.at).toISOString(), warning: data.warning ?? configWarning ?? (candidates.length ? null : 'No supported single-file GGUF artifact was found. Search for a conversion and inspect its provenance.') };
  }

  candidate(id) {
    const candidate = this.candidates.get(id);
    if (!candidate) throw fail('discovery_unknown_candidate', 'Inspect a model again to select a current server-issued artifact.');
    return candidate;
  }

  async assess({ candidate_id }, signal) {
    const candidate = this.candidate(candidate_id); cancelled(signal);
    const assessment = await this.assessCandidate(candidate, signal); cancelled(signal);
    return { schema: 'badi.discovery.assessment.v1', candidate, assessment,
      download_allowed: candidate.download_eligible && assessment?.download_allowed === true };
  }

  async download({ candidate_id }, signal) {
    const assessed = await this.assess({ candidate_id }, signal); const candidate = assessed.candidate;
    if (!assessed.download_allowed) throw fail('discovery_fit_rejected', 'This artifact did not pass the current compatibility and resource gates. Inspect its assessment.');
    await secureDirectory(this.cacheDir);
    const weights = join(this.cacheDir, `${candidate.sha256}.gguf`);
    const part = weights + '.part'; const lockPath = weights + '.lock';
    const lock = await privateFile(lockPath, constants.O_RDWR | constants.O_CREAT);
    try {
      // Linux flock attaches to this inherited open-file description. The
      // parent keeps it open; process death releases it automatically without
      // stale PID files or a race that unlinks another downloader's lock.
      await lockFile(lock.fd);
      this.progress = { candidate_id, state: 'verifying_cache', received_bytes: 0, total_bytes: candidate.bytes };
      let reused = await verifiedFile(weights, candidate, signal);
      if (!reused) {
        // Require current exact revision/size/hash metadata before contacting the
        // CDN. An offline/stale metadata record never authorizes new downloads.
        const url = `${HUB}/api/models/${candidate.repo}/revision/${candidate.revision}?blobs=true`;
        this.cache.delete(url);
        const metadata = await this.metadata(url, signal);
        const fresh = hubCandidates(metadata.value).find(value => value.candidate_id === candidate_id);
        if (metadata.cache_state === 'stale' || !fresh?.download_eligible) throw fail('discovery_identity_changed', 'The pinned Hub artifact identity or access metadata changed.');
        await this.downloadBytes(candidate, weights, part, signal);
        if (!await verifiedFile(part, candidate, signal)) { await unlink(part); throw fail('discovery_hash_mismatch', 'Artifact bytes, GGUF header or SHA-256 do not match the pinned Hub metadata.'); }
        cancelled(signal); await rename(part, weights);
      }
      cancelled(signal);
      const alias = `${candidate.repo.split('/').at(-1).slice(0, 55)}-${candidate.quantization ?? 'GGUF'}-${candidate.sha256.slice(0, 8)}`.replace(/[^A-Za-z0-9._-]/gu, '_').slice(0, 96);
      const descriptor = { schema: 'badi.lab-model-artifact.v1', alias, weights_path: weights, sha256: candidate.sha256, bytes: candidate.bytes };
      const path = join(this.cacheDir, `${candidate.sha256}.json`);
      await atomicJSON(path, descriptor);
      await atomicJSON(join(this.cacheDir, `${candidate.sha256}.provenance.json`), { schema: 'badi.discovery.provenance.v1', candidate, verified_at: new Date(this.now()).toISOString() });
      const artifact = await readModelArtifact(path);
      const artifact_id = idFor([candidate_id, artifact.sha256]);
      hold(this.artifacts, artifact_id, { artifact, candidate });
      this.progress = { candidate_id, state: 'verified', received_bytes: candidate.bytes, total_bytes: candidate.bytes };
      return { schema: 'badi.discovery.download.v1', artifact_id, candidate, reused, verified: true, state: 'downloaded_verified', assessment: assessed.assessment };
    } catch (error) {
      if (this.progress?.candidate_id === candidate_id) this.progress.state = signal?.aborted ? 'cancelled' : 'interrupted';
      throw error;
    } finally { await lock.close(); }
  }

  async downloadBytes(candidate, weights, part, signal) {
    let handle = await privateFile(part, constants.O_RDWR | constants.O_CREAT);
    try {
      let offset = Number((await handle.stat()).size);
      if (offset > candidate.bytes) { await handle.truncate(0); offset = 0; }
      if (offset === candidate.bytes) return;
      const room = async () => { if (await this.freeBytes(this.cacheDir) < candidate.bytes - offset + 256 * 1024 ** 2) throw fail('discovery_disk_full', 'Insufficient free disk space for the remaining artifact plus 256 MiB headroom.'); };
      await room();
      const deadline = AbortSignal.timeout(20 * 60 * 1000);
      const controller = new AbortController();
      const combined = AbortSignal.any([...(signal ? [signal] : []), deadline, controller.signal]);
      let timer; let response;
      const armTimeout = () => { clearTimeout(timer); timer = setTimeout(() => controller.abort(), 30000); timer.unref?.(); };
      armTimeout();
      try {
        const url = `${HUB}/${candidate.repo}/resolve/${candidate.revision}/${candidate.file.split('/').map(encodeURIComponent).join('/')}?download=true`;
        response = await this.request(url, { signal: combined, metadata: false, headers: offset ? { Range: `bytes=${offset}-` } : {} });
        const range = response.headers.get('content-range');
        if (response.status === 206 && range !== `bytes ${offset}-${candidate.bytes - 1}/${candidate.bytes}`) {
          await response.body?.cancel(); throw fail('discovery_invalid_range', 'The server returned an unexpected download range.');
        }
        if (response.status === 200 && offset) { await handle.truncate(0); offset = 0; await room(); }
        if (![200, 206].includes(response.status)) { await response.body?.cancel(); throw fail('discovery_download_failed', `The pinned artifact returned HTTP ${response.status}. Retry to resume.`); }
        const expected = candidate.bytes - offset;
        if (response.headers.has('content-length') && Number(response.headers.get('content-length')) !== expected) { await response.body?.cancel(); throw fail('discovery_size_mismatch'); }
        if (response.headers.has('content-encoding') && response.headers.get('content-encoding') !== 'identity') { await response.body?.cancel(); throw fail('discovery_size_mismatch'); }
        this.progress = { candidate_id: candidate.candidate_id, state: 'downloading', received_bytes: offset, total_bytes: candidate.bytes };
        let checkedAt = offset;
        for await (const chunk of response.body) {
          cancelled(signal); armTimeout();
          if (offset + chunk.length > candidate.bytes) throw fail('discovery_size_mismatch', 'Download exceeded its pinned byte size.');
          let wrote = 0;
          while (wrote < chunk.length) { const result = await handle.write(chunk, wrote, chunk.length - wrote, offset + wrote); if (!result.bytesWritten) throw fail('discovery_download_failed'); wrote += result.bytesWritten; }
          offset += chunk.length; this.progress.received_bytes = offset;
          if (offset - checkedAt >= 64 * 1024 ** 2) { await room(); checkedAt = offset; }
        }
        cancelled(signal);
        if (offset !== candidate.bytes) throw fail('discovery_incomplete', 'Download was interrupted. Retry to resume the verified revision.');
        await handle.sync();
      } catch (error) {
        cancelled(signal);
        if (error.code?.startsWith('discovery_')) throw error;
        throw fail('discovery_download_failed', 'The download stopped or timed out. Retry to resume the pinned artifact.');
      } finally {
        clearTimeout(timer); controller.abort();
        // Errors before iteration (including a failed disk recheck after the
        // CDN ignored Range) must close the response body as well.
        if (response?.body && !response.body.locked) await response.body.cancel().catch(() => {});
      }
    } finally { await handle.close(); }
  }

  async selection(artifact_id, signal) {
    const selected = this.artifacts.get(artifact_id);
    if (!selected) throw fail('discovery_unknown_artifact', 'Download and verify an artifact in this server session before selecting it.');
    const assessment = await this.assess({ candidate_id: selected.candidate.candidate_id }, signal);
    if (!assessment.download_allowed) throw fail('discovery_fit_rejected', 'Current resource assessment no longer allows this model.');
    if (!await verifiedFile(selected.artifact.descriptor.weights_path, selected.candidate, signal)) throw fail('discovery_hash_mismatch', 'The selected artifact changed; download and verify it again.');
    const artifact = await readModelArtifact(selected.artifact.path);
    if (artifact.sha256 !== selected.artifact.sha256) throw fail('discovery_identity_changed', 'The selected descriptor changed.');
    return { artifact, candidate: selected.candidate, artifact_id, assessment: assessment.assessment };
  }
}

async function lockFile(fd) {
  return new Promise((resolveLock, reject) => {
    const child = spawn('flock', ['--nonblock', '3'], { stdio: ['ignore', 'ignore', 'ignore', fd], timeout: 2000 });
    child.once('error', () => reject(fail('discovery_lock_unavailable', 'Linux util-linux flock is required for safe resumable downloads.')));
    child.once('close', code => code === 0 ? resolveLock() : reject(fail('discovery_artifact_busy', 'Another downloader owns this artifact, or its lock could not be acquired.')));
  });
}
async function atomicJSON(path, data) {
  const temporary = `${path}.${randomUUID()}.tmp`; const file = await privateFile(temporary, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL);
  try { await file.writeFile(JSON.stringify(data, null, 2) + '\n'); await file.sync(); }
  catch (error) { await unlink(temporary); throw error; }
  finally { await file.close(); }
  try { await rename(temporary, path); } catch (error) { await unlink(temporary); throw error; }
}
