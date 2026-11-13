#!/usr/bin/env node
// A scripted stand-in for bridge.mjs, so plugin tests control timing, grants,
// policy and exits. It records each operation's name and flags, never its
// text, to FAKE_BRIDGE_LOG.
import { appendFileSync } from 'node:fs';
import { createInterface } from 'node:readline';
import { nextRevisionMode, revisionDiff } from '../../shared/revision.mjs';

const log = entry => appendFileSync(process.env.FAKE_BRIDGE_LOG, `${JSON.stringify(entry)}\n`);
const send = message => process.stdout.write(`${JSON.stringify(message)}\n`);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
// Ends of the text before the caret, and what each one suggests.
const RULES = [
  ['thank you', ' for your time'],
  ['thank you for', ' your time'],
  ['Grüße und سلام ', 'Welt bleibt'],
  ['slow', ' reply'],
  ['grant later', ' arrives late'],
  ['crash', ' never'],
];
// What the request key on a shown suggestion offers next, in turn.
const ALTERNATIVES = new Map([[' for your time', ' so much'], [' so much', ' again'], [' again', ' for your time']]);
// Suffix corrections, most specific suffix first.
const CORRECTIONS = [
  ['slow adress ', { text: 'address ', before: 'adress ', grant: 'adress ', late: true }],
  ['revoked adress ', { text: 'address ', before: 'adress ', grant: 'adress ', clear: true }],
  ['other adress ', { text: 'address ', before: 'adress ', grant: 'other ' }],
  [' adress ', { text: 'address ', before: 'adress ', grant: 'adress ' }],
];
let candidate = null;
let replaceBefore = null;
let grantReplaceBefore = null;
let lateAccept = false;
let clearBeforeGrant = false;
let generation = 0;
let revisionSelection = null;
let synonymIndex = 0;
const clearCandidate = () => {
  candidate = null;
  replaceBefore = null;
  grantReplaceBefore = null;
  lateAccept = false;
  clearBeforeGrant = false;
  revisionSelection = null;
};
// The broker's policy as bridge.mjs reports it. FAKE_BRIDGE_POLICY 'refused'
// or 'manual' starts so; 'silent' allows but reports only when asked.
const initial = process.env.FAKE_BRIDGE_POLICY || 'automatic';
const revisionReasons = JSON.parse(process.env.FAKE_BRIDGE_REVISION_REASONS || '[]');
let policy = { connected: true, allowed: initial !== 'refused', automatic: initial === 'automatic' || initial === 'silent' };
const report = state => send({ event: 'state', state, ...policy });

log({ op: 'start' });
if (initial !== 'silent') report(policy.allowed ? 'ready' : 'app_disabled');
for await (const line of createInterface({ input: process.stdin })) {
  const command = JSON.parse(line);
  log({ op: command.op, explicit: command.explicit, acceptance: command.acceptance, status: command.status, mode: command.mode });
  if (command.op === 'revise') {
    clearCandidate();
    if (command.selected.startsWith('slow ')) await delay(400);
    const reason = revisionReasons.shift();
    if (reason) {
      report(reason);
      send(reason === 'no_change'
        ? { id: command.id, ok: true, text: null, reason, mode: command.mode,
          next_mode: nextRevisionMode(command.mode, command.selected) }
        : { id: command.id, ok: false, reason });
      continue;
    }
    let text = command.selected.replace('has went', 'went');
    if (command.mode === 'refine') text = text.charAt(0).toUpperCase() + text.slice(1);
    if (command.mode === 'synonyms') text = ['large', 'huge'][synonymIndex++ % 2];
    candidate = text === command.selected ? null : text;
    revisionSelection = command.selection;
    lateAccept = command.selected.startsWith('late ');
    clearBeforeGrant = command.selected.startsWith('revoked ');
    if (command.selected.startsWith('wrong ')) revisionSelection = { ...revisionSelection, anchor: revisionSelection.anchor + 1 };
    send({ id: command.id, ok: true, text: candidate, mode: command.mode,
      next_mode: nextRevisionMode(command.mode, command.selected),
      ...(candidate ? { diff: revisionDiff(command.selected, text) } : { reason: 'no_change' }) });
  } else if (command.op === 'accept_revision') {
    const text = candidate, selection = revisionSelection;
    if (clearBeforeGrant) send({ event: 'clear' });
    if (lateAccept) await delay(400);
    send({ id: command.id, ok: true, text, replace_selection: selection });
  } else if (command.op === 'suggest') {
    if (!policy.allowed || (!command.explicit && !policy.automatic)) {
      send({ id: command.id, ok: false, reason: policy.allowed ? 'manual_required' : 'app_disabled_or_model_paused' });
      continue;
    }
    const mine = ++generation;
    clearCandidate();
    const correction = CORRECTIONS.find(([end]) => command.before.endsWith(end))?.[1];
    const rule = correction ? null : RULES.find(([end]) => command.before.endsWith(end));
    if (rule?.[0] === 'crash') process.exit(3);
    if (rule?.[0] === 'slow') await delay(400);
    if (mine !== generation) { send({ id: command.id, ok: false, reason: 'expired_or_unavailable' }); continue; }
    candidate = correction?.text ?? (rule ? rule[1] : null);
    replaceBefore = correction?.before ?? null;
    grantReplaceBefore = correction?.grant ?? null;
    lateAccept = correction?.late === true;
    clearBeforeGrant = correction?.clear === true;
    send({ id: command.id, ok: true, text: candidate,
      word: replaceBefore ? candidate : candidate?.match(/^ ?\S+/u)?.[0],
      ...(replaceBefore ? { replace_before: replaceBefore } : {}) });
  } else if (command.op === 'alternative') {
    if (candidate === null) { send({ id: command.id, ok: false, reason: 'expired_or_unavailable' }); continue; }
    if (candidate === ' for your time') { clearCandidate(); await delay(400); send({ id: command.id, ok: true, words: ['so', 'again'] }); continue; }
    if (candidate === ' so much') { clearCandidate(); send({ id: command.id, ok: true, words: ['again'] }); continue; }
    candidate = ALTERNATIVES.get(candidate) ?? null;
    replaceBefore = null;
    grantReplaceBefore = null;
    lateAccept = false;
    clearBeforeGrant = false;
    send({ id: command.id, ok: true, text: candidate, word: candidate?.match(/^ ?\S+/u)?.[0] });
  } else if (command.op === 'pick') {
    candidate = { so: ' so much', again: ' again', for: ' for your time' }[command.word] ?? null;
    replaceBefore = null;
    grantReplaceBefore = null;
    lateAccept = false;
    clearBeforeGrant = false;
    send({ id: command.id, ok: true, text: candidate, word: candidate?.match(/^ ?\S+/u)?.[0] });
  } else if (command.op === 'accept') {
    const text = candidate === null ? null
      : replaceBefore ? candidate
      : command.acceptance === 'word' ? candidate.match(/^ ?\S+/u)[0] : candidate;
    const grant = grantReplaceBefore;
    if (clearBeforeGrant) send({ event: 'clear' });
    if (candidate === ' arrives late' || lateAccept) await delay(400);
    send(text === null ? { id: command.id, ok: false, reason: 'expired_or_unavailable' }
      : { id: command.id, ok: true, text, ...(grant ? { replace_before: grant } : {}) });
  } else if (command.op === 'cancel' || command.op === 'dismiss') {
    generation++;
    clearCandidate();
    send({ ok: true });
  } else if (command.op === 'result') {
    clearCandidate();
    send({ id: command.id, ok: true });
  } else if (command.op === 'expire') {
    clearCandidate();
    send({ event: 'clear' });
  } else if (command.op === 'status') {
    send({ id: command.id, ok: true, ...policy });
  } else if (command.op === 'policy') {
    // A settings change, as the broker's new authority epoch reports it.
    policy = { connected: true, allowed: command.allowed === true, automatic: command.automatic === true };
    clearCandidate();
    report(policy.allowed ? 'ready' : 'paused');
  }
}
