import type {
  CommitAuthorization,
  CommitAuthorizationRequest,
  SuggestionAddress,
  SuggestionClearEvent,
  SuggestionRequest,
  SuggestionResponse,
  SuggestionTransport,
} from "../shared/model";
import { sanitizeSuggestion } from "../content/context";
import type { MonacoBridge } from "./monaco-runtime-bridge";
import type { MonacoSnapshot } from "./monaco-main-world";
import { MonacoGhostView, type MonacoSuggestionView } from "./monaco-view";

const MAX_GENERATION_AGE_MS = 600;
const DILLINGER_PRODUCT_LANGUAGE = "en";

interface PendingSuggestion {
  readonly request: SuggestionRequest;
  readonly snapshot: MonacoSnapshot;
  readonly generation: number;
  readonly deadlineAt: number;
  deadlineTimer: ReturnType<typeof setTimeout> | null;
}

interface VisibleSuggestion {
  readonly request: SuggestionRequest;
  readonly snapshot: MonacoSnapshot;
  readonly text: string;
  readonly suggestionId: string;
  readonly expiresAt: number;
}

export interface MonacoControllerOptions {
  readonly transport: SuggestionTransport;
  readonly bridge: MonacoBridge;
  readonly document?: Document;
  readonly view?: MonacoSuggestionView;
  readonly sessionId: string;
  readonly origin: string;
  readonly isCurrentDocument: () => boolean;
  readonly now?: () => number;
  readonly idFactory?: () => string;
  readonly debounceMs?: number;
}

function snapshotIdentity(snapshot: MonacoSnapshot): string {
  return [
    snapshot.modelUri,
    snapshot.languageId,
    snapshot.versionId,
    snapshot.valueLength,
    snapshot.offset,
    snapshot.lineNumber,
    snapshot.column,
    snapshot.before,
    snapshot.after,
  ].join("\u001f");
}

function fingerprint(value: string): string {
  let hash = 0x811c9dc5;
  for (const character of value) {
    hash ^= character.codePointAt(0) ?? 0;
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, "0");
}

function addressesMatch(left: SuggestionAddress, right: SuggestionAddress): boolean {
  return (
    left.requestId === right.requestId &&
    left.sessionId === right.sessionId &&
    left.focusEpoch === right.focusEpoch &&
    left.revision === right.revision &&
    left.fingerprint === right.fingerprint &&
    left.suggestionId === right.suggestionId
  );
}

export class MonacoController {
  readonly #transport: SuggestionTransport;
  readonly #bridge: MonacoBridge;
  readonly #document: Document;
  readonly #view: MonacoSuggestionView;
  readonly #sessionId: string;
  readonly #origin: string;
  readonly #isCurrentDocument: () => boolean;
  readonly #now: () => number;
  readonly #idFactory: () => string;
  readonly #debounceMs: number;

  #started = false;
  #paused = true;
  #disposed = false;
  #focusEpoch = 0;
  #revision = 0;
  #generation = 0;
  #refreshSequence = 0;
  #activeIdentity: string | null = null;
  #debounceTimer: ReturnType<typeof setTimeout> | null = null;
  #expiryTimer: ReturnType<typeof setTimeout> | null = null;
  #pending: PendingSuggestion | null = null;
  #visible: VisibleSuggestion | null = null;
  #authorizing: VisibleSuggestion | null = null;

  constructor(options: MonacoControllerOptions) {
    this.#transport = options.transport;
    this.#bridge = options.bridge;
    this.#document = options.document ?? globalThis.document;
    this.#view = options.view ?? new MonacoGhostView(this.#document);
    this.#sessionId = options.sessionId;
    this.#origin = options.origin;
    this.#isCurrentDocument = options.isCurrentDocument;
    this.#now = options.now ?? (() => performance.now());
    this.#idFactory = options.idFactory ?? (() => crypto.randomUUID());
    this.#debounceMs = options.debounceMs ?? 140;
  }

  get suggestionVisible(): boolean {
    return this.#visible !== null && this.#view.visible;
  }

  start(): void {
    if (this.#started || this.#disposed) return;
    this.#started = true;
    this.#document.addEventListener("input", this.#onActivity, true);
    this.#document.addEventListener("keyup", this.#onActivity, true);
    this.#document.addEventListener("mouseup", this.#onActivity, true);
    this.#document.addEventListener("selectionchange", this.#onActivity, true);
    this.#document.addEventListener("keydown", this.#onKeyDown, true);
    this.#document.addEventListener("visibilitychange", this.#onVisibilityChange, true);
    const window = this.#document.defaultView;
    window?.addEventListener("focus", this.#onActivity, true);
    window?.addEventListener("blur", this.#onBlur, true);
    window?.addEventListener("scroll", this.#onActivity, true);
    window?.addEventListener("resize", this.#onActivity, true);
  }

  resume(): void {
    if (this.#disposed) return;
    this.#paused = false;
    this.#schedule();
  }

  pause(): void {
    if (this.#disposed) return;
    this.#paused = true;
    this.#invalidateGeneration();
    this.#activeIdentity = null;
    this.#clearSuggestion();
  }

  invalidateTransport(): void {
    this.pause();
  }

  dismiss(): void {
    const visible = this.#visible;
    if (visible !== null) {
      void Promise.resolve(this.#transport.dismissSuggestion?.(this.#address(visible))).catch(
        () => undefined,
      );
    }
    this.#clearSuggestion();
  }

  acceptAll(): void {
    const visible = this.#visible;
    if (
      visible === null ||
      this.#paused ||
      this.#disposed ||
      !this.#view.visible ||
      this.#authorizing !== null ||
      !this.#documentIsUsable() ||
      this.#now() >= visible.expiresAt
    ) {
      this.#clearSuggestion();
      return;
    }
    const request: CommitAuthorizationRequest = {
      ...this.#address(visible),
      expectedText: visible.text,
      acceptance: "all",
    };
    this.#authorizing = visible;
    void this.#transport.authorizeCommit(request).then(
      (authorization) => this.#applyAuthorization(visible, request, authorization),
      () => {
        if (this.#authorizing === visible) {
          this.#authorizing = null;
          this.#clearSuggestion();
        }
      },
    );
  }

  revokeCommit(address: SuggestionAddress): void {
    const visible = this.#visible;
    if (visible !== null && addressesMatch(this.#address(visible), address)) {
      this.#authorizing = null;
      this.#clearSuggestion();
    }
  }

  clearFromBroker(event: SuggestionClearEvent): void {
    const visible = this.#visible;
    if (visible === null) return;
    const address = this.#address(visible);
    if (
      address.sessionId === event.sessionId &&
      address.focusEpoch === event.focusEpoch &&
      address.revision === event.revision &&
      address.fingerprint === event.fingerprint &&
      (event.suggestionId === null || address.suggestionId === event.suggestionId)
    ) {
      this.#authorizing = null;
      this.#clearSuggestion();
    }
  }

  dispose(invalidateTransport = false): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.#paused = true;
    this.#invalidateGeneration();
    this.#clearSuggestion();
    this.#document.removeEventListener("input", this.#onActivity, true);
    this.#document.removeEventListener("keyup", this.#onActivity, true);
    this.#document.removeEventListener("mouseup", this.#onActivity, true);
    this.#document.removeEventListener("selectionchange", this.#onActivity, true);
    this.#document.removeEventListener("keydown", this.#onKeyDown, true);
    this.#document.removeEventListener("visibilitychange", this.#onVisibilityChange, true);
    const window = this.#document.defaultView;
    window?.removeEventListener("focus", this.#onActivity, true);
    window?.removeEventListener("blur", this.#onBlur, true);
    window?.removeEventListener("scroll", this.#onActivity, true);
    window?.removeEventListener("resize", this.#onActivity, true);
    this.#view.dispose();
    if (invalidateTransport) this.#transport.dispose?.();
    void Promise.resolve(this.#transport.closeSession?.(this.#sessionId)).catch(() => undefined);
  }

  readonly #onActivity = (): void => {
    this.#schedule();
  };

  readonly #onBlur = (): void => {
    this.#invalidateGeneration();
    this.#activeIdentity = null;
    this.#clearSuggestion();
  };

  readonly #onVisibilityChange = (): void => {
    if (this.#document.visibilityState !== "visible") this.#onBlur();
    else this.#schedule();
  };

  readonly #onKeyDown = (event: KeyboardEvent): void => {
    if (event.isTrusted && event.key === "Escape" && this.suggestionVisible) {
      this.dismiss();
    }
  };

  #schedule(): void {
    if (!this.#started || this.#paused || this.#disposed || !this.#documentIsUsable()) {
      this.#invalidateGeneration();
      this.#clearSuggestion();
      return;
    }
    if (this.#debounceTimer !== null) clearTimeout(this.#debounceTimer);
    this.#debounceTimer = setTimeout(() => {
      this.#debounceTimer = null;
      void this.#refresh();
    }, this.#debounceMs);
  }

  async #refresh(): Promise<void> {
    if (this.#paused || this.#disposed || !this.#documentIsUsable()) return;
    const refreshSequence = ++this.#refreshSequence;
    let snapshot: MonacoSnapshot | null;
    try {
      snapshot = await this.#bridge.snapshot(this.#sessionId);
    } catch {
      snapshot = null;
    }
    if (refreshSequence !== this.#refreshSequence || this.#paused || this.#disposed) return;
    if (snapshot === null || !this.#documentIsUsable()) {
      this.#activeIdentity = null;
      this.#clearSuggestion();
      return;
    }
    const identity = snapshotIdentity(snapshot);
    if (identity === this.#activeIdentity) {
      if (this.#visible !== null) {
        this.#view.show(this.#visible.text, snapshot.geometry);
        if (!this.#view.visible) this.#clearSuggestion();
      }
      return;
    }
    if (this.#activeIdentity === null) this.#focusEpoch += 1;
    this.#activeIdentity = identity;
    this.#revision += 1;
    const generation = ++this.#generation;
    this.#cancelPending();
    this.#clearSuggestion();
    const request: SuggestionRequest = {
      requestId: this.#idFactory(),
      sessionId: this.#sessionId,
      origin: this.#origin,
      focusEpoch: this.#focusEpoch,
      revision: this.#revision,
      monotonicMs: Math.max(0, Math.floor(this.#now())),
      context: {
        fingerprint: fingerprint(identity),
        before: snapshot.before,
        after: snapshot.after,
        // The product slice is frozen to Dillinger's explicitly English cell;
        // Monaco's "markdown" model id is not a natural-language inference.
        language: DILLINGER_PRODUCT_LANGUAGE,
        selection: { start: snapshot.offset, end: snapshot.offset, direction: "none" },
        field: {
          purpose: "normal",
          editable: true,
          multiline: true,
          composing: false,
          sensitive: false,
          identityKnown: true,
        },
        activation: "always",
        explicit: false,
      },
    };
    const deadlineAt = this.#now() + MAX_GENERATION_AGE_MS;
    const pending: PendingSuggestion = {
      request,
      snapshot,
      generation,
      deadlineAt,
      deadlineTimer: null,
    };
    pending.deadlineTimer = setTimeout(() => {
      if (this.#pending !== pending) return;
      this.#pending = null;
      void Promise.resolve(this.#transport.cancelSuggestion(request)).catch(() => undefined);
    }, MAX_GENERATION_AGE_MS);
    this.#pending = pending;
    void this.#transport.requestSuggestion(request).then(
      (response) => this.#receive(pending, response),
      () => {
        if (this.#pending === pending) {
          this.#pending = null;
          this.#clearPendingDeadline(pending);
        }
      },
    );
  }

  async #receive(pending: PendingSuggestion, response: SuggestionResponse): Promise<void> {
    if (this.#pending !== pending) return;
    this.#pending = null;
    this.#clearPendingDeadline(pending);
    if (
      pending.generation !== this.#generation ||
      this.#now() >= pending.deadlineAt ||
      this.#paused ||
      this.#disposed ||
      !this.#documentIsUsable() ||
      response.requestId !== pending.request.requestId ||
      response.sessionId !== pending.request.sessionId ||
      response.focusEpoch !== pending.request.focusEpoch ||
      response.revision !== pending.request.revision ||
      response.fingerprint !== pending.request.context.fingerprint
    ) {
      return;
    }
    const text = sanitizeSuggestion(response.suggestion ?? "");
    if (
      text === null ||
      response.suggestionId === null ||
      (response.ttlMs !== null &&
        (!Number.isInteger(response.ttlMs) || response.ttlMs < 1 || response.ttlMs > 600))
    ) {
      return;
    }
    let current: MonacoSnapshot | null;
    try {
      current = await this.#bridge.snapshot(this.#sessionId);
    } catch {
      current = null;
    }
    if (
      current === null ||
      snapshotIdentity(current) !== snapshotIdentity(pending.snapshot) ||
      pending.generation !== this.#generation ||
      !this.#documentIsUsable()
    ) {
      return;
    }
    const visible: VisibleSuggestion = {
      request: pending.request,
      snapshot: current,
      text,
      suggestionId: response.suggestionId,
      expiresAt: this.#now() + (response.ttlMs ?? 600),
    };
    this.#view.show(text, current.geometry);
    if (!this.#view.visible) return;
    this.#visible = visible;
    this.#expiryTimer = setTimeout(() => {
      if (this.#visible === visible) this.#clearSuggestion();
    }, Math.max(0, visible.expiresAt - this.#now()));
  }

  async #applyAuthorization(
    visible: VisibleSuggestion,
    request: CommitAuthorizationRequest,
    authorization: CommitAuthorization,
  ): Promise<void> {
    const matches =
      this.#authorizing === visible &&
      this.#visible === visible &&
      authorization.text === request.expectedText &&
      authorization.acceptance === request.acceptance &&
      addressesMatch(authorization, request);
    if (!matches || !this.#documentIsUsable()) {
      if (this.#authorizing === visible) this.#authorizing = null;
      this.#clearSuggestion();
      await this.#report(visible, "stale");
      return;
    }
    let applied = false;
    try {
      applied = await this.#bridge.apply(
        this.#sessionId,
        visible.snapshot,
        authorization,
      );
    } catch {
      applied = false;
    }
    const stillCurrent =
      this.#authorizing === visible && this.#visible === visible && this.#documentIsUsable();
    this.#authorizing = null;
    this.#clearSuggestion();
    await this.#report(visible, applied ? "applied" : "stale");
    if (applied && stillCurrent) {
      this.#activeIdentity = null;
      this.#schedule();
    }
  }

  async #report(
    visible: VisibleSuggestion,
    status: "applied" | "stale",
  ): Promise<void> {
    await Promise.resolve(
      this.#transport.reportCommit({ ...this.#address(visible), status }),
    ).catch(() => undefined);
  }

  #address(visible: VisibleSuggestion): SuggestionAddress {
    return {
      requestId: visible.request.requestId,
      sessionId: visible.request.sessionId,
      focusEpoch: visible.request.focusEpoch,
      revision: visible.request.revision,
      monotonicMs: Math.max(0, Math.floor(this.#now())),
      fingerprint: visible.request.context.fingerprint,
      suggestionId: visible.suggestionId,
    };
  }

  #documentIsUsable(): boolean {
    try {
      return (
        this.#isCurrentDocument() &&
        this.#document.visibilityState === "visible" &&
        this.#document.hasFocus()
      );
    } catch {
      return false;
    }
  }

  #invalidateGeneration(): void {
    this.#generation += 1;
    this.#refreshSequence += 1;
    if (this.#debounceTimer !== null) {
      clearTimeout(this.#debounceTimer);
      this.#debounceTimer = null;
    }
    this.#cancelPending();
    this.#authorizing = null;
  }

  #cancelPending(): void {
    const pending = this.#pending;
    this.#pending = null;
    if (pending === null) return;
    this.#clearPendingDeadline(pending);
    void Promise.resolve(this.#transport.cancelSuggestion(pending.request)).catch(
      () => undefined,
    );
  }

  #clearPendingDeadline(pending: PendingSuggestion): void {
    if (pending.deadlineTimer !== null) {
      clearTimeout(pending.deadlineTimer);
      pending.deadlineTimer = null;
    }
  }

  #clearSuggestion(): void {
    if (this.#expiryTimer !== null) {
      clearTimeout(this.#expiryTimer);
      this.#expiryTimer = null;
    }
    this.#visible = null;
    this.#view.hide();
  }
}
