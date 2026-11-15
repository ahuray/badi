#!/usr/bin/env node
// Neovim's link to the broker. The editor owns acquisition, display and the
// edit; this process only speaks the broker protocol through the shared
// client, one JSON line per message on stdin and stdout. It never prints
// buffer text anywhere but its reply to Neovim, and logs nothing.
import { boundedContext, refusalReason } from '../shared/broker-client.mjs';
import { createBridgeStdio } from '../shared/bridge-stdio.mjs';
import { requestBroker } from '../shared/request-broker.mjs';
import { RecoveringBrokerClient } from '../shared/recovering-client.mjs';
import { recordActivity } from '../shared/activity.mjs';
import { bridgeLanguage } from '../shared/writing-language.mjs';
import { nextRevisionMode, revisionDiff, revisionModes } from '../shared/revision.mjs';

const LANGUAGE = /^[A-Za-z]{2,3}(?:-[A-Za-z0-9]{1,8})*$/;
const ACCEPTANCE = new Set(['all', 'word']);
const RESULTS = new Set(['applied', 'stale', 'failed']);

const client = new RecoveringBrokerClient('neovim', { textReplacement: true, selectionRevision: true });
let grant = null;
let revisionRequest = null;
// Exit after ten idle minutes unless a command is in flight or Neovim holds
// its terminal's focus (the broker drops the focus report with the connection).
const stdio = createBridgeStdio({ concurrent: true, onClose: () => client.close() });
const send = stdio.writeJson;

// The client clears its candidate on expiry, revocation, pause and
// disconnect; Neovim withdraws the preview it shows for that candidate.
client.on('clear', () => send({ event: 'clear' }));
client.on('update', text => send({ event: 'update', text, word: client.acceptWord }));
// Each state carries the policy as it now stands, so Neovim reads no buffer
// text for a request the broker would refuse.
const policy = () => ({ connected: client.connected, allowed: client.allowed, automatic: client.automaticAllowed });
client.on('state', state => {
  if (typeof state === 'string' && /^[a-z_]{1,48}$/.test(state)) {
    if (revisionRequest) revisionRequest.reason = state;
    send({ event: 'state', state, ...policy() });
  }
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
  if (!client.connected) {
    if (command.explicit) client.pauseRetries();
    const ready = await requestBroker({ waitMs: command.explicit ? 20000 : 0 });
    await client.connect(undefined, { immediate: command.explicit || ready }).catch(() => false);
  }
  if (!client.allowed) return { ok: false, reason: reason() };
  if (!command.explicit && !client.automaticAllowed) return { ok: false, reason: 'manual_required' };
  const { before, after } = boundedContext(command.before, command.after ?? '');
  const language = bridgeLanguage(before, command.language);
  if (!language) return { ok: false, reason: 'language_unavailable' };
  recordActivity('neovim', 'context', command.explicit ? 'explicit_buffer' : 'automatic_buffer');
  grant = null;
  const text = await client.suggest(before, { language, explicit: command.explicit, after });
  return text ? { ok: true, text, word: client.acceptWord,
    ...(client.replacementBefore !== undefined ? { replace_before: client.replacementBefore } : {}) }
    : { ok: true, text: null };
}

async function handle(command) {
  switch (command.op) {
    case 'revise': {
      if (!validContext(command.before, 4096) || !validContext(command.after ?? '', 1024) ||
          !validContext(command.selected, 2400) || !revisionModes.has(command.mode)) {
        return { ok: false, reason: 'invalid_request' };
      }
      if (!client.connected) {
        client.pauseRetries();
        await requestBroker({ waitMs: 20000 });
        await client.connect(undefined, { immediate: true }).catch(() => false);
      }
      if (!client.allowed) return { ok: false, reason: reason() };
      grant = null;
      const { before, after } = boundedContext(command.before, command.after ?? '');
      const pending = { reason: null };
      revisionRequest = pending;
      try {
        const result = await client.revise({ before, after, selected: command.selected, selection: command.selection,
          mode: command.mode, language: command.language });
        if (revisionRequest !== pending) return { ok: false, reason: 'stale' };
        // A clear names its own refusal. Only no_change permits the next
        // revision mode; missing assets, policy and provider failures do not.
        if (!result && pending.reason !== 'no_change') {
          return { ok: false, reason: pending.reason ?? 'expired_or_unavailable' };
        }
        return { ok: true, text: result?.text ?? null, mode: command.mode,
          next_mode: nextRevisionMode(command.mode, command.selected),
          ...(result ? { diff: revisionDiff(command.selected, result.text) } : { reason: 'no_change' }) };
      } finally {
        if (revisionRequest === pending) revisionRequest = null;
      }
    }
    case 'accept_revision': {
      grant = null;
      grant = await client.authorizeRevision();
      return { ok: true, text: grant.text, replace_selection: grant.replaceSelection };
    }
    case 'suggest':
      return suggest(command);
    case 'alternative': {
      grant = null;
      const result = await client.alternative();
      if (result && typeof result === 'object') return { ok: true, words: [...result.words] };
      return result ? { ok: true, text: result, word: client.acceptWord } : { ok: true, text: null };
    }
    case 'pick': {
      if (!validContext(command.word, 128)) return { ok: false, reason: 'invalid_request' };
      grant = null;
      const text = await client.pickAlternative(command.word);
      return text ? { ok: true, text, word: client.acceptWord } : { ok: true, text: null };
    }
    case 'accept': {
      if (!ACCEPTANCE.has(command.acceptance)) return { ok: false, reason: 'invalid_request' };
      grant = null;
      grant = await client.authorize(command.acceptance);
      recordActivity('neovim', 'commit', 'authorized');
      return { ok: true, text: grant.text,
        ...(grant.replaceBefore !== undefined ? { replace_before: grant.replaceBefore } : {}) };
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
      stdio.setHeld(command.focused);
      if (!client.connected) await client.connect().catch(() => false);
      const withheld = await client.reportFocus(stdio.held).catch(() => false);
      return { ok: true, withheld };
    }
    case 'status': {
      if (!client.connected) {
        const explicit = command.explicit === true;
        if (explicit) client.pauseRetries();
        const ready = await requestBroker({ waitMs: explicit ? 20000 : 0 });
        await client.connect(undefined, { immediate: explicit || ready }).catch(() => false);
      }
      return { ok: true, ...policy() };
    }
    default:
      return { ok: false, reason: 'invalid_request' };
  }
}

try {
  void client.connect().catch(() => undefined);
  await stdio.serve({
    onInvalid() {
      send({ ok: false, reason: 'invalid_request' });
    },
    async onCommand(command) {
      const id = Number.isSafeInteger(command.id) && command.id >= 0 ? command.id : undefined;
      // Commands run concurrently: a newer suggestion supersedes an older one
      // in the client, which rejects the older request with a changed context.
      try {
        const reply = await handle(command);
        send({ id, ...reply });
      } catch {
        if (['accept', 'suggest', 'revise', 'accept_revision'].includes(command.op)) grant = null;
        send({ id, ok: false, reason: reason() });
      }
    },
  });
} finally {
  stdio.close();
}
