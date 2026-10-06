import test from 'node:test';
import assert from 'node:assert/strict';
import { reviewsForConfiguration } from './public/qualification.mjs';

test('only explicit full-addition judgments bind to the current server result targets', () => {
  const target = { run_id: 'run', case_id: 'a', config_id: 'healed', result_sha256: '1'.repeat(64), outcome: 'suggestion' };
  const config = { config: { id: 'healed' }, review_targets: [target] };
  const record = { review_target: target, config: { id: 'healed' }, case: { id: 'a', expected: ['reference'] }, result: { text: 'correct word but unwanted tail' } };
  assert.deepEqual(reviewsForConfiguration({ records: [record] }, config), []);
  for (const [label, judgment, substantive] of [['Useful', 'useful', true], ['Acceptable alternative', 'neutral', false], ['Harmful', 'harmful', false]]) {
    const reviews = reviewsForConfiguration({ records: [{ ...record, review: { judgment: label } }] }, config);
    assert.deepEqual(reviews, [{ ...target, judgment, full_addition_reviewed: true, substantive }]);
    assert.equal(JSON.stringify(reviews).includes('reference'), false);
    assert.equal(JSON.stringify(reviews).includes('tail'), false);
  }
  assert.throws(() => reviewsForConfiguration({ records: [{ ...record, review: { judgment: 'Useful' } }] }, { ...config, review_targets: [] }), /targets changed/u);
  assert.throws(() => reviewsForConfiguration({ records: [{ ...record, review_target: { ...target, result_sha256: '9'.repeat(64) }, review: { judgment: 'Useful' } }] }, config), /different result/u);
  const missing = { ...target, outcome: 'missing' };
  const missingConfig = { ...config, review_targets: [missing] };
  assert.deepEqual(reviewsForConfiguration({ records: [], missingReviews: [missing] }, missingConfig), [{ ...missing, judgment: 'unhelpful', full_addition_reviewed: true, substantive: false }]);
  assert.throws(() => reviewsForConfiguration({ records: [], missingReviews: [{ ...missing, result_sha256: '2'.repeat(64) }] }, missingConfig), /targets changed/u);
});
