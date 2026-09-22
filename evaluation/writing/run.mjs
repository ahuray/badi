import { execFile } from 'node:child_process';
import { createReadStream } from 'node:fs';
import { readFile, mkdir, writeFile, realpath, stat, readdir, readlink } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { parseArgs, promisify } from 'node:util';
import { resolve, isAbsolute } from 'node:path';
import { BrokerClient } from '../../adapters/shared/broker-client.mjs';
import { ROOT, HERE, sha256, loadCorpus, aggregate, reviewTemplate } from './lib.mjs';

const exec = promisify(execFile);
const { values } = parseArgs({ options: { split: { type: 'string' }, output: { type: 'string' },
  socket: { type: 'string' }, badictl: { type: 'string' }, 'broker-pid': { type: 'string' },
  label: { type: 'string' }, language: { type: 'string' }, limit: { type: 'string' },
  manifest: { type: 'string', default: 'manifest.json' } }, strict: true });
for (const name of ['split', 'output', 'socket', 'badictl', 'broker-pid', 'label']) {
  if (!values[name]) throw new Error(`Missing --${name}`);
}
if (!['socket', 'badictl'].every(name => isAbsolute(values[name])) || !/^[1-9][0-9]*$/u.test(values['broker-pid'])) {
  throw new Error('Use absolute socket/control paths and a positive broker PID');
}
const corpus = await loadCorpus(values.split, HERE, values.manifest);
let rows = corpus.rows;
if (values.language) {
  rows = rows.filter(row => row.language === values.language);
  if (!rows.length) throw new Error('No samples for requested language');
}
if (values.limit) {
  if (values.split !== 'development' || !/^[1-9][0-9]*$/u.test(values.limit)) throw new Error('Limits are development-only');
  rows = rows.slice(0, Number(values.limit));
}
const output = resolve(values.output);
if (!output.startsWith(resolve(ROOT, 'output/writing') + '/')) throw new Error('Write evaluations under ignored output/writing/');
await mkdir(resolve(ROOT, 'output/writing'), { recursive: true, mode: 0o700 });
await mkdir(output, { mode: 0o700 }); // Never overwrite a prior run or labels.
const save = (name, data) => writeFile(resolve(output, name), JSON.stringify(data, null, 2) + '\n', { mode: 0o600, flag: 'wx' });
const status = async () => JSON.parse((await exec(values.badictl, ['--socket', values.socket, 'status'], { timeout: 5000 })).stdout);
async function fileIdentity(path) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  const metadata = await stat(path);
  return { path: await realpath(path), sha256: hash.digest('hex'), bytes: metadata.size };
}
async function processIdentity(pid) {
  const command = (await readFile(`/proc/${pid}/cmdline`, 'utf8')).split('\0').filter(Boolean);
  const processStat = await readFile(`/proc/${pid}/stat`, 'utf8');
  return { pid: Number(pid), start_ticks: processStat.slice(processStat.lastIndexOf(')') + 2).split(' ')[19],
    executable: await fileIdentity(`/proc/${pid}/exe`), command };
}
const initial = await status();
if (initial.provider !== 'local_model' || initial.paused || initial.active?.present || initial.control_plane_degraded) {
  throw new Error('Evaluation needs an idle, unpaused real local-model broker');
}
const broker = await processIdentity(values['broker-pid']);
if (!broker.executable.path.endsWith('/badi-broker')) throw new Error('Specified PID is not badi-broker');
const unixSockets = (await readFile(`/proc/${broker.pid}/net/unix`, 'utf8')).trim().split('\n').slice(1)
  .map(line => line.trim().split(/\s+/u)).filter(parts => parts[7] === values.socket).map(parts => `socket:[${parts[6]}]`);
const descriptors = await Promise.all((await readdir(`/proc/${broker.pid}/fd`))
  .map(fd => readlink(`/proc/${broker.pid}/fd/${fd}`).catch(() => null)));
if (!descriptors.some(fd => unixSockets.includes(fd))) throw new Error('Specified broker does not own the selected Unix socket');
const children = (await readFile(`/proc/${values['broker-pid']}/task/${values['broker-pid']}/children`, 'utf8')).trim().split(/\s+/u).filter(Boolean);
const runtimes = [];
for (const child of children) {
  const executable = await realpath(`/proc/${child}/exe`);
  if (executable.endsWith('/llama-server')) runtimes.push(await processIdentity(child));
}
if (runtimes.length !== 1) throw new Error('Expected exactly one llama-server child of the specified broker');
const runtime = runtimes[0];
const modelIndex = runtime.command.findIndex(arg => ['-m', '--model'].includes(arg));
// Badi passes runtime configuration in its environment together with a secret
// API key. Read the mapped GGUF identity instead of reading that environment.
const mappedModels = [...new Set((await readFile(`/proc/${runtime.pid}/maps`, 'utf8')).trim().split('\n')
  .map(line => line.trim().split(/\s+/u).slice(5).join(' ')).filter(path => isAbsolute(path) && path.endsWith('.gguf')))];
const modelPath = modelIndex >= 0 ? runtime.command[modelIndex + 1] : mappedModels.length === 1 ? mappedModels[0] : null;
if (!modelPath || !isAbsolute(modelPath) || !mappedModels.includes(modelPath)) throw new Error('Cannot identify unique mapped local model');
const model = await fileIdentity(modelPath);
const sourcePaths = ['broker/src/writing.rs', 'broker/src/provider.rs', 'broker/src/semantic/client.rs',
  'broker/src/semantic/runtime.rs', 'adapters/shared/broker-client.mjs', 'evaluation/writing/run.mjs', 'evaluation/writing/lib.mjs'];
const sources = Object.fromEntries(await Promise.all(sourcePaths.map(async path => [path, sha256(await readFile(resolve(ROOT, path)))])));
const gitHead = (await exec('git', ['rev-parse', 'HEAD'], { cwd: ROOT })).stdout.trim();
const metadata = { schema: 'badi.writing-evaluation.v1', label: values.label, started_at: new Date().toISOString(),
  corpus: { split: values.split, manifest: corpus.manifest, manifest_name: corpus.manifestName,
    manifest_sha256: corpus.manifestSha256,
    language_filter: values.language ?? null, sample_limit: values.limit ? Number(values.limit) : null },
  provenance: { git_head: gitHead, working_source_sha256: sources, broker, runtime, model,
    node: process.version, socket: values.socket, control: await fileIdentity(values.badictl),
    note: 'Executed binary hashes identify the running implementation. Working source hashes need not match installed binaries.' },
  measurement: 'Sequential explicit synthetic requests through BrokerClient; measures request-to-client-result including transport and model, excludes adapter debounce, UI rendering and actual acceptance. First sample is unprimed; model was already loaded.',
  initial_status: initial };
await save('run.json', metadata);
const client = new BrokerClient('obsidian');
const samples = [];
let runError = null;
try {
  if (!await client.connect(values.socket)) throw new Error('Obsidian policy denies this evaluation; grant separately');
  for (const row of rows) {
    let reason = 'unknown';
    const state = value => { reason = value; };
    client.on('state', state);
    const start = performance.now();
    try {
      const text = await client.suggest(row.prefix, { language: row.language });
      samples.push({ ...row, text, outcome: text === null ? 'abstention' : 'suggestion', reason,
        latency_ms: Math.round((performance.now() - start) * 1000) / 1000 });
    } catch (error) {
      samples.push({ ...row, text: null, outcome: 'error', reason: String(error.message),
        latency_ms: Math.round((performance.now() - start) * 1000) / 1000 });
    } finally { client.off('state', state); client.cancel(); }
    if (samples.length % 10 === 0) process.stdout.write(`Evaluated ${samples.length}/${rows.length}\n`);
  }
} catch (error) { runError = String(error.message); }
finally { client.close(); }
await save('samples.json', samples);
let finalStatus = null;
let sameProcess = false;
try {
  finalStatus = await status();
  const finalBroker = await processIdentity(broker.pid);
  const finalRuntime = await processIdentity(runtime.pid);
  sameProcess = finalBroker.start_ticks === broker.start_ticks && finalRuntime.start_ticks === runtime.start_ticks &&
    finalBroker.executable.sha256 === broker.executable.sha256 && finalRuntime.executable.sha256 === runtime.executable.sha256;
} catch (error) { runError ??= String(error.message); }
const authorityStable = finalStatus?.authority_epoch === initial.authority_epoch && finalStatus?.settings_revision === initial.settings_revision;
const summary = { schema: 'badi.writing-evaluation-summary.v1', completed_at: new Date().toISOString(),
  label: values.label, split: values.split, complete: samples.length === rows.length && runError === null,
  valid_run_identity: sameProcess && authorityStable, run_error: runError, final_status: finalStatus,
  metrics_delta: finalStatus ? Object.fromEntries(Object.entries(finalStatus.metrics).map(([key, count]) => [key, count - initial.metrics[key]])) : null,
  measurements: aggregate(samples), quality_claim: 'No semantic quality claim until separately reviewed. This run does not establish application compatibility or Cotypist parity.' };
await save('summary.json', summary);
await save('review-template.json', reviewTemplate(samples));
console.log(JSON.stringify({ output, complete: summary.complete, valid_run_identity: summary.valid_run_identity,
  overall: summary.measurements.overall }, null, 2));
if (!summary.complete || !summary.valid_run_identity) process.exitCode = 1;
