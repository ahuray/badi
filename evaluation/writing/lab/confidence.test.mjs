import test from 'node:test';
import assert from 'node:assert/strict';
import { confidenceCoverage } from './public/confidence.mjs';

const row = (id, language, outcome, score, review, onTime = true) => ({
  case: { id, language }, config: { mode: 'context_confidence' },
  result: { outcome, within_production_budget: onTime,
    candidate_mean_token_logprob: score, candidate_logprob_token_count: score === null ? null : 2 },
  ...(review ? { review: { judgment: review } } : {}),
});

test('coverage keeps every opportunity and withholds precision until full retained review', () => {
  const records = [
    row('a', 'en', 'suggestion', -0.2, 'Useful'),
    row('b', 'en', 'suggestion', -0.5, null),
    row('c', 'en', 'suggestion', null, 'Harmful'),
    row('d', 'en', 'deadline', null, null),
    row('e', 'fa', 'suggestion', -0.2, 'Harmful'),
    row('f', 'fa', 'suggestion', -0.1, 'Useful', false),
  ];
  const [all, english, persian] = confidenceCoverage(records);
  assert.deepEqual([all.requests, all.on_time_suggestions, all.scored_suggestions,
    all.missing_scores, all.reviewed_scores], [6, 4, 3, 1, 2]);
  assert.deepEqual(all.points.map(point => point.threshold), [-0.2, -0.5]);
  assert.deepEqual(all.points[0], { threshold: -0.2, retained: 2, reviewed: 2,
    useful: 1, harmful: 1, coverage: 2 / 6, useful_per_request: 1 / 6, precision: .5 });
  assert.equal(all.points[1].retained, 3);
  assert.equal(all.points[1].precision, null);
  assert.equal(all.points[1].useful_per_request, null);
  assert.equal(english.requests, 4);
  assert.equal(persian.requests, 2);
  assert.equal(persian.on_time_suggestions, 1);
  assert.equal(persian.points[0].harmful, 1);
});

test('unsupported or absent confidence data cannot become scored coverage', () => {
  const records = [row('a', 'de', 'suggestion', NaN, 'Useful'), row('b', 'de', 'abstention', null, null)];
  records.push({ ...row('c', 'de', 'suggestion', -0.1, 'Useful'), config: { mode: 'context' } });
  const [group] = confidenceCoverage(records);
  assert.equal(group.requests, 2);
  assert.equal(group.scored_suggestions, 0);
  assert.equal(group.missing_scores, 1);
  assert.deepEqual(group.points, []);
});
