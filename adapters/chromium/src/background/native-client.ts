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
  messageId,
  parseCommitAuthorization,
  parseGlobalControlResult,
  parseHelloAckPaused,
  parseSuggestionClearEvent,
  parseSuggestionReply,
  replyMatchesRequest,
  sessionOpenEnvelope,
  sessionCloseEnvelope,
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

interface TimedPendingOperation {
  operationTimer: ReturnType<typeof setTimeout> | null;
}

interface PendingReply extends TimedPendingOperation {
  readonly request: SuggestionRequest;
  readonly resolve: (response: SuggestionResponse) => void;
  readonly reject: (error: Error) => void;
}

interface PendingCommit extends TimedPendingOperation {
  readonly request: CommitAuthorizationRequest;
  readonly resolve: (authorization: CommitAuthorization) => void;
  readonly reject: (error: Error) => void;
  authorization: CommitAuthorization | null;
  grantTimer: ReturnType<typeof setTimeout> | null;
}

interface PendingGlobalControl extends TimedPendingOperation {
  readonly action: "pause" | "resume" | "pause_toggle";
  readonly promise: Promise<boolean>;
  readonly resolve: (paused: boolean) => void;
  readonly reject: (error: Error) => void;
}

interface OpenSession {
  request: SuggestionRequest;
  opened: boolean;
  closed: boolean;
}

export class NativeBrokerClient {
  readonly #factory: NativePortFactory;
  readonly #now: () => number;
  readonly #handshakeTimeoutMs: number;
  readonly #operationTimeoutMs: number;
  readonly #pending = new Map<string, PendingReply>();
  readonly #pendingCommits = new Map<string, PendingCommit>();
  readonly #pendingGlobalControls = new Map<string, PendingGlobalControl>();
  readonly #authorizedCommits = new Map<string, CommitAuthorizationRequest>();
  readonly #sessions = new Map<string, OpenSession>();

  #port: NativePortLike | null = null;
  #ready: Promise<void> | null = null;
  #resolveReady: (() => void) | null = null;
  #rejectReady: ((error: Error) => void) | null = null;
  #controlSequence = 0;
  #paused: boolean | null = null;
  #commitRevocationHandler: ((request: CommitAuthorizationRequest) => void) | null = null;
  #suggestionClearHandler: ((event: SuggestionClearEvent) => void) | null = null;
  #disconnectHandler: (() => void) | null = null;

  constructor(
    factory: NativePortFactory,
    options: {
      readonly now?: () => number;
      readonly handshakeTimeoutMs?: number;
      readonly operationTimeoutMs?: number;
    } = {},
  ) {
    this.#factory = factory;
    this.#now = options.now ?? (() => performance.now());
    this.#handshakeTimeoutMs = options.handshakeTimeoutMs ?? 3_000;
    this.#operationTimeoutMs = Math.max(1, options.operationTimeoutMs ?? 3_000);
  }

  async requestSuggestion(request: SuggestionRequest): Promise<SuggestionResponse> {
    let session = this.#sessions.get(request.sessionId);
    if (session === undefined || session.closed) {
      session = { request, opened: false, closed: false };
      this.#sessions.set(request.sessionId, session);
    }
    await this.#ensureReady();
    if (session.closed || this.#sessions.get(request.sessionId) !== session) {
      throw new Error("Suggestion session closed before dispatch");
    }
    if (this.#paused === true) {
      if (!session.opened) this.#sessions.delete(request.sessionId);
      throw new Error("Broker is paused");
    }
    if (!session.opened) {
      this.#post(sessionOpenEnvelope(request));
      session.opened = true;
    }
    session.request = request;
    let pending!: PendingReply;
    const promise = new Promise<SuggestionResponse>((resolve, reject) => {
      const previous = this.#pending.get(request.requestId);
      if (previous !== undefined) {
        this.#pending.delete(request.requestId);
        this.#clearOperationTimer(previous);
        previous.reject(new Error("Suggestion request superseded"));
      }
      pending = {
        request,
        resolve,
        reject,
        operationTimer: null,
      };
      this.#pending.set(request.requestId, pending);
      pending.operationTimer = this.#startOperationTimer(() => {
        if (this.#pending.get(request.requestId) !== pending) return;
        this.#pending.delete(request.requestId);
        pending.operationTimer = null;
        pending.reject(new Error("Native broker suggestion operation timed out"));
      });
    });
    try {
      for (const envelope of suggestionRequestEnvelopes(request)) {
        this.#post(envelope);
      }
    } catch (error) {
      if (this.#pending.get(request.requestId) === pending) {
        this.#pending.delete(request.requestId);
        this.#clearOperationTimer(pending);
        pending.reject(
          error instanceof Error ? error : new Error("Native broker suggestion dispatch failed"),
        );
      }
    }
    return promise;
  }

  async cancelSuggestion(request: SuggestionRequest): Promise<void> {
    const pending = this.#pending.get(request.requestId);
    if (pending !== undefined) {
      this.#pending.delete(request.requestId);
      this.#clearOperationTimer(pending);
      pending.reject(new Error("Suggestion request superseded"));
    }
    await this.#ensureReady();
    this.#post(cancelEnvelope(request));
  }

  async closeSession(sessionId: string): Promise<void> {
    const session = this.#sessions.get(sessionId);
    if (session === undefined) return;
    session.closed = true;
    this.#sessions.delete(sessionId);
    this.#rejectSessionWork(sessionId, new Error("Suggestion session closed"));
    if (!session.opened) return;
    await this.#ensureReady();
    this.#post(sessionCloseEnvelope(session.request));
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
    if (this.#pendingCommits.has(id)) {
      throw new Error("Commit authorization is already pending");
    }
    let pending!: PendingCommit;
    const authorization = new Promise<CommitAuthorization>((resolve, reject) => {
      pending = {
        request,
        resolve,
        reject,
        authorization: null,
        grantTimer: null,
        operationTimer: null,
      };
      this.#pendingCommits.set(id, pending);
      pending.operationTimer = this.#startOperationTimer(() => {
        if (this.#pendingCommits.get(id) !== pending) return;
        this.#pendingCommits.delete(id);
        if (pending.grantTimer !== null) clearTimeout(pending.grantTimer);
        pending.operationTimer = null;
        pending.reject(new Error("Native broker commit operation timed out"));
      });
    });
    try {
      this.#post(envelope);
    } catch (error) {
      if (this.#pendingCommits.get(id) === pending) {
        this.#pendingCommits.delete(id);
        this.#clearOperationTimer(pending);
        pending.reject(
          error instanceof Error ? error : new Error("Native broker commit dispatch failed"),
        );
      }
    }
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
    const inFlight = this.#pendingGlobalControls.values().next().value;
    if (inFlight !== undefined) {
      if (inFlight.action === action) return inFlight.promise;
      throw new Error(`Global control ${inFlight.action} is already in progress`);
    }
    const monotonicMs = Math.max(0, Math.floor(this.#now()));
    const correlationId = `chromium.${action}.${monotonicMs}.${++this.#controlSequence}`;
    let resolveResult!: (paused: boolean) => void;
    let rejectResult!: (error: Error) => void;
    const result = new Promise<boolean>((resolve, reject) => {
      resolveResult = resolve;
      rejectResult = reject;
    });
    const pending: PendingGlobalControl = {
      action,
      promise: result,
      resolve: resolveResult,
      reject: rejectResult,
      operationTimer: null,
    };
    this.#pendingGlobalControls.set(correlationId, pending);
    pending.operationTimer = this.#startOperationTimer(() => {
      if (this.#pendingGlobalControls.get(correlationId) !== pending) return;
      this.#pendingGlobalControls.delete(correlationId);
      pending.operationTimer = null;
      pending.reject(new Error("Native broker global control operation timed out"));
    });
    try {
      this.#post(globalControlEnvelope(action, monotonicMs, correlationId));
    } catch (error) {
      if (this.#pendingGlobalControls.get(correlationId) === pending) {
        this.#pendingGlobalControls.delete(correlationId);
        this.#clearOperationTimer(pending);
        pending.reject(
          error instanceof Error
            ? error
            : new Error("Native broker global control dispatch failed"),
        );
      }
    }
    return result;
  }

  async bootstrap(): Promise<boolean> {
    await this.#ensureReady();
    if (this.#paused === null) {
      throw new Error("Native broker did not provide pause state");
    }
    return this.#paused;
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
      const error = new Error("Native broker handshake timed out");
      port.disconnect();
      this.#reset(error);
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
    const helloPaused = parseHelloAckPaused(message);
    if (helloPaused !== null) {
      const resolve = this.#resolveReady;
      this.#paused = helloPaused;
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
        this.#clearOperationTimer(pendingGlobal);
        pendingGlobal.reject(new Error("Broker rejected global control"));
        return;
      }
      const paused = parseGlobalControlResult(message, id, pendingGlobal.action);
      if (paused !== null) {
        this.#pendingGlobalControls.delete(id);
        this.#clearOperationTimer(pendingGlobal);
        this.#paused = paused;
        pendingGlobal.resolve(paused);
        return;
      }
    }
    const pendingCommit = id === null ? undefined : this.#pendingCommits.get(id);
    if (id !== null && pendingCommit !== undefined) {
      if (isCommitRevocation(message, pendingCommit.request)) {
        if (pendingCommit.grantTimer !== null) clearTimeout(pendingCommit.grantTimer);
        this.#pendingCommits.delete(id);
        this.#clearOperationTimer(pendingCommit);
        pendingCommit.reject(new Error("Broker revoked commit authorization"));
        return;
      }
      if (isCommitRejection(message, pendingCommit.request)) {
        if (pendingCommit.grantTimer !== null) clearTimeout(pendingCommit.grantTimer);
        this.#pendingCommits.delete(id);
        this.#clearOperationTimer(pendingCommit);
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
          this.#clearOperationTimer(pendingCommit);
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
        this.#clearOperationTimer(pendingCommit);
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
        this.#clearOperationTimer(pending);
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
      this.#clearOperationTimer(pending);
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
    this.#sessions.clear();
    this.#paused = null;
    for (const pending of this.#pending.values()) {
      this.#clearOperationTimer(pending);
      pending.reject(error);
    }
    this.#pending.clear();
    for (const pending of this.#pendingCommits.values()) {
      if (pending.grantTimer !== null) clearTimeout(pending.grantTimer);
      this.#clearOperationTimer(pending);
      pending.reject(error);
    }
    this.#pendingCommits.clear();
    for (const pending of this.#pendingGlobalControls.values()) {
      this.#clearOperationTimer(pending);
      pending.reject(error);
    }
    this.#pendingGlobalControls.clear();
    this.#authorizedCommits.clear();
  }

  #rejectSessionWork(sessionId: string, error: Error): void {
    for (const [id, pending] of this.#pending) {
      if (pending.request.sessionId !== sessionId) continue;
      this.#pending.delete(id);
      this.#clearOperationTimer(pending);
      pending.reject(error);
    }
    for (const [id, pending] of this.#pendingCommits) {
      if (pending.request.sessionId !== sessionId) continue;
      if (pending.grantTimer !== null) clearTimeout(pending.grantTimer);
      this.#pendingCommits.delete(id);
      this.#clearOperationTimer(pending);
      pending.reject(error);
    }
    for (const [id, request] of this.#authorizedCommits) {
      if (request.sessionId === sessionId) this.#authorizedCommits.delete(id);
    }
  }

  #startOperationTimer(onTimeout: () => void): ReturnType<typeof setTimeout> {
    return setTimeout(onTimeout, this.#operationTimeoutMs);
  }

  #clearOperationTimer(pending: TimedPendingOperation): void {
    if (pending.operationTimer === null) return;
    clearTimeout(pending.operationTimer);
    pending.operationTimer = null;
  }
}
