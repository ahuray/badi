import type {
  CommitAuthorization,
  CommitAuthorizationRequest,
  CommitResultNotice,
  SuggestionAddress,
  SuggestionClearEvent,
  SuggestionRequest,
  SuggestionResponse,
} from "../shared/model";
import {
  NATIVE_HOST_NAME,
  acceptanceControlEnvelope,
  cancelEnvelope,
  commitResultEnvelope,
  dismissEnvelope,
  globalControlEnvelope,
  helloEnvelope,
  isCommitRejection,
  isCommitRevocation,
  isGlobalControlError,
  isGlobalControlRejection,
  isHelloAck,
  messageId,
  parseCommitAuthorization,
  parseGlobalControlResult,
  parseSuggestionClearEvent,
  parseSuggestionReply,
  replyMatchesRequest,
  sessionOpenEnvelope,
  suggestionErrorMatchesRequest,
  suggestionRequestEnvelopes,
  type WireEnvelope,
} from "./protocol-mapper";

export interface NativeEvent<T> {
  addListener(listener: (value: T) => void): void;
  removeListener(listener: (value: T) => void): void;
}

export interface NativePortLike {
  readonly onMessage: NativeEvent<unknown>;
  readonly onDisconnect: NativeEvent<NativePortLike>;
  postMessage(message: unknown): void;
  disconnect(): void;
}

export interface NativePortFactory {
  connectNative(hostName: string): NativePortLike;
}

interface PendingReply {
  readonly request: SuggestionRequest;
  readonly resolve: (response: SuggestionResponse) => void;
  readonly reject: (error: Error) => void;
}

interface PendingCommit {
  readonly request: CommitAuthorizationRequest;
  readonly resolve: (authorization: CommitAuthorization) => void;
  readonly reject: (error: Error) => void;
  authorization: CommitAuthorization | null;
  grantTimer: ReturnType<typeof setTimeout> | null;
}

interface PendingGlobalControl {
  readonly action: "pause" | "resume" | "pause_toggle";
  readonly resolve: (paused: boolean) => void;
  readonly reject: (error: Error) => void;
}

export class NativeBrokerClient {
  readonly #factory: NativePortFactory;
  readonly #now: () => number;
  readonly #handshakeTimeoutMs: number;
  readonly #pending = new Map<string, PendingReply>();
  readonly #pendingCommits = new Map<string, PendingCommit>();
  readonly #pendingGlobalControls = new Map<string, PendingGlobalControl>();
  readonly #authorizedCommits = new Map<string, CommitAuthorizationRequest>();
  readonly #openedSessions = new Set<string>();

  #port: NativePortLike | null = null;
  #ready: Promise<void> | null = null;
  #resolveReady: (() => void) | null = null;
  #rejectReady: ((error: Error) => void) | null = null;
  #controlSequence = 0;
  #commitRevocationHandler: ((request: CommitAuthorizationRequest) => void) | null = null;
  #suggestionClearHandler: ((event: SuggestionClearEvent) => void) | null = null;
  #disconnectHandler: (() => void) | null = null;

  constructor(
    factory: NativePortFactory,
    options: { readonly now?: () => number; readonly handshakeTimeoutMs?: number } = {},
  ) {
    this.#factory = factory;
    this.#now = options.now ?? (() => performance.now());
    this.#handshakeTimeoutMs = options.handshakeTimeoutMs ?? 3_000;
  }

  async requestSuggestion(request: SuggestionRequest): Promise<SuggestionResponse> {
    await this.#ensureReady();
    if (!this.#openedSessions.has(request.sessionId)) {
      this.#post(sessionOpenEnvelope(request));
      this.#openedSessions.add(request.sessionId);
    }
    const promise = new Promise<SuggestionResponse>((resolve, reject) => {
      this.#pending.set(request.requestId, { request, resolve, reject });
    });
    for (const envelope of suggestionRequestEnvelopes(request)) {
      this.#post(envelope);
    }
    return promise;
  }

  async cancelSuggestion(request: SuggestionRequest): Promise<void> {
    const pending = this.#pending.get(request.requestId);
    if (pending !== undefined) {
      this.#pending.delete(request.requestId);
      pending.reject(new Error("Suggestion request superseded"));
    }
    await this.#ensureReady();
    this.#post(cancelEnvelope(request));
  }

  async dismissSuggestion(address: SuggestionAddress): Promise<void> {
    await this.#ensureReady();
    this.#post(dismissEnvelope(address));
  }

  async authorizeCommit(
    request: CommitAuthorizationRequest,
  ): Promise<CommitAuthorization> {
    await this.#ensureReady();
    const envelope = acceptanceControlEnvelope(request);
    const id = envelope.id;
    if (id === undefined) {
      throw new Error("Commit control is missing a correlation id");
    }
    const authorization = new Promise<CommitAuthorization>((resolve, reject) => {
      this.#pendingCommits.set(id, {
        request,
        resolve,
        reject,
        authorization: null,
        grantTimer: null,
      });
    });
    this.#post(envelope);
    return authorization;
  }

  async reportCommit(notice: CommitResultNotice): Promise<void> {
    await this.#ensureReady();
    for (const [id, request] of this.#authorizedCommits) {
      if (
        request.sessionId === notice.sessionId &&
        request.focusEpoch === notice.focusEpoch &&
        request.revision === notice.revision &&
        request.fingerprint === notice.fingerprint &&
        request.suggestionId === notice.suggestionId
      ) {
        this.#authorizedCommits.delete(id);
      }
    }
    this.#post(commitResultEnvelope(notice));
  }

  setCommitRevocationHandler(
    handler: ((request: CommitAuthorizationRequest) => void) | null,
  ): void {
    this.#commitRevocationHandler = handler;
  }

  setSuggestionClearHandler(
    handler: ((event: SuggestionClearEvent) => void) | null,
  ): void {
    this.#suggestionClearHandler = handler;
  }

  setDisconnectHandler(handler: (() => void) | null): void {
    this.#disconnectHandler = handler;
  }

  async globalControl(action: "pause" | "resume" | "pause_toggle"): Promise<boolean> {
    await this.#ensureReady();
    const monotonicMs = Math.max(0, Math.floor(this.#now()));
    const correlationId = `chromium.${action}.${monotonicMs}.${++this.#controlSequence}`;
    const result = new Promise<boolean>((resolve, reject) => {
      this.#pendingGlobalControls.set(correlationId, { action, resolve, reject });
    });
    this.#post(globalControlEnvelope(action, monotonicMs, correlationId));
    return result;
  }

  dispose(): void {
    this.#port?.disconnect();
    this.#reset(new Error("Native broker client disposed"));
  }

  async #ensureReady(): Promise<void> {
    if (this.#ready !== null) {
      return this.#ready;
    }
    const port = this.#factory.connectNative(NATIVE_HOST_NAME);
    this.#port = port;
    port.onMessage.addListener(this.#onMessage);
    port.onDisconnect.addListener(this.#onDisconnect);
    this.#ready = new Promise<void>((resolve, reject) => {
      this.#resolveReady = resolve;
      this.#rejectReady = reject;
    });
    const ready = this.#ready;
    const timeout = setTimeout(() => {
      this.#rejectReady?.(new Error("Native broker handshake timed out"));
      this.#reset(new Error("Native broker handshake timed out"));
    }, this.#handshakeTimeoutMs);
    void ready.finally(() => clearTimeout(timeout)).catch(() => undefined);
    this.#post(helloEnvelope(Math.max(0, Math.floor(this.#now()))));
    return ready;
  }

  #post(envelope: WireEnvelope): void {
    if (this.#port === null) {
      throw new Error("Native broker port is unavailable");
    }
    this.#port.postMessage(envelope);
  }

  readonly #onMessage = (message: unknown): void => {
    if (isHelloAck(message)) {
      const resolve = this.#resolveReady;
      this.#resolveReady = null;
      this.#rejectReady = null;
      resolve?.();
      return;
    }
    const clearEvent = parseSuggestionClearEvent(message);
    if (clearEvent !== null) {
      this.#suggestionClearHandler?.(clearEvent);
    }
    const id = messageId(message);
    const pendingGlobal = id === null ? undefined : this.#pendingGlobalControls.get(id);
    if (id !== null && pendingGlobal !== undefined) {
      if (
        isGlobalControlError(message, id) ||
        isGlobalControlRejection(message, id, pendingGlobal.action)
      ) {
        this.#pendingGlobalControls.delete(id);
        pendingGlobal.reject(new Error("Broker rejected global control"));
        return;
      }
      const paused = parseGlobalControlResult(message, id, pendingGlobal.action);
      if (paused !== null) {
        this.#pendingGlobalControls.delete(id);
        pendingGlobal.resolve(paused);
        return;
      }
    }
    const pendingCommit = id === null ? undefined : this.#pendingCommits.get(id);
    if (id !== null && pendingCommit !== undefined) {
      if (isCommitRevocation(message, pendingCommit.request)) {
        if (pendingCommit.grantTimer !== null) clearTimeout(pendingCommit.grantTimer);
        this.#pendingCommits.delete(id);
        pendingCommit.reject(new Error("Broker revoked commit authorization"));
        return;
      }
      if (isCommitRejection(message, pendingCommit.request)) {
        if (pendingCommit.grantTimer !== null) clearTimeout(pendingCommit.grantTimer);
        this.#pendingCommits.delete(id);
        pendingCommit.reject(new Error("Broker denied commit authorization"));
        return;
      }
      const authorization = parseCommitAuthorization(message, pendingCommit.request);
      if (authorization !== null) {
        if (pendingCommit.authorization !== null) {
          return;
        }
        pendingCommit.authorization = authorization;
        pendingCommit.grantTimer = setTimeout(() => {
          if (this.#pendingCommits.get(id) !== pendingCommit) {
            return;
          }
          this.#pendingCommits.delete(id);
          this.#authorizedCommits.set(id, pendingCommit.request);
          pendingCommit.resolve(authorization);
        }, 0);
        return;
      }
      if (
        typeof message === "object" &&
        message !== null &&
        "type" in message &&
        message.type === "commit.prepare"
      ) {
        if (pendingCommit.grantTimer !== null) clearTimeout(pendingCommit.grantTimer);
        this.#pendingCommits.delete(id);
        pendingCommit.reject(new Error("Broker returned mismatched commit authorization"));
        return;
      }
    }
    if (id !== null) {
      const authorized = this.#authorizedCommits.get(id);
      if (authorized !== undefined && isCommitRevocation(message, authorized)) {
        this.#authorizedCommits.delete(id);
        this.#commitRevocationHandler?.(authorized);
        return;
      }
    }
    for (const [id, pending] of this.#pending) {
      if (suggestionErrorMatchesRequest(message, pending.request)) {
        this.#pending.delete(id);
        pending.reject(new Error("Broker rejected suggestion request"));
        return;
      }
      if (!replyMatchesRequest(message, pending.request)) {
        continue;
      }
      const response = parseSuggestionReply(message, pending.request);
      if (response === null) {
        continue;
      }
      this.#pending.delete(id);
      pending.resolve(response);
      return;
    }
  };

  readonly #onDisconnect = (): void => {
    const runtimeError =
      typeof chrome === "undefined" ? undefined : chrome.runtime.lastError?.message;
    this.#reset(new Error(runtimeError ?? "Native broker disconnected"));
    this.#disconnectHandler?.();
  };

  #reset(error: Error): void {
    const port = this.#port;
    if (port !== null) {
      port.onMessage.removeListener(this.#onMessage);
      port.onDisconnect.removeListener(this.#onDisconnect);
    }
    this.#port = null;
    this.#rejectReady?.(error);
    this.#ready = null;
    this.#resolveReady = null;
    this.#rejectReady = null;
    this.#openedSessions.clear();
    for (const pending of this.#pending.values()) {
      pending.reject(error);
    }
    this.#pending.clear();
    for (const pending of this.#pendingCommits.values()) {
      if (pending.grantTimer !== null) clearTimeout(pending.grantTimer);
      pending.reject(error);
    }
    this.#pendingCommits.clear();
    for (const pending of this.#pendingGlobalControls.values()) {
      pending.reject(error);
    }
    this.#pendingGlobalControls.clear();
    this.#authorizedCommits.clear();
  }
}
