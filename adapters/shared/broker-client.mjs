import { EventEmitter } from 'node:events';
import { createConnection } from 'node:net';
import { lstatSync } from 'node:fs';
import { dirname, isAbsolute, join } from 'node:path';
import { randomUUID, createHash } from 'node:crypto';
import { recordActivity } from './activity.mjs';
import { hasUnsafeText, validCorrection } from './text-safety.mjs';

const PROFILES = Object.freeze({
  obsidian: { kind: 'obsidian', app_id: 'obsidian' },
  terminal: { kind: 'terminal', app_id: 'bash' },
});
const CAPABILITIES = ['context', 'suggestion', 'commit.applied', 'control', 'policy'];
const FRAME_LIMIT = 65536;
const counter = value => Number.isSafeInteger(value) && value >= 0;
const words = new Intl.Segmenter('en', { granularity: 'word' });
export const validSuggestion = value => typeof value === 'string' && [...value].length <= 64 &&
  value.trim().length > 0 && !hasUnsafeText(value) &&
  [...words.segment(value)].filter(part => part.isWordLike).length <= 4;

export class BrokerClient extends EventEmitter {
  #socket;
  #buffer = Buffer.alloc(0);
  #pending = new Map();
  #session = null;
  #candidate = null;
  #expiry;
  #epoch = null;
  #closed = false;
  #hello = false;
  #generation = 0;
  #profile;
  #policy = null;
  #textReplacement;

  constructor(profile, { textReplacement = false } = {}) {
    super();
    if (!Object.hasOwn(PROFILES, profile)) throw new Error('Unknown editor adapter');
    this.#profile = PROFILES[profile];
    this.#textReplacement = textReplacement === true;
    this.on('activity', event => recordActivity(profile, 'transport', event));
    this.on('state', state => recordActivity(profile, 'state', state));
  }

  get allowed() {
    return !this.#closed && this.#policy?.authority_epoch === this.#epoch &&
      this.#policy.paused === false && this.#policy.context_allowed === true &&
      this.#policy.display_allowed === true && this.#policy.suggestions_allowed === true &&
      this.#policy.activation !== 'never';
  }

  get connected() { return this.#hello && this.#epoch !== null && !this.#closed; }
  get automaticAllowed() { return this.allowed && this.#policy.activation === 'always'; }
  get replacementBefore() { return this.#candidate?.replaceBefore; }

  async checkPolicy() {
    if (!this.connected) return false;
    await this.#refreshPolicy();
    return this.allowed;
  }

  async connect(socketPath = join(process.env.XDG_RUNTIME_DIR ?? '', 'badi/broker.sock')) {
    if (this.#socket || !isAbsolute(socketPath)) throw new Error('Invalid broker connection');
    const directory = lstatSync(dirname(socketPath));
    const endpoint = lstatSync(socketPath);
    if (!directory.isDirectory() || directory.uid !== process.getuid() || (directory.mode & 0o077) ||
        !endpoint.isSocket() || endpoint.uid !== process.getuid() || (endpoint.mode & 0o777) !== 0o600) {
      throw new Error('Broker socket must be private and owned by this user');
    }
    this.#socket = createConnection(socketPath);
    this.#socket.on('data', chunk => {
      try { this.#receive(chunk); } catch { this.#fail('Invalid broker response'); }
    });
    this.#socket.on('error', error => this.#fail(`Broker unavailable: ${error.code ?? 'socket_error'}`));
    this.#socket.on('close', () => this.#fail('Broker disconnected'));
    try {
      const capabilities = [...CAPABILITIES, ...(this.#textReplacement ? ['text_replacement'] : [])];
      const hello = await this.#rpc('hello', { min_v: 2, max_v: 2,
        adapter: { kind: this.#profile.kind, name: `badi-${this.#profile.kind}`, version: '0.1.0' },
        capabilities }, 'hello.ack');
      const value = hello.payload;
      if (value.selected_v !== 2 || value.max_frame_bytes !== FRAME_LIMIT ||
          value.max_before_chars !== 512 || value.max_after_chars !== 128 ||
          !Array.isArray(value.enabled_capabilities) ||
          value.enabled_capabilities.length !== capabilities.length ||
          !capabilities.every(capability => value.enabled_capabilities.includes(capability))) {
        throw new Error('Incompatible broker capabilities');
      }
      this.#hello = true;
      if (this.#epoch === null) await new Promise((resolve, reject) => {
        const timeout = setTimeout(() => { this.off('authority', ready); reject(new Error('Broker authority timed out')); }, 2000);
        const ready = () => { clearTimeout(timeout); resolve(); };
        this.once('authority', ready);
      });
      await this.#refreshPolicy();
      return this.allowed;
    } catch (error) { this.close(); throw error; }
  }

  #target(id) { return { ...this.#profile, target_id: id }; }

  async #refreshPolicy() {
    this.#policy = null;
    const epoch = this.#epoch;
    const response = await this.#rpc('policy.query', { target: this.#target(randomUUID()) }, 'policy.status');
    const policy = response.payload;
    if (!counter(policy.authority_epoch) || !counter(policy.settings_revision) ||
        !['paused', 'context_allowed', 'display_allowed', 'suggestions_allowed', 'learning_allowed']
          .every(key => typeof policy[key] === 'boolean') ||
        !['always', 'manual', 'never'].includes(policy.activation) ||
        policy.learning_allowed || (policy.suggestions_allowed && (!policy.context_allowed || !policy.display_allowed)) ||
        (policy.paused && (policy.context_allowed || policy.display_allowed || policy.suggestions_allowed))) {
      throw new Error('Invalid editor policy');
    }
    if (epoch !== this.#epoch || policy.authority_epoch !== epoch) return;
    this.#policy = policy;
    this.emit('state', this.allowed ? 'ready' : policy.paused ? 'paused' : 'app_disabled');
  }

  async suggest(before, { language = 'en', explicit = true } = {}) {
    if (!this.allowed) throw new Error('App disabled or model paused');
    if (typeof explicit !== 'boolean' || (!explicit && !this.automaticAllowed)) throw new Error('Manual invocation required');
    if (typeof language !== 'string' || language.length < 2 || language.length > 35 ||
        !/^[A-Za-z0-9]+(?:-[A-Za-z0-9]+)*$/u.test(language)) {
      throw new Error('Invalid writing language');
    }
    if (typeof before !== 'string' || !before.trim() || [...before].length > 512 ||
        hasUnsafeText(before, true)) {
      throw new Error('Invalid writing context');
    }
    this.cancel();
    const generation = this.#generation;
    const id = randomUUID();
    const session = { session_id: id, focus_epoch: 1, revision: 1,
      fingerprint: createHash('sha256').update(randomUUID()).update(before).digest('hex') };
    this.#session = session;
    this.#send('session.open', { target: this.#target(id), activation: 'always' }, randomUUID(), { ...session, revision: 0 });
    this.#send('context.changed', { fingerprint: session.fingerprint, before, after: '', language,
      selection: { anchor: [...before].length, head: [...before].length, unit: 'unicode_scalar_values' },
      field: { purpose: this.#profile.kind === 'terminal' ? 'terminal' : 'normal', editable: true,
        multiline: this.#profile.kind !== 'terminal', composing: false, sensitive: false,
        identity_known: true, focused: true, lock_screen: false }, activation: explicit ? 'manual' : 'always', explicit,
    }, randomUUID(), session);
    this.emit('state', 'thinking');
    const response = await this.#rpc('suggest.request', { fingerprint: session.fingerprint, explicit },
      'suggestion.show', session);
    if (generation !== this.#generation || this.#session !== session || !this.allowed) throw new Error('Context changed');
    if (response.type === 'suggestion.clear') { this.emit('state', response.payload.reason); return null; }
    const value = response.payload;
    const replacement = value.replace_before !== undefined;
    const validText = replacement
      ? this.#textReplacement && validCorrection(before, value.replace_before, value.text) && value.accept_word === value.text
      : validSuggestion(value.text) && validSuggestion(value.accept_word) && value.text.startsWith(value.accept_word);
    if (!validText ||
        !counter(value.ttl_ms) || value.ttl_ms < 1 || value.ttl_ms > 5000 ||
        typeof value.suggestion_id !== 'string' || value.fingerprint !== session.fingerprint) {
      this.cancel(); throw new Error(`Invalid model suggestion (text=${validSuggestion(value.text)}, words=${typeof value.text === 'string' ? value.text.trim().split(/\s+/u).length : 0}, ttl=${value.ttl_ms}, binding=${value.fingerprint === session.fingerprint})`);
    }
    this.#candidate = { ...session, suggestion_id: value.suggestion_id, text: value.text, acceptWord: value.accept_word,
      ...(replacement ? { replaceBefore: value.replace_before } : {}),
      expires: performance.now() + value.ttl_ms };
    this.#expiry = setTimeout(() => this.cancel(), value.ttl_ms);
    this.emit('state', 'suggestion');
    return value.text;
  }

  async authorize(acceptance = 'all') {
    if (!['all', 'word'].includes(acceptance)) throw new Error('Invalid acceptance mode');
    const candidate = this.#candidate;
    const generation = this.#generation;
    if (!candidate || !this.allowed || performance.now() >= candidate.expires) throw new Error('Suggestion expired');
    this.#candidate = null;
    const text = acceptance === 'word' ? candidate.acceptWord : candidate.text;
    const response = await this.#rpc('control.request', { action: acceptance === 'word' ? 'accept_word' : 'accept_all',
      fingerprint: candidate.fingerprint, suggestion_id: candidate.suggestion_id }, 'commit.prepare', candidate);
    if (generation !== this.#generation || !this.allowed || performance.now() >= candidate.expires ||
        response.payload.acceptance !== acceptance || response.payload.text !== text || response.payload.fingerprint !== candidate.fingerprint ||
        response.payload.suggestion_id !== candidate.suggestion_id || response.payload.replace_before !== candidate.replaceBefore) {
      throw new Error('Acceptance was revoked');
    }
    return Object.freeze({ ...candidate, text, control_id: response.id });
  }

  report(grant, status) {
    if (!['applied', 'stale', 'blocked', 'failed'].includes(status)) throw new Error('Invalid commit result');
    if (this.#session?.session_id !== grant.session_id || !this.allowed) return;
    this.#send('commit.result', { fingerprint: grant.fingerprint, suggestion_id: grant.suggestion_id, status },
      grant.control_id, grant);
    this.cancel();
    this.emit('state', status);
  }

  cancel(closeSession = true) {
    this.#generation++;
    clearTimeout(this.#expiry);
    this.#candidate = null;
    const session = this.#session;
    if (closeSession) this.#session = null;
    for (const [id, pending] of this.#pending) {
      if (pending.session) { clearTimeout(pending.timer); this.#pending.delete(id); pending.reject(new Error('Context changed')); }
    }
    if (closeSession && session && !this.#closed) this.#send('session.close', { reason: 'session_closed' }, randomUUID(), session);
    this.emit('clear');
  }

  close() { this.#fail('Adapter closed'); }

  #fail(reason) {
    if (this.#closed) return;
    this.#closed = true;
    this.cancel();
    for (const pending of this.#pending.values()) { clearTimeout(pending.timer); pending.reject(new Error(reason)); }
    this.#pending.clear();
    this.#socket?.destroy();
    this.emit('diagnostic', reason);
    this.emit('state', 'offline');
  }

  #rpc(type, payload, expected, session) {
    if (this.#closed || this.#pending.size >= 16) return Promise.reject(new Error('Broker unavailable or busy'));
    const id = randomUUID();
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.#pending.delete(id); reject(new Error('Broker operation timed out')); }, 2000);
      this.#pending.set(id, { resolve, reject, timer, expected, session });
      try { this.#send(type, payload, id, session); }
      catch (error) { clearTimeout(timer); this.#pending.delete(id); reject(error); }
    });
  }

  #send(type, payload, id, session) {
    if (this.#closed || !this.#socket || this.#socket.writableLength > FRAME_LIMIT * 2) throw new Error('Broker unavailable');
    const body = Buffer.from(JSON.stringify({ v: 2, id, type, mono_ms: Math.floor(performance.now()), payload,
      ...(session ? { session_id: session.session_id, focus_epoch: session.focus_epoch, revision: session.revision } : {}) }));
    if (body.length > FRAME_LIMIT) throw new Error('Broker frame too large');
    const frame = Buffer.allocUnsafe(body.length + 4);
    frame.writeUInt32LE(body.length); body.copy(frame, 4); this.#socket.write(frame);
    this.emit('activity', `sent ${type}`);
  }

  #receive(chunk) {
    this.#buffer = Buffer.concat([this.#buffer, chunk]);
    while (this.#buffer.length >= 4) {
      const length = this.#buffer.readUInt32LE();
      if (length < 1 || length > FRAME_LIMIT) throw new Error('Frame limit');
      if (this.#buffer.length < length + 4) return;
      const message = JSON.parse(this.#buffer.subarray(4, length + 4).toString('utf8'));
      this.#buffer = this.#buffer.subarray(length + 4);
      if (message?.v !== 2 || typeof message.type !== 'string' || !counter(message.mono_ms) ||
          !message.payload || typeof message.payload !== 'object') throw new Error('Invalid envelope');
      this.emit('activity', `received ${message.type}`);
      if (message.type === 'authority.changed') {
        const epoch = message.payload.authority_epoch;
        const initial = this.#epoch === null;
        if (!counter(epoch) || (this.#epoch !== null && epoch <= this.#epoch)) throw new Error('Invalid authority');
        this.#epoch = epoch; this.#policy = null; this.#session = null; this.cancel();
        this.#send('authority.ack', { authority_epoch: epoch });
        this.emit('authority');
        if (this.#hello && !initial) void this.#refreshPolicy().catch(() => this.#fail('Policy unavailable'));
        continue;
      }
      const pending = this.#pending.get(message.id);
      if (pending && pending.session && ['session_id', 'focus_epoch', 'revision'].some(key => message[key] !== pending.session[key]) &&
          message.type !== 'control.result' && message.type !== 'error') throw new Error('Reply context mismatch');
      if (pending && (message.type === pending.expected || message.type === 'error' ||
          (pending.expected === 'suggestion.show' && message.type === 'suggestion.clear') ||
          (message.type === 'control.result' && message.payload.accepted === false))) {
        clearTimeout(pending.timer); this.#pending.delete(message.id);
        if (message.type === 'error' || message.payload.accepted === false) pending.reject(new Error('Broker rejected the request'));
        else pending.resolve(message);
      } else if (message.type === 'suggestion.clear' || message.type === 'commit.revoke') {
        // A clear can precede the authority notice that retires this session.
        // Do not echo a close for coordinates the broker may already have revoked.
        if (this.#session?.session_id === message.session_id) this.cancel(false);
      }
    }
  }
}
