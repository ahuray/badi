import { createHash } from 'node:crypto';
import { open, readFile, mkdir, lstat, realpath } from 'node:fs/promises';
import { resolve, relative, join, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { parseArgs } from 'node:util';
import { createLabServer, validateConfigs } from './server.mjs';
import { LabWorker } from './worker.mjs';
import { validateSuite, planComparison, summarizeResults } from './cases.mjs';
import { modelArtifactPath, readModelArtifact, modelArtifactProvenance, modelArtifactArgs,
  assertModelArtifactUnchanged, assertModelArtifactIdentity } from './model-artifact.mjs';

export const ROOT = fileURLToPath(new URL('../../../', import.meta.url));
export const OUTPUT_ROOT = resolve(ROOT, 'output/writing');
export const WORKER_BINARY = resolve(ROOT, 'target/release/badi-writing-lab');
const SOURCE_PATHS = ['evaluation/writing/lab/run.mjs', 'evaluation/writing/lab/paced.mjs', 'evaluation/writing/lab/server.mjs',
  'evaluation/writing/lab/worker.mjs', 'evaluation/writing/lab/cases.mjs', 'evaluation/writing/lab/model-artifact.mjs',
  'evaluation/writing/lab/device-qualification.mjs', 'evaluation/writing/lab/gguf.mjs',
  'evaluation/writing/lab/discovery.mjs', 'evaluation/writing/lab/discovery-server.mjs',
  'evaluation/writing/lab/qualification-evidence.mjs', 'evaluation/writing/lab/qualification-diagnostics.mjs',
  'evaluation/writing/lab/qualification-server.mjs',
  'broker/src/model_selection/qualification.rs', 'broker/src/model_selection/qualification/device.rs',
  'broker/src/bin/badi-writing-lab.rs', 'broker/src/writing_lab.rs', 'broker/src/writing_lab/process.rs',
  'broker/src/writing_lab/prefill_probe.rs', 'broker/src/writing_lab/paced_probe.rs', 'broker/src/writing_lab/artifact.rs',
  'broker/src/writing_lab/attestation.rs',
  'broker/src/writing_lab/paced_probe/scheduler.rs', 'broker/src/writing.rs', 'broker/src/semantic/client.rs',
  'broker/src/semantic/client/writing_lab.rs', 'broker/src/semantic/client/prefill_probe.rs', 'broker/src/semantic/runtime.rs',
  'Cargo.toml', 'Cargo.lock', 'broker/Cargo.toml', 'broker/src/lib.rs', 'broker/src/provider.rs',
  'broker/src/segment.rs', 'broker/src/protocol.rs', 'broker/src/model_selection.rs',
  'broker/src/semantic/candidate.rs', 'broker/src/semantic/provenance.rs', 'broker/data/writing-lexicon/en.txt'];
const hash = value => createHash('sha256').update(value).digest('hex');
const jsonBytes = value => JSON.stringify(value, null, 2) + '\n';

export function parseOptions(args, cwd = process.cwd()) {
  const { values } = parseArgs({ args, strict: true, options: {
    suite: { type: 'string' }, output: { type: 'string' },
    modes: { type: 'string', default: 'production_baseline,context' },
    'budget-ms': { type: 'string', default: '550' }, 'max-tokens': { type: 'string', default: '8' },
    seed: { type: 'string', default: '42' }, 'cache-prompt': { type: 'string', default: 'true' },
    'model-artifact': { type: 'string' }, 'prefill-batch': { type: 'string' },
  } });
  if (!values.suite || !values.output) throw new Error('Supply --suite PATH and --output NEWDIR under output/writing.');
  const integer = (name, min, max) => {
    const value = values[name];
    if (!/^(0|[1-9][0-9]*)$/u.test(value) || Number(value) < min || Number(value) > max) throw new Error(`--${name} must be an integer from ${min} to ${max}.`);
    return Number(value);
  };
  const budget = integer('budget-ms', 550, 10000);
  const tokens = integer('max-tokens', 8, 64);
  const seed = integer('seed', 0, 2147483647);
  if (!['true', 'false'].includes(values['cache-prompt'])) throw new Error('--cache-prompt must be true or false.');
  if (values['prefill-batch'] !== undefined && !['16', '64'].includes(values['prefill-batch'])) throw new Error('Use --prefill-batch 16 or 64.');
  const configs = validateConfigs(values.modes.split(',').map(mode => {
    const fixed = ['production_baseline', 'production_boundary'].includes(mode);
    return { id: mode, mode, budget_ms: fixed ? 550 : budget, max_tokens: fixed ? 8 : tokens,
      cache_prompt: fixed || values['cache-prompt'] === 'true', temperature: 0, seed: 42 };
  }));
  return { suitePath: resolve(cwd, values.suite), outputPath: resolve(cwd, values.output), configs, seed,
    ...(values['prefill-batch'] !== undefined ? { prefillBatch: Number(values['prefill-batch']) } : {}),
    ...(values['model-artifact'] !== undefined ? { modelArtifactPath: modelArtifactPath(values['model-artifact']) } : {}) };
}

export function validateOutputPath(path, outputRoot = OUTPUT_ROOT) {
  const root = resolve(outputRoot);
  const destination = resolve(path);
  if (!destination.startsWith(root + sep)) throw new Error('Write the new run directory under ignored output/writing/.');
  return destination;
}

export async function prepareOutput(path, outputRoot = OUTPUT_ROOT) {
  const destination = validateOutputPath(path, outputRoot);
  const root = resolve(outputRoot);
  await mkdir(root, { recursive: true, mode: 0o700 });
  if (await realpath(root) !== root) throw new Error('The writing output root must not redirect through a symlink.');
  const parts = relative(root, destination).split(sep);
  let parent = root;
  for (const part of parts.slice(0, -1)) {
    parent = join(parent, part);
    try { await mkdir(parent, { mode: 0o700 }); } catch (error) { if (error.code !== 'EEXIST') throw error; }
    const metadata = await lstat(parent);
    if (!metadata.isDirectory() || metadata.isSymbolicLink()) throw new Error('Run directories cannot redirect through symlinks.');
  }
  await mkdir(destination, { mode: 0o700 }); // Exclusive: never replace an earlier run.
  return destination;
}

export async function fileIdentity(path, cache = new Map()) {
  const handle = await open(path, 'r');
  try {
    const before = await handle.stat({ bigint: true });
    if (!before.isFile()) throw new Error('Provenance requires a regular file.');
    const key = [before.dev, before.ino, before.size, before.mtimeNs, before.ctimeNs].join(':');
    let digest = cache.get(key);
    if (!digest) {
      const hasher = createHash('sha256');
      for await (const chunk of handle.createReadStream({ autoClose: false })) hasher.update(chunk);
      digest = hasher.digest('hex');
    }
    const after = await handle.stat({ bigint: true });
    if ([after.dev, after.ino, after.size, after.mtimeNs, after.ctimeNs].join(':') !== key) throw new Error('A provenance file changed while it was being hashed.');
    cache.set(key, digest);
    return { path: await realpath(path), sha256: digest, bytes: Number(before.size) };
  } finally { await handle.close(); }
}

export async function sourceHashes(paths = SOURCE_PATHS, root = ROOT) {
  return Object.fromEntries(await Promise.all(paths.map(async path => [path, hash(await readFile(resolve(root, path)))])));
}

export function validateCleanup(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)
    || !Number.isInteger(value.process_id) || value.process_id <= 1 || value.process_id > 0xffffffff
    || typeof value.runtime_identity_sha256 !== 'string' || !/^[a-f0-9]{64}$/u.test(value.runtime_identity_sha256)
    || typeof value.challenge_completed !== 'boolean' || typeof value.reaped !== 'boolean'
    || (value.exit_code !== null && !Number.isInteger(value.exit_code))) throw new Error('Invalid runtime cleanup receipt.');
  return value;
}

export class ObservedWorker extends LabWorker {
  constructor({ artifact = null, ...options } = {}) {
    super({ ...options, args: options.args ?? [] });
    this.args = [...this.originalArgs, ...modelArtifactArgs(artifact)];
    this.artifact = artifact; this.executions = []; this.cleanups = []; this.fileCache = new Map();
  }
  async start() {
    try {
      await assertModelArtifactUnchanged(this.artifact);
      const identity = await super.start();
      const pid = this.session?.child.pid;
      if (pid && !this.executions.some(value => value.pid === pid)) {
        const stat = await readFile(`/proc/${pid}/stat`, 'utf8');
        const start = stat.slice(stat.lastIndexOf(')') + 2).split(' ')[19];
        this.executions.push({ pid, start_ticks: start, executable: await fileIdentity(`/proc/${pid}/exe`, this.fileCache), runtime_identity: identity });
      }
      // super.start has already captured runtime ownership for awaited cleanup.
      assertModelArtifactIdentity(identity, this.artifact);
      return identity;
    } catch (error) {
      await this.stop();
      throw error;
    }
  }
  async stop() {
    const pid = this.session?.child.pid;
    try {
      const receipt = await super.stop();
      if (pid) this.cleanups.push({ worker_pid: pid, completed: true, runtime_receipt: receipt ?? null });
      return receipt;
    } catch (error) {
      if (pid) this.cleanups.push({ worker_pid: pid, completed: false, error: error.message });
      throw error;
    }
  }
}

export async function* decodeEvents(body) {
  if (!body) throw new Error('The lab returned no event stream.');
  const reader = body.pipeThrough(new TextDecoderStream()).getReader();
  let pending = '';
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      pending += value;
      if (Buffer.byteLength(pending) > 2 * 1024 * 1024) throw new Error('The lab event exceeded its bound.');
      for (;;) {
        const index = pending.indexOf('\n');
        if (index < 0) break;
        const line = pending.slice(0, index); pending = pending.slice(index + 1);
        if (!line) continue;
        const event = JSON.parse(line);
        if (!event || typeof event !== 'object' || Array.isArray(event)) throw new Error('Invalid lab event.');
        yield event;
      }
    }
    if (pending.trim()) throw new Error('The lab stream ended with an incomplete event.');
  } finally { await reader.cancel().catch(() => {}); reader.releaseLock(); }
}

export async function runSuite(options, { worker: suppliedWorker, outputRoot = OUTPUT_ROOT, signal,
  onProgress = () => {}, serverFactory = createLabServer } = {}) {
  signal?.throwIfAborted();
  const suiteBytes = await readFile(options.suitePath);
  if (suiteBytes.length > 512 * 1024) throw new Error('Suite input exceeds 512 KiB.');
  let document;
  try { document = JSON.parse(suiteBytes.toString('utf8')); }
  catch { throw new Error('Suite input must contain valid JSON.'); }
  if (!document || typeof document !== 'object' || Array.isArray(document)) throw new Error('Suite input must be an object.');
  const suite = validateSuite(document.schema === 'badi.prediction-lab.session.v1' ? document.suite : document);
  if (suite.cases.length > 300) throw new Error('Run at most 300 cases at a time.');
  const configs = validateConfigs(options.configs);
  const plan = planComparison(suite.cases, configs, options.seed);
  const body = JSON.stringify({ suite, configs, seed: options.seed });
  if (Buffer.byteLength(body) > 512 * 1024) throw new Error('The normalized lab request exceeds 512 KiB.');
  const artifact = options.modelArtifactPath ? await readModelArtifact(options.modelArtifactPath) : null;
  const output = await prepareOutput(options.outputPath, outputRoot);
  const save = (name, value) => open(resolve(output, name), 'wx', 0o600).then(async file => {
    try { await file.writeFile(typeof value === 'string' || Buffer.isBuffer(value) ? value : jsonBytes(value)); }
    finally { await file.close(); }
  });
  const beforeSources = await sourceHashes();
  const worker = suppliedWorker ?? new ObservedWorker({ artifact, prefillBatch: options.prefillBatch ?? 16 });
  const declaredWorker = suppliedWorker ? null : await fileIdentity(WORKER_BINARY);
  const started = new Date().toISOString();
  const initial = { schema: 'badi.prediction-lab.cli-run.v1', started_at: started,
    input: { path: await realpath(options.suitePath), sha256: hash(suiteBytes), bytes: suiteBytes.length },
    normalized_suite_sha256: hash(jsonBytes(suite)), configs, order_seed: options.seed, plan,
    provenance: { execution_kind: suppliedWorker ? 'injected_fixture' : 'local_model_worker', node: process.version,
      ...(options.prefillBatch !== undefined ? { requested_prefill_batch: options.prefillBatch } : {}),
      declared_worker: declaredWorker, source_sha256: beforeSources, model_artifact: modelArtifactProvenance(artifact) },
    evidence: 'Explicit development inputs; expected outputs are scoring-only append strings. This run does not qualify applications, human usefulness or Cotypist parity.' };
  await save('suite-input.json', suiteBytes);
  if (artifact) await save('model-artifact-input.json', artifact.source);
  await save('run.json', initial);
  const eventFile = await open(resolve(output, 'events.ndjson'), 'wx', 0o600);
  const session = { schema: 'badi.prediction-lab.session.v1', suite, configs, seed: options.seed, records: [], complete: false };
  let lab; let failure = null; let cleanupError = null; let ended = null; let receivedPlan = false;
  try {
    lab = await serverFactory({ worker });
    const page = await fetch(lab.origin, { signal });
    if (!page.ok) throw new Error('Cannot open the owned lab page.');
    const token = (await page.text()).match(/name="badi-lab-token" content="([a-f0-9]{64})"/u)?.[1];
    if (!token) throw new Error('The owned lab page did not supply a session capability.');
    const response = await fetch(`${lab.origin}/api/run`, { method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Badi-Lab-Token': token }, body, signal });
    if (!response.ok) throw new Error(`The lab rejected the run (HTTP ${response.status}).`);
    for await (const event of decodeEvents(response.body)) {
      await eventFile.write(JSON.stringify(event) + '\n');
      if (ended) throw new Error('The lab emitted another event after its terminal outcome.');
      if (event.type === 'plan') {
        if (receivedPlan || JSON.stringify(event.plan) !== JSON.stringify(plan) || JSON.stringify(event.suite) !== JSON.stringify(suite)
          || JSON.stringify(event.configs) !== JSON.stringify(configs)) throw new Error('The lab plan differs from the frozen run inputs.');
        receivedPlan = true; session.plan = event;
      } else if (event.type === 'progress') {
        if (!receivedPlan) throw new Error('The lab emitted progress before its plan.');
      } else if (event.type === 'record') {
        const item = plan[session.records.length];
        const record = event.record;
        if (!receivedPlan || !item || JSON.stringify(record.case) !== JSON.stringify(suite.cases.find(value => value.id === item.case_id))
          || JSON.stringify(record.config) !== JSON.stringify(configs.find(value => value.id === item.config_id))) throw new Error('The lab result differs from its scheduled case or configuration.');
        await assertModelArtifactUnchanged(artifact);
        assertModelArtifactIdentity(record.result?.identity, artifact);
        session.records.push(record);
        onProgress({ completed: session.records.length, total: plan.length, case_id: item.case_id, config_id: item.config_id });
      } else if (event.type === 'cleanup') {
        if (!receivedPlan || session.cleanup) throw new Error('Cleanup must occur after the plan and before final cleanup.');
        const receipt = validateCleanup(event.cleanup);
        if (event.group !== undefined) (session.group_cleanups ??= []).push({ group: event.group, cleanup: receipt });
        else session.cleanup = receipt;
        if (!receipt.reaped) cleanupError = 'The owned runtime was not confirmed reaped.';
      } else if (event.type === 'qualification_plan') {
        session.qualification_plan = event;
      } else if (event.type === 'qualification') {
        session.qualification = event;
      } else if (['complete', 'error', 'cancelled'].includes(event.type)) {
        ended = event.type;
        if (event.type === 'complete') {
          if (!receivedPlan || session.records.length !== plan.length) throw new Error('The lab completed without all scheduled results.');
          session.summaries = event.summaries;
        } else failure = event.error ?? `The lab ended with ${event.type}.`;
      } else throw new Error('Unknown lab event.');
    }
    if (!ended) throw new Error('The lab stream ended without a terminal outcome.');
  } catch (error) { failure = signal?.aborted ? 'Run cancelled.' : error.message; }
  finally {
    try { if (lab) await lab.close(); else await worker.stop(); } catch (error) { cleanupError = error.message; }
    await eventFile.close();
  }
  let afterSources = null;
  try { afterSources = await sourceHashes(); } catch { failure ??= 'Could not capture final source provenance.'; }
  let afterArtifact = null;
  try { if (artifact) afterArtifact = await readModelArtifact(artifact.path); }
  catch { failure ??= 'Could not capture final model artifact provenance.'; }
  const artifactStable = !artifact || afterArtifact?.sha256 === artifact.sha256;
  if (!artifactStable) failure ??= 'The frozen Lab model artifact descriptor changed.';
  const executions = worker.executions ?? [];
  const sourceStable = JSON.stringify(beforeSources) === JSON.stringify(afterSources);
  const executedHashes = [...new Set(executions.map(value => value.executable.sha256))];
  session.complete = ended === 'complete' && !failure && !cleanupError;
  const modelIdentityMatches = executions.every(value => {
    try { assertModelArtifactIdentity(value.runtime_identity, artifact); return true; } catch { return false; }
  });
  const report = { schema: 'badi.prediction-lab.cli-report.v1', started_at: started, finished_at: new Date().toISOString(),
    input: initial.input, normalized_suite_sha256: initial.normalized_suite_sha256,
    complete: session.complete, cancelled: Boolean(signal?.aborted || ended === 'cancelled'), error: failure,
    cleanup: { completed: !cleanupError, error: cleanupError, server_receipt: session.cleanup ?? null, group_receipts: session.group_cleanups ?? [], worker_receipts: worker.cleanups ?? [] },
    completed_requests: session.records.length, scheduled_requests: plan.length,
    provenance: { ...initial.provenance, executed_workers: executions, source_sha256_after: afterSources, source_stable: sourceStable,
      model_artifact_after: modelArtifactProvenance(afterArtifact), model_artifact_stable: artifactStable,
      valid_executed_identity: !suppliedWorker && executions.length > 0 && executedHashes.length === 1
        && executedHashes[0] === declaredWorker?.sha256 && sourceStable && artifactStable && modelIdentityMatches },
    measurements: summarizeResults(session.records), limitation: initial.evidence };
  await save('session.json', session);
  await save('report.json', report);
  return { output, session, report };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const cancellation = new AbortController();
  const cancel = () => cancellation.abort();
  process.once('SIGINT', cancel); process.once('SIGTERM', cancel);
  try {
    const result = await runSuite(parseOptions(process.argv.slice(2)), { signal: cancellation.signal,
      onProgress: progress => process.stdout.write(`${progress.completed}/${progress.total} ${progress.case_id} ${progress.config_id}\n`) });
    process.stdout.write(JSON.stringify({ output: result.output, complete: result.report.complete,
      requests: result.report.completed_requests, valid_executed_identity: result.report.provenance.valid_executed_identity }) + '\n');
    if (!result.report.complete || !result.report.provenance.valid_executed_identity) process.exitCode = 1;
  } catch (error) { process.stderr.write(`Prediction Lab could not run: ${error.message}\n`); process.exitCode = 1; }
  finally { process.off('SIGINT', cancel); process.off('SIGTERM', cancel); }
}
