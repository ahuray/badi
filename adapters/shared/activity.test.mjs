import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, existsSync, chmodSync, lstatSync, unlinkSync, symlinkSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import { recordActivity } from './activity.mjs';
import { validSuggestion } from './broker-client.mjs';

function debugRuntime(context) {
  // Disabled lookups are remembered for a second: advance a mocked clock past it.
  context.mock.timers.enable({ apis: ['Date'], now: Date.now() });
  const root = mkdtempSync('/tmp/badi-activity-test-');
  const previous = process.env.XDG_RUNTIME_DIR;
  process.env.XDG_RUNTIME_DIR = root;
  const dir = join(root, 'badi'); mkdirSync(dir, { mode: 0o700 });
  context.after(() => {
    context.mock.timers.reset();
    if (previous === undefined) delete process.env.XDG_RUNTIME_DIR; else process.env.XDG_RUNTIME_DIR = previous;
    rmSync(root, { recursive: true, force: true });
  });
  return { dir, marker: join(dir, 'debug-control.json'), snapshot: join(dir, 'debug-obsidian.json'),
    later: () => context.mock.timers.tick(1000) };
}

test('activity is opt-in, private, expiring and excludes typed content', context => {
  const { marker, snapshot, later } = debugRuntime(context);
  const control = { id: randomUUID(), expires_at: Math.floor(Date.now() / 1000) + 60 };
  recordActivity('obsidian', 'context', 'input'); assert.equal(existsSync(snapshot), false);
  writeFileSync(marker, JSON.stringify(control), { mode: 0o600 });
  later(); recordActivity('obsidian', 'context', 'input');
  const state = JSON.parse(readFileSync(snapshot, 'utf8'));
  assert.equal(state.counts.context, 1);
  assert.equal(lstatSync(snapshot).mode & 0o777, 0o600);
  assert.equal(Object.hasOwn(state, 'before'), false);
  unlinkSync(snapshot);
  recordActivity('obsidian', 'context', 'private text with digits 123'); assert.equal(existsSync(snapshot), false);
  chmodSync(marker, 0o644); recordActivity('obsidian', 'context', 'input'); assert.equal(existsSync(snapshot), false);
  chmodSync(marker, 0o600);
  writeFileSync(marker, JSON.stringify({ ...control, expires_at: 0 }));
  later(); recordActivity('obsidian', 'context', 'input'); assert.equal(existsSync(snapshot), false);
  writeFileSync(marker, JSON.stringify({ ...control, id: randomUUID() }));
  later(); recordActivity('obsidian', 'context', 'input');
  assert.equal(JSON.parse(readFileSync(snapshot, 'utf8')).counts.context, 1);
  unlinkSync(snapshot); unlinkSync(marker); symlinkSync('/dev/null', marker);
  later(); recordActivity('obsidian', 'context', 'input'); assert.equal(existsSync(snapshot), false);
});

test('while debugging is off, the control file is looked for at most once a second', context => {
  const { marker, snapshot, later } = debugRuntime(context);
  recordActivity('obsidian', 'context', 'input');
  writeFileSync(marker, JSON.stringify({ id: randomUUID(), expires_at: Math.floor(Date.now() / 1000) + 60 }), { mode: 0o600 });
  for (let frame = 0; frame < 50; frame++) recordActivity('obsidian', 'transport', 'sent context.changed');
  assert.equal(existsSync(snapshot), false, 'Frames within a second of a disabled lookup touch no file');
  later(); recordActivity('obsidian', 'transport', 'sent context.changed');
  assert.equal(JSON.parse(readFileSync(snapshot, 'utf8')).counts.transport, 1);
  // While enabled every event rechecks, so badi debug off stops recording at once.
  unlinkSync(marker); unlinkSync(snapshot);
  recordActivity('obsidian', 'transport', 'sent context.changed');
  assert.equal(existsSync(snapshot), false);
});

test('punctuation does not count as an extra predicted word', () => {
  assert.equal(validSuggestion(' — the connection is lost'), true);
  assert.equal(validSuggestion(' one two three four five'), false);
  for (const value of ['', ' ', 'word\nnext', 'word\u202enext']) assert.equal(validSuggestion(value), false);
});
