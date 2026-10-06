import assert from 'node:assert/strict';
import test from 'node:test';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFile, mkdtemp, chmod, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { processIdentity } from './worker.mjs';
import { SpellingWorker, readSpellingManifest, validateSpellingRequest, validateSpellingManifest,
  spellingIdentityHash, expectedSpellingIdentity, validateSpellingResult } from './spelling-worker.mjs';

// A Node supervisor and a disposable copy of sleep exercise actual process
// ownership/hash/cleanup. These fixtures never launch Hunspell or a model.
const fixture = `
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { createInterface } from 'node:readline';
import { setTimeout as delay } from 'node:timers/promises';
const [mode,path,language]=process.argv.slice(1);
const raw=await readFile(path);const m=JSON.parse(raw);const d=m.dictionaries[language];
const hash=v=>createHash('sha256').update(v).digest('hex');
const identity={contract_id:'badi.spelling-lab.hunspell.v1',manifest_sha256:hash(raw),
engine_sha256:m.binary.sha256,engine_size:m.binary.bytes,library_sha256:m.library.sha256,
library_size:m.library.bytes,language,dictionary_id:d.id,aff_sha256:d.aff.sha256,aff_size:d.aff.bytes,
dic_sha256:d.dic.sha256,dic_size:d.dic.bytes};
const identity_sha256=hash(JSON.stringify(identity));
let engine;let exited;let quitting=false;
const emit=v=>process.stdout.write(JSON.stringify(v)+'\\n');
process.stdout.on('error',()=>{});
const shutdown=async()=>{if(quitting)return;quitting=true;if(engine){engine.kill('SIGTERM');await exited;
emit({type:'stopped',cleanup:{process_id:engine.pid,identity_sha256,reaped:true,exit_code:0,forced:false}})}process.exit(0)};
process.on('SIGTERM',shutdown);process.on('SIGINT',shutdown);
if(mode==='startup-hang'){setInterval(()=>{},1000)}
else if(mode==='startup-eof'){process.stdout.end();setInterval(()=>{},1000)}
else {
engine=spawn(m.binary.path,['60'],{detached:mode!=='wrong-group',stdio:'ignore'});
exited=once(engine,'exit');await once(engine,'spawn');
const ready={type:'ready',schema:'badi.spelling-lab.worker.v1',engine_pid:engine.pid,identity,identity_sha256};
if(mode==='wrong-parent')ready.engine_pid=process.pid;
if(mode==='wrong-digest')ready.identity={...identity,engine_sha256:'0'.repeat(64)};
if(mode==='delayed-ready')await delay(150);
emit(ready);
}
createInterface({input:process.stdin}).on('line',async line=>{
const request=JSON.parse(line);
if(mode==='hang')return;
if(mode==='malformed'){process.stdout.write('not JSON\\n');return}
if(mode==='oversized'){process.stdout.write('x'.repeat(70000));return}
if(mode==='eof'){process.stdout.end();return}
const value={type:'result',schema:'badi.spelling-lab.result.v1',id:request.id,
outcome:mode==='deadline'?'deadline':'suggestion',reason:mode==='deadline'?'query_deadline':'single_admissible_returned_candidate',
text:mode==='deadline'?null:'Berlin ',replace_before:mode==='deadline'?null:'Berliin ',
filtered_candidate_count:1,query_count:2,query_ms:1,latency_ms:2,identity,identity_sha256,
warnings:['returned_candidates_not_exhaustive','unknown_proper_names_not_detected']};
if(mode==='wrong-id')value.id='foreign';
if(mode==='wrong-suffix')value.replace_before='other ';
if(mode==='missing-space')value.text='Berlin';
emit(value);if(mode==='deadline')await shutdown();
});
`;
const request = (id = 'fixture') => ({ schema: 'badi.spelling-lab.request.v1', id,
  before: 'Wir treffen uns in Berliin ', language: 'de', protected_words: [] });

async function makeWorker(t, mode = 'normal', options = {}) {
  const directory = await mkdtemp(join(tmpdir(), 'badi-spelling-transport-'));
  const engine = join(directory, 'hunspell'); await copyFile('/usr/bin/sleep', engine); await chmod(engine, 0o700);
  const file = async (name, text) => {
    const path = join(directory, name); await writeFile(path, text, { mode: 0o600 });
    return { path, bytes: Buffer.byteLength(text), sha256: createHash('sha256').update(text).digest('hex') };
  };
  const bytes = await readFile(engine);
  const manifest = { schema: 'badi.spelling-artifact.v1', binary: { path: engine, bytes: bytes.length,
    sha256: createHash('sha256').update(bytes).digest('hex') }, library: await file('library', 'fixture'),
  dictionaries: { de: { id: 'fixture-de', aff: await file('de.aff', 'a'), dic: await file('de.dic', 'd') },
    fa: { id: 'fixture-fa', aff: await file('fa.aff', 'a'), dic: await file('fa.dic', 'd') } } };
  const path = join(directory, 'manifest.json'); await writeFile(path, JSON.stringify(manifest), { mode: 0o600 });
  const captured = [];
  const worker = new SpellingWorker({ manifest: await readSpellingManifest(path), language: 'de',
    startupMs: 1000, requestMs: 500, spawnProcess: (executable, args, opts) => {
      captured.push({ executable, args });
      return spawn(process.execPath, ['--input-type=module', '-e', fixture, mode, path, 'de'], opts);
    }, ...options });
  t.after(async () => { try { await worker.stop(); } catch (error) {
    assert.match(error.message, /spelling_cleanup_unverified/u);
  } finally { await rm(directory, { recursive: true, force: true }); } });
  return { worker, captured, path };
}

test('native transport uses only spelling dispatch and retains one verified engine for sequential checks', async t => {
  const { worker, captured } = await makeWorker(t);
  const [a, b] = await Promise.all([worker.start(), worker.start()]);
  assert.deepEqual(a, b); assert.equal(captured.length, 1);
  assert.deepEqual(captured[0].args.slice(0, 2), ['--spelling-lab', '--spelling-manifest']);
  assert.deepEqual(captured[0].args.slice(-2), ['--language', 'de']);
  const owner = worker.session.child.pid; const engine = worker.ready.engine_pid;
  assert.equal((await processIdentity(engine)).parent, owner);
  assert.equal((await worker.check(request())).text, 'Berlin ');
  assert.equal((await worker.check(request('next'))).replace_before, 'Berliin ');
  assert.equal(worker.ready.engine_pid, engine);
  const cleanup = await worker.stop();
  assert.equal(cleanup.process_id, engine); assert.equal(cleanup.reaped, true);
  assert.equal(cleanup.identity_sha256, spellingIdentityHash(a));
  assert.equal(await processIdentity(engine), null); assert.equal(await processIdentity(owner), null);
});

test('invalid native inputs and unsupported language never start a process', async t => {
  const { worker, captured } = await makeWorker(t);
  for (const extra of [{ expected: ['answer'] }, { before: 'x'.repeat(2049) }, { language: 'en' },
    { language: 'fa' }, { protected_words: Array(33).fill('word') }, { protected_words: ['two words'] },
    { protected_words: ['x'.repeat(25)] }, { protected_words: ['کتاب'] }, { protected_words: ['\u0300abc'] },
    { before: 'متن\u200c' }, { before: 'secret\u202e' }]) {
    await assert.rejects(worker.check({ ...request(), ...extra }), /spelling_invalid_input/u);
  }
  assert.equal(captured.length, 0);
  assert.deepEqual(validateSpellingRequest({ ...request(), language: 'de-DE' }).language, 'de-DE');
  assert.deepEqual(validateSpellingRequest({ ...request(), language: 'DE-de' }).language, 'DE-de');
  assert.deepEqual(validateSpellingRequest({ ...request(), protected_words: undefined }), { ...request(), protected_words: [] });
});

test('manifest schema rejects arbitrary runtime flags and malformed artifact values', () => {
  assert.throws(() => validateSpellingManifest({ schema: 'badi.spelling-artifact.v1', args: [] }));
});

test('manifest reader rejects dictionary-list injection, oversize and changed descriptors', async t => {
  const { worker, path, captured } = await makeWorker(t);
  const dictionaryList = structuredClone(worker.manifest.descriptor);
  dictionaryList.dictionaries.de.aff.path += ',other';
  assert.throws(() => validateSpellingManifest(dictionaryList), /spelling_invalid_manifest/u);
  await writeFile(path, ' '.repeat(16 * 1024 + 1));
  await assert.rejects(readSpellingManifest(path), /spelling_invalid_manifest/u);
  await writeFile(path, JSON.stringify(worker.manifest.descriptor) + '\n');
  await assert.rejects(worker.start(), /spelling_manifest_changed/u);
  assert.equal(captured.length, 0);
});

test('result boundary enforces whole-word protection, native confirmation, scripts, case, edits and budget', async t => {
  const { worker } = await makeWorker(t);
  const identity = expectedSpellingIdentity(worker.manifest, 'de');
  const base = { type: 'result', schema: 'badi.spelling-lab.result.v1', id: request().id,
    outcome: 'suggestion', reason: 'single_admissible_returned_candidate', text: 'Berlin ', replace_before: 'Berliin ',
    filtered_candidate_count: 1, query_count: 2, query_ms: 1, latency_ms: 2,
    identity, identity_sha256: spellingIdentityHash(identity),
    warnings: ['returned_candidates_not_exhaustive', 'unknown_proper_names_not_detected'] };
  const validate = (result = {}, input = {}) => validateSpellingResult({ ...base, ...result }, { ...request(), ...input }, identity);
  validate();
  validate({}, { protected_words: ['Berlin'] }); // Protection never hides a competing destination.
  for (const result of [{ filtered_candidate_count: 0 }, { filtered_candidate_count: 2 }, { query_count: 1 },
    { latency_ms: 550.001 }, { warnings: [] }, { text: 'berlin ' }, { text: 'BerLin ' },
    { text: 'Bremen ' }, { text: 'Berlیn ' }, { text: 'Berl-in ' }, { text: 'Berlin  ' },
    { text: 'Berl\u200cin ' }, { replace_before: 'erliin ', text: 'erlin ' }]) {
    assert.throws(() => validate(result), /spelling_invalid_result/u);
  }
  for (const input of [{ protected_words: ['Berliin'] }, { before: 'word,Berliin ' }, { before: 'xBerliin ' },
    { before: 'Berliin  ' }, { before: 'Berliin\t ' }]) {
    assert.throws(() => validate({}, input), /spelling_invalid_result/u);
  }
  validate({ replace_before: 'Berlni ', text: 'Berlin ' }, { before: 'Berlni ' });
  const faIdentity = expectedSpellingIdentity(worker.manifest, 'fa');
  const fa = (before, text) => validateSpellingResult({ ...base, identity: faIdentity,
    identity_sha256: spellingIdentityHash(faIdentity), replace_before: before, text },
  { ...request(), before, language: 'fa' }, faIdentity);
  fa('گزاررش ', 'گزارش ');
  fa('می\u200cرروم ', 'می\u200cروم ');
  for (const [before, text] of [['میروم ', 'می\u200cروم '], ['می\u200cروم ', 'میروم '],
    ['می\u200cروم ', 'میر\u200cوم ']]) assert.throws(() => fa(before, text), /spelling_invalid_result/u);
});

test('stop during asynchronous manifest loading prevents a later process launch', async t => {
  const { worker, captured } = await makeWorker(t);
  const starting = worker.start();
  await worker.stop();
  await assert.rejects(starting, /spelling_cancelled/u);
  assert.equal(captured.length, 0); assert.equal(worker.session, null);
});

test('cancellation during healthy delayed readiness awaits identity and cleanup, then permits a new start', async t => {
  const { worker, captured } = await makeWorker(t, 'delayed-ready');
  const starting = worker.start();
  const rejectedStart = assert.rejects(starting, /spelling_cancelled/u);
  while (!worker.session) await new Promise(resolve => setImmediate(resolve));
  const session = worker.session;
  const supervisor = session.child.pid;
  let sentRequests = 0;
  const write = session.child.stdin.write.bind(session.child.stdin);
  session.child.stdin.write = (...args) => { sentRequests++; return write(...args); };
  assert.equal(worker.ready, null);
  const controller = new AbortController();
  const cancelledCheck = assert.rejects(worker.check(request(), controller.signal), /spelling_cancelled/u);
  controller.abort();
  const stopping = worker.stop();
  await rejectedStart;
  assert.equal(session.shutdownSignalled, undefined);
  assert.notEqual(await processIdentity(supervisor), null);
  const receipt = await stopping;
  await cancelledCheck;
  assert.equal(receipt.reaped, true); assert.equal(sentRequests, 0);
  assert.equal(await processIdentity(receipt.process_id), null);
  assert.equal(await processIdentity(supervisor), null);
  assert.equal(worker.cleanupError, null);
  await worker.start(); assert.equal(captured.length, 2);
  const nextEngine = worker.ready.engine_pid;
  await worker.stop(); assert.equal(await processIdentity(nextEngine), null);
});

test('cancelling during failed readiness stays bounded and retains cleanup uncertainty', async t => {
  for (const mode of ['startup-hang', 'startup-eof', 'wrong-digest']) {
    const { worker } = await makeWorker(t, mode, { startupMs: 200 });
    const starting = worker.start();
    const rejectedStart = assert.rejects(starting, /spelling_cancelled/u);
    while (!worker.session) await new Promise(resolve => setImmediate(resolve));
    const supervisor = worker.session.child.pid;
    const stopping = worker.stop();
    await rejectedStart;
    await assert.rejects(stopping, /spelling_cleanup_unverified/u);
    assert.equal(await processIdentity(supervisor), null);
    assert.equal(worker.lastCleanup, null);
    await assert.rejects(worker.start(), /spelling_cleanup_unverified/u);
  }
});

test('a concurrent check is refused without stopping the already owned request', async t => {
  const { worker } = await makeWorker(t, 'hang', { requestMs: 1000 }); await worker.start();
  const controller = new AbortController();
  const first = worker.check(request(), controller.signal);
  await assert.rejects(worker.check(request('other')), /spelling_busy/u);
  assert.notEqual(worker.session, null);
  controller.abort(); await assert.rejects(first);
  assert.equal(worker.session, null);
});

test('unexpected identities, process groups and hashes are refused with unverified-startup cleanup', async t => {
  for (const mode of ['wrong-parent', 'wrong-group', 'wrong-digest']) {
    const { worker } = await makeWorker(t, mode);
    await assert.rejects(worker.start(), /spelling_invalid_identity/u);
    await assert.rejects(worker.stop(), /spelling_cleanup_unverified/u);
    assert.equal(worker.session, null);
  }
});

test('bad frames, foreign IDs and non-exact replacement seams retire verified engines', async t => {
  for (const mode of ['malformed', 'oversized', 'wrong-id', 'wrong-suffix', 'missing-space', 'eof']) {
    const { worker } = await makeWorker(t, mode); await worker.start();
    const engine = worker.ready.engine_pid;
    await assert.rejects(worker.check(request()), /spelling_/u);
    assert.equal(await processIdentity(engine), null);
    assert.equal(worker.session, null);
  }
});

test('deadline result is returned only after native terminal cleanup and process absence', async t => {
  const { worker } = await makeWorker(t, 'deadline'); await worker.start();
  const engine = worker.ready.engine_pid;
  const result = await worker.check(request());
  assert.equal(result.outcome, 'deadline'); assert.equal(result.text, null);
  assert.equal(worker.lastCleanup.process_id, engine);
  assert.equal(await processIdentity(engine), null);
  assert.equal(worker.session, null);
});

test('abort and transport timeout await verified engine cleanup', async t => {
  for (const abort of [true, false]) {
    const { worker } = await makeWorker(t, 'hang', { requestMs: 100 }); await worker.start();
    const engine = worker.ready.engine_pid;
    const controller = new AbortController(); const pending = worker.check(request(), controller.signal);
    if (abort) controller.abort();
    await assert.rejects(pending);
    assert.equal(await processIdentity(engine), null); assert.equal(worker.session, null);
  }
});

test('startup timeout and EOF reap supervisor but do not invent unobserved engine receipts', async t => {
  for (const mode of ['startup-hang', 'startup-eof']) {
    const { worker } = await makeWorker(t, mode, { startupMs: 150 });
    await assert.rejects(worker.start(), /spelling_startup_timeout|spelling_output_closed/u);
    await assert.rejects(worker.stop(), /spelling_cleanup_unverified/u);
    assert.equal(worker.lastCleanup, null); assert.equal(worker.session, null);
  }
});
