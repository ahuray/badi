// The settings popout's keyboard model (KeyModel.js), loaded as the QML
// engine loads a `.pragma library` file: plain functions, no QML.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const source = readFileSync(new URL('../KeyModel.js', import.meta.url), 'utf8');
assert.match(source, /^\.pragma library\n/u);
const KeyModel = vm.runInNewContext(`${source.replace(/^\.pragma library\n/u, '')}
({ step, choice, action, removable, filter })`);

const toggle = { kind: 'toggle', label: 'Mid-sentence suggestions', detail: 'Also suggest', section: 'Writing',
  enabled: true, activate() {} };
const question = { kind: 'choice', label: 'Finish words from what you see?', detail: 'On this device', section: 'Context',
  enabled: true, value: '', options: [{ value: 'on' }, { value: 'off' }], choose() {} };
const model = { kind: 'choice', label: 'Writing model', detail: 'Recommended', section: 'Model', enabled: true,
  value: 'gemma-3-1b', options: [{ value: 'qwen3-0.6b' }, { value: 'gemma-3-1b' }], choose() {} };
const site = { kind: 'site', label: 'https://mail.example', detail: 'Allowed', section: 'Websites', enabled: true,
  activate() {}, remove() {} };
const field = { kind: 'field', label: 'Add a site', detail: 'Enter allows', section: 'Websites', enabled: true };

test('j and k move one row, start at the first and stop at either end', () => {
  assert.equal(KeyModel.step(-1, 4, 1), 0, 'the first j lands on the first row');
  assert.equal(KeyModel.step(-1, 4, -1), 0);
  assert.equal(KeyModel.step(0, 4, 1), 1);
  assert.equal(KeyModel.step(3, 4, 1), 3);
  assert.equal(KeyModel.step(0, 4, -1), 0);
  assert.equal(KeyModel.step(-1, 0, 1), -1, 'no rows, no cursor');
});

test('h and l step a choice within its options and pick the first when none is chosen', () => {
  assert.equal(KeyModel.choice(question, 1), 'on', 'l answers the one-time question with its first option');
  assert.equal(KeyModel.choice(question, -1), null);
  assert.equal(KeyModel.choice({ ...question, value: 'on' }, 1), 'off');
  assert.equal(KeyModel.choice(model, -1), 'qwen3-0.6b');
  assert.equal(KeyModel.choice(model, 1), null, 'l at the last option changes nothing');
  assert.equal(KeyModel.choice({ ...model, enabled: false }, -1), null);
  assert.equal(KeyModel.choice(toggle, 1), null, 'h and l leave other rows alone');
  assert.equal(KeyModel.choice(undefined, 1), null);
});

test('Enter activates or focuses enabled rows and x removes only enabled site rules', () => {
  assert.equal(KeyModel.action(toggle), 'activate');
  assert.equal(KeyModel.action(field), 'focus');
  assert.equal(KeyModel.action({ ...toggle, enabled: false }), null);
  assert.equal(KeyModel.action(question), null, 'a choice is answered with h and l');
  assert.equal(KeyModel.action(undefined), null);
  assert.equal(KeyModel.removable(site), true);
  assert.equal(KeyModel.removable({ ...site, enabled: false }), false);
  assert.equal(KeyModel.removable(toggle), false);
});

test('search keeps rows whose label, detail or section contains the query, ignoring case', () => {
  const rows = [toggle, question, model, site, field];
  assert.equal(KeyModel.filter(rows, ''), rows);
  assert.equal(KeyModel.filter(rows, '   '), rows);
  assert.deepEqual(KeyModel.filter(rows, 'CONTEXT'), [question]);
  assert.deepEqual(KeyModel.filter(rows, 'websites'), [site, field]);
  assert.deepEqual(KeyModel.filter(rows, 'recommended'), [model]);
  assert.deepEqual(KeyModel.filter(rows, 'nothing like this'), []);
});
