import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, existsSync, chmodSync, lstatSync, unlinkSync, symlinkSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import { recordActivity } from './activity.mjs';
import { validSuggestion } from './broker-client.mjs';

test('activity is opt-in, private, expiring and excludes typed content', () => {
  const root = mkdtempSync('/tmp/badi-activity-test-');
  const previous = process.env.XDG_RUNTIME_DIR;
  process.env.XDG_RUNTIME_DIR = root;
  const dir = join(root, 'badi'); mkdirSync(dir, { mode: 0o700 });
  const marker = join(dir, 'debug-control.json');
  const snapshot = join(dir, 'debug-obsidian.json');
  const control = { id: randomUUID(), expires_at: Math.floor(Date.now() / 1000) + 60 };
  try {
    recordActivity('obsidian', 'context', 'input'); assert.equal(existsSync(snapshot), false);
    writeFileSync(marker, JSON.stringify(control), { mode: 0o600 });
    recordActivity('obsidian', 'context', 'input');
    const state = JSON.parse(readFileSync(snapshot, 'utf8'));
    assert.equal(state.counts.context, 1);
    assert.equal(lstatSync(snapshot).mode & 0o777, 0o600);
    assert.equal(Object.hasOwn(state, 'before'), false);
    unlinkSync(snapshot);
    recordActivity('obsidian', 'context', 'private text with digits 123'); assert.equal(existsSync(snapshot), false);
    chmodSync(marker, 0o644); recordActivity('obsidian', 'context', 'input'); assert.equal(existsSync(snapshot), false);
    chmodSync(marker, 0o600);
    writeFileSync(marker, JSON.stringify({ ...control, expires_at: 0 }));
    recordActivity('obsidian', 'context', 'input'); assert.equal(existsSync(snapshot), false);
    writeFileSync(marker, JSON.stringify({ ...control, id: randomUUID() }));
    recordActivity('obsidian', 'context', 'input');
    assert.equal(JSON.parse(readFileSync(snapshot, 'utf8')).counts.context, 1);
    unlinkSync(snapshot); unlinkSync(marker); symlinkSync('/dev/null', marker);
    recordActivity('obsidian', 'context', 'input'); assert.equal(existsSync(snapshot), false);
  } finally {
    if (previous === undefined) delete process.env.XDG_RUNTIME_DIR; else process.env.XDG_RUNTIME_DIR = previous;
    rmSync(root, { recursive: true, force: true });
  }
});

test('punctuation does not count as an extra predicted word', () => {
  assert.equal(validSuggestion(' — the connection is lost'), true);
  assert.equal(validSuggestion(' one two three four five'), false);
  for (const value of ['', ' ', 'word\nnext', 'word\u202enext']) assert.equal(validSuggestion(value), false);
});
