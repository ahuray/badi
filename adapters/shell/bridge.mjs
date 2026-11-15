#!/usr/bin/env node
import { BrokerClient, boundedContext, refusalReason } from '../shared/broker-client.mjs';
import { createBridgeStdio } from '../shared/bridge-stdio.mjs';
import { requestBroker } from '../shared/request-broker.mjs';
import { recordActivity } from '../shared/activity.mjs';
import { bridgeLanguage } from '../shared/writing-language.mjs';

// Bash prints a reason as it is, so the offline one names the next step.
const OFFLINE = 'model_offline_run_badi_doctor';
const reason = () => (refusalReason(client) === 'model_offline' ? OFFLINE : refusalReason(client));

let client = new BrokerClient('terminal', { textReplacement: true });
let connectionGeneration = 0;
let grant = null;
// Exit after ten idle minutes without a request so a quiet shell does not keep
// a Node process (about 60 MB); badi.bash starts a new bridge on the next Ctrl-X Tab.
const stdio = createBridgeStdio({ onClose: () => client.close() });
const respond = (status, value = '') => stdio.writeLine(`${status} ${value}`);
async function connect() {
  if (client.connected) return;
  // The bridge process exists only because the user asked. Wait for the model
  // to bind its socket; Bash allows this first reply that long.
  await requestBroker({ waitMs: 20000 });
  client.close();
  client = new BrokerClient('terminal', { textReplacement: true });
  grant = null;
  await client.connect();
  connectionGeneration++;
}
try {
  await connect();
  respond('READY', String(connectionGeneration));
  await stdio.serve({
    onInvalid({ reason: invalid }) {
      if (invalid === 'input_too_large') {
        respond('ERROR', 'input_too_large');
      } else if (invalid === 'invalid_object') {
        respond('ERROR', 'invalid_operation');
      } else {
        grant = null;
        client.cancel();
        respond('ERROR', reason());
      }
    },
    async onCommand(command) {
      try {
        if (command.operation === 'status') {
          await connect();
          await client.checkPolicy();
          respond(client.allowed ? 'READY' : 'BLOCKED', client.allowed
            ? String(connectionGeneration) : 'app_disabled_or_model_paused');
        } else if (command.operation === 'suggest') {
          if (!client.allowed) { respond('BLOCKED', 'app_disabled_or_model_paused'); return; }
          if (typeof command.before !== 'string' || !/^[A-Za-z0-9+/]*={0,2}$/.test(command.before)) throw new Error();
          const source = Buffer.from(command.before, 'base64').toString('utf8');
          if (Buffer.from(source).toString('base64') !== command.before) throw new Error();
          recordActivity('terminal', 'context', 'explicit_buffer');
          const { before } = boundedContext(source);
          const language = bridgeLanguage(before);
          if (!language) throw new Error('Writing language unavailable');
          const text = await client.suggest(before, { language });
          respond(text ? 'SUGGEST' : 'EMPTY', text ? Buffer.from(text).toString('base64') + (
            client.replacementBefore ? ` ${Buffer.from(client.replacementBefore).toString('base64')}` : '') : '');
        } else if (command.operation === 'accept') {
          grant = await client.authorize();
          respond('INSERT', Buffer.from(grant.text).toString('base64') + (
            grant.replaceBefore ? ` ${Buffer.from(grant.replaceBefore).toString('base64')}` : ''));
        } else if (command.operation === 'result' && grant) {
          const status = ['applied', 'stale', 'failed'].includes(command.status)
            ? command.status : command.applied === true ? 'applied' : 'failed';
          client.report(grant, status);
          grant = null;
          respond('DONE');
        } else if (command.operation === 'cancel' || command.operation === 'dismiss') {
          // Only dismiss (Ctrl-X Escape) tells the broker the user declined it.
          grant = null;
          if (command.operation === 'dismiss') client.dismiss(); else client.cancel();
          respond('DONE');
        } else {
          respond('ERROR', 'invalid_operation');
        }
      } catch {
        grant = null;
        client.cancel();
        respond('ERROR', reason());
      }
    },
  });
} catch {
  respond('ERROR', OFFLINE);
} finally {
  stdio.close();
}
