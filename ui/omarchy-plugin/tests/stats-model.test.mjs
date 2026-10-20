// The popout's usage history (StatsModel.js), loaded as the QML engine loads
// a `.pragma library` file: plain functions, no QML. Its arrays come from
// another realm, so each is spread into this one before comparing.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const source = readFileSync(new URL('../StatsModel.js', import.meta.url), 'utf8');
assert.match(source, /^\.pragma library\n/u);
const StatsModel = vm.runInNewContext(`${source.replace(/^\.pragma library\n/u, '')}
({ totals, week, label, leaders })`);

const zero = { shown: 0, accepted: 0, words: 0 };
const summary = {
  today: '2026-03-02',
  retention_days: 90,
  totals: {
    today: { shown: 3, accepted: 2, words: 5 },
    last_7_days: { shown: 12, accepted: 6, words: 12 },
    retained: { shown: 40, accepted: 20, words: 31 },
  },
  days: [
    { day: '2026-02-20', shown: 28, accepted: 14, words: 19 },
    { day: '2026-02-28', shown: 9, accepted: 4, words: 7 },
    { day: '2026-03-02', shown: 3, accepted: 2, words: 5 },
  ],
  subjects: [
    { identity: { kind: 'browser_origin', adapter: 'chromium', scheme: 'https', host: 'example.com', port: 443 },
      today: zero, last_7_days: { shown: 9, accepted: 4, words: 7 }, retained: { shown: 37, accepted: 18, words: 26 } },
    { identity: { kind: 'linux_app', adapter: 'fcitx', app_id: 'com.mitchellh.ghostty' },
      today: { shown: 3, accepted: 2, words: 5 }, last_7_days: { shown: 3, accepted: 2, words: 5 },
      retained: { shown: 3, accepted: 2, words: 5 } },
    { identity: { kind: 'browser_origin', adapter: 'chromium', scheme: 'http', host: 'mail.example', port: 8080 },
      today: zero, last_7_days: zero, retained: { shown: 1, accepted: 0, words: 0 } },
  ],
};
const names = { 'com.mitchellh.ghostty': 'Ghostty' };

test('totals read each window and default to zero', () => {
  assert.deepEqual({ ...StatsModel.totals(summary, 'today') }, { shown: 3, accepted: 2, words: 5 });
  assert.deepEqual({ ...StatsModel.totals(summary, 'retained') }, { shown: 40, accepted: 20, words: 31 });
  assert.deepEqual({ ...StatsModel.totals({}, 'today') }, zero);
  assert.deepEqual({ ...StatsModel.totals({ totals: { today: { shown: -1, words: 1.5 } } }, 'today') }, zero,
    'negative or fractional counts are not counts');
});

test('the week is seven days ending today, across a month end, with empty days at zero', () => {
  const days = [...StatsModel.week(summary)].map(({ day, words }) => [day, words]);
  assert.deepEqual(days, [
    ['2026-02-24', 0], ['2026-02-25', 0], ['2026-02-26', 0], ['2026-02-27', 0],
    ['2026-02-28', 7], ['2026-03-01', 0], ['2026-03-02', 5],
  ]);
  assert.deepEqual([...StatsModel.week({ ...summary, today: 'today' })], [], 'no date, no bars');
  assert.deepEqual([...StatsModel.week(undefined)], []);
});

test('labels name known apps, other app ids and sites with a port only when it is not the default', () => {
  assert.equal(StatsModel.label(summary.subjects[0].identity, names), 'example.com');
  assert.equal(StatsModel.label(summary.subjects[1].identity, names), 'Ghostty');
  assert.equal(StatsModel.label(summary.subjects[1].identity, {}), 'com.mitchellh.ghostty');
  assert.equal(StatsModel.label(summary.subjects[2].identity, names), 'mail.example:8080');
  assert.equal(StatsModel.label(null, names), '');
});

test('leaders rank the last seven days by words, then shown, and leave out quiet apps', () => {
  const ranked = [...StatsModel.leaders(summary, names, 3)].map(({ label, words }) => [label, words]);
  assert.deepEqual(ranked, [['example.com', 7], ['Ghostty', 5]]);
  assert.equal(StatsModel.leaders(summary, names, 1).length, 1);
  const tie = { subjects: [
    { identity: { kind: 'linux_app', adapter: 'fcitx', app_id: 'omawrite' }, last_7_days: { shown: 2, accepted: 0, words: 0 } },
    { identity: { kind: 'linux_app', adapter: 'fcitx', app_id: 'telegram' }, last_7_days: { shown: 5, accepted: 0, words: 0 } },
  ] };
  assert.deepEqual([...StatsModel.leaders(tie, {}, 3)].map(({ label }) => label), ['telegram', 'omawrite']);
  assert.deepEqual([...StatsModel.leaders({}, names, 3)], []);
});
