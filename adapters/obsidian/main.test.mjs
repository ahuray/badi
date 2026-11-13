import { test } from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { readFile } from 'node:fs/promises';
import { runInNewContext } from 'node:vm';
import { transform } from 'esbuild';
import { writingLanguage } from '../shared/writing-language.mjs';
import { validCorrection } from '../shared/text-safety.mjs';
import { boundedContext } from '../shared/broker-client.mjs';
import * as revisions from '../shared/revision.mjs';

const source = (await transform(await readFile(new URL('./main.mjs', import.meta.url), 'utf8'),
  { format: 'cjs', target: 'es2022' })).code;
const revisionSource = (await transform(await readFile(new URL('./revision.mjs', import.meta.url), 'utf8'),
  { format: 'cjs', target: 'es2022' })).code;
const settle = async () => { for (let turn = 0; turn < 12; turn++) await Promise.resolve(); };

async function harness(context) {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const clients = [];
  class Client extends EventEmitter {
    constructor() { super(); clients.push(this); }
    connected = true;
    allowed = true;
    automaticAllowed = true;
    acceptWord = ' for';
    requests = [];
    modes = [];
    reports = [];
    revisions = [];
    dismissed = 0;
    connect() { this.emit('state', 'ready'); return Promise.resolve(true); }
    dismiss() { this.dismissed++; }
    suggest(before, options) { this.requests.push({ before, ...options }); return Promise.resolve(' for your time'); }
    authorize(mode) { this.modes.push(mode); return Promise.resolve({ text: mode === 'word' ? ' for' : ' for your time' }); }
    revise(request) {
      this.emit('clear');
      this.revisions.push(request);
      this.revisionText = request.selected.replace('has went', 'went');
      if (this.revisionText === request.selected) this.emit('state', 'no_change');
      return Promise.resolve(this.revisionText === request.selected ? null
        : { text: this.revisionText, original: request.selected, mode: request.mode });
    }
    authorizeRevision() { return Promise.resolve({ text: this.revisionText, replaceSelection: this.revisions.at(-1).selection }); }
    report(grant, status) { this.reports.push({ text: grant.text, status }); }
    cancel() { this.emit('clear'); }
    close() {}
  }
  class Plugin {
    commands = [];
    extensions = [];
    loadData() { return Promise.resolve({ language: 'en' }); }
    saveData() { return Promise.resolve(); }
    addStatusBarItem() { return { text: '', setText(text) { this.text = text; } }; }
    registerEditorExtension(value) { this.extensions.push(value); this.extension = value; }
    addCommand(value) { this.commands.push(value); }
  }
  const modules = {
    obsidian: { Plugin, Notice: class {} },
    '@codemirror/view': { WidgetType: class {}, ViewPlugin: { fromClass: value => value },
      tooltips: value => value,
      showTooltip: { from: field => field }, getTooltip: (view, tooltip) => view.tooltip?.value === tooltip ? view.tooltip : null,
      Decoration: { none: [], set: value => value, widget: value => ({ range: position => ({ ...value, position }) }) } },
    '@codemirror/state': { Prec: { highest: value => value }, Transaction: { userEvent: { of: value => ({ userEvent: value }) } },
      StateEffect: { define: () => ({ of: value => ({ value }) }) }, StateField: { define: value => value } },
    '@codemirror/commands': { isolateHistory: { of: value => ({ isolateHistory: value }) } },
    '@codemirror/autocomplete': { completionStatus: state => state.completion ?? null },
    '../shared/broker-client.mjs': { boundedContext },
    '../shared/recovering-client.mjs': { RecoveringBrokerClient: Client },
    '../shared/activity.mjs': { recordActivity() {} },
    '../shared/writing-language.mjs': { writingLanguage },
    '../shared/text-safety.mjs': { validCorrection },
    '../shared/revision.mjs': revisions,
  };
  const revisionModule = { exports: {} };
  runInNewContext(revisionSource, { module: revisionModule, exports: revisionModule.exports, require: id => modules[id] });
  modules['./revision.mjs'] = revisionModule.exports;
  const module = { exports: {} };
  runInNewContext(source, { module, exports: module.exports, require: id => {
    assert.ok(Object.hasOwn(modules, id), `Unexpected dependency: ${id}`);
    return modules[id];
  }, setTimeout, clearTimeout, queueMicrotask, navigator: { language: 'en-US' } });
  const plugin = new module.exports.default();
  await plugin.onload();
  const state = (text, head = text.length, anchor = head) => ({
    // Like CodeMirror's Text: sliceString has no default start, so a
    // whole-document read must use toString().
    doc: { length: text.length, toString: () => text,
      sliceString: (from, to = text.length) => (typeof from === 'number' ? text.slice(from, to) : ''),
      lineAt: pos => ({ from: text.lastIndexOf('\n', pos - 1) + 1, to: text.indexOf('\n', pos) < 0 ? text.length : text.indexOf('\n', pos) }) },
    selection: { ranges: [{}], main: { head, anchor, from: Math.min(head, anchor), to: Math.max(head, anchor), empty: anchor === head },
      eq(other) { return other.main.head === head && other.main.anchor === anchor && other.ranges.length === 1; } },
    readOnly: false,
  });
  const editor = (text, hasFocus = true) => {
    let controller;
    const domListeners = new Map();
    const rect = { left: 20, top: 30, right: 620, bottom: 130, width: 600, height: 100 };
    const document = { hasFocus: () => view.hasFocus, documentElement: { clientWidth: 1200, clientHeight: 900 },
      createElement: () => ({ children: [], isConnected: true, clientWidth: 600, scrollWidth: 600, clientHeight: 100, scrollHeight: 100,
        getBoundingClientRect: () => ({ ...rect }), setAttribute() {}, append(...children) { this.children.push(...children); } }) };
    const view = { hasFocus, composing: false, state: state(text), transactions: [],
      dom: { ownerDocument: document }, coordsAtPos: () => ({ top: 100, bottom: 120, left: 30, right: 30 }),
      scrollDOM: { getBoundingClientRect: () => ({ top: 0, bottom: 900, left: 0, right: 1200 }) },
      contentDOM: {
        addEventListener(type, listener, options) { assert.equal(options.passive, true); domListeners.set(type, listener); },
        removeEventListener(type, listener) { if (domListeners.get(type) === listener) domListeners.delete(type); },
      },
      dispatch(transaction) {
        if (transaction.effects) {
          assert.ok(plugin.extensions.includes(revisionModule.exports.revisionTooltip), 'Revision tooltip field is registered');
          const value = transaction.effects.value;
          this.tooltip = value ? { value, ...value.create(this) } : null;
        }
        if (!transaction.changes) return;
        this.transactions.push(transaction);
        const before = this.state;
        this.state = state(before.doc.sliceString(0, transaction.changes.from) + transaction.changes.insert +
          before.doc.sliceString(transaction.changes.to), transaction.selection.anchor);
        controller.update({ docChanged: true, state: this.state, startState: before });
      },
    };
    controller = new plugin.extension(view);
    context.after(() => controller.destroy());
    return { controller, view, domListeners };
  };
  const { controller, view, domListeners } = editor('thank you');
  const tick = async ms => { context.mock.timers.tick(ms); await settle(); };
  await settle();
  context.after(() => context.mock.timers.reset());
  return { plugin, controller, view, client: controller.client, clients, editor, tick, state, domListeners,
    revisionHelpers: revisionModule.exports };
}

test('Obsidian revises a reverse Unicode selection with marked text and one isolated whole-range transaction', async context => {
  const { controller, view, client, state } = await harness(context);
  const original = '🙂 é Before I has went. After';
  const from = original.indexOf('I has'), to = original.indexOf('. After') + 1;
  view.state = state(original, from, to);
  await controller.revise(); await settle();
  assert.equal(view.state.doc.toString(), original);
  assert.deepEqual(JSON.parse(JSON.stringify(client.revisions[0].selection)), {
    anchor: [...original.slice(0, to)].length, head: [...original.slice(0, from)].length, unit: 'unicode_scalar_values' });
  const diff = view.tooltip.dom.children[1];
  assert.ok(diff.children.some(part => part.textContent === '[-has ]'));
  assert.equal(controller.key({ isTrusted: true, key: 'Tab' }), true);
  await settle();
  assert.equal(view.state.doc.toString(), original.replace('has went', 'went'));
  assert.equal(view.transactions.length, 1);
  assert.equal(view.transactions[0].changes.from, from);
  assert.equal(view.transactions[0].changes.to, to);
  assert.equal(view.transactions[0].annotations[1].isolateHistory, 'full');
  assert.deepEqual(client.reports, [{ text: 'I went.', status: 'applied' }]);
});

test('Obsidian selection capture refuses multiline, multiple ranges, unsafe text and split surrogate endpoints', async context => {
  const { revisionHelpers, state } = await harness(context);
  for (const [text, from, to] of [['one\ntwo', 0, 7], ['🙂word', 1, 6], ['word🙂', 0, 5],
    [' ', 0, 1], ['a\u202eb', 0, 3], ['a'.repeat(601), 0, 601]]) {
    assert.equal(revisionHelpers.captureRevision(state(text, to, from)), null, text);
  }
  const multiple = state('word', 4, 0); multiple.selection.ranges.push({});
  assert.equal(revisionHelpers.captureRevision(multiple), null);
  const accepted = revisionHelpers.captureRevision(state('a'.repeat(600), 600, 0));
  assert.equal(accepted.selected.length, 600);
});

test('Obsidian checks policy before reading a selection and keeps revision explicit', async context => {
  const { controller, view, client, state, tick } = await harness(context);
  view.state = state('I has went.', 11, 0);
  await tick(500);
  assert.equal(client.revisions.length, 0);
  client.allowed = false;
  view.state.doc.sliceString = () => { throw new Error('denied text read'); };
  await controller.revise();
  assert.equal(client.revisions.length, 0);
});

test('Obsidian cycles past no-change and keeps repeated synonyms until dismissal', async context => {
  const { controller, view, client, state, domListeners } = await harness(context);
  view.state = state('big', 3, 0);
  const modes = [];
  client.revise = async request => {
    modes.push(request.mode);
    if (request.mode !== 'synonyms') client.emit('state', 'no_change');
    return request.mode === 'synonyms' ? { original: 'big', mode: request.mode, text: modes.length === 3 ? 'large' : 'huge' } : null;
  };
  for (let index = 0; index < 4; index++) { await controller.revise(); await settle(); }
  assert.deepEqual(modes, ['grammar', 'refine', 'synonyms', 'synonyms']);
  assert.equal(controller.text, 'huge');
  domListeners.get('keydown')({ key: 'Escape', isTrusted: true });
  await controller.revise();
  assert.equal(modes.at(-1), 'grammar', 'dismissal ends the cycle');
});

test('Obsidian preserves revision failure reasons without claiming no change or advancing the cycle', async context => {
  const { plugin, controller, view, client, state } = await harness(context);
  for (const reason of ['revision_unavailable', 'unsupported_language', 'invalid_output', 'provider_timeout', 'provider_error']) {
    view.state = state('I has went.', 11, 0);
    client.revise = async () => { client.emit('state', reason); return null; };
    await controller.revise();
    assert.equal(plugin.status.text, `Badi · ${reason.replaceAll('_', ' ')}${reason === 'revision_unavailable' ? ' · run badi revise' : ''}`);
    assert.equal(controller.revisionReason, reason, 'The original broker reason is retained');
    assert.equal(controller.revisionCycle, null);
    assert.equal(controller.text, null);
    assert.equal(view.state.doc.toString(), 'I has went.');
  }
  client.revise = async () => { throw new Error('Transport failure'); };
  await controller.revise();
  assert.equal(plugin.status.text, 'Badi · revision request failed · retry');
  assert.equal(controller.revisionCycle, null);
  assert.equal(controller.text, null);
  assert.equal(view.state.doc.toString(), 'I has went.');
});

test('Obsidian refuses clipped or hidden marked revisions before authorization', async context => {
  const { controller, view, client, state } = await harness(context);
  view.state = state('I has went.', 11, 0);
  let authorizations = 0;
  client.authorizeRevision = async () => { authorizations++; return {}; };
  await controller.revise(); await settle();
  view.tooltip.dom.scrollHeight = 101;
  await controller.acceptRevision();
  assert.equal(authorizations, 0);
  await controller.revise(); await settle();
  view.scrollDOM.getBoundingClientRect = () => ({ top: 80, bottom: 900, left: 0, right: 1200 });
  await controller.acceptRevision();
  assert.equal(authorizations, 0, 'the host toolbar clips a tooltip above the editor pane');
  assert.equal(view.state.doc.toString(), 'I has went.');
  await controller.revise(); await settle();
  view.tooltip.dom.getBoundingClientRect = () => ({ left: 0, right: 600, top: -10000, bottom: -9900, width: 600, height: 100 });
  await controller.acceptRevision();
  assert.equal(authorizations, 0);
});

test('Obsidian selection changes, focus loss, revocation and wrong grants cannot replace text', async context => {
  const { controller, view, client, state } = await harness(context);
  for (const change of ['selection', 'document', 'focus', 'clear', 'range', 'text', 'clipped']) {
    view.hasFocus = true;
    view.state = state('I has went.', 11, 0);
    await controller.revise(); await settle();
    let resolve;
    client.authorizeRevision = () => new Promise(done => { resolve = done; });
    const accepting = controller.acceptRevision();
    const range = client.revisions.at(-1).selection;
    if (change === 'selection') view.state = { ...view.state, selection: state('I has went.', 10, 0).selection };
    if (change === 'document') view.state = state('I has went?', 11, 0);
    if (change === 'focus') view.hasFocus = false;
    if (change === 'clear') client.emit('clear');
    if (change === 'clipped') view.tooltip.dom.scrollWidth = 601;
    resolve({ text: change === 'text' ? 'other' : 'I went.', replaceSelection: change === 'range' ? { ...range, head: 10 } : range });
    await accepting;
    assert.equal(view.transactions.length, 0, change);
  }
});

test('Obsidian automatically requests, accepts one word or all, and isolates each undo transaction', async context => {
  const { controller, view, client, tick } = await harness(context);
  await tick(250);
  assert.deepEqual(client.requests, [{ before: 'thank you', language: 'en', explicit: false, after: '' }]);
  assert.equal(controller.text, ' for your time');
  assert.equal(controller.decorations[0].widget.word, ' for', 'The preview brightens what Tab takes');
  assert.equal(controller.key({ isTrusted: true, key: 'Tab' }), true);
  await settle();
  assert.equal(view.state.doc.toString(), 'thank you for');
  assert.deepEqual(client.modes, ['word']);
  assert.deepEqual(client.reports, [{ text: ' for', status: 'applied' }]);
  assert.equal(view.transactions[0].annotations[1].isolateHistory, 'full');
  await tick(250);
  assert.equal(client.requests.length, 2, 'The remainder requires a fresh request after insertion');
  assert.equal(controller.key({ isTrusted: true, key: 'ArrowRight', ctrlKey: true }), true);
  await settle();
  assert.deepEqual(client.modes, ['word', 'all']);
  assert.equal(view.state.doc.toString(), 'thank you for for your time');
  assert.equal(controller.key({ isTrusted: true, key: 'Tab', shiftKey: true }), false);
});

test('Obsidian Ctrl+Shift+Space lists other first words and a digit picks one', async context => {
  const { controller, view, client, tick } = await harness(context);
  await tick(250);
  assert.equal(controller.text, ' for your time');
  client.alternative = async () => ({ words: ['so', 'again'] });
  const picks = [];
  client.pickAlternative = async word => {
    picks.push(word);
    client.acceptWord = ' again';
    return ' again soon';
  };
  const chord = { isTrusted: true, key: ' ', ctrlKey: true, shiftKey: true };
  assert.equal(controller.key(chord), true);
  await settle();
  assert.equal(controller.text, null);
  assert.deepEqual(controller.listWords, ['so', 'again']);
  assert.equal(controller.decorations[0].widget.text, ' 1.so  2.again');
  assert.equal(controller.decorations[0].widget.list, true);
  assert.equal(controller.key({ isTrusted: true, key: '3' }), false, 'a digit past the list types normally');
  assert.equal(controller.key({ isTrusted: true, key: '2' }), true);
  await settle();
  assert.deepEqual(picks, ['again']);
  assert.equal(controller.text, ' again soon');
  assert.equal(controller.listWords, null);
  client.authorize = mode => Promise.resolve({ text: mode === 'word' ? ' again' : ' again soon' });
  assert.equal(controller.key({ isTrusted: true, key: 'Tab' }), true);
  await settle();
  assert.equal(view.state.doc.toString(), 'thank you again');
});

test('Obsidian Ctrl+Shift+Space on a shown suggestion replaces it with one that starts with another word', async context => {
  const { controller, view, client, tick } = await harness(context);
  await tick(250);
  assert.equal(controller.text, ' for your time');
  const answers = [' so much', null];
  client.alternative = async () => {
    const text = answers.shift();
    client.acceptWord = text?.match(/^ ?\S+/u)?.[0];
    return text;
  };
  const chord = { isTrusted: true, key: ' ', ctrlKey: true, shiftKey: true };
  assert.equal(controller.key(chord), true);
  await settle();
  assert.equal(controller.text, ' so much');
  assert.equal(controller.decorations[0].widget.text, ' so much');
  assert.equal(controller.decorations[0].widget.word, ' so');
  assert.equal(client.requests.length, 1, 'The chord asked for an alternative, not a new request');
  client.authorize = mode => Promise.resolve({ text: mode === 'word' ? ' so' : ' so much' });
  assert.equal(controller.key({ isTrusted: true, key: 'Tab' }), true);
  await settle();
  assert.equal(view.state.doc.toString(), 'thank you so');

  // Without another one, the shown suggestion is withdrawn.
  await tick(250);
  assert.equal(controller.text, ' for your time');
  assert.equal(controller.key(chord), true);
  await settle();
  assert.equal(controller.text, null);
  assert.equal(controller.decorations.length, 0);
});

test('Obsidian replaces exactly the misspelled suffix including a typed space in one undo transaction', async context => {
  const { controller, view, client, state } = await harness(context);
  controller.invalidate();
  view.state = state('Please check the adress ');
  client.suggest = async () => { client.replacementBefore = 'adress '; return 'address '; };
  client.authorize = async () => ({ text: 'address ', replaceBefore: 'adress ' });
  await controller.request();
  assert.equal(controller.text, 'address ');
  assert.equal(controller.decorations[0].widget.text, ' adress → address');
  assert.equal(controller.decorations[0].widget.word, '', 'A correction is drawn whole');
  await controller.accept('word');
  assert.equal(view.state.doc.toString(), 'Please check the address ');
  assert.equal(view.transactions.length, 1);
  assert.equal(view.transactions[0].changes.to - view.transactions[0].changes.from, 'adress '.length);
  assert.equal(view.transactions[0].annotations[1].isolateHistory, 'full');
  assert.deepEqual(client.reports, [{ text: 'address ', status: 'applied' }]);
});

// Editor-range coverage only: the real broker proposes corrections only
// when the request has no text after the caret.
test('Obsidian accepts a scripted correction without touching text after the caret', async context => {
  const { controller, view, client, state } = await harness(context);
  controller.invalidate();
  const text = 'Please check the adress please';
  view.state = state(text, 'Please check the adress '.length);
  client.suggest = async () => { client.replacementBefore = 'adress '; return 'address '; };
  client.authorize = async () => ({ text: 'address ', replaceBefore: 'adress ' });
  await controller.request();
  assert.equal(controller.text, 'address ');
  await controller.accept('word');
  assert.equal(view.state.doc.toString(), 'Please check the address please');
  assert.deepEqual(client.reports, [{ text: 'address ', status: 'applied' }]);
});

test('Obsidian drops a surrogate half split by its before-caret pre-slice', async context => {
  const { controller, view, client, state } = await harness(context);
  controller.invalidate();
  const before = '😀' + 'a'.repeat(1023);
  view.state = state(before + '😀' + 'b'.repeat(300), before.length);
  await controller.request();
  const request = client.requests.at(-1);
  assert.ok(request);
  assert.ok(!request.before.includes('😀'), 'the split emoji is outside the bounded context');
  assert.ok([...request.before, ...request.after].every(character => {
    const code = character.codePointAt(0);
    return code < 0xd800 || code > 0xdfff;
  }), 'no lone surrogate reaches the broker client');
});

test('Obsidian refuses a different replacement grant without changing the note', async context => {
  const { controller, view, client, state } = await harness(context);
  controller.invalidate();
  view.state = state('Please check the adress ');
  client.suggest = async () => { client.replacementBefore = 'adress '; return 'address '; };
  client.authorize = async () => ({ text: 'address ', replaceBefore: 'other ' });
  await controller.request();
  await controller.accept('word');
  assert.equal(view.state.doc.toString(), 'Please check the adress ');
  assert.equal(view.transactions.length, 0);
  assert.deepEqual(client.reports, [{ text: 'address ', status: 'stale' }]);
});

test('Obsidian inserts only a grant for the text it shows', async context => {
  const { controller, view, client, tick } = await harness(context);
  await tick(250);
  assert.equal(controller.text, ' for your time');
  // The client adopted a longer streamed continuation this editor never drew.
  for (const [mode, text] of [['all', ' for your time and more'], ['word', ' fur'], ['word', '']]) {
    client.authorize = async () => ({ text });
    await controller.accept(mode);
    assert.equal(view.transactions.length, 0, `${mode} ${JSON.stringify(text)}`);
    assert.deepEqual(client.reports.at(-1), { text, status: 'stale' });
    assert.equal(controller.text, null, 'the preview goes with the refused grant');
    await controller.request();
    assert.equal(controller.text, ' for your time');
  }
});

test('Obsidian preserves manual policy and refuses a grant after focus changes', async context => {
  const { controller, view, client, tick } = await harness(context);
  client.automaticAllowed = false;
  await tick(250);
  assert.equal(client.requests.length, 0);
  assert.equal(controller.key({ isTrusted: true, key: 'Tab' }), false, 'Tab indents when no suggestion is visible');
  assert.equal(client.requests.length, 0);
  assert.equal(controller.key({ isTrusted: true, key: ' ', ctrlKey: true, shiftKey: true }), true);
  await settle();
  assert.equal(client.requests[0].explicit, true);
  let authorize;
  client.authorize = () => new Promise(resolve => { authorize = resolve; });
  const accepting = controller.accept('word');
  view.hasFocus = false;
  controller.update({ focusChanged: true });
  authorize({ text: ' for' });
  await accepting;
  assert.equal(view.transactions.length, 0);
  assert.equal(client.reports[0].status, 'stale');
});

test('Obsidian suggests mid-note with the text after the caret and inserts there', async context => {
  const { controller, view, client, state } = await harness(context);
  controller.invalidate();
  client.requests.length = 0;
  view.state = state('thank you. See you soon', 9);
  assert.equal(controller.blockedReason(), null);
  await controller.request(false);
  assert.deepEqual(client.requests, [{ before: 'thank you', language: 'en', explicit: false, after: '. See you soon' }]);
  await controller.accept('all');
  assert.equal(view.state.doc.toString(), 'thank you for your time. See you soon');
  assert.equal(view.state.selection.main.head, 23);
  assert.deepEqual(client.reports.at(-1), { text: ' for your time', status: 'applied' });
});

test('Obsidian yields Tab during composition and stores an explicit language preference', async context => {
  const { plugin, controller, view, client, state } = await harness(context);
  view.state = state('Vielen Dank');
  view.composing = true;
  assert.equal(controller.key({ isTrusted: true, key: 'Tab' }), false);
  view.composing = false;
  await plugin.commands.find(command => command.id === 'language-de').callback();
  await controller.request();
  assert.equal(client.requests[0].language, 'de');
});

test('Obsidian observes host-prevented Escape without consuming it or requesting again', async context => {
  const { controller, view, client, tick, domListeners } = await harness(context);
  await tick(250);
  const event = { key: 'Escape', isTrusted: true, defaultPrevented: true,
    preventDefault() { assert.fail('An observer must preserve the host event'); },
    stopPropagation() { assert.fail('An observer must preserve event routing'); } };
  assert.equal(domListeners.get('keydown')(event), undefined);
  assert.equal(client.dismissed, 1, 'A visible suggestion is dismissed without taking the host key');
  assert.equal(controller.text, null);
  assert.equal(controller.decorations.length, 0);
  assert.equal(view.transactions.length, 0, 'Dismissal does not modify the note');
  await tick(600);
  assert.equal(client.requests.length, 1, 'Dismissal stays quiet until an edit or explicit request');
  const generation = controller.generation;
  domListeners.get('keydown')(event);
  assert.equal(controller.generation, generation, 'Escape without Badi work does nothing');
  await controller.request();
  assert.equal(controller.text, ' for your time', 'An explicit request still works after dismissal');
  controller.destroy();
  assert.equal(domListeners.size, 0, 'Closing an editor removes its observer');
});

test('Obsidian Escape cancels an in-flight suggestion and ignores foreign input states', async context => {
  const { controller, view, client, tick, domListeners } = await harness(context);
  await tick(250);
  const observe = overrides => domListeners.get('keydown')({ key: 'Escape', isTrusted: true, ...overrides });
  for (const overrides of [{ isTrusted: false }, { isComposing: true }, { ctrlKey: true },
    { metaKey: true }, { altKey: true }, { shiftKey: true }]) {
    observe(overrides);
    assert.equal(controller.text, ' for your time');
  }
  view.composing = true; observe(); assert.equal(controller.text, ' for your time');
  view.composing = false; view.hasFocus = false; observe(); assert.equal(controller.text, ' for your time');
  view.hasFocus = true;
  controller.invalidate();
  let resolve;
  client.suggest = () => new Promise(done => { resolve = done; });
  const request = controller.request();
  assert.equal(controller.pending, true);
  observe({ defaultPrevented: true });
  assert.equal(controller.pending, false);
  resolve(' for your time');
  await request;
  assert.equal(controller.text, null, 'A dismissed response cannot restore the preview');
  assert.equal(view.transactions.length, 0);
});

test('Obsidian editors share one broker client and only the requesting editor cancels its session', async context => {
  const { plugin, controller, client, clients, editor, tick } = await harness(context);
  await tick(250);
  assert.equal(controller.text, ' for your time');
  const second = editor('another note', false);
  await settle();
  assert.equal(second.controller.client, client);
  assert.equal(clients.length, 1, 'Opening an editor adds no broker connection');
  let cancels = 0;
  client.cancel = () => { cancels++; client.emit('clear'); };
  second.controller.invalidate();
  assert.equal(cancels, 0, 'Another editor cannot withdraw the session it did not open');
  assert.equal(controller.text, ' for your time');
  controller.invalidate();
  assert.equal(cancels, 1);
  plugin.commands.find(command => command.id === 'retry-connection').callback();
  assert.equal(clients.length, 2, 'Reconnecting replaces the one shared client');
  assert.equal(second.controller.client, clients[1]);
});
