// Plugin behavior against a scripted bridge: binding, display, acceptance,
// undo, fall-through keys, denial and disconnect. The real-broker lane is
// broker.test.mjs.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { delay, neovimSkipReason, PLUGIN, startNeovim } from './tests/harness.mjs';

const skip = neovimSkipReason();
const FAKE = join(PLUGIN, 'tests/fake-bridge.mjs');

async function fakeEditor(t, config = {}) {
  const editor = await startNeovim({ config: { bridge: FAKE, delay_ms: 40, ...config } });
  const log = join(editor.root, 'bridge.log');
  await writeFile(log, '');
  editor.environment.FAKE_BRIDGE_LOG = log;
  // The bridge inherits Neovim's environment; set its log path there too.
  await editor.command(`let $FAKE_BRIDGE_LOG = ${JSON.stringify(log)}`);
  editor.operations = async () => (await readFile(log, 'utf8')).trim().split('\n').filter(Boolean).map(line => JSON.parse(line));
  t.after(() => editor.close());
  return editor;
}

test('context helpers keep UTF-8 boundaries, bounds and type-through exact', { skip }, async () => {
  const { stdout } = await promisify(execFile)('nvim', ['--headless', '--clean', '-l', join(PLUGIN, 'tests/context_spec.lua')],
    { encoding: 'utf8', timeout: 10000 });
  assert.equal(stdout, 'context helpers ok\n');
});

async function markdown(editor, keys) {
  await editor.command('setfiletype markdown');
  await editor.send(keys);
}

test('a pause shows inline grey text; Ctrl+Right inserts it all as one undo step', { skip }, async t => {
  const editor = await fakeEditor(t);
  await markdown(editor, 'iDear team,<CR>thank you');
  let state = await editor.until(current => current.preview === ' for your time', 'preview');
  assert.deepEqual(state.lines, ['Dear team,', 'thank you'], 'The preview is not buffer text');
  assert.equal(state.screen, 'thank you for your time', 'Inline virtual text follows the caret');
  assert.equal(state.mode, 'i');
  await editor.send('<C-Right>');
  state = await editor.until(current => current.lines[1] === 'thank you for your time', 'acceptance');
  assert.deepEqual(state.cursor, [2, 23]);
  assert.equal(state.preview, null);
  assert.equal(state.status.status, 'applied');
  await editor.send(' again<Esc>');
  await editor.until(current => current.lines[1] === 'thank you for your time again', 'typing after');
  await editor.send('u');
  state = await editor.until(current => current.lines[1] === 'thank you for your time', 'first undo');
  await editor.send('u');
  state = await editor.until(current => current.lines[1] === 'thank you', 'second undo');
  assert.deepEqual(state.lines, ['Dear team,', 'thank you'], 'Undo removes the acceptance alone');
  const operations = await editor.operations();
  assert.deepEqual(operations.filter(entry => entry.op === 'accept' || entry.op === 'result')
    .map(entry => entry.acceptance ?? entry.status), ['all', 'applied']);
  assert.ok(operations.every(entry => entry.explicit !== true), 'Only automatic requests were made');
  assert.doesNotMatch(state.messages, /thank|Dear|your time/, 'No typed or suggested text reaches messages');
});

test('another window on the same buffer never shows the preview', { skip }, async t => {
  const editor = await fakeEditor(t);
  await editor.command('setfiletype markdown | split');
  await editor.send('ithank you');
  const state = await editor.until(current => current.preview === ' for your time', 'preview');
  assert.equal(state.rows.filter(row => row === 'thank you').length, 1, 'The other window shows the text alone');
  assert.equal(state.rows.filter(row => row === 'thank you for your time').length, 1);
});

test('Tab takes one word and typing through keeps the rest', { skip }, async t => {
  const editor = await fakeEditor(t, { delay_ms: 1500 });
  await markdown(editor, 'ithank you');
  await editor.command('lua require("badi").request(false)');
  await editor.until(current => current.preview === ' for your time', 'preview');
  const accepted = performance.now();
  await editor.send('<Tab>');
  const state = await editor.until(current => current.preview === ' your time', 'carried remainder');
  assert.deepEqual(state.lines, ['thank you for']);
  assert.deepEqual(state.cursor, [1, 13]);
  assert.ok(performance.now() - accepted < 1000, 'The remainder is asked for at once, not after the 1.5 s pause');
  const operations = await editor.operations();
  assert.deepEqual(operations.filter(entry => entry.op === 'result').map(entry => entry.status), ['applied']);
});

test('multibyte text before the caret keeps byte-exact placement and undo', { skip }, async t => {
  const editor = await fakeEditor(t);
  await markdown(editor, 'iمتن اول<CR>Grüße und سلام ');
  let state = await editor.until(current => current.preview === 'Welt bleibt', 'preview');
  // 'arabicshape' draws Persian in presentation forms; check the Latin ends.
  assert.match(state.screen, /^Grüße und .+ Welt bleibt$/u);
  await editor.send('<C-Right>');
  state = await editor.until(current => current.lines[1] === 'Grüße und سلام Welt bleibt', 'acceptance');
  assert.deepEqual(state.cursor, [2, Buffer.byteLength('Grüße und سلام Welt bleibt')]);
  await editor.send('<Esc>u');
  state = await editor.until(current => current.lines[1] === 'Grüße und سلام ', 'undo');
  assert.deepEqual(state.lines, ['متن اول', 'Grüße und سلام ']);
});

test('a result for text that changed in flight is never shown', { skip }, async t => {
  const editor = await fakeEditor(t);
  await markdown(editor, 'iplease be slow');
  await delay(100);
  await editor.send('er');
  await delay(700);
  const state = await editor.state();
  assert.equal(state.preview, null);
  assert.deepEqual(state.lines, ['please be slower']);
  const operations = await editor.operations();
  assert.ok(operations.some(entry => entry.op === 'cancel'), 'The superseded request is cancelled');
});

test('a grant that arrives after an edit is reported stale and inserts nothing', { skip }, async t => {
  const editor = await fakeEditor(t);
  await markdown(editor, 'iI will grant later');
  await editor.until(current => current.preview === ' arrives late', 'preview');
  await editor.send('<Tab>');
  await delay(50);
  await editor.send('!');
  await delay(600);
  const state = await editor.state();
  assert.deepEqual(state.lines, ['I will grant later!']);
  const operations = await editor.operations();
  assert.deepEqual(operations.filter(entry => entry.op === 'result').map(entry => entry.status), ['stale']);
});

test('Tab and Ctrl+Right stay the user’s keys while nothing is shown', { skip }, async t => {
  const editor = await fakeEditor(t);
  await editor.command('inoremap <Tab> [tab]');
  await markdown(editor, 'ihello<Tab>');
  let state = await editor.until(current => current.lines[0] === 'hello[tab]', 'global Tab mapping');
  await editor.command('inoremap <buffer> <Tab> [buffer-tab]');
  await editor.send('<CR>thank you');
  await editor.until(current => current.preview === ' for your time', 'preview');
  await editor.send('<Left>');
  state = await editor.until(current => current.preview === null, 'caret move withdraws');
  await editor.send('<End><Tab>');
  state = await editor.until(current => current.lines[1] === 'thank you[buffer-tab]', 'restored buffer mapping');
  assert.match(await editor.expr('maparg("<Tab>", "i")'), /\[buffer-tab\]/);
});

test('the completion menu keeps Tab and suppresses previews', { skip }, async t => {
  const editor = await fakeEditor(t);
  await editor.command('inoremap <expr> <Tab> pumvisible() ? "\\<C-n>" : "\\<Tab>"');
  await markdown(editor, 'ithankful thanks<CR>tha<C-n>');
  let state = await editor.until(current => current.mode === 'ic' || current.mode === 'ix', 'completion menu');
  await editor.send('<Tab>');
  await delay(200);
  state = await editor.state();
  assert.equal(state.preview, null);
  assert.ok(['thankful', 'thanks'].includes(state.lines[1]), state.lines[1]);
  const operations = await editor.operations();
  assert.equal(operations.filter(entry => entry.op === 'suggest').length, 0, 'No request while completing');
});

test('read-only, special, non-prose and sensitive buffers are never read', { skip }, async t => {
  const editor = await fakeEditor(t);
  const cases = [
    ['setfiletype markdown | setlocal nomodifiable', 'read_only'],
    ['setfiletype markdown | setlocal readonly', 'read_only'],
    ['setfiletype markdown | setlocal buftype=nofile', 'special_buffer'],
    ['setfiletype lua', 'filetype_not_prose'],
    ['file /dev/shm/pass.secret.txt | setfiletype text', 'sensitive_buffer'],
    ['setfiletype markdown | set paste', 'paste_mode'],
    ['setfiletype markdown | let b:badi_disable = v:true', 'disabled'],
  ];
  for (const [setup, reason] of cases) {
    await editor.command('enew!');
    await editor.command('set nopaste');
    // Read-only buffers refuse typing, so the text is there first.
    await editor.command('call setline(1, "thank you")');
    await editor.command(setup);
    // As if inserting: Neovim itself refuses Insert mode in a nomodifiable buffer.
    const facts = '{mode = "i", focused = true}';
    assert.equal((await editor.expr(
      `luaeval('require("badi.context").blocked_reason(0, {filetypes = {"markdown", "text"}}, ${facts})')`)).trim(),
      reason, setup);
    await editor.send('A');
    await delay(120);
    const output = await editor.command('lua require("badi").request(true)');
    const state = await editor.state();
    const live = state.mode === 'i' ? reason : 'not_insert_mode';
    assert.equal(state.status.blocked, live, setup);
    assert.equal(state.preview, null, setup);
    assert.match(output, new RegExp(`Badi: ${live.replaceAll('_', ' ')}`), setup);
    await editor.send('<Esc>');
  }
  await editor.command('enew! | terminal true');
  await editor.command('startinsert');
  await editor.command('lua require("badi").request(true)');
  assert.equal(((await editor.operations()).filter(entry => entry.op === 'suggest')).length, 0,
    'No denied buffer reached the bridge');
});

test('Escape dismisses, expiry withdraws, and a bridge exit leaves no stuck preview', { skip }, async t => {
  const editor = await fakeEditor(t);
  await markdown(editor, 'ithank you');
  await editor.until(current => current.preview === ' for your time', 'preview');
  await editor.send('<Esc>');
  await editor.until(current => current.preview === null && current.mode === 'n', 'dismissed');
  assert.ok((await editor.operations()).some(entry => entry.op === 'dismiss'));
  assert.notEqual(await editor.expr('get(maparg("<Tab>", "i", 0, 1), "desc", "")'), 'Badi',
    'The preview mapping is gone');

  await editor.send('o<CR>thank you');
  await editor.until(current => current.preview === ' for your time', 'second preview');
  await editor.command(`call chansend(${await bridgeJob(editor)}, '{"op":"expire"}' .. "\\n")`);
  await editor.until(current => current.preview === null, 'expiry');

  await editor.send(' crash');
  let state = await editor.until(current => current.status.status === 'bridge_exited', 'bridge exit');
  assert.equal(state.preview, null);
  assert.equal(state.status.bridge, false);
  await editor.send('<Tab>');
  // Markdown's recommended style expands Tab to spaces.
  state = await editor.until(current => /crash\s+$/u.test(current.lines.at(-1)), 'plain Tab after exit');
  await editor.send('<C-u>thank you');
  state = await editor.until(current => current.preview === ' for your time', 'restarted bridge');
  assert.equal(state.status.bridge, true);
});

// The fake bridge accepts an 'expire' line on stdin to emulate the shared
// client's expiry timer, so tests reach the job through its channel id.
async function bridgeJob(editor) {
  const channels = JSON.parse(await editor.expr('json_encode(nvim_list_chans())'));
  const job = channels.find(channel => channel.stream === 'job');
  assert.ok(job, 'bridge job');
  return job.id;
}
