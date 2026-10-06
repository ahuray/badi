import test from 'node:test';
import assert from 'node:assert/strict';
import { mountDiscovery } from './public/discovery.mjs';

function setup() {
  class Element {
    constructor(tag) { this.tag = tag; this.textContent = ''; this.children = []; this.dataset = {}; this.classList = { add() {} }; this.value = ''; }
    append(...children) { this.children.push(...children); }
    replaceChildren(...children) { this.children = children; }
    setAttribute() {}
    querySelectorAll(tag) { return this.children.flatMap(child => [...(child.tag === tag ? [child] : []), ...child.querySelectorAll(tag)]); }
  }
  const document = { createElement: tag => new Element(tag) }; const root = new Element('section'); root.ownerDocument = document;
  const requests = []; const selected = [];
  const ui = mountDiscovery({ root, request: (path, body, signal) => new Promise((resolve, reject) => requests.push({ path, body, signal, resolve, reject })), onSelected: value => selected.push(value) });
  return { root, ui, requests, selected, button: text => root.querySelectorAll('button').find(button => button.textContent === text),
    submit: () => root.querySelectorAll('form')[0].onsubmit({ preventDefault() {} }), status: () => root.children[3].textContent };
}
const turn = () => new Promise(resolve => setImmediate(resolve));
const candidate = { candidate_id: 'b'.repeat(64), repo: 'publisher/small', file: 'small.gguf', bytes: 200,
  architecture: 'llama', quantization: 'Q8_0', languages: ['en'], license: 'apache-2.0', provenance: { certainty: 'Upstream conversion revision unknown.' }, rejection_reasons: [] };

test('browser workflow sends search metadata and server IDs only and labels fit/download as unqualified', async () => {
  const ui = setup(); ui.root.querySelectorAll('input')[0].value = 'small';
  const searching = ui.submit(); assert.deepEqual(ui.requests[0].body, { query: 'small' });
  ui.requests[0].resolve({ models: [{ repo: 'publisher/small', model_id: 'a'.repeat(64), url: 'https://huggingface.co/publisher/small', languages: ['en'] }], page: 1, page_limit: 3, cache_state: 'fresh' }); await searching;
  ui.button('Inspect GGUF artifacts').onclick(); assert.deepEqual(ui.requests[1].body, { model_id: 'a'.repeat(64) });
  ui.requests[1].resolve({ model: { repo: 'publisher/small', revision: 'a'.repeat(40) }, candidates: [candidate], cache_state: 'fresh' }); await turn();
  ui.button('Assess this device').onclick(); ui.requests[2].resolve({ assessment: { state: 'estimated_to_fit' }, download_allowed: true }); await turn();
  assert.match(ui.status(), /unverified/u);
  ui.button('Download and verify').onclick(); assert.deepEqual(ui.requests[3].body, { candidate_id: candidate.candidate_id });
  ui.requests[3].resolve({ artifact_id: 'c'.repeat(64), reused: true, verified: true }); await turn();
  assert.match(ui.status(), /not yet qualified/u);
  ui.button('Select for Lab comparison').onclick(); assert.deepEqual(ui.requests[4].body, { artifact_id: 'c'.repeat(64) });
  ui.requests[4].resolve({ selected: { candidate, qualification: 'unqualified' } }); await turn();
  assert.equal(ui.selected.length, 1);
  assert.ok(ui.requests.every(request => !Object.hasOwn(request.body ?? {}, 'before') && !Object.hasOwn(request.body ?? {}, 'weights_path')));
});

test('cancel ignores late results and does not reenable controls until reset succeeds', async () => {
  const ui = setup(); ui.root.querySelectorAll('input')[0].value = 'small'; const searching = ui.submit();
  const reset = ui.button('Cancel').onclick(); assert.equal(ui.requests[0].signal.aborted, true);
  ui.requests[0].resolve({ models: [{ repo: 'MUST NOT SHOW' }], page: 1 }); await searching;
  assert.equal(ui.root.querySelectorAll('article').length, 0); assert.equal(ui.button('Search Hugging Face').disabled, true);
  ui.requests[1].reject(new Error('connection unavailable')); await reset;
  assert.equal(ui.button('Search Hugging Face').disabled, true); assert.match(ui.status(), /Could not confirm/u);
  const retry = ui.button('Cancel').onclick(); ui.requests[2].resolve({}); await retry;
  assert.equal(ui.button('Search Hugging Face').disabled, false);
});

test('failed fit leaves the download control disabled with inspectable reasons', async () => {
  const ui = setup(); ui.root.querySelectorAll('input')[0].value = 'small'; const searching = ui.submit();
  ui.requests[0].resolve({ models: [{ model_id: 'a'.repeat(64), repo: 'publisher/small', languages: [] }], page: 1 }); await searching;
  ui.button('Inspect GGUF artifacts').onclick(); ui.requests[1].resolve({ model: { repo: 'publisher/small', revision: 'a'.repeat(40) }, candidates: [candidate] }); await turn();
  ui.button('Assess this device').onclick(); ui.requests[2].resolve({ assessment: { rejection_reasons: ['Insufficient memory'] }, download_allowed: false }); await turn();
  assert.equal(ui.button('Download and verify').disabled, true); assert.match(ui.status(), /failed/u);
  assert.ok(ui.root.querySelectorAll('pre').some(node => node.textContent.includes('Insufficient memory')));
});
