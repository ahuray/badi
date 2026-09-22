import test from 'node:test';
import assert from 'node:assert/strict';
import { resolveObjectURL } from 'node:buffer';
import { spellingInput, correctionParts, spellingAgreement, attachSpellingUI } from './public/spelling.mjs';

test('spelling request excludes reference answers and retains exact draft bytes', () => {
  const before = 'Ich besuche die Bibliotek ';
  assert.deepEqual(spellingInput({ before, language: 'de', protectedWords: 'Marlene\nBadi',
    expected_word: 'ANSWER_MUST_NOT_REACH_ENGINE', arbitrary: 'ignored' }),
  { before, language: 'de', protected_words: ['Marlene', 'Badi'] });
  assert.equal(spellingInput({ before: 'من با دوچرجه ', language: 'fa' }).before, 'من با دوچرجه ');
});

test('language, Unicode length, invalid UTF-16 and protected-word bounds fail before dispatch', () => {
  for (const fields of [
    { before: 'word ', language: 'en' }, { before: '\ud800', language: 'de' },
    { before: 'a'.repeat(2049), language: 'de' },
    { before: 'Wort ', language: 'de', protectedWords: Array(33).fill('Badi').join('\n') },
    { before: 'Wort ', language: 'de', protectedWords: 'two words' },
    { before: 'واژه ', language: 'fa', protectedWords: 'م'.repeat(25) },
  ]) assert.throws(() => spellingInput(fields));
});

test('preview replaces only its exact completed suffix, including Persian joiners', () => {
  assert.deepEqual(correctionParts({ before: 'Ich gehe in die Bibliotek ' },
    { outcome: 'suggestion', replace_before: 'Bibliotek ', text: 'Bibliothek ' }),
  { prefix: 'Ich gehe in die ', original: 'Bibliotek', corrected: 'Bibliothek' });
  const before = 'من نامه را می‌نویصم ';
  assert.deepEqual(correctionParts({ before },
    { outcome: 'suggestion', replace_before: 'می‌نویصم ', text: 'می‌نویسم ' }),
  { prefix: 'من نامه را ', original: 'می‌نویصم', corrected: 'می‌نویسم' });
  assert.equal(before, 'من نامه را می‌نویصم ', 'preview must not mutate the input');
});

test('stale, embedded, spacing-changing and unchanged corrections cannot render as proposals', () => {
  for (const [before, original, corrected] of [
    ['The otherword ', 'Bibliotek ', 'Bibliothek '],
    ['noteBibliotek ', 'Bibliotek ', 'Bibliothek '],
    ['Bibliotek ', 'Bibliotek ', 'Bibliothek'],
    ['Bibliotek ', 'Bibliotek ', 'Biblio thek '],
    ['Bibliotek ', 'Bibliotek ', 'Bibliothek  '],
    ['Bibliotek ', 'Bibliotek ', 'Bibliotek '],
  ]) assert.throws(() => correctionParts({ before }, { outcome: 'suggestion', replace_before: original, text: corrected }));
});

test('expected spelling is scored locally; abstention remains a missed reference', () => {
  const input = { before: 'Bibliotek ' };
  const result = { outcome: 'suggestion', replace_before: 'Bibliotek ', text: 'Bibliothek ' };
  assert.deepEqual(spellingAgreement(input, result, 'Bibliothek'), { scorable: true, exact_match: true });
  assert.deepEqual(spellingAgreement(input, result, 'bibliothek'), { scorable: true, exact_match: false });
  assert.deepEqual(spellingAgreement(input, { outcome: 'abstention' }, 'Bibliothek'), { scorable: true, exact_match: false });
  assert.deepEqual(spellingAgreement(input, result, ''), { scorable: false, exact_match: null });
});

// Small DOM stand-in: deferred fetches exercise the UI's actual async handlers.
function uiHarness() {
  class Element {
    constructor(tag = 'div') {
      this.tag = tag; this.value = ''; this.children = []; this.attributes = {};
      this.listeners = {}; this.textContent = ''; this.hidden = false;
      this.classList = { toggle() {} };
    }
    append(...children) { this.children.push(...children); }
    prepend(child) { this.children.unshift(child); }
    replaceChildren(...children) { this.children = children; }
    setAttribute(name, value) { this.attributes[name] = value; }
    addEventListener(name, listener) { this.listeners[name] = listener; }
    querySelectorAll(tag) { return this.children.flatMap(child => [
      ...(child.tag === tag ? [child] : []), ...child.querySelectorAll(tag),
    ]); }
    focus() { document.activeElement = this; }
    click() { this.clicked = true; }
  }
  const elements = new Map(); const created = [];
  const document = {
    getElementById(id) { if (!elements.has(id)) elements.set(id, new Element()); return elements.get(id); },
    createElement(tag) { const element = new Element(tag); created.push(element); return element; },
    querySelector() { return { content: 'test-capability' }; },
  };
  const $ = id => document.getElementById(id);
  $('spelling-before').value = 'Bibliotek ';
  $('spelling-language').value = 'de';
  $('spelling-expected').value = 'Bibliothek';
  const requests = [];
  const fetch = (url, options) => {
    if (url.endsWith('/status')) return Promise.resolve({ ok: true, json: async () => ({ configured_languages: ['de', 'fa'] }) });
    let resolve;
    const response = new Promise(r => { resolve = r; });
    requests.push({ url, options, respond: (body = {}, status = 200) => resolve({
      ok: status < 400, status, json: async () => body,
    }) });
    return response;
  };
  attachSpellingUI(document, fetch);
  return { $, document, requests, created, submit: () => $('spelling-form').onsubmit({ preventDefault() {} }) };
}
const settle = () => new Promise(resolve => setImmediate(resolve));
const correctionResponse = {
  schema: 'badi.spelling-lab.response.v1',
  result: { outcome: 'suggestion', replace_before: 'Bibliotek ', text: 'Bibliothek ' },
};

test('editing during a pending check aborts and ignores even a successful late response', async () => {
  const ui = uiHarness(); await settle();
  const running = ui.submit();
  assert.deepEqual(JSON.parse(ui.requests[0].options.body), { before: 'Bibliotek ', language: 'de', protected_words: [] });
  ui.$('spelling-before').value = 'Buch ';
  ui.$('spelling-before').listeners.input();
  assert.equal(ui.requests[0].options.signal.aborted, true);
  assert.equal(ui.requests[1].url, '/api/spelling/reset');
  assert.equal(ui.$('spelling-check').disabled, true);
  ui.requests[0].respond(correctionResponse); await running;
  assert.equal(ui.$('spelling-count').textContent, '0');
  ui.requests[1].respond(); await settle();
  assert.equal(ui.$('spelling-before').value, 'Buch ');
  assert.equal(ui.$('spelling-check').disabled, false);
});

test('failed reset keeps checks disabled until cleanup succeeds on a later clear', async () => {
  const ui = uiHarness(); await settle();
  ui.$('spelling-clear').onclick();
  ui.requests[0].respond({}, 503); await settle();
  assert.equal(ui.$('spelling-check').disabled, true);
  await ui.submit(); assert.equal(ui.requests.length, 1);
  ui.$('spelling-clear').onclick();
  ui.requests[1].respond(); await settle();
  assert.equal(ui.$('spelling-check').disabled, false);
  assert.equal(ui.$('spelling-before').value, '');
});

test('tab navigation preserves a completed correction and both editor drafts', async () => {
  const ui = uiHarness(); await settle();
  ui.$('prefix').value = 'My prediction draft ';
  const running = ui.submit(); ui.requests[0].respond(correctionResponse); await running;
  assert.equal(ui.$('spelling-count').textContent, '1');
  assert.equal(ui.$('spelling-before').value, 'Bibliotek ');
  ui.$('prediction-tab').onkeydown({ key: 'End', preventDefault() {} });
  assert.equal(ui.$('prediction-panel').hidden, true);
  assert.equal(ui.document.activeElement, ui.$('lookup-tab'));
  ui.$('lookup-tab').onkeydown({ key: 'ArrowLeft', preventDefault() {} });
  assert.equal(ui.document.activeElement, ui.$('spelling-tab'));
  ui.$('prediction-tab').onclick();
  assert.equal(ui.$('spelling-panel').hidden, true);
  assert.equal(ui.$('prefix').value, 'My prediction draft ');
  assert.equal(ui.$('spelling-before').value, 'Bibliotek ');
  assert.equal(ui.$('spelling-count').textContent, '1');
});

test('explicit export includes unsaved draft and locally reviewed results without sending references', async () => {
  const ui = uiHarness(); await settle();
  ui.$('spelling-export').onclick();
  let link = ui.created.findLast(element => element.tag === 'a');
  assert.equal(link.clicked, true);
  let exported = JSON.parse(await resolveObjectURL(link.href).text());
  assert.equal(exported.draft.expected_word, 'Bibliothek');
  assert.deepEqual(exported.records, []);
  assert.equal(ui.requests.length, 0, 'export must not send prose to the server');
  const running = ui.submit(); ui.requests[0].respond(correctionResponse); await running;
  ui.created.find(element => element.tag === 'button' && element.textContent === 'Useful').onclick();
  ui.$('spelling-export').onclick();
  link = ui.created.findLast(element => element.tag === 'a');
  exported = JSON.parse(await resolveObjectURL(link.href).text());
  assert.equal(exported.records.length, 1);
  assert.equal(exported.records[0].review.judgment, 'Useful');
  assert.equal(exported.records[0].expected_word, 'Bibliothek');
  assert.equal(JSON.parse(ui.requests[0].options.body).expected_word, undefined);
});
