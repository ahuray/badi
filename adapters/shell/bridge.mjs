#!/usr/bin/env node
import { createInterface } from 'node:readline';
import { BrokerClient } from '../shared/broker-client.mjs';
import { recordActivity } from '../shared/activity.mjs';
import { writingLanguage } from '../shared/writing-language.mjs';

let client = new BrokerClient('terminal', { textReplacement: true });
let connectionGeneration = 0;
let grant = null;
const respond = (status, value = '') => process.stdout.write(`${status} ${value}\n`);
const lines = createInterface({ input: process.stdin, terminal: false });
async function connect() {
  if (client.connected) return;
  client.close();
  client = new BrokerClient('terminal', { textReplacement: true });
  grant = null;
  await client.connect();
  connectionGeneration++;
}
try {
  await connect();
  respond('READY', String(connectionGeneration));
  for await (const line of lines) {
    if (line.length > 16384) { respond('ERROR', 'input_too_large'); continue; }
    try {
      const command = JSON.parse(line);
      if (command.operation === 'status') {
        await connect();
        await client.checkPolicy();
        respond(client.allowed ? 'READY' : 'BLOCKED', client.allowed
          ? String(connectionGeneration) : 'app_disabled_or_model_paused');
      } else if (command.operation === 'suggest') {
        if (!client.allowed) { respond('BLOCKED', 'app_disabled_or_model_paused'); continue; }
        if (typeof command.before !== 'string' || !/^[A-Za-z0-9+/]*={0,2}$/.test(command.before)) throw new Error();
        const source = Buffer.from(command.before, 'base64').toString('utf8');
        if (Buffer.from(source).toString('base64') !== command.before) throw new Error();
        recordActivity('terminal', 'context', 'explicit_buffer');
        const before = [...source].slice(-512).join('');
        const language = writingLanguage(before, process.env.BADI_LANGUAGE ?? Intl.DateTimeFormat().resolvedOptions().locale);
        if (!language) throw new Error('Writing language unavailable');
        const text = await client.suggest(before, { language });
        respond(text ? 'SUGGEST' : 'EMPTY', text ? Buffer.from(text).toString('base64') + (
          client.replacementBefore ? ` ${Buffer.from(client.replacementBefore).toString('base64')}` : '') : '');
      } else if (command.operation === 'accept') {
        grant = await client.authorize();
        respond('INSERT', Buffer.from(grant.text).toString('base64') + (
          grant.replaceBefore ? ` ${Buffer.from(grant.replaceBefore).toString('base64')}` : ''));
      } else if (command.operation === 'result' && grant) {
        client.report(grant, command.applied === true ? 'applied' : 'failed');
        grant = null;
        respond('DONE');
      } else if (command.operation === 'cancel') {
        grant = null;
        client.cancel();
        respond('DONE');
      } else {
        respond('ERROR', 'invalid_operation');
      }
    } catch {
      grant = null;
      client.cancel();
      respond('ERROR', !client.connected ? 'model_offline_run_badi_doctor'
        : client.allowed ? 'expired_or_unavailable' : 'app_disabled_or_model_paused');
    }
  }
} catch {
  respond('ERROR', 'model_offline_run_badi_doctor');
} finally {
  client.close();
  lines.close();
}
