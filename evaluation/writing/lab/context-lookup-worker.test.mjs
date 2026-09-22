import assert from 'node:assert/strict';
import test from 'node:test';
import { spawn } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';
import { ContextLookupWorker, validateContextLookupRequest, validateContextLookupResult } from './context-lookup-worker.mjs';
import { processIdentity } from './worker.mjs';

const input = (overrides = {}) => ({ schema: 'badi.context-lookup.request.v1', id: 'fixture', before: 'Use the note',
  language: 'en', context: 'The notebook is ready.', style_examples: [], ...overrides });
const result = (overrides = {}) => ({ schema: 'badi.context-lookup.result.v1', id: 'fixture',
  contract_id: 'badi.context-lookup.exact.v1', outcome: 'suggestion', reason: 'unique_source_word',
  text: 'book', matched_word: 'notebook', matched_candidate_count: 1, sources: ['context'], latency_ms: 1,
  warnings: ['context_match_does_not_prove_intent'], ...overrides });

const fixture = `
const mode=process.argv[1];let chunks=[];
if(mode==='forced')process.on('SIGTERM',()=>{});
if(mode==='premature')process.stdout.write('{}\\n');
process.stdin.on('data',chunk=>chunks.push(chunk));
process.stdin.on('end',()=>{
if(mode==='hang'||mode==='forced'){setInterval(()=>{},1000);return}
const raw=Buffer.concat(chunks).toString();const value=JSON.parse(raw);
if(!raw.endsWith('\\n')||raw.indexOf('\\n')!==raw.length-1||Object.keys(value).sort().join(',')!=='before,context,id,language,schema,style_examples')process.exit(9);
const out={schema:'badi.context-lookup.result.v1',id:value.id,contract_id:'badi.context-lookup.exact.v1',
outcome:'suggestion',reason:'unique_source_word',text:'book',matched_word:'notebook',matched_candidate_count:1,sources:['context'],latency_ms:1,warnings:['context_match_does_not_prove_intent']};
if(mode==='wrong-id')out.id='foreign';if(mode==='wrong-word')out.matched_word='notepaper';
if(mode==='malformed'){process.stdout.write('invalid\\n');return}
if(mode==='oversized'){process.stdout.write('x'.repeat(70000));return}
process.stdout.write(JSON.stringify(out)+(mode==='no-newline'?'':'\\n'));
if(mode==='extra')process.stdout.write('{}\\n');
if(mode==='hold')setInterval(()=>{},1000);
if(mode==='exit-failure')process.exitCode=1;
});
`;

function setup(t, mode = 'normal', options = {}) {
  const calls = [];
  const worker = new ContextLookupWorker({ executable: '/usr/bin/node', timeoutMs: 3000, killMs: 100,
    spawnProcess: (path, args, opts) => {
      const child = mode === 'wrong-executable' ? spawn('/usr/bin/sleep', ['60'], opts)
        : spawn(path, ['-e', fixture, mode], opts);
      const call = { path, args, pid: child.pid, frames: [] }; calls.push(call);
      const end = child.stdin.end.bind(child.stdin);
      child.stdin.end = (value, ...rest) => { call.frames.push(value); return end(value, ...rest); };
      return child;
    }, ...options });
  t.after(() => worker.stop());
  return { worker, calls };
}

test('lookup input boundary rejects references, unknown fields and malformed bounded multilingual data', () => {
  validateContextLookupRequest(input());
  validateContextLookupRequest(input({ language: 'FA-ir', before: 'من می‌ر', context: 'می‌روم.', style_examples: ['من می‌روم.'] }));
  for (const value of [input({ expected: ['book'] }), input({ config: {} }), input({ before: ' ' }),
    input({ before: 'x'.repeat(2049) }), input({ context: 'x'.repeat(4097) }), input({ style_examples: Array(9).fill('word') }),
    input({ style_examples: ['x'.repeat(1025)] }), input({ style_examples: [' '] }), input({ before: '\ud800' }),
    input({ before: 'من می‌' }), input({ context: 'secret\u202e' }), input({ language: 'fr' }), input({ id: 'x'.repeat(129) })]) {
    assert.throws(() => validateContextLookupRequest(value), /context_lookup_invalid_input/u);
  }
});

test('lookup reply proves exact suffix, closed source field, script and original provenance', () => {
  validateContextLookupResult(result(), input());
  for (const [reply, request] of [[result({ text: ' book' }), input()], [result({ matched_word: 'notepaper' }), input()],
    [result(), input({ context: 'notebook' })], [result(), input({ context: 'notebook-word.' })],
    [result(), input({ context: 'xnotebook.' })], [result(), input({ before: 'note' })],
    [result(), input({ before: 'Use/note' })], [result({ sources: ['draft'] }), input()],
    [result({ matched_candidate_count: 2 }), input()], [result({ sources: ['context', 'context'] }), input()],
    [result({ text: 'بوک', matched_word: 'noteبوک' }), input({ context: 'noteبوک.' })]]) {
    assert.throws(() => validateContextLookupResult(reply, request), /context_lookup_invalid_result/u);
  }
  validateContextLookupResult(result({ sources: ['style', 'draft'] }), input({ before: 'A notebook and note', context: '', style_examples: ['notebook.'] }));
  assert.throws(() => validateContextLookupResult(result({ sources: ['draft'] }), input({ before: 'notebook and note', context: '' })));
  validateContextLookupResult(result({ outcome: 'abstention', reason: 'ambiguous_matches', text: null,
    matched_word: null, matched_candidate_count: 2, sources: [] }), input());
});

test('one-shot lookup verifies direct process/executable, sends only one frame+EOF, and reaps before success', async t => {
  const { worker, calls } = setup(t);
  const first = await worker.check(input());
  assert.deepEqual(first.result, result());
  assert.deepEqual(calls[0].args, ['--context-lookup']);
  assert.deepEqual(JSON.parse(calls[0].frames[0]), input());
  assert.match(first.worker.sha256, /^[a-f0-9]{64}$/u); assert.ok(first.worker.bytes > 0);
  assert.deepEqual(first.cleanup, { process_id: calls[0].pid, reaped: true, exit_code: 0, forced: false });
  assert.equal(await processIdentity(calls[0].pid), null);
  await worker.check(input({ id: 'next' }));
  assert.equal(calls.length, 2); assert.notEqual(calls[0].pid, calls[1].pid);
});

test('invalid input and wrong binary pin cannot dispatch; cancellation during verification cannot launch later', async t => {
  const { worker, calls } = setup(t);
  await assert.rejects(worker.check(input({ expected: 'never' })));
  const pending = worker.check(input()); const rejected = assert.rejects(pending, /context_lookup_cancelled/u);
  await worker.stop(); await rejected;
  assert.equal(calls.length, 0);
  const pinned = setup(t, 'normal', { expectedWorker: { bytes: 1, sha256: '0'.repeat(64) } });
  await assert.rejects(pinned.worker.check(input()), /context_lookup_worker_changed/u);
  assert.equal(pinned.calls.length, 0);
});

test('wrong executable or premature output is refused before request data is written', async t => {
  for (const mode of ['wrong-executable', 'premature']) {
    const { worker, calls } = setup(t, mode);
    await assert.rejects(worker.check(input()), /context_lookup_/u);
    assert.equal(calls[0].frames.length, 0);
    assert.equal(await processIdentity(calls[0].pid), null);
  }
});

test('malformed, excess, foreign, incomplete and unsuccessful frames never become a successful reply', async t => {
  for (const mode of ['wrong-id', 'wrong-word', 'malformed', 'oversized', 'no-newline', 'extra', 'exit-failure']) {
    const { worker, calls } = setup(t, mode);
    await assert.rejects(worker.check(input()), /context_lookup_/u);
    assert.equal(await processIdentity(calls[0].pid), null);
  }
});

test('timeout, abort and output-without-exit await direct child cleanup, including forced termination', async t => {
  for (const mode of ['hang', 'hold', 'forced']) {
    const { worker, calls } = setup(t, mode, { timeoutMs: 1000 });
    await assert.rejects(worker.check(input()), /context_lookup_deadline/u);
    assert.equal(await processIdentity(calls[0].pid), null);
    assert.equal(worker.lastCleanup.forced, mode === 'forced');
  }
  const { worker, calls } = setup(t, 'hang');
  const controller = new AbortController();
  const pending = worker.check(input(), controller.signal); const rejected = assert.rejects(pending, /context_lookup_cancelled/u);
  while (!calls[0]?.frames.length) await delay(1);
  controller.abort(); await rejected;
  assert.equal(await processIdentity(calls[0].pid), null);
});
