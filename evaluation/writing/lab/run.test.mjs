import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdtemp, writeFile, readFile, readdir, stat, rm, mkdir, symlink, copyFile, chmod } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { parseOptions, validateOutputPath, prepareOutput, fileIdentity, sourceHashes, labCrateSources, brokerSources, buildWorker, WORKER_BUILD, validateCleanup, decodeEvents, runSuite, runRevisionSuite, ObservedWorker } from './run.mjs';
import { createLabServer } from './server.mjs';
import { readModelArtifact } from './model-artifact.mjs';

const hash = value => createHash('sha256').update(value).digest('hex');
const input = { schema: 'badi.prediction-suite.v1', name: 'CLI fixture', cases: [{ id: 'append-case', language: 'en',
  prefix: 'PRIVATE_PREFIX docum', context: 'PRIVATE_CONTEXT', style: 'PRIVATE_STYLE', expected: ['ent'] }] };
const config = { id: 'context', mode: 'context', budget_ms: 550, max_tokens: 8, cache_prompt: true, temperature: 0, seed: 42 };
const cleanupReceipt = { process_id: 42, runtime_identity_sha256: 'a'.repeat(64), challenge_completed: true, reaped: true, exit_code: 0 };

async function artifactFixture(directory) {
  const weights_path = join(directory, 'fixture.gguf'), path = join(directory, 'artifact.json');
  await writeFile(weights_path, 'fixture');
  await writeFile(path, JSON.stringify({ schema: 'badi.lab-model-artifact.v1', weights_path,
    sha256: hash('fixture'), bytes: 7, alias: 'fixture-artifact' }) + '\n');
  return readModelArtifact(path);
}

const artifactIdentity = artifact => ({ model_origin: 'explicit_lab_artifact', model_sha256: artifact.descriptor.sha256,
  model_size: artifact.descriptor.bytes, model_alias: artifact.descriptor.alias });

async function fixture(t, document = input) {
  const directory = await mkdtemp(join(tmpdir(), 'badi-lab-run-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const suitePath = join(directory, 'suite.json');
  const bytes = JSON.stringify(document) + '\n';
  await writeFile(suitePath, bytes);
  const outputRoot = join(directory, 'output/writing');
  const options = { suitePath, outputPath: join(outputRoot, 'result'), configs: [config], seed: 42 };
  return { directory, suitePath, outputRoot, options, bytes };
}

class FakeWorker {
  constructor(behavior = async () => ({ outcome: 'suggestion', text: 'ent', word_complete: true, latency_ms: 4 })) {
    this.behavior = behavior; this.calls = []; this.stopped = 0; this.session = null;
  }
  async predict(request, signal) {
    this.calls.push(request); this.session = { identity: { kind: 'fixture' } };
    return this.behavior(request, signal);
  }
  async stop() { this.stopped++; const hadSession = Boolean(this.session); this.session = null; return hadSession ? { ...cleanupReceipt } : null; }
}

test('CLI parsing keeps generation seed fixed and baseline settings independent of experiment settings', () => {
  const parsed = parseOptions(['--suite', 'input.json', '--output', 'output/writing/new', '--modes', 'production_boundary,healed',
    '--budget-ms', '2500', '--max-tokens', '32', '--seed', '19', '--cache-prompt', 'false'], '/work/badi');
  assert.equal(parsed.suitePath, '/work/badi/input.json');
  assert.equal(parsed.outputPath, '/work/badi/output/writing/new');
  assert.equal(parsed.seed, 19);
  assert.equal(parsed.configs[0].budget_ms, 550);
  assert.equal(parsed.configs[0].max_tokens, 8);
  assert.equal(parsed.configs[0].cache_prompt, true);
  assert.equal(parsed.configs[1].budget_ms, 2500);
  assert.equal(parsed.configs[1].max_tokens, 32);
  assert.equal(parsed.configs[1].cache_prompt, false);
  assert.ok(parsed.configs.every(value => value.seed === 42 && value.temperature === 0));
  assert.equal(parseOptions(['--suite', 'input.json', '--output', 'output/writing/new', '--memory-map', 'false']).memoryMap, false);
  for (const extra of [['--seed', '-1'], ['--seed', '1.5'], ['--budget-ms', '549'], ['--max-tokens', '65'],
    ['--modes', 'unknown'], ['--modes', 'context,context'], ['--cache-prompt', 'yes'], ['--memory-map', '0'], ['--unknown', 'value']]) {
    assert.throws(() => parseOptions(['--suite', 'suite.json', '--output', 'out', ...extra]));
  }
  assert.throws(() => parseOptions([]));
});

test('CLI production mode keeps production settings despite experiment overrides', () => {
  const base = ['--suite', 'suite.json', '--output', 'output/writing/new'];
  const parsed = parseOptions([...base, '--modes', 'production_boundary,context,instructed,healed',
    '--budget-ms', '5000', '--max-tokens', '64', '--cache-prompt', 'false', '--seed', '123']);
  assert.equal(parsed.configs.length, 4);
  assert.deepEqual(parsed.configs.find(value => value.mode === 'production_boundary'), { id: 'production_boundary',
    mode: 'production_boundary', budget_ms: 550, max_tokens: 8, cache_prompt: true, temperature: 0, seed: 42 });
  assert.deepEqual(parsed.configs.find(value => value.mode === 'framed'), undefined);
  const framed = parseOptions([...base, '--modes', 'framed,field_context', '--budget-ms', '5000', '--max-tokens', '64', '--cache-prompt', 'false']);
  for (const mode of ['framed', 'field_context']) {
    assert.deepEqual(framed.configs.find(value => value.mode === mode), { id: mode, mode, budget_ms: 550, max_tokens: 8, cache_prompt: true, temperature: 0, seed: 42 });
  }
  assert.equal(parsed.seed, 123);
  assert.equal(parsed.warmPrefix, false);
  assert.equal(parseOptions([...base, '--warm-prefix', 'true']).warmPrefix, true);
  assert.throws(() => parseOptions([...base, '--warm-prefix', 'false']));
  assert.equal(parsed.configs.find(value => value.mode === 'context').budget_ms, 5000);
  assert.throws(() => parseOptions([...base, '--modes', 'production_boundary_unknown']));
  // The historical unhealed baseline is retired; production is the only baseline.
  assert.throws(() => parseOptions([...base, '--modes', 'production_baseline']));
});

test('each suggestion length is its own production configuration, normal keeping the plain request', () => {
  const base = ['--suite', 'suite.json', '--output', 'output/writing/new'];
  const production = { mode: 'production_boundary', budget_ms: 550, max_tokens: 8, cache_prompt: true, temperature: 0, seed: 42 };
  const parsed = parseOptions([...base, '--modes', 'production_boundary,healed', '--lengths', 'short,normal']);
  assert.deepEqual(parsed.configs.map(value => value.id), ['production_boundary-short', 'production_boundary', 'healed']);
  assert.deepEqual(parsed.configs[0], { ...production, id: 'production_boundary-short', length: 'short' });
  assert.deepEqual(parsed.configs[1], { ...production, id: 'production_boundary' });
  assert.equal(Object.hasOwn(parsed.configs[2], 'length'), false);
  assert.deepEqual(parseOptions([...base, '--modes', 'production_boundary']).configs, [{ ...production, id: 'production_boundary' }]);
  for (const lengths of ['long', 'medium', 'short,short', 'short,']) {
    assert.throws(() => parseOptions([...base, '--modes', 'production_boundary', '--lengths', lengths]), lengths);
  }
  assert.throws(() => parseOptions([...base, '--modes', 'healed', '--lengths', 'short']));
});

test('artifact CLI option is explicit and cannot pass arbitrary runtime flags', () => {
  const args = ['--suite', 'suite.json', '--output', 'output/writing/new'];
  assert.equal(Object.hasOwn(parseOptions(args), 'modelArtifactPath'), false);
  assert.equal(parseOptions([...args, '--model-artifact', '/private/model.json']).modelArtifactPath, '/private/model.json');
  for (const extra of [['--model-artifact', 'relative.json'], ['--model-artifact', '/tmp/../model.json'],
    ['--runtime-args', '--threads=16'], ['--model-artifact', '']]) assert.throws(() => parseOptions([...args, ...extra]));
});

test('prefill batch is an explicit bounded launch choice, separate from generation configs', () => {
  const args = ['--suite', 'suite.json', '--output', 'output/writing/new'];
  const defaults = parseOptions(args);
  assert.equal(Object.hasOwn(defaults, 'prefillBatch'), false);
  for (const batch of ['16', '64']) {
    const selected = parseOptions([...args, '--prefill-batch', batch]);
    assert.equal(selected.prefillBatch, Number(batch));
    assert.deepEqual(selected.configs, defaults.configs);
  }
  for (const batch of ['0', '32', '064', '64.0', '', '16,64']) {
    assert.throws(() => parseOptions([...args, '--prefill-batch', batch]), /prefill-batch/u);
  }
});

test('instructed healing is an explicit CLI experiment with unchanged defaults and bounds', () => {
  const base = ['--suite', 'suite.json', '--output', 'output/writing/new'];
  assert.deepEqual(parseOptions(base).configs.map(config => config.mode), ['production_boundary', 'context']);
  const selected = parseOptions([...base, '--modes', 'instructed_healed', '--budget-ms', '1500',
    '--max-tokens', '16', '--cache-prompt', 'false', '--seed', '73']);
  assert.deepEqual(selected.configs, [{ id: 'instructed_healed', mode: 'instructed_healed',
    budget_ms: 1500, max_tokens: 16, cache_prompt: false, temperature: 0, seed: 42 }]);
  assert.equal(selected.seed, 73);
  assert.throws(() => parseOptions([...base, '--modes', 'instructed_word']));
  assert.equal(parseOptions([...base, '--modes', 'context_confidence']).configs[0].mode, 'context_confidence');
  for (const extra of [['--modes', 'instructed_healed_typo'], ['--budget-ms', '549'], ['--max-tokens', '65']]) {
    assert.throws(() => parseOptions([...base, '--modes', 'instructed_healed', ...extra]));
  }
});

test('retired attested recovery cannot be selected', () => {
  const base = ['--suite', 'suite.json', '--output', 'output/writing/new'];
  assert.throws(() => parseOptions([...base, '--modes', 'healed_attested']));
  assert.throws(() => parseOptions([...base, '--modes', 'healed,healed_attested']));
});

test('new run directories stay under the output root, exclude symlink escapes and never overwrite', async t => {
  const f = await fixture(t);
  assert.equal(validateOutputPath(f.options.outputPath, f.outputRoot), f.options.outputPath);
  for (const path of [f.outputRoot, resolve(f.outputRoot, '../outside'), `${f.outputRoot}-elsewhere/new`]) {
    assert.throws(() => validateOutputPath(path, f.outputRoot));
  }
  await prepareOutput(f.options.outputPath, f.outputRoot);
  assert.equal((await stat(f.options.outputPath)).mode & 0o777, 0o700);
  await writeFile(join(f.options.outputPath, 'preserved'), 'previous result');
  await assert.rejects(prepareOutput(f.options.outputPath, f.outputRoot), /EEXIST/u);
  assert.equal(await readFile(join(f.options.outputPath, 'preserved'), 'utf8'), 'previous result');
  const outside = join(f.directory, 'outside'); await mkdir(outside);
  await symlink(outside, join(f.outputRoot, 'redirect'));
  await assert.rejects(prepareOutput(join(f.outputRoot, 'redirect/new'), f.outputRoot), /symlink/u);
  await assert.rejects(stat(join(outside, 'new')), /ENOENT/u);
});

test('file/source provenance hashes exact bytes and cached hashing notices content replacement', async t => {
  const f = await fixture(t);
  const cache = new Map();
  const first = await fileIdentity(f.suitePath, cache);
  assert.equal(first.sha256, hash(f.bytes));
  assert.equal(first.bytes, Buffer.byteLength(f.bytes));
  assert.deepEqual(await fileIdentity(f.suitePath, cache), first);
  await writeFile(f.suitePath, 'different bytes');
  const changed = await fileIdentity(f.suitePath, cache);
  assert.notEqual(changed.sha256, first.sha256);
  assert.deepEqual(await sourceHashes(['suite.json'], f.directory), { 'suite.json': changed.sha256 });
  const sources = await sourceHashes();
  for (const path of ['evaluation/writing/lab-worker/src/transport.rs', 'Cargo.lock', 'broker/src/provider.rs',
    'broker/src/semantic/process.rs', 'evaluation/writing/lab-worker/src/prefill_probe.rs',
    'evaluation/writing/lab/paced.mjs', 'evaluation/writing/lab-worker/src/paced_probe.rs',
    'evaluation/writing/lab-worker/src/paced_probe/scheduler.rs', 'evaluation/writing/lab-worker/src/main.rs',
    'evaluation/writing/lab-worker/src/qualification/device.rs', 'evaluation/writing/lab-worker/Cargo.toml',
    'evaluation/writing/lab/model-artifact.mjs', 'broker/src/semantic/wire.rs', 'broker/src/segment.rs',
    'broker/data/writing-lexicon/en.txt']) assert.match(sources[path], /^[a-f0-9]{64}$/u);
  assert.equal(sources['evaluation/writing/lab-worker/src/context_lookup.rs'],
    hash(await readFile(new URL('../lab-worker/src/context_lookup.rs', import.meta.url))));
});

test('run provenance hashes every Lab worker source file in a stable order', async () => {
  const crate = new URL('../lab-worker/', import.meta.url);
  const onDisk = (await readdir(new URL('src/', crate), { recursive: true, withFileTypes: true }))
    .filter(entry => entry.isFile())
    .map(entry => relative(fileURLToPath(new URL('../../../', import.meta.url)), join(entry.parentPath, entry.name)));
  assert.ok(onDisk.includes('evaluation/writing/lab-worker/src/lib.rs') && onDisk.length > 20);
  const listed = await labCrateSources();
  assert.deepEqual(listed, ['evaluation/writing/lab-worker/Cargo.toml', ...[...onDisk].sort()]);
  const sources = await sourceHashes();
  for (const path of listed) assert.match(sources[path], /^[a-f0-9]{64}$/u, path);
  assert.equal(Object.keys(sources).some(path => path.startsWith('broker/src/writing_lab')), false);
});

test('run provenance hashes every broker source the worker links', async () => {
  const root = fileURLToPath(new URL('../../../', import.meta.url));
  const onDisk = (await readdir(new URL('../../../broker/src/', import.meta.url), { recursive: true, withFileTypes: true }))
    .filter(entry => entry.isFile())
    .map(entry => relative(root, join(entry.parentPath, entry.name)));
  assert.ok(onDisk.includes('broker/src/vocabulary.rs') && onDisk.includes('broker/src/vocabulary/learned.rs'));
  const listed = await brokerSources();
  assert.deepEqual(listed.slice(0, onDisk.length), [...onDisk].sort());
  const sources = await sourceHashes();
  for (const path of listed) assert.match(sources[path], /^[a-f0-9]{64}$/u, path);
});

test('a terminal run builds its worker first and stops when the build fails', async () => {
  const calls = [];
  await buildWorker((command, args) => { calls.push([command, ...args]); return 0; });
  assert.deepEqual(calls, [['cargo', ...WORKER_BUILD]]);
  assert.deepEqual(WORKER_BUILD, ['build', '--release', '--locked', '-p', 'badi-writing-lab']);
  await assert.rejects(buildWorker(() => 101), /Building the Lab worker failed/u);
  await assert.rejects(buildWorker(() => Promise.reject(new Error('spawn cargo ENOENT'))), /ENOENT/u);
});

test('cleanup receipts preserve runtime identity and reject malformed lifecycle observations', () => {
  assert.deepEqual(validateCleanup(cleanupReceipt), cleanupReceipt);
  assert.equal(validateCleanup({ ...cleanupReceipt, reaped: false, exit_code: null }).reaped, false);
  for (const invalid of [null, [], {}, { ...cleanupReceipt, process_id: 1 }, { ...cleanupReceipt, reaped: 'true' },
    { ...cleanupReceipt, runtime_identity_sha256: 'invalid' }, { ...cleanupReceipt, exit_code: '0' }]) {
    assert.throws(() => validateCleanup(invalid), /cleanup receipt/u);
  }
});

test('event decoder handles split multibyte Unicode and refuses truncated or non-object events', async () => {
  const bytes = new TextEncoder().encode('{"type":"record","text":"می‌تواند"}\n');
  const body = new ReadableStream({ start(controller) { for (let i = 0; i < bytes.length; i += 3) controller.enqueue(bytes.slice(i, i + 3)); controller.close(); } });
  const events = []; for await (const event of decodeEvents(body)) events.push(event);
  assert.deepEqual(events, [{ type: 'record', text: 'می‌تواند' }]);
  for (const invalid of ['{"type":"complete"}', 'null\n', '[]\n']) {
    const stream = new Response(invalid).body;
    await assert.rejects(async () => { for await (const _event of decodeEvents(stream)) { /* Drain to validate the boundary. */ } });
  }
});

test('CLI uses real HTTP/server scoring and saves exclusive private artifacts without logging prose', async t => {
  const f = await fixture(t);
  const worker = new FakeWorker(); const progress = [];
  const result = await runSuite(f.options, { worker, outputRoot: f.outputRoot, onProgress: value => progress.push(value) });
  assert.equal(result.report.complete, true);
  assert.equal(result.report.measurements.overall.first_word.matches, 1);
  assert.equal(result.report.provenance.execution_kind, 'injected_fixture');
  assert.equal(result.report.provenance.valid_executed_identity, false);
  assert.equal(result.report.cleanup.completed, true);
  assert.deepEqual(result.report.cleanup.server_receipt, cleanupReceipt);
  assert.deepEqual(result.session.cleanup, cleanupReceipt);
  assert.ok(worker.stopped >= 1);
  assert.equal(worker.session, null);
  assert.deepEqual(progress, [{ completed: 1, total: 1, case_id: 'append-case', config_id: 'context' }]);
  assert.equal(JSON.stringify(progress).includes('PRIVATE'), false);
  assert.equal(Object.hasOwn(worker.calls[0], 'expected'), false);
  for (const name of ['suite-input.json', 'run.json', 'events.ndjson', 'session.json', 'report.json']) {
    assert.equal((await stat(join(result.output, name))).mode & 0o777, 0o600);
  }
  assert.equal(await readFile(join(result.output, 'suite-input.json'), 'utf8'), f.bytes);
  const run = JSON.parse(await readFile(join(result.output, 'run.json'), 'utf8'));
  assert.equal(run.input.sha256, hash(f.bytes));
  assert.equal(run.plan.length, 1);
  assert.equal(JSON.stringify(run).includes('x-badi-lab-token'), false);
  const saved = JSON.parse(await readFile(join(result.output, 'session.json'), 'utf8'));
  assert.deepEqual(saved.records[0].score, result.session.records[0].score);
  assert.deepEqual(saved.cleanup, cleanupReceipt);
  const events = (await readFile(join(result.output, 'events.ndjson'), 'utf8')).trim().split('\n').map(JSON.parse);
  assert.deepEqual(events.slice(-3).map(event => event.type), ['cleanup', 'qualification', 'complete']);
  await assert.rejects(runSuite(f.options, { worker, outputRoot: f.outputRoot }), /EEXIST/u);
  assert.equal(worker.calls.length, 1);
});

test('an imported session contributes cases only; previous predictions and reviews are not replayed', async t => {
  const f = await fixture(t, { schema: 'badi.prediction-lab.session.v1', suite: input, records: [{ result: 'OLD_OUTPUT_NEVER_USE' }] });
  const worker = new FakeWorker();
  const result = await runSuite(f.options, { worker, outputRoot: f.outputRoot });
  assert.equal(result.report.complete, true);
  assert.equal(worker.calls.length, 1);
  assert.equal(JSON.stringify(worker.calls).includes('OLD_OUTPUT_NEVER_USE'), false);
  assert.equal(JSON.stringify(result.session).includes('OLD_OUTPUT_NEVER_USE'), false);
});

test('artifact runs freeze exact descriptor bytes and require model identity before retaining scored records', async t => {
  const f = await fixture(t); const artifact = await artifactFixture(f.directory);
  const worker = new FakeWorker(async () => ({ outcome: 'suggestion', text: 'ent', word_complete: true,
    latency_ms: 4, identity: artifactIdentity(artifact) }));
  const result = await runSuite({ ...f.options, modelArtifactPath: artifact.path }, { worker, outputRoot: f.outputRoot });
  assert.equal(result.report.complete, true);
  assert.equal(result.report.provenance.model_artifact.sha256, artifact.sha256);
  assert.deepEqual(result.report.provenance.model_artifact, result.report.provenance.model_artifact_after);
  assert.equal(result.report.provenance.model_artifact_stable, true);
  assert.equal(result.report.provenance.valid_executed_identity, false);
  assert.equal(result.report.provenance.execution_kind, 'injected_fixture');
  const saved = join(result.output, 'model-artifact-input.json');
  assert.equal(await readFile(saved, 'utf8'), artifact.source.toString());
  assert.equal((await stat(saved)).mode & 0o777, 0o600);
  assert.equal(JSON.stringify(worker.calls).includes('model_origin'), false);
  for (const mode of ['mismatch', 'changed']) {
    const next = await fixture(t); const expected = await artifactFixture(next.directory);
    const invalid = new FakeWorker(async () => {
      if (mode === 'changed') await writeFile(expected.path, expected.source.toString() + ' ');
      return { outcome: 'suggestion', text: 'ent', word_complete: true, latency_ms: 4,
        identity: { ...artifactIdentity(expected), ...(mode === 'mismatch' ? { model_alias: 'wrong' } : {}) } };
    });
    const rejected = await runSuite({ ...next.options, modelArtifactPath: expected.path }, { worker: invalid, outputRoot: next.outputRoot });
    assert.equal(rejected.report.complete, false);
    assert.equal(rejected.report.completed_requests, 0);
    assert.equal(rejected.report.cleanup.completed, true);
    assert.equal(invalid.session, null);
    if (mode === 'changed') assert.equal(rejected.report.provenance.model_artifact_stable, false);
  }
});

test('malformed artifact prevents server startup and output creation', async t => {
  const f = await fixture(t); const artifact = await artifactFixture(f.directory);
  await writeFile(artifact.path, '{PRIVATE_INVALID_ARTIFACT');
  const worker = new FakeWorker();
  await assert.rejects(runSuite({ ...f.options, modelArtifactPath: artifact.path }, { worker, outputRoot: f.outputRoot }), /descriptor/u);
  assert.equal(worker.calls.length, 0);
  await assert.rejects(stat(f.options.outputPath), /ENOENT/u);
});

const artifactWorkerFixture = `
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { readFile } from 'node:fs/promises';
import { createInterface } from 'node:readline';
const [runtimePath,digest,mode,flag,path] = process.argv.slice(1);
if(path && flag!=='--model-artifact') throw Error('unexpected fixture argument');
const descriptor = path ? JSON.parse(await readFile(path,'utf8')) : null;
const runtime = spawn(runtimePath,['60'],{detached:true,stdio:'ignore'});
const exited = once(runtime,'exit'); await once(runtime,'spawn');
const identity = {fixture:true,binary_sha256:digest,runtime_pid:runtime.pid,batch_size:16,ubatch_size:16,
  ...(descriptor?{model_origin:'explicit_lab_artifact',model_sha256:descriptor.sha256,
    model_size:descriptor.bytes,model_alias:descriptor.alias}:{})};
if(mode==='mismatch') identity.model_sha256='0'.repeat(64);
const emit = value=>process.stdout.write(JSON.stringify(value)+'\\n');
let stopping=false;
process.on('SIGTERM',async()=>{if(stopping)return;stopping=true;runtime.kill('SIGTERM');await exited;
  emit({type:'stopped',cleanup:{process_id:runtime.pid,runtime_identity_sha256:'a'.repeat(64),challenge_completed:true,reaped:true,exit_code:0}});process.exit(0);});
emit({type:'ready',schema:'badi.prediction-lab.worker.v1',runtime_pid:runtime.pid,identity});
createInterface({input:process.stdin}).on('line',line=>{const request=JSON.parse(line);
  emit({type:'result',id:request.id,outcome:'suggestion',text:' word',word_complete:true,latency_ms:1,identity});});
`;

test('observed worker forwards only the descriptor flag and reaps a verified nested runtime on model mismatch', async t => {
  const f = await fixture(t); const artifact = await artifactFixture(f.directory);
  const runtimePath = join(f.directory, 'llama-server');
  await copyFile('/usr/bin/sleep', runtimePath); await chmod(runtimePath, 0o700);
  const digest = hash(await readFile(runtimePath));
  for (const mode of ['default', 'matched', 'mismatch']) {
    const worker = new ObservedWorker({ artifact: mode === 'default' ? null : artifact, executable: process.execPath,
      args: ['--input-type=module', '-e', artifactWorkerFixture, runtimePath, digest, mode], startupMs: 2000 });
    t.after(() => worker.stop());
    if (mode === 'mismatch') {
      await assert.rejects(worker.start(), /model identity/u);
      assert.equal(worker.session, null);
    } else {
      const ready = await worker.start();
      assert.equal(Object.hasOwn(ready, 'model_origin'), mode !== 'default');
      await worker.stop();
    }
    assert.equal(worker.executions.length, 1);
    assert.equal(worker.cleanups.at(-1).runtime_receipt.reaped, true);
    const runtimePid = worker.executions[0].runtime_identity.runtime_pid;
    assert.throws(() => process.kill(runtimePid, 0), error => error.code === 'ESRCH');
    assert.throws(() => process.kill(worker.executions[0].pid, 0), error => error.code === 'ESRCH');
  }
});

test('worker failure preserves partial evidence and fails the report while awaiting server cleanup', async t => {
  const f = await fixture(t);
  const worker = new FakeWorker(async () => { throw new Error('Disposable worker stopped'); });
  const result = await runSuite(f.options, { worker, outputRoot: f.outputRoot });
  assert.equal(result.report.complete, false);
  assert.equal(result.report.scheduled_requests, 1);
  assert.equal(result.report.completed_requests, 0);
  assert.match(result.report.error, /stopped/u);
  assert.ok(worker.stopped >= 1);
  assert.equal(result.report.cleanup.completed, true);
  const events = (await readFile(join(result.output, 'events.ndjson'), 'utf8')).trim().split('\n').map(JSON.parse);
  assert.deepEqual(events.slice(-3).map(event => event.type), ['cleanup', 'qualification', 'error']);
  assert.equal(JSON.parse(await readFile(join(result.output, 'session.json'), 'utf8')).complete, false);
});

test('an event after terminal completion is rejected while preserving all diagnostic records', async t => {
  const f = await fixture(t);
  const worker = new FakeWorker();
  const serverFactory = async options => {
    const lab = await createLabServer(options);
    lab.server.on('request', (request, response) => {
      if (request.url !== '/api/run') return;
      const end = response.end;
      response.end = function (...args) {
        this.write('{"type":"future_event"}\n');
        return end.apply(this, args);
      };
    });
    return lab;
  };
  const result = await runSuite(f.options, { worker, outputRoot: f.outputRoot, serverFactory });
  assert.equal(result.report.complete, false);
  assert.match(result.report.error, /after its terminal outcome/u);
  assert.equal(result.report.completed_requests, 1);
  assert.deepEqual(result.session.cleanup, cleanupReceipt);
  const events = (await readFile(join(result.output, 'events.ndjson'), 'utf8')).trim().split('\n').map(JSON.parse);
  assert.equal(events.at(-1).type, 'future_event');
});

test('cancelling the CLI aborts in-flight inference and writes an incomplete private report', async t => {
  const f = await fixture(t);
  const cancellation = new AbortController();
  const worker = new FakeWorker(async (_request, signal) => new Promise((_resolve, reject) => {
    signal.addEventListener('abort', () => reject(new Error('Fixture aborted')), { once: true });
    cancellation.abort();
  }));
  const result = await runSuite(f.options, { worker, outputRoot: f.outputRoot, signal: cancellation.signal });
  assert.equal(result.report.complete, false);
  assert.equal(result.report.cancelled, true);
  assert.equal(result.report.cleanup.completed, true);
  assert.equal(worker.session, null);
});

test('invalid suite input fails before creating an output directory or starting the worker', async t => {
  const f = await fixture(t, { ...input, cases: [{ ...input.cases[0], expected: [42] }] });
  const worker = new FakeWorker();
  await assert.rejects(runSuite(f.options, { worker, outputRoot: f.outputRoot }), /Expected output/u);
  assert.equal(worker.calls.length, 0);
  await assert.rejects(stat(f.options.outputPath), /ENOENT/u);
  await writeFile(f.suitePath, '{PRIVATE_TEXT_THAT_MUST_NOT_REACH_STDERR');
  await assert.rejects(runSuite(f.options, { worker, outputRoot: f.outputRoot }), error => {
    assert.equal(error.message, 'Suite input must contain valid JSON.');
    assert.equal(error.message.includes('PRIVATE_TEXT'), false);
    return true;
  });
});

const revisionSuite = { schema: 'badi.revision-suite.v1', name: 'Revision CLI fixture', cases: [
  { id: 'en-g', language: 'en', mode: 'grammar', kind: 'error', before: 'I said ', selected: 'she dont know', after: ' twice.',
    expected: ['PRIVATE_EXPECTED'] },
  { id: 'en-r', language: 'en', mode: 'refine', selected: 'pls call Tom at 14:30', facts: ['Tom', '14:30'] }] };

test('revision modes run apart from prediction modes and take no prediction option', () => {
  const base = ['--suite', 'suite.json', '--output', 'output/writing/new'];
  assert.deepEqual(parseOptions([...base, '--modes', 'revision_refine,revision_grammar', '--seed', '9'], '/work/badi'),
    { revision: true, suitePath: '/work/badi/suite.json', outputPath: '/work/badi/output/writing/new', modes: ['refine', 'grammar'], seed: 9 });
  for (const extra of [['--modes', 'revision_grammar,context'], ['--modes', 'revision_grammar,revision_grammar'],
    ['--modes', 'revision_grammar', '--budget-ms', '550'], ['--modes', 'revision_grammar', '--cache-prompt', 'true'],
    ['--modes', 'revision_refine', '--prefill-batch', '64'], ['--modes', 'revision_refine', '--warm-prefix', 'true']]) {
    assert.throws(() => parseOptions([...base, ...extra]));
  }
  assert.equal(parseOptions([...base, '--budget-ms', '2500', '--modes', 'context']).configs[0].budget_ms, 2500);
});

test('a revision run keeps answers out of requests and records outcomes, scores and cleanup', async t => {
  const f = await fixture(t, revisionSuite);
  const worker = new FakeWorker(async request => ({ type: 'revision_result', id: request.id,
    ...(request.mode === 'grammar' ? { outcome: 'shown', text: "she doesn't know", output: "She doesn't know." }
      : { outcome: 'facts', text: null, output: 'Please call him at 14:30.' }), latency_ms: 5, generated_tokens: 4 }));
  const progress = [];
  const options = { revision: true, suitePath: f.suitePath, outputPath: f.options.outputPath, modes: ['grammar', 'refine'], seed: 3 };
  const result = await runRevisionSuite(options, { worker, outputRoot: f.outputRoot, onProgress: value => progress.push(value) });
  assert.equal(result.report.complete, true);
  assert.equal(result.report.cleanup.completed, true);
  assert.deepEqual(result.report.cleanup.runtime_receipt, cleanupReceipt);
  assert.equal(worker.calls.length, 2);
  assert.equal(JSON.stringify(worker.calls).includes('PRIVATE_EXPECTED'), false);
  assert.ok(worker.calls.every(call => !Object.hasOwn(call, 'facts') && !Object.hasOwn(call, 'expected')));
  assert.deepEqual(worker.calls.find(call => call.mode === 'grammar').after, ' twice.');
  assert.equal(JSON.stringify(progress).includes('dont'), false);
  const grammar = result.report.measurements.by_mode.grammar.by_language.en;
  assert.deepEqual(grammar.errors, { cases: 1, fixed: 0, shown_wrong: 1, not_shown: {} });
  const refine = result.report.measurements.by_mode.refine.overall;
  assert.deepEqual([refine.shown, refine.not_shown], [0, { facts: 1 }]);
  for (const name of ['suite-input.json', 'run.json', 'events.ndjson', 'session.json', 'report.json']) {
    assert.equal((await stat(join(result.output, name))).mode & 0o777, 0o600);
  }
  const events = (await readFile(join(result.output, 'events.ndjson'), 'utf8')).trim().split('\n').map(JSON.parse);
  assert.deepEqual(events.map(event => event.type), ['plan', 'progress', 'record', 'progress', 'record', 'cleanup', 'complete']);
  assert.deepEqual(JSON.parse(await readFile(join(result.output, 'run.json'), 'utf8')).plan, result.session.plan);
});

test('a revision worker failure keeps partial evidence and an incomplete report', async t => {
  const f = await fixture(t, revisionSuite);
  let calls = 0;
  const worker = new FakeWorker(async request => {
    if (calls++) throw new Error('Disposable revision worker stopped');
    return { type: 'error', id: request.id, error: 'invalid_request' };
  });
  const options = { revision: true, suitePath: f.suitePath, outputPath: f.options.outputPath, modes: ['grammar', 'refine'], seed: 3 };
  const result = await runRevisionSuite(options, { worker, outputRoot: f.outputRoot });
  assert.equal(result.report.complete, false);
  assert.match(result.report.error, /stopped/u);
  assert.equal(result.session.records.length, 1);
  assert.equal(result.session.records[0].result.outcome, 'error');
  assert.ok(worker.stopped >= 1);
});
