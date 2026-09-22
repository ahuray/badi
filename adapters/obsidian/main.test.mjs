import { test } from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { readFile } from 'node:fs/promises';
import { runInNewContext } from 'node:vm';
import { transform } from 'esbuild';
import { writingLanguage } from '../shared/writing-language.mjs';
import { validCorrection } from '../shared/text-safety.mjs';

const source = (await transform(await readFile(new URL('./main.mjs', import.meta.url), 'utf8'),
  { format: 'cjs', target: 'es2022' })).code;
const settle = async () => { for (let turn = 0; turn < 12; turn++) await Promise.resolve(); };

async function harness(context) {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  class Client extends EventEmitter {
    connected = true;
    allowed = true;
    automaticAllowed = true;
    requests = [];
    modes = [];
    reports = [];
    connect() { this.emit('state', 'ready'); return Promise.resolve(true); }
    suggest(before, options) { this.requests.push({ before, ...options }); return Promise.resolve(' for your time'); }
    authorize(mode) { this.modes.push(mode); return Promise.resolve({ text: mode === 'word' ? ' for' : ' for your time' }); }
    report(grant, status) { this.reports.push({ text: grant.text, status }); }
    cancel() { this.emit('clear'); }
    close() {}
  }
  class Plugin {
    commands = [];
    loadData() { return Promise.resolve({ language: 'en' }); }
    saveData() { return Promise.resolve(); }
    addStatusBarItem() { return { setText() {} }; }
    registerEditorExtension(value) { this.extension = value; }
    addCommand(value) { this.commands.push(value); }
  }
  const modules = {
    obsidian: { Plugin, Notice: class {} },
    '@codemirror/view': { WidgetType: class {}, ViewPlugin: { fromClass: value => value },
      Decoration: { none: [], set: value => value, widget: value => ({ range: position => ({ ...value, position }) }) } },
    '@codemirror/state': { Prec: { highest: value => value }, Transaction: { userEvent: { of: value => ({ userEvent: value }) } } },
    '@codemirror/commands': { isolateHistory: { of: value => ({ isolateHistory: value }) } },
    '@codemirror/autocomplete': { completionStatus: state => state.completion ?? null },
    '../shared/recovering-client.mjs': { RecoveringBrokerClient: Client },
    '../shared/activity.mjs': { recordActivity() {} },
    '../shared/writing-language.mjs': { writingLanguage },
    '../shared/text-safety.mjs': { validCorrection },
  };
  const module = { exports: {} };
  runInNewContext(source, { module, exports: module.exports, require: id => {
    assert.ok(Object.hasOwn(modules, id), `Unexpected dependency: ${id}`);
    return modules[id];
  }, setTimeout, clearTimeout, queueMicrotask, navigator: { language: 'en-US' } });
  const plugin = new module.exports.default();
  await plugin.onload();
  const state = (text, head = text.length) => ({
    doc: { length: text.length, sliceString: (from = 0, to = text.length) => text.slice(from, to) },
    selection: { ranges: [{}], main: { head, empty: true }, eq(other) { return other.main.head === head; } },
    readOnly: false,
  });
  let controller;
  const domListeners = new Map();
  const view = { hasFocus: true, composing: false, state: state('thank you'), transactions: [],
    dom: { ownerDocument: { hasFocus: () => view.hasFocus } },
    contentDOM: {
      addEventListener(type, listener, options) { assert.equal(options.passive, true); domListeners.set(type, listener); },
      removeEventListener(type, listener) { if (domListeners.get(type) === listener) domListeners.delete(type); },
    },
    dispatch(transaction) {
      if (!transaction.changes) return;
      this.transactions.push(transaction);
      const before = this.state;
      this.state = state(before.doc.sliceString(0, transaction.changes.from) + transaction.changes.insert,
        transaction.selection.anchor);
      controller.update({ docChanged: true, state: this.state, startState: before });
    },
  };
  controller = new plugin.extension(view);
  const tick = async ms => { context.mock.timers.tick(ms); await settle(); };
  await settle();
  context.after(() => { controller.destroy(); context.mock.timers.reset(); });
  return { plugin, controller, view, client: controller.client, tick, state, domListeners };
}

test('Obsidian automatically requests, accepts one word or all, and isolates each undo transaction', async context => {
  const { controller, view, client, tick } = await harness(context);
  await tick(250);
  assert.deepEqual(client.requests, [{ before: 'thank you', language: 'en', explicit: false }]);
  assert.equal(controller.text, ' for your time');
  assert.equal(controller.key({ isTrusted: true, key: 'Tab' }), true);
  await settle();
  assert.equal(view.state.doc.sliceString(), 'thank you for');
  assert.deepEqual(client.modes, ['word']);
  assert.deepEqual(client.reports, [{ text: ' for', status: 'applied' }]);
  assert.equal(view.transactions[0].annotations[1].isolateHistory, 'full');
  await tick(250);
  assert.equal(client.requests.length, 2, 'The remainder requires a fresh request after insertion');
  assert.equal(controller.key({ isTrusted: true, key: 'ArrowRight', ctrlKey: true }), true);
  await settle();
  assert.deepEqual(client.modes, ['word', 'all']);
  assert.equal(view.state.doc.sliceString(), 'thank you for for your time');
  assert.equal(controller.key({ isTrusted: true, key: 'Tab', shiftKey: true }), false);
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
  await controller.accept('word');
  assert.equal(view.state.doc.sliceString(), 'Please check the address ');
  assert.equal(view.transactions.length, 1);
  assert.equal(view.transactions[0].changes.to - view.transactions[0].changes.from, 'adress '.length);
  assert.equal(view.transactions[0].annotations[1].isolateHistory, 'full');
  assert.deepEqual(client.reports, [{ text: 'address ', status: 'applied' }]);
});

test('Obsidian refuses a different replacement grant without changing the note', async context => {
  const { controller, view, client, state } = await harness(context);
  controller.invalidate();
  view.state = state('Please check the adress ');
  client.suggest = async () => { client.replacementBefore = 'adress '; return 'address '; };
  client.authorize = async () => ({ text: 'address ', replaceBefore: 'other ' });
  await controller.request();
  await controller.accept('word');
  assert.equal(view.state.doc.sliceString(), 'Please check the adress ');
  assert.equal(view.transactions.length, 0);
  assert.deepEqual(client.reports, [{ text: 'address ', status: 'stale' }]);
});

test('Obsidian preserves manual policy and refuses a grant after focus changes', async context => {
  const { controller, view, client, tick } = await harness(context);
  client.automaticAllowed = false;
  await tick(250);
  assert.equal(client.requests.length, 0);
  assert.equal(controller.key({ isTrusted: true, key: 'Tab' }), true);
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

test('Obsidian suppresses unsupported cursor/composition states and stores an explicit language preference', async context => {
  const { plugin, controller, view, client, tick, state } = await harness(context);
  view.state = state('thank you', 2);
  await tick(250);
  assert.equal(client.requests.length, 0);
  assert.equal(controller.blockedReason(), 'caret_not_at_note_end');
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
