import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { runInNewContext } from 'node:vm';

// Discovery has its own rendered module tests; this harness isolates draft/import
// races without making a real metadata request through that independent module.
const source = (await readFile(new URL('./public/app.mjs', import.meta.url), 'utf8'))
  .replace(/^import \{ mountDiscovery \} from '\.\/discovery\.mjs';\n/u, '')
  .replace(/^import \{ mountQualification \} from '\.\/qualification\.mjs';\n/u, '');
const suite = (id, extra = {}) => ({ schema: 'badi.prediction-suite.v1', name: 'Import fixture',
  cases: [{ id, language: 'en', prefix: `Draft ${id}`, ...extra }] });

function harness() {
  class Element {
    constructor() { this.value = ''; this.children = []; this.textContent = ''; }
    append(...children) { this.children.push(...children); }
    replaceChildren(...children) { this.children = children; }
    addEventListener() {}
  }
  const elements = new Map();
  const get = id => {
    if (!elements.has(id)) elements.set(id, new Element());
    return elements.get(id);
  };
  const requests = [];
  runInNewContext(source, { structuredClone, mountDiscovery: () => {}, mountQualification: () => {}, document: {
    getElementById: get, createElement: () => new Element(),
    querySelector: selector => selector === '.editor' ? get('editor') : { content: 'fixture-token' },
  }, fetch: async url => { requests.push(url); return { ok: true }; } });
  const load = async (value, text = async () => JSON.stringify(value)) => {
    get('import').files = [{ size: 100, text }];
    await get('import').onchange();
  };
  return { get, load, requests };
}

test('prediction UI imports and adds cases without an expected answer or inference', async () => {
  const ui = harness();
  await ui.load(suite('first'));
  assert.equal(ui.get('case-count').textContent, '1');
  assert.equal(ui.get('prefix').value, 'Draft first');
  assert.equal(ui.get('expected').value, '');
  ui.get('case-name').value = 'second';
  ui.get('prefix').value = 'A new draft';
  ui.get('add-case').onclick();
  assert.equal(ui.get('case-count').textContent, '2');
  assert.deepEqual(ui.requests, []);
});

test('malformed explicit answers cannot replace an already imported test set', async () => {
  const ui = harness();
  await ui.load(suite('kept', { expected: [' continuation'] }));
  for (const expected of [null, 'answer', [3]]) {
    await ui.load(suite('invalid', { expected }));
    assert.match(ui.get('status').textContent, /invalid shape/u);
    assert.equal(ui.get('case-count').textContent, '1');
    assert.equal(ui.get('prefix').value, 'Draft kept');
    assert.equal(ui.get('expected').value, ' continuation');
  }
});

test('a delayed answer-free import cannot restore drafts after clear or a newer import', async () => {
  for (const reset of [true, false]) {
    const ui = harness(); let finish;
    const pending = ui.load(null, () => new Promise(resolve => { finish = resolve; }));
    if (reset) await ui.get('clear-all').onclick();
    else await ui.load(suite('newer'));
    finish(JSON.stringify(suite('stale'))); await pending;
    assert.equal(ui.get('case-count').textContent, reset ? '0' : '1');
    assert.equal(ui.get('prefix').value, reset ? '' : 'Draft newer');
    assert.deepEqual(ui.requests, reset ? ['/api/reset'] : []);
  }
});
