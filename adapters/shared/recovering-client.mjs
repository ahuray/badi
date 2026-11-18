import { EventEmitter } from 'node:events';
import { isAbsolute } from 'node:path';
import { BrokerClient } from './broker-client.mjs';
import { brokerSocketReady } from './request-broker.mjs';

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
  #started = false;
  #closed = false;
  #retriesPaused = false;
  // The last focus this editor reported; a new connection repeats a held
  // focus, because the broker forgets it with the old connection.
  #focused = false;

  constructor(profile, options = {}) { super(); this.#profile = profile; this.#options = { ...options }; }
  get connected() { return !this.#closed && this.#client?.connected === true; }
  get allowed() { return !this.#closed && this.#client?.allowed === true; }
  get automaticAllowed() { return !this.#closed && this.#client?.automaticAllowed === true; }
  get replacementBefore() { return this.#client?.replacementBefore; }
  get acceptWord() { return this.#client?.acceptWord; }
  get alternativeWords() { return this.#client?.alternativeWords ?? null; }

  pauseRetries() {
    this.#retriesPaused = true;
    clearTimeout(this.#timer);
    this.#timer = null;
  }

  connect(socketPath = this.#socketPath, { immediate = false } = {}) {
    if (this.#closed) return Promise.reject(new Error('Adapter closed'));
    if (this.#connection && !immediate) return this.#connection;
    if (this.connected) return Promise.resolve(this.allowed);
    if (this.#timer !== null) {
      this.#attempts = 0;
      if (!immediate) return Promise.resolve(false);
      clearTimeout(this.#timer);
      this.#timer = null;
    }
    this.#retriesPaused = false;
    this.#socketPath = socketPath;
    this.#started = true;
    this.#attempts = 0;
    return this.#attempt();
  }

  #resolvedSocketPath() {
    if (this.#socketPath) return this.#socketPath;
    const runtime = process.env.XDG_RUNTIME_DIR ?? '';
    return runtime && isAbsolute(runtime) ? `${runtime}/badi/broker.sock` : null;
  }

  #attempt() {
    this.#attempts++;
    const quietFocusRetry = this.#attempts > 10;
    if (quietFocusRetry) {
      const target = this.#resolvedSocketPath();
      if (!target || !brokerSocketReady(target)) {
        this.#schedule();
        return Promise.resolve(false);
      }
    }
    const previous = this.#client;
    const client = new BrokerClient(this.#profile, this.#options);
    this.#client = client;
    previous?.close();
    for (const event of ['clear', 'update', 'activity', 'state']) {
      client.on(event, value => {
        if (this.#closed || this.#client !== client) return;
        if (event === 'state' && value === 'offline') {
          if (!quietFocusRetry) this.emit(event, value);
          this.#schedule();
          return;
        }
        this.emit(event, value);
      });
    }
    const connection = client.connect(this.#socketPath).then(allowed => {
      if (this.#client !== client || this.#closed) return false;
      this.#attempts = 0;
      if (this.#focused) void client.reportFocus(true).catch(() => undefined);
      return allowed;
    }).catch(error => {
      client.close();
      if (this.#client === client) this.#schedule();
      throw error;
    }).finally(() => {
      if (this.#connection === connection) this.#connection = null;
    });
    this.#connection = connection;
    return connection;
  }

  #schedule() {
    if (this.#closed || this.#retriesPaused || this.#timer !== null) return;
    if (this.#attempts >= 10) {
      if (this.#attempts === 10) this.emit('state', 'offline_retry');
      // While holding terminal focus (ADR 0009), keep watching quietly for an
      // on-demand broker start so the terminal's typed-line path yields once up.
      if (!this.#focused) return;
    }
    const delay = this.#attempts >= 10 ? 1000 : Math.min(5000, 250 * 2 ** Math.max(0, this.#attempts - 1));
    this.#timer = setTimeout(() => {
      this.#timer = null;
      if (!this.#closed && !this.#retriesPaused) void this.#attempt().catch(() => undefined);
    }, delay);
    this.#timer.unref?.();
  }

  suggest(...args) {
    if (!this.allowed) return Promise.reject(new Error('App disabled or model paused'));
    return this.#client.suggest(...args);
  }
  alternative() {
    if (!this.allowed) return Promise.reject(new Error('App disabled or model paused'));
    return this.#client.alternative();
  }
  pickAlternative(...args) {
    if (!this.allowed) return Promise.reject(new Error('App disabled or model paused'));
    return this.#client.pickAlternative(...args);
  }
  authorize(...args) {
    if (!this.allowed) return Promise.reject(new Error('Suggestion expired'));
    return this.#client.authorize(...args);
  }
  revise(...args) {
    if (!this.allowed) return Promise.reject(new Error('App disabled or model paused'));
    return this.#client.revise(...args);
  }
  authorizeRevision() {
    if (!this.allowed) return Promise.reject(new Error('Revision expired'));
    return this.#client.authorizeRevision();
  }
  report(...args) { if (this.allowed) this.#client.report(...args); }
  reportFocus(focused) {
    this.#focused = focused === true;
    if (!this.#focused && this.#attempts >= 10 && this.#timer !== null) {
      clearTimeout(this.#timer);
      this.#timer = null;
    }
    if (!this.connected) {
      if (this.#focused && !this.#closed && !this.#retriesPaused && this.#started && !this.#connection && this.#timer === null) {
        this.#schedule();
      }
      return Promise.resolve(false);
    }
    return this.#client.reportFocus(this.#focused);
  }
  dismiss() { this.#client?.dismiss(); }
  cancel() { this.#client?.cancel(); }
  close() {
    this.#closed = true;
    this.#retriesPaused = true;
    clearTimeout(this.#timer);
    this.#timer = null;
    this.#client?.close();
  }
}
