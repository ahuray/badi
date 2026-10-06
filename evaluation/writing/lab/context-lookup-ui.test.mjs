import test from 'node:test';
import assert from 'node:assert/strict';
import { resolveObjectURL } from 'node:buffer';
import { lookupInput, lookupParts, attachLookupUI } from './public/context-lookup.mjs';

test('only explicit lookup fields are submitted; references and judgments are excluded', () => {
  const input = lookupInput({ before: 'The sched', language: 'en', context: 'schedule.',
    style: 'One sample.\n\nAnother sample.', expected_text: 'SECRET_REFERENCE', review: 'useful' }, 'case-1');
  assert.deepEqual(input, { schema: 'badi.context-lookup.request.v1', id: 'case-1', before: 'The sched',
    language: 'en', context: 'schedule.', style_examples: ['One sample.', 'Another sample.'] });
  for (const changed of [{ before: '' }, { before: '\ud800' }, { before: 'x'.repeat(2049) },
    { language: 'fr' }, { context: 'x'.repeat(4097) }, { style: Array(9).fill('sample').join('\n\n') },
    { style: 'x'.repeat(1025) }]) assert.throws(() => lookupInput({ before: 'The sched', language: 'en', ...changed }, 'case-1'));
});

test('grey preview appends the exact suffix without changing Persian joins or the draft', () => {
  const input = { before: 'او هر روز می‌نوی' };
  const result = { outcome: 'suggestion', text: 'سد', matched_word: 'می‌نویسد', matched_candidate_count: 1 };
  assert.deepEqual(lookupParts(input, result), { before: input.before, suffix: 'سد' });
  assert.equal(input.before, 'او هر روز می‌نوی');
  for (const changed of [{ matched_word: 'مینویسد' }, { matched_candidate_count: 2 }, { text: 'سد ' }, { text: '' }]) {
    assert.throws(() => lookupParts(input, { ...result, ...changed }));
  }
  assert.equal(lookupParts(input, { outcome: 'abstention' }), null);
});

function harness() {
  class Element {
    constructor(tag = 'div') {
      this.tag = tag; this.value = ''; this.children = []; this.attributes = {}; this.listeners = {};
      this.textContent = ''; this.hidden = false; this.classList = { toggle() {} };
    }
    append(...children) { this.children.push(...children); }
    prepend(child) { this.children.unshift(child); }
    replaceChildren(...children) { this.children = children; }
    setAttribute(name, value) { this.attributes[name] = value; }
    addEventListener(name, listener) { this.listeners[name] = listener; }
    querySelectorAll(tag) { return this.children.flatMap(child => [...(child.tag === tag ? [child] : []), ...child.querySelectorAll(tag)]); }
    click() { this.clicked = true; }
  }
  const elements = new Map(); const created = [];
  const document = {
    getElementById(id) { if (!elements.has(id)) elements.set(id, new Element()); return elements.get(id); },
    createElement(tag) { const element = new Element(tag); created.push(element); return element; },
    querySelector() { return { content: 'test-token' }; },
  };
  const $ = id => document.getElementById(id);
  $('lookup-before').value = 'The sched'; $('lookup-language').value = 'en';
  $('lookup-context').value = 'schedule.'; $('lookup-expected').value = 'ule';
  const requests = [];
  const fetch = (url, options) => new Promise(resolve => {
    requests.push({ url, options, respond: (body = {}, status = 200) => resolve({ ok: status < 400, status, json: async () => body }) });
  });
  attachLookupUI(document, fetch);
  return { $, requests, created, submit: () => $('lookup-form').onsubmit({ preventDefault() {} }) };
}
const settle = () => new Promise(resolve => setImmediate(resolve));
const responseFor = request => ({ schema: 'badi.context-lookup.response.v1', cleanup: { reaped: true },
  result: { schema: 'badi.context-lookup.result.v1', id: JSON.parse(request.options.body).id,
    outcome: 'suggestion', text: 'ule', matched_word: 'schedule', matched_candidate_count: 1 } });

test('editing cancels an in-flight lookup and ignores a successful late result', async () => {
  const ui = harness(); const running = ui.submit();
  ui.$('lookup-before').value = 'The calendar'; ui.$('lookup-before').listeners.input();
  assert.equal(ui.requests[0].options.signal.aborted, true);
  assert.equal(ui.requests[1].url, '/api/context-lookup/reset');
  assert.equal(ui.$('lookup-check').disabled, true);
  ui.requests[0].respond(responseFor(ui.requests[0])); await running;
  assert.equal(ui.$('lookup-count').textContent, '0');
  ui.requests[1].respond(); await settle();
  assert.equal(ui.$('lookup-check').disabled, false);
  assert.equal(ui.$('lookup-before').value, 'The calendar');
});

test('clear waits for cleanup and failed cleanup prevents another check until recovery', async () => {
  const ui = harness(); const running = ui.submit();
  ui.$('lookup-clear').onclick();
  assert.equal(ui.$('lookup-before').value, '');
  ui.requests[0].respond(responseFor(ui.requests[0])); await running;
  ui.requests[1].respond({}, 503); await settle();
  assert.equal(ui.$('lookup-check').disabled, true);
  await ui.submit(); assert.equal(ui.requests.length, 2);
  ui.$('lookup-clear').onclick(); ui.requests[2].respond(); await settle();
  assert.equal(ui.$('lookup-check').disabled, false);
  assert.equal(ui.$('lookup-count').textContent, '0');
});

test('mismatched response identity and unconfirmed cleanup cannot display an offer', async () => {
  for (const invalid of [body => { body.result.id = 'other'; }, body => { body.cleanup.reaped = false; }]) {
    const ui = harness(); const running = ui.submit(); const body = responseFor(ui.requests[0]); invalid(body);
    ui.requests[0].respond(body); await running;
    assert.equal(ui.$('lookup-count').textContent, '0');
    assert.match(ui.$('lookup-status').textContent, /invalid result/u);
  }
});

test('explicit export preserves draft, expected suffix and user judgment without network export', async () => {
  const ui = harness(); const running = ui.submit();
  assert.equal(Object.hasOwn(JSON.parse(ui.requests[0].options.body), 'expected_text'), false);
  ui.requests[0].respond(responseFor(ui.requests[0])); await running;
  assert.equal(ui.$('lookup-before').value, 'The sched');
  ui.created.find(element => element.tag === 'button' && element.textContent === 'Useful').onclick();
  ui.$('lookup-export').onclick();
  const link = ui.created.findLast(element => element.tag === 'a');
  const data = JSON.parse(await resolveObjectURL(link.href).text());
  assert.equal(link.clicked, true); assert.equal(data.draft.expected_text, 'ule');
  assert.equal(data.records[0].review.judgment, 'Useful'); assert.equal(ui.requests.length, 1);
});
