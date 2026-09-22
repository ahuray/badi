import { EventEmitter } from 'node:events';
import { BrokerClient } from './broker-client.mjs';

// Recovery only establishes transport and policy. It never replays prose,
// suggestions or grants from the connection that failed.
export class RecoveringBrokerClient extends EventEmitter {
  #profile;
  #options;
  #client = null;
  #connection = null;
  #socketPath;
  #timer = null;
  #attempts = 0;
  #closed = false;

  constructor(profile, options = {}) { super(); this.#profile = profile; this.#options = { ...options }; }
  get connected() { return !this.#closed && this.#client?.connected === true; }
  get allowed() { return !this.#closed && this.#client?.allowed === true; }
  get automaticAllowed() { return !this.#closed && this.#client?.automaticAllowed === true; }
  get replacementBefore() { return this.#client?.replacementBefore; }

  connect(socketPath = this.#socketPath) {
    if (this.#closed) return Promise.reject(new Error('Adapter closed'));
    if (this.#connection) return this.#connection;
    if (this.connected) return Promise.resolve(this.allowed);
    if (this.#timer !== null) return Promise.resolve(false);
    this.#socketPath = socketPath;
    this.#attempts = 0;
    return this.#attempt();
  }

  #attempt() {
    this.#attempts++;
    const previous = this.#client;
    const client = new BrokerClient(this.#profile, this.#options);
    this.#client = client;
    previous?.close();
    for (const event of ['clear', 'diagnostic', 'activity', 'state']) {
      client.on(event, value => {
        if (this.#closed || this.#client !== client) return;
        this.emit(event, value);
        if (event === 'state' && value === 'offline') this.#schedule();
      });
    }
    const connection = client.connect(this.#socketPath).then(allowed => {
      if (this.#client !== client || this.#closed) return false;
      this.#attempts = 0;
      return allowed;
    }).catch(error => {
      client.close();
      this.#schedule();
      throw error;
    }).finally(() => {
      if (this.#connection === connection) this.#connection = null;
    });
    this.#connection = connection;
    return connection;
  }

  #schedule() {
    if (this.#closed || this.#timer !== null) return;
    if (this.#attempts >= 10) { this.emit('state', 'offline_retry'); return; }
    const delay = Math.min(5000, 250 * 2 ** Math.max(0, this.#attempts - 1));
    this.#timer = setTimeout(() => {
      this.#timer = null;
      if (!this.#closed) void this.#attempt().catch(() => undefined);
    }, delay);
    this.#timer.unref?.();
  }

  suggest(...args) {
    if (!this.allowed) return Promise.reject(new Error('App disabled or model paused'));
    return this.#client.suggest(...args);
  }
  authorize(...args) {
    if (!this.allowed) return Promise.reject(new Error('Suggestion expired'));
    return this.#client.authorize(...args);
  }
  report(...args) { if (this.allowed) this.#client.report(...args); }
  cancel() { this.#client?.cancel(); }
  close() {
    this.#closed = true;
    clearTimeout(this.#timer);
    this.#timer = null;
    this.#client?.close();
  }
}
