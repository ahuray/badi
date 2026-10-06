import { test } from 'node:test';
import assert from 'node:assert/strict';
import { writingLanguage } from './writing-language.mjs';

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
