import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { hasUnsafeText, validCorrection } from './text-safety.mjs';
import { validSuggestion } from './broker-client.mjs';

test('all editor text guards share the exact Persian orthographic joiner contract', async () => {
  const corpus = JSON.parse(await readFile(new URL('../../protocol/orthographic-joiner-fixtures.json', import.meta.url), 'utf8'));
  for (const fixture of corpus.fixtures) {
    assert.equal(!hasUnsafeText(fixture.text), fixture.valid, fixture.name);
    assert.equal(validSuggestion(fixture.text), fixture.valid, fixture.name);
  }
  assert.equal(hasUnsafeText('normal\ncontext\ttext', true), false);
  assert.equal(hasUnsafeText('normal\ncontext\ttext'), true);
  assert.equal(hasUnsafeText('unsafe\u202econtext', true), true);
});

test('corrections preserve one exact whole-word suffix and its optional space', () => {
  for (const [before, original, replacement] of [
    ['Use this adress', 'adress', 'address'], ['Use this adress ', 'adress ', 'address '],
    ['adress ', 'adress ', 'address '],
  ]) assert.equal(validCorrection(before, original, replacement), true);
  for (const [before, original, replacement] of [
    ['badadress ', 'adress ', 'address '], ['adress ', 'adress ', 'address'],
    ['adress', 'adress', 'address '], ['adress ', 'adress ', 'adress '],
    ['adress  ', 'adress  ', 'address  '], ['adress\n', 'adress\n', 'address\n'],
    ['adress ', 'adress ', 'address\n'], ['adress\t', 'adress\t', 'address\t'],
    ['adress ', 'other ', 'address '], ['Adress ', 'Adress ', 'Address '],
    ['adress ', 'adress ', 'address; '],
  ]) assert.equal(validCorrection(before, original, replacement), false, JSON.stringify([before, original, replacement]));
});
