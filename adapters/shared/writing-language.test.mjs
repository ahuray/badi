import { test } from 'node:test';
import assert from 'node:assert/strict';
import { bridgeLanguage, writingLanguage } from './writing-language.mjs';
import { boundedContext } from './broker-client.mjs';

test('language follows authorized script switches and retains explicit Latin language hints', () => {
  assert.equal(writingLanguage('Please send the', 'en-us'), 'en-US');
  assert.equal(writingLanguage('Bitte senden Sie', 'de-DE'), 'de-DE');
  assert.equal(writingLanguage('Please reply: لطفا برای من', 'en-US'), 'fa');
  assert.equal(writingLanguage('لطفا برای من ... 123', 'de-DE'), 'fa');
  assert.equal(writingLanguage('فارسی then English', 'en-US'), 'en-US');
  assert.equal(writingLanguage('مرحبا', 'ar'), 'ar');
  assert.equal(writingLanguage('فارسی', 'not_a_language'), undefined);
  assert.equal(writingLanguage('plain untagged'), undefined);
});

test('a bridge prefers the editor language, then BADI_LANGUAGE, and reads the broker-bounded context', () => {
  const saved = process.env.BADI_LANGUAGE;
  try {
    process.env.BADI_LANGUAGE = 'de';
    assert.equal(bridgeLanguage('Bitte senden Sie'), 'de');
    assert.equal(bridgeLanguage('Please send the', 'en'), 'en');
    assert.equal(bridgeLanguage('لطفا برای من'), 'fa', 'the script nearest the caret still decides Persian');
  } finally {
    if (saved === undefined) delete process.env.BADI_LANGUAGE; else process.env.BADI_LANGUAGE = saved;
  }
  const persian = 'س'.repeat(600);
  const window = boundedContext(`x${persian}`, `${persian}y`);
  assert.equal(window.before, persian.slice(-512), 'the last 512 scalars before the caret');
  assert.equal(window.after, persian.slice(0, 128), 'the first 128 after it');
  assert.equal([...boundedContext('😀'.repeat(600)).before].length, 512, 'counted in scalars, not UTF-16 units');
  assert.equal(boundedContext('thank you').after, '');
});
