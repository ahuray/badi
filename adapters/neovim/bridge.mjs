#!/usr/bin/env node
// Neovim's link to the broker. The editor owns acquisition, display and the
// edit; this process only speaks the broker protocol through the shared
// client, one JSON line per message on stdin and stdout. It never prints
// buffer text anywhere but its reply to Neovim, and logs nothing.
import { createInterface } from 'node:readline';
import { boundedContext, refusalReason } from '../shared/broker-client.mjs';
import { RecoveringBrokerClient } from '../shared/recovering-client.mjs';
import { recordActivity } from '../shared/activity.mjs';
import { bridgeLanguage } from '../shared/writing-language.mjs';

// Exit after this long without a command so an idle editor does not keep a
// Node process (about 60 MB); the plugin starts a new bridge when it next needs one.
const IDLE_MS = /^[1-9][0-9]{0,8}$/.test(process.env.BADI_BRIDGE_IDLE_MS ?? '')
  ? Number(process.env.BADI_BRIDGE_IDLE_MS) : 10 * 60 * 1000;
const LINE_LIMIT = 16384;
const LANGUAGE = /^[A-Za-z]{2,3}(?:-[A-Za-z0-9]{1,8})*$/;
const ACCEPTANCE = new Set(['all', 'word']);
const RESULTS = new Set(['applied', 'stale', 'failed']);

const client = new RecoveringBrokerClient('neovim');
let grant = null;
// While Neovim holds its terminal's focus the bridge stays connected, since
// the broker drops the focus report with the connection.
let focused = false;
const send = message => process.stdout.write(`${JSON.stringify(message)}\n`);
const lines = createInterface({ input: process.stdin, terminal: false });
let idle = null;
const armIdleExit = () => {
  clearTimeout(idle);
  if (focused) return;
  idle = setTimeout(() => { lines.close(); process.stdin.destroy(); }, IDLE_MS);
  idle.unref();
};

// The client clears its candidate on expiry, revocation, pause and
// disconnect; Neovim withdraws the preview it shows for that candidate.
client.on('clear', () => send({ event: 'clear' }));
client.on('update', text => send({ event: 'update', text }));
// Each state carries the policy as it now stands, so Neovim reads no buffer
// text for a request the broker would refuse.
const policy = () => ({ connected: client.connected, allowed: client.allowed, automatic: client.automaticAllowed });
client.on('state', state => {
  if (typeof state === 'string' && /^[a-z_]{1,48}$/.test(state)) send({ event: 'state', state, ...policy() });
});

const reason = () => refusalReason(client);

// A decoded line with U+FFFD came from bytes that were not UTF-8; refuse it
// rather than ask for a continuation of text the buffer does not contain.
const validContext = (value, limit) => typeof value === 'string' && Buffer.byteLength(value) <= limit &&
  !value.includes('�');

async function suggest(command) {
  if (!validContext(command.before, 4096) || !validContext(command.after ?? '', 1024) ||
      typeof command.explicit !== 'boolean' ||
      (command.language !== undefined && (typeof command.language !== 'string' || !LANGUAGE.test(command.language)))) {
    return { ok: false, reason: 'invalid_request' };
  }
  if (!client.connected) await client.connect().catch(() => false);
  if (!client.allowed) return { ok: false, reason: reason() };
  if (!command.explicit && !client.automaticAllowed) return { ok: false, reason: 'manual_required' };
  const { before, after } = boundedContext(command.before, command.after ?? '');
  const language = bridgeLanguage(before, command.language);
  if (!language) return { ok: false, reason: 'language_unavailable' };
  recordActivity('neovim', 'context', command.explicit ? 'explicit_buffer' : 'automatic_buffer');
  grant = null;
  const text = await client.suggest(before, { language, explicit: command.explicit, after });
  return text ? { ok: true, text } : { ok: true, text: null };
}

async function handle(command) {
  switch (command.op) {
    case 'suggest':
      return suggest(command);
    case 'accept': {
      if (!ACCEPTANCE.has(command.acceptance)) return { ok: false, reason: 'invalid_request' };
      grant = null;
      grant = await client.authorize(command.acceptance);
      recordActivity('neovim', 'commit', 'authorized');
      return { ok: true, text: grant.text };
    }
    case 'result': {
      if (!RESULTS.has(command.status) || !grant) return { ok: false, reason: 'no_grant' };
      client.report(grant, command.status);
      recordActivity('neovim', 'commit', command.status);
      grant = null;
      return { ok: true };
    }
    case 'dismiss':
      grant = null;
      client.dismiss();
      return { ok: true };
    case 'cancel':
      grant = null;
      client.cancel();
      return { ok: true };
    case 'focus': {
      if (typeof command.focused !== 'boolean') return { ok: false, reason: 'invalid_request' };
      focused = command.focused;
      if (!client.connected) await client.connect().catch(() => false);
      const withheld = await client.reportFocus(focused).catch(() => false);
      return { ok: true, withheld };
    }
    case 'status':
      if (!client.connected) await client.connect().catch(() => false);
      return { ok: true, ...policy() };
    default:
      return { ok: false, reason: 'invalid_request' };
  }
}

try {
  void client.connect().catch(() => undefined);
  armIdleExit();
  for await (const line of lines) {
    clearTimeout(idle);
    let command;
    try {
      if (line.length > LINE_LIMIT) throw new Error('input_too_large');
      command = JSON.parse(line);
      if (!command || typeof command !== 'object' || Array.isArray(command)) throw new Error('invalid');
    } catch {
      send({ ok: false, reason: 'invalid_request' });
      armIdleExit();
      continue;
    }
    const id = Number.isSafeInteger(command.id) && command.id >= 0 ? command.id : undefined;
    // Commands run concurrently: a newer suggestion supersedes an older one
    // in the client, which rejects the older request with a changed context.
    void handle(command).then(reply => send({ id, ...reply }), () => {
      if (command.op === 'accept' || command.op === 'suggest') grant = null;
      send({ id, ok: false, reason: reason() });
    }).finally(armIdleExit);
  }
} finally {
  clearTimeout(idle);
  client.close();
  lines.close();
}
