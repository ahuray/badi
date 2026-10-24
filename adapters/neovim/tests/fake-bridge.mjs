#!/usr/bin/env node
// A scripted stand-in for bridge.mjs, so plugin tests control timing, grants,
// policy and exits. It records each operation's name and flags, never its
// text, to FAKE_BRIDGE_LOG.
import { appendFileSync } from 'node:fs';
import { createInterface } from 'node:readline';

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
let candidate = null;
let generation = 0;
// The broker's policy as bridge.mjs reports it. FAKE_BRIDGE_POLICY 'refused'
// or 'manual' starts so; 'silent' allows but reports only when asked.
const initial = process.env.FAKE_BRIDGE_POLICY || 'automatic';
let policy = { connected: true, allowed: initial !== 'refused', automatic: initial === 'automatic' || initial === 'silent' };
const report = state => send({ event: 'state', state, ...policy });

log({ op: 'start' });
if (initial !== 'silent') report(policy.allowed ? 'ready' : 'app_disabled');
for await (const line of createInterface({ input: process.stdin })) {
  const command = JSON.parse(line);
  log({ op: command.op, explicit: command.explicit, acceptance: command.acceptance, status: command.status });
  if (command.op === 'suggest') {
    if (!policy.allowed || (!command.explicit && !policy.automatic)) {
      send({ id: command.id, ok: false, reason: policy.allowed ? 'manual_required' : 'app_disabled_or_model_paused' });
      continue;
    }
    const mine = ++generation;
    candidate = null;
    const rule = RULES.find(([end]) => command.before.endsWith(end));
    if (rule?.[0] === 'crash') process.exit(3);
    if (rule?.[0] === 'slow') await delay(400);
    if (mine !== generation) { send({ id: command.id, ok: false, reason: 'expired_or_unavailable' }); continue; }
    candidate = rule ? rule[1] : null;
    send({ id: command.id, ok: true, text: candidate });
  } else if (command.op === 'accept') {
    const text = candidate === null ? null
      : command.acceptance === 'word' ? candidate.match(/^ ?\S+/u)[0] : candidate;
    if (candidate === ' arrives late') await delay(400);
    send(text === null ? { id: command.id, ok: false, reason: 'expired_or_unavailable' } : { id: command.id, ok: true, text });
  } else if (command.op === 'cancel' || command.op === 'dismiss') {
    generation++;
    candidate = null;
    send({ ok: true });
  } else if (command.op === 'result') {
    candidate = null;
    send({ id: command.id, ok: true });
  } else if (command.op === 'expire') {
    candidate = null;
    send({ event: 'clear' });
  } else if (command.op === 'status') {
    send({ id: command.id, ok: true, ...policy });
  } else if (command.op === 'policy') {
    // A settings change, as the broker's new authority epoch reports it.
    policy = { connected: true, allowed: command.allowed === true, automatic: command.automatic === true };
    candidate = null;
    report(policy.allowed ? 'ready' : 'paused');
  }
}
