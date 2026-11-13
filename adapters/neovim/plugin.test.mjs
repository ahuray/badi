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

async function fakeEditor(t, config = {}, policy = 'automatic') {
  const editor = await startNeovim({ config: { bridge: FAKE, delay_ms: 40, ...config } });
  const log = join(editor.root, 'bridge.log');
  await writeFile(log, '');
  editor.environment.FAKE_BRIDGE_LOG = log;
  // The bridge inherits Neovim's environment; set its log path there too.
  await editor.command(`let $FAKE_BRIDGE_LOG = ${JSON.stringify(log)}`);
  await editor.command(`let $FAKE_BRIDGE_POLICY = ${JSON.stringify(policy)}`);
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

test('a Visual selection has a marked revision preview, exact replacement and one native undo', { skip }, async t => {
  const editor = await fakeEditor(t, { automatic: false });
  await markdown(editor, 'iBefore I has went. After<Esc>0wvt.<C-S-Space>');
  let state = await editor.until(current => current.status.status === 'revision', 'selection revision');
  assert.equal(state.mode, 'v');
  assert.deepEqual(state.lines, ['Before I has went. After'], 'A revision preview is never buffer text');
  assert.ok(state.chunks.some(chunk => chunk[1] === 'BadiRevisionRemoved'));
  assert.match(state.preview, /\[-has /);
  await editor.send('<Tab>');
  state = await editor.until(current => current.status.status === 'applied', 'selection replacement');
  assert.deepEqual(state.lines, ['Before I went. After']);
  await editor.send('A again<Esc>u');
  await editor.until(current => current.lines[0] === 'Before I went. After', 'undo later typing');
  await editor.send('u');
  state = await editor.until(current => current.lines[0] === 'Before I has went. After', 'undo only revision');
  assert.equal(state.preview, null);
  assert.doesNotMatch(state.messages, /Before|has went|After/);
});

test('selection revision preserves reverse and exclusive Unicode ranges', { skip }, async t => {
  const editor = await fakeEditor(t, { automatic: false });
  await editor.command('setfiletype markdown');
  for (const setting of ['inclusive', 'exclusive']) {
    await editor.send('<Esc>');
    await editor.command(`set selection=${setting}`);
    await editor.command('call setline(1, "Grüße 🙂 has went.")');
    // Reverse the entire characterwise selection; exclusive selects up to
    // the period, which remains outside the replacement.
    await editor.send(setting === 'inclusive' ? '$v0<C-S-Space>' : '0v$<C-S-Space>');
    await editor.until(current => current.status.status === 'revision', `${setting} Unicode preview`);
    await editor.send('<Tab>');
    await editor.until(current => current.lines[0] === 'Grüße 🙂 went.', `${setting} exact Unicode replacement`);
    await editor.send('u');
    await editor.until(current => current.lines[0] === 'Grüße 🙂 has went.', `${setting} undo`);
  }
});

test('the entire marked revision remains visible at the bottom of a window', { skip }, async t => {
  const editor = await fakeEditor(t, { automatic: false });
  await editor.command('setfiletype markdown');
  await editor.command('call setline(1, repeat(["I has went."], 40))');
  await editor.send('20Gzb0v$<C-S-Space>');
  const state = await editor.until(current => current.preview !== null, 'bottom-row marked preview');
  assert.match(state.rows.join('\n'), /Badi grammar: I \[-has \]went\./);
  assert.match(state.rows.join('\n'), /Tab apply · Esc dismiss/);
  await editor.send('<Tab>');
  await editor.until(current => current.lines[19] === 'I went.', 'bottom-row acceptance');
  await editor.send('u');
  await editor.until(current => current.lines[19] === 'I has went.', 'bottom-row undo');
});

test('selection changes, unrelated keys and late or mismatched grants cannot apply a revision', { skip }, async t => {
  const editor = await fakeEditor(t, { automatic: false });
  await markdown(editor, 'islow has went.<Esc>0v$<C-S-Space>');
  await editor.until(current => current.status.pending, 'pending revision');
  await editor.send('h');
  await delay(500);
  assert.equal((await editor.state()).preview, null, 'The late response cannot bind to a moved selection');
  for (const prefix of ['late', 'wrong', 'revoked']) {
    await editor.send('<Esc>');
    await editor.command(`call setline(1, "${prefix} has went.")`);
    await editor.send('0v$<C-S-Space>');
    await editor.until(current => current.status.status === 'revision', `${prefix} preview`);
    await editor.send('<Tab>');
    if (prefix === 'late') await editor.send('h');
    await delay(500);
    const state = await editor.state();
    assert.equal(state.lines[0], `${prefix} has went.`, `${prefix} grant inserts nothing`);
    assert.equal(state.preview, null);
  }
  await editor.send('<Esc>');
  await editor.command('call setline(1, "I has went.")');
  await editor.send('0v$<C-S-Space>');
  await editor.until(current => current.status.status === 'revision', 'preview before unrelated key');
  await editor.send('<F12>');
  await editor.until(current => current.preview === null, 'unrelated key dismisses without editing');
});

test('selection revision is explicit and denied policy reads no selection text', { skip }, async t => {
  const editor = await fakeEditor(t, { automatic: false }, 'refused');
  await markdown(editor, 'iI has went.<Esc>0v$');
  await editor.command("lua require('badi.revision').capture = function() error('selection was read') end");
  await editor.send('<C-S-Space>');
  await editor.until(current => ['app_disabled_or_model_paused', 'app_disabled'].includes(current.status.status), 'refused selection');
  assert.ok((await editor.operations()).every(operation => operation.op !== 'revise'));
  assert.doesNotMatch((await editor.state()).messages, /selection was read/);
});

test('only no_change advances a revision cycle, and unavailable offers content-free recovery', { skip }, async t => {
  for (const reason of ['revision_unavailable', 'unsupported_language', 'invalid_output', 'provider_timeout', 'provider_error']) {
    const editor = await fakeEditor(t, { automatic: false });
    await editor.command(`let $FAKE_BRIDGE_REVISION_REASONS = ${JSON.stringify(JSON.stringify(['no_change', reason]))}`);
    await markdown(editor, 'iI has went.<Esc>0v$<C-S-Space>');
    await editor.until(current => !current.status.pending && current.status.status === 'no_change', 'unchanged grammar');
    await editor.send('<C-S-Space>');
    const state = await editor.until(current => !current.status.pending && current.status.status === reason, reason);
    assert.equal(state.preview, null);
    assert.deepEqual(state.lines, ['I has went.']);
    assert.doesNotMatch(state.messages, /I has went/);
    const status = await editor.expr('execute("BadiStatus")');
    assert.match(status, new RegExp(reason), 'The raw reason remains diagnostic');
    if (reason === 'revision_unavailable') {
      assert.match(state.messages, /Badi revisions unavailable — run badi revise/);
      assert.match(status, /run badi revise/);
    } else {
      assert.doesNotMatch(state.messages + status, /run badi revise/);
    }
    await editor.send('<C-S-Space>');
    const retry = await editor.until(current => current.status.status === 'revision', 'grammar after refusal');
    assert.match(retry.preview, /^Badi grammar:/);
    assert.deepEqual((await editor.operations()).filter(value => value.op === 'revise').map(value => value.mode),
      ['grammar', 'refine', 'grammar']);
  }
});

test('a stale unavailable revision cannot show a recovery notice', { skip }, async t => {
  const editor = await fakeEditor(t, { automatic: false });
  await editor.command(`let $FAKE_BRIDGE_REVISION_REASONS = ${JSON.stringify('["revision_unavailable"]')}`);
  await markdown(editor, 'islow has went.<Esc>0v$<C-S-Space>');
  await editor.until(current => current.status.pending, 'pending revision');
  await editor.send('h');
  await delay(500);
  const state = await editor.state();
  assert.equal(state.preview, null);
  assert.deepEqual(state.lines, ['slow has went.']);
  assert.doesNotMatch(state.messages, /run badi revise/);
});

test('revision dismissal restores Visual mappings and resizing retires its visible authority', { skip }, async t => {
  const editor = await fakeEditor(t, { automatic: false });
  await editor.command('setfiletype markdown');
  await editor.command('vmap <buffer> <Tab> :<C-U>let g:original_visual_tab = 1<CR>');
  const mapping = async () => {
    const value = JSON.parse(await editor.expr('json_encode(maparg("<Tab>", "x", 0, 1))'));
    return [value.rhs, value.mode, value.buffer, value.expr, value.noremap];
  };
  const original = await mapping();
  await editor.send('iI has went.<Esc>0v$<C-S-Space>');
  await editor.until(current => current.preview !== null, 'revision with existing Visual map');
  await editor.send('<Esc>');
  await editor.until(current => current.preview === null, 'dismissed revision');
  assert.deepEqual(await mapping(), original);
  await editor.send('0v$<C-S-Space>');
  await editor.until(current => current.preview !== null, 'revision before resize');
  await editor.command('set columns=70');
  await editor.until(current => current.preview === null, 'resize withdraws preview');
  assert.equal((await editor.state()).lines[0], 'I has went.');
  assert.deepEqual(await mapping(), original);
  await editor.send('<Tab>');
  assert.equal(await editor.expr('g:original_visual_tab'), '1', 'The restored user mapping executes');
});

test('a pause shows inline grey text; Ctrl+Right inserts it all as one undo step', { skip }, async t => {
  const editor = await fakeEditor(t);
  await markdown(editor, 'iDear team,<CR>thank you');
  let state = await editor.until(current => current.preview === ' for your time', 'preview');
  assert.deepEqual(state.chunks, [[' for', 'BadiSuggestionWord'], [' your time', 'BadiSuggestion']],
    'What Tab takes is drawn a step brighter');
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

test('a correction replaces exactly the misspelled suffix including a typed space in one undo step', { skip }, async t => {
  const editor = await fakeEditor(t);
  await markdown(editor, 'iGrüße, die adress ');
  let state = await editor.until(current => current.preview === ' adress → address', 'correction preview');
  assert.deepEqual(state.chunks, [[' adress → address', 'BadiSuggestion']], 'A correction is drawn whole');
  assert.deepEqual(state.lines, ['Grüße, die adress '], 'The correction preview is not buffer text');
  assert.equal(state.screen, 'Grüße, die adress  adress → address');
  await editor.send('<Tab>');
  state = await editor.until(current => current.lines[0] === 'Grüße, die address ', 'correction acceptance');
  assert.deepEqual(state.cursor, [1, Buffer.byteLength('Grüße, die address ')]);
  assert.equal(state.preview, null);
  assert.equal(state.status.status, 'applied');
  await editor.send('today<Esc>');
  await editor.until(current => current.lines[0] === 'Grüße, die address today', 'typing after correction');
  await editor.send('u');
  state = await editor.until(current => current.lines[0] === 'Grüße, die address ', 'typing after removed alone');
  await editor.send('u');
  state = await editor.until(current => current.lines[0] === 'Grüße, die adress ', 'correction undone');
  const operations = await editor.operations();
  assert.deepEqual(operations.filter(entry => entry.op === 'accept' || entry.op === 'result')
    .map(entry => entry.acceptance ?? entry.status), ['word', 'applied']);
  assert.doesNotMatch(state.messages, /adress|address/, 'No corrected text reaches messages');
});

// Editor-range coverage only: the real broker proposes corrections only
// when the request has no text after the caret (for Neovim, the caret line).
test('Ctrl+Right accepts a scripted correction without touching text after the caret', { skip }, async t => {
  const editor = await fakeEditor(t);
  await markdown(editor, 'iPlease check the adress please');
  await editor.send('<Left><Left><Left><Left><Left><Left>');
  await editor.command('lua require("badi").request(true)');
  const shown = await editor.until(current => current.preview === ' adress → address', 'mid-line correction preview');
  assert.equal(shown.screen, 'Please check the adress  adress → addressplease');
  await editor.send('<C-Right>');
  const state = await editor.until(current => current.lines[0] === 'Please check the address please', 'mid-line acceptance');
  assert.deepEqual(state.cursor, [1, Buffer.byteLength('Please check the address ')]);
  assert.equal(state.status.status, 'applied');
  const operations = await editor.operations();
  assert.deepEqual(operations.filter(entry => entry.op === 'accept' || entry.op === 'result')
    .map(entry => entry.acceptance ?? entry.status), ['all', 'applied']);
});

test('a correction revoked while its grant is in flight inserts nothing', { skip }, async t => {
  const editor = await fakeEditor(t);
  await markdown(editor, 'iPlease check the revoked adress ');
  await editor.until(current => current.preview === ' adress → address', 'correction preview');
  await editor.send('<Tab>');
  const state = await editor.until(current => current.preview === null, 'revocation withdraws');
  assert.deepEqual(state.lines, ['Please check the revoked adress ']);
  const operations = await editor.operations();
  assert.deepEqual(operations.filter(entry => entry.op === 'result').map(entry => entry.status), ['stale']);
});

test('Tab while an alternative is in flight keeps the preview and sends no acceptance', { skip }, async t => {
  const editor = await fakeEditor(t);
  await markdown(editor, 'ithank you');
  await editor.until(current => current.preview === ' for your time', 'preview');
  await editor.send('<C-S-Space>');
  await delay(50);
  await editor.send('<Tab>');
  const state = await editor.until(current => current.preview === ' 1.so  2.again', 'alternative list after Tab');
  assert.deepEqual(state.lines, ['thank you']);
  const operations = await editor.operations();
  assert.deepEqual(operations.filter(entry => entry.op === 'accept'), []);
});

test('a different replacement grant is refused without changing the buffer', { skip }, async t => {
  const editor = await fakeEditor(t);
  await markdown(editor, 'iPlease check the other adress ');
  await editor.until(current => current.preview === ' adress → address', 'correction preview');
  await editor.send('<Tab>');
  const state = await editor.until(current => current.preview === null && current.status.status === 'stale', 'stale grant');
  assert.deepEqual(state.lines, ['Please check the other adress ']);
  const operations = await editor.operations();
  assert.deepEqual(operations.filter(entry => entry.op === 'result').map(entry => entry.status), ['stale']);
});

test('a stale correction grant inserts nothing', { skip }, async t => {
  const editor = await fakeEditor(t);
  await markdown(editor, 'iPlease check the slow adress ');
  await editor.until(current => current.preview === ' adress → address', 'correction preview');
  await editor.send('<Tab>');
  await delay(50);
  await editor.send('!');
  await delay(600);
  const state = await editor.state();
  assert.deepEqual(state.lines, ['Please check the slow adress !']);
  const operations = await editor.operations();
  assert.deepEqual(operations.filter(entry => entry.op === 'result').map(entry => entry.status), ['stale']);
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

test('the request key on a shown suggestion shows one that starts with another word', { skip }, async t => {
  const editor = await fakeEditor(t);
  await markdown(editor, 'ithank you');
  await editor.until(current => current.preview === ' for your time', 'preview');
  await editor.send('<C-S-Space>');
  let state = await editor.until(current => current.preview === ' 1.so  2.again', 'a numbered list');
  assert.equal(state.screen, 'thank you 1.so  2.again');
  await editor.send('1');
  state = await editor.until(current => current.preview === ' so much', 'a picked alternative');
  assert.deepEqual(state.chunks, [[' so', 'BadiSuggestionWord'], [' much', 'BadiSuggestion']]);
  assert.equal(state.screen, 'thank you so much');
  await editor.send('<C-S-Space>');
  await editor.until(current => current.preview === ' 1.again', 'the remaining list');
  await editor.send('1');
  await editor.until(current => current.preview === ' again', 'another picked alternative');
  await editor.send('<Tab>');
  state = await editor.until(current => current.lines[0] === 'thank you again', 'acceptance');
  const operations = await editor.operations();
  assert.deepEqual(operations.filter(entry => ['alternative', 'pick', 'accept'].includes(entry.op)).map(entry => entry.op),
    ['alternative', 'pick', 'alternative', 'pick', 'accept']);
  assert.ok(!operations.some(entry => entry.op === 'suggest' && entry.explicit), 'The key replaced the shown one');

  // Without another one, the shown suggestion is withdrawn.
  await editor.send(' thank you for');
  await editor.until(current => current.preview === ' your time', 'a suggestion without alternatives');
  await editor.send('<C-S-Space>');
  state = await editor.until(current => current.preview === null && current.status.status === 'no_suggestion',
    'withdrawn');
  assert.deepEqual(state.lines, ['thank you again thank you for']);
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
  // Once the bridge has reported its policy, a pause sends the text at once.
  await markdown(editor, 'i');
  await editor.until(current => current.status.policy === 'automatic', 'policy');
  await editor.send('please be slow');
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

test('a paused broker or a blocked Neovim gets no buffer text, and a grant resumes', { skip }, async t => {
  const editor = await fakeEditor(t, {}, 'refused');
  await markdown(editor, 'ithank you');
  await editor.until(current => current.status.policy === 'refused', 'refused policy');
  await delay(200);
  const output = await editor.command('lua require("badi").request(true)');
  assert.match(output, /Badi: app disabled or model paused/u);
  let state = await editor.state();
  assert.equal(state.status.status, 'app_disabled_or_model_paused');
  assert.equal(state.preview, null);
  assert.deepEqual((await editor.operations()).filter(entry => entry.op === 'suggest'), [],
    'Neither a typing pause nor an explicit request sent text');
  await editor.command(`call chansend(${await bridgeJob(editor)}, '{"op":"policy","allowed":true,"automatic":true}' .. "\\n")`);
  await editor.until(current => current.status.policy === 'automatic', 'granted policy');
  await editor.send(' <BS>');
  state = await editor.until(current => current.preview === ' for your time', 'preview after the grant');
  await editor.command(`call chansend(${await bridgeJob(editor)}, '{"op":"policy","allowed":false}' .. "\\n")`);
  await editor.until(current => current.status.policy === 'refused', 'paused again');
  const suggested = (await editor.operations()).filter(entry => entry.op === 'suggest').length;
  await editor.send('<CR>thank you');
  await delay(200);
  assert.equal((await editor.operations()).filter(entry => entry.op === 'suggest').length, suggested,
    'A pause after the broker paused sends nothing');
});

test('manual policy reads the buffer only for an explicit request', { skip }, async t => {
  const editor = await fakeEditor(t, {}, 'manual');
  await markdown(editor, 'ithank you');
  await editor.until(current => current.status.policy === 'manual', 'manual policy');
  await delay(200);
  assert.deepEqual((await editor.operations()).filter(entry => entry.op === 'suggest'), []);
  assert.equal((await editor.state()).status.status, 'manual_required');
  await editor.command('lua require("badi").request(true)');
  await editor.until(current => current.preview === ' for your time', 'explicit preview');
  assert.deepEqual((await editor.operations()).filter(entry => entry.op === 'suggest').map(entry => entry.explicit), [true]);
});

test('a bridge that has not reported its policy is asked before any text is sent', { skip }, async t => {
  const editor = await fakeEditor(t, {}, 'silent');
  await markdown(editor, 'ithank you');
  await editor.until(current => current.preview === ' for your time', 'preview');
  const operations = (await editor.operations()).map(entry => entry.op);
  assert.ok(operations.indexOf('status') >= 0 && operations.indexOf('status') < operations.indexOf('suggest'), operations.join());
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
