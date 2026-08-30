import type {
  CommitAuthorization,
  CommitAuthorizationRequest,
  EditableField,
  SelectionSnapshot,
  SuggestionAddress,
  SuggestionClearEvent,
  SuggestionRequest,
  SuggestionResponse,
  SuggestionTransport,
  SuggestionView,
} from "../shared/model";
import { readSelection, selectionsEqual } from "../shared/model";
import {
  captureContextOrNull,
  nextSuggestionWord,
  sanitizeSuggestion,
} from "./context";
import { evaluateField } from "./field-policy";
import { AnchoredGhostView } from "./ghost-view";

const OBSERVED_POLICY_ATTRIBUTES = [
  "hidden",
  "readonly",
  "disabled",
  "type",
  "autocomplete",
  "id",
  "name",
  "form",
  "data-omatype",
  "data-omatype-field",
  "style",
  "class",
  "inert",
  "aria-hidden",
  "maxlength",
  "minlength",
  "required",
  "pattern",
] as const;

const IDENTITY_ATTRIBUTES = new Set(["id", "name", "form", "data-omatype-field"]);

interface FieldState {
  focusEpoch: number;
  revision: number;
  composing: boolean;
  lastSelection: SelectionSnapshot | null;
  debounceTimer: ReturnType<typeof setTimeout> | null;
  pending: PendingSuggestion | null;
}

interface PendingSuggestion {
  readonly request: SuggestionRequest;
  readonly value: string;
  readonly selection: SelectionSnapshot;
}

interface VisibleSuggestion {
  readonly field: EditableField;
  readonly focusEpoch: number;
  readonly revision: number;
  readonly value: string;
  readonly selection: SelectionSnapshot;
  readonly text: string;
  readonly request: SuggestionRequest;
  readonly suggestionId: string;
  readonly preferredWord: string | null;
  readonly expiresAt: number;
  readonly brokerBound: boolean;
  readonly sourceAddress: SuggestionAddress;
}

interface PendingAuthorization {
  readonly visible: VisibleSuggestion;
  readonly request: CommitAuthorizationRequest;
  readonly remainder: string;
}

export interface FieldControllerOptions {
  readonly transport: SuggestionTransport;
  readonly document?: Document;
  readonly view?: SuggestionView;
  readonly debounceMs?: number;
  readonly now?: () => number;
  readonly idFactory?: () => string;
  readonly sessionId?: string;
  readonly origin?: string;
  readonly fingerprintSalt?: string;
  /** Test-only seam: jsdom cannot construct trusted keyboard events. */
  readonly allowUntrustedKeyboardForTesting?: boolean;
}

function defaultId(): string {
  return globalThis.crypto.randomUUID();
}

function eventField(event: Event): Element | null {
  return event.target instanceof Element ? event.target : null;
}

export class FieldController {
  readonly #transport: SuggestionTransport;
  readonly #document: Document;
  readonly #view: SuggestionView;
  readonly #debounceMs: number;
  readonly #now: () => number;
  readonly #idFactory: () => string;
  readonly #sessionId: string;
  readonly #origin: string;
  readonly #fingerprintSalt: string;
  readonly #allowUntrustedKeyboardForTesting: boolean;
  readonly #states = new WeakMap<EditableField, FieldState>();
  readonly #internalMutations = new WeakSet<EditableField>();

  #focusSequence = 0;
  #activeField: EditableField | null = null;
  #visible: VisibleSuggestion | null = null;
  #expiryTimer: ReturnType<typeof setTimeout> | null = null;
  #mutationObserver: MutationObserver | null = null;
  #activeAuthorization: PendingAuthorization | null = null;
  #paused = false;
  #started = false;

  constructor(options: FieldControllerOptions) {
    this.#transport = options.transport;
    this.#document = options.document ?? globalThis.document;
    this.#view = options.view ?? new AnchoredGhostView(this.#document);
    this.#debounceMs = options.debounceMs ?? 140;
    this.#now = options.now ?? (() => performance.now());
    this.#idFactory = options.idFactory ?? defaultId;
    this.#sessionId = options.sessionId ?? this.#idFactory();
    this.#origin = options.origin ?? this.#document.location.origin;
    this.#fingerprintSalt = options.fingerprintSalt ?? defaultId();
    this.#allowUntrustedKeyboardForTesting =
      options.allowUntrustedKeyboardForTesting ?? false;
  }

  get paused(): boolean {
    return this.#paused;
  }

  get suggestionVisible(): boolean {
    return this.#visible !== null && this.#view.visible;
  }

  start(): void {
    if (this.#started) {
      return;
    }
    this.#started = true;
    this.#document.addEventListener("focusin", this.#onFocusIn, true);
    this.#document.addEventListener("focusout", this.#onFocusOut, true);
    this.#document.addEventListener("input", this.#onInput, true);
    this.#document.addEventListener("keydown", this.#onKeyDown, true);
    this.#document.addEventListener("select", this.#onSelectionChange, true);
    this.#document.addEventListener("selectionchange", this.#onSelectionChange, true);
    this.#document.addEventListener("compositionstart", this.#onCompositionStart, true);
    this.#document.addEventListener("compositionend", this.#onCompositionEnd, true);
    this.#document.addEventListener("visibilitychange", this.#onVisibilityChange, true);
    this.#document.defaultView?.addEventListener("blur", this.#onWindowBlur, true);
    const root = this.#document.documentElement;
    const Observer = this.#document.defaultView?.MutationObserver;
    if (root !== null && Observer !== undefined) {
      this.#mutationObserver = new Observer(this.#onMutations);
      this.#mutationObserver.observe(root, {
        subtree: true,
        childList: true,
        attributes: true,
        attributeFilter: [...OBSERVED_POLICY_ATTRIBUTES],
      });
    }
  }

  pause(): void {
    if (this.#paused) {
      return;
    }
    this.#paused = true;
    this.#cancelActiveWork();
    this.#clearSuggestion();
  }

  resume(): void {
    if (!this.#paused) {
      return;
    }
    this.#paused = false;
    if (this.#activeField !== null) {
      this.#schedule(this.#activeField);
    }
  }

  dismiss(): void {
    const visible = this.#visible;
    if (visible === null || !this.#view.visible) {
      return;
    }
    this.#notifyDismissal(visible);
    this.#cancelActiveWork();
    this.#clearSuggestion();
  }

  acceptWord(): void {
    if (this.suggestionVisible) {
      this.#accept("word");
    }
  }

  acceptAll(): void {
    if (this.suggestionVisible) {
      this.#accept("all");
    }
  }

  invalidateTransport(): void {
    this.#invalidateActiveState();
  }

  revokeCommit(address: SuggestionAddress): void {
    const visible = this.#visible;
    if (visible === null) {
      return;
    }
    const current = this.#addressFor(visible);
    if (
      current.requestId === address.requestId &&
      current.sessionId === address.sessionId &&
      current.focusEpoch === address.focusEpoch &&
      current.revision === address.revision &&
      current.fingerprint === address.fingerprint &&
      current.suggestionId === address.suggestionId
    ) {
      this.#clearSuggestion();
    }
  }

  clearFromBroker(event: SuggestionClearEvent): void {
    const visible = this.#visible;
    if (visible === null) return;
    const source = visible.sourceAddress;
    if (
      source.sessionId === event.sessionId &&
      source.focusEpoch === event.focusEpoch &&
      source.revision === event.revision &&
      source.fingerprint === event.fingerprint &&
      (event.suggestionId === null || source.suggestionId === event.suggestionId)
    ) {
      this.#clearSuggestion();
    }
  }

  dispose(): void {
    if (!this.#started) {
      return;
    }
    this.#started = false;
    this.#cancelActiveWork();
    this.#clearSuggestion();
    this.#document.removeEventListener("focusin", this.#onFocusIn, true);
    this.#document.removeEventListener("focusout", this.#onFocusOut, true);
    this.#document.removeEventListener("input", this.#onInput, true);
    this.#document.removeEventListener("keydown", this.#onKeyDown, true);
    this.#document.removeEventListener("select", this.#onSelectionChange, true);
    this.#document.removeEventListener("selectionchange", this.#onSelectionChange, true);
    this.#document.removeEventListener("compositionstart", this.#onCompositionStart, true);
    this.#document.removeEventListener("compositionend", this.#onCompositionEnd, true);
    this.#document.removeEventListener("visibilitychange", this.#onVisibilityChange, true);
    this.#document.defaultView?.removeEventListener("blur", this.#onWindowBlur, true);
    this.#mutationObserver?.disconnect();
    this.#mutationObserver = null;
    this.#view.dispose();
    this.#transport.dispose?.();
    this.#activeField = null;
  }

  readonly #onFocusIn = (event: FocusEvent): void => {
    const target = eventField(event);
    if (target === null) {
      return;
    }
    if (this.#document.activeElement !== target || !this.#document.hasFocus()) {
      return;
    }

    if (this.#activeField !== null && this.#activeField !== target) {
      this.#cancelActiveWork();
      this.#clearSuggestion();
      this.#activeField = null;
    }

    // This decision deliberately precedes state/context capture.
    const decision = evaluateField(target);
    if (!decision.allowed) {
      return;
    }

    const field = decision.field;
    const state = this.#stateFor(field);
    this.#cancelStateWork(state);
    // Moving focus terminates any prior IME ownership for this field. A fresh
    // focus epoch must not inherit a compositionstart whose compositionend was
    // suppressed by blur/navigation.
    state.composing = false;
    state.focusEpoch = ++this.#focusSequence;
    state.revision += 1;
    state.lastSelection = readSelection(field);
    this.#activeField = field;
    this.#clearSuggestion();
    this.#schedule(field);
  };

  readonly #onFocusOut = (event: FocusEvent): void => {
    if (event.target !== this.#activeField) {
      return;
    }
    this.#cancelActiveWork();
    this.#clearSuggestion();
    this.#activeField = null;
  };

  readonly #onInput = (event: Event): void => {
    const target = eventField(event);
    if (target === null) {
      return;
    }

    // A denied target returns before accessing its value or selection.
    const decision = evaluateField(target);
    if (!decision.allowed) {
      return;
    }
    const field = decision.field;
    if (this.#internalMutations.has(field) || field !== this.#activeField) {
      return;
    }

    const state = this.#stateFor(field);
    const previouslyVisible = this.#visible;
    this.#activeAuthorization = null;
    state.revision += 1;
    state.lastSelection = readSelection(field);
    this.#cancelStateWork(state);
    if (
      !state.composing &&
      previouslyVisible !== null &&
      this.#reconcileTypeThrough(field, state, event, previouslyVisible)
    ) {
      return;
    }
    this.#clearSuggestion();
    if (!state.composing) {
      this.#schedule(field);
    }
  };

  readonly #onSelectionChange = (): void => {
    const field = this.#activeField;
    if (field === null || this.#internalMutations.has(field)) {
      return;
    }
    const state = this.#stateFor(field);
    const selection = readSelection(field);
    if (
      selection === null ||
      (state.lastSelection !== null && selectionsEqual(selection, state.lastSelection))
    ) {
      return;
    }
    state.lastSelection = selection;
    state.revision += 1;
    this.#cancelStateWork(state);
    this.#clearSuggestion();
    this.#schedule(field);
  };

  readonly #onCompositionStart = (event: CompositionEvent): void => {
    const target = eventField(event);
    const decision = target === null ? null : evaluateField(target);
    if (decision === null || !decision.allowed || decision.field !== this.#activeField) {
      return;
    }
    const state = this.#stateFor(decision.field);
    state.composing = true;
    state.revision += 1;
    this.#cancelStateWork(state);
    this.#clearSuggestion();
  };

  readonly #onCompositionEnd = (event: CompositionEvent): void => {
    const target = eventField(event);
    const decision = target === null ? null : evaluateField(target);
    if (decision === null || !decision.allowed || decision.field !== this.#activeField) {
      return;
    }
    const state = this.#stateFor(decision.field);
    state.composing = false;
    state.revision += 1;
    state.lastSelection = readSelection(decision.field);
    this.#schedule(decision.field);
  };

  readonly #onVisibilityChange = (): void => {
    if (this.#document.visibilityState !== "visible") {
      this.#invalidateActiveState();
    }
  };

  readonly #onWindowBlur = (): void => {
    if (!this.#document.hasFocus()) {
      this.#invalidateActiveState();
    }
  };

  readonly #onMutations: MutationCallback = (records): void => {
    const field = this.#activeField;
    if (field === null) return;

    let needsPolicyRecheck = false;
    for (const record of records) {
      if (record.type === "childList") {
        if (
          !field.isConnected ||
          Array.from(record.removedNodes).some(
            (node) => node === field || (node instanceof Element && node.contains(field)),
          )
        ) {
          this.#invalidateActiveState();
          return;
        }
        needsPolicyRecheck = true;
        continue;
      }
      if (record.type !== "attributes") continue;
      const target = record.target;
      if (target === field || (target instanceof Element && target.contains(field))) {
        this.#invalidateActiveState();
        return;
      }
      if (
        record.attributeName !== null &&
        IDENTITY_ATTRIBUTES.has(record.attributeName)
      ) {
        needsPolicyRecheck = true;
      }
    }

    if (!needsPolicyRecheck) return;
    try {
      if (!evaluateField(field).allowed) this.#invalidateActiveState();
    } catch {
      this.#invalidateActiveState();
    }
  };

  readonly #onKeyDown = (event: KeyboardEvent): void => {
    if (event.target !== this.#activeField || !this.suggestionVisible) {
      return;
    }
    if (!event.isTrusted && !this.#allowUntrustedKeyboardForTesting) {
      return;
    }

    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      this.dismiss();
      return;
    }
    if (event.key === "Tab" && !event.altKey && !event.ctrlKey && !event.metaKey) {
      event.preventDefault();
      event.stopPropagation();
      this.#accept("all");
      return;
    }
    if (
      event.key === "ArrowRight" &&
      (event.ctrlKey || event.metaKey) &&
      !event.altKey &&
      !event.shiftKey
    ) {
      event.preventDefault();
      event.stopPropagation();
      this.#accept("word");
    }
  };

  #stateFor(field: EditableField): FieldState {
    const existing = this.#states.get(field);
    if (existing !== undefined) {
      return existing;
    }
    const created: FieldState = {
      focusEpoch: 0,
      revision: 0,
      composing: false,
      lastSelection: null,
      debounceTimer: null,
      pending: null,
    };
    this.#states.set(field, created);
    return created;
  }

  #schedule(field: EditableField): void {
    if (
      this.#paused ||
      this.#document.visibilityState !== "visible" ||
      !this.#document.hasFocus() ||
      this.#document.activeElement !== field ||
      field !== this.#activeField
    ) {
      return;
    }
    const state = this.#stateFor(field);
    if (state.composing) {
      return;
    }
    if (state.debounceTimer !== null) {
      clearTimeout(state.debounceTimer);
    }
    state.debounceTimer = setTimeout(() => {
      state.debounceTimer = null;
      this.#request(field, state);
    }, this.#debounceMs);
  }

  #request(field: EditableField, state: FieldState): void {
    if (
      this.#paused ||
      this.#document.visibilityState !== "visible" ||
      !this.#document.hasFocus() ||
      this.#document.activeElement !== field ||
      field !== this.#activeField ||
      state.composing
    ) {
      return;
    }

    // Re-run local policy before reading any user text. Attribute/type changes
    // between scheduling and dispatch therefore fail closed.
    const decision = evaluateField(field);
    if (!decision.allowed) {
      this.#clearSuggestion();
      return;
    }
    const selection = readSelection(field);
    if (selection === null || selection.start !== selection.end) {
      this.#clearSuggestion();
      return;
    }
    if (state.lastSelection !== null && !selectionsEqual(state.lastSelection, selection)) {
      state.lastSelection = selection;
      state.revision += 1;
    }

    const context = captureContextOrNull({
      field,
      purpose: decision.purpose,
      selection,
      composing: state.composing,
      activation: "always",
      explicit: false,
      fingerprintSalt: this.#fingerprintSalt,
    });
    if (context === null) {
      this.#clearSuggestion();
      return;
    }
    const request: SuggestionRequest = {
      requestId: this.#idFactory(),
      sessionId: this.#sessionId,
      origin: this.#origin,
      focusEpoch: state.focusEpoch,
      revision: state.revision,
      monotonicMs: Math.max(0, Math.floor(this.#now())),
      context,
    };
    const pending: PendingSuggestion = {
      request,
      value: field.value,
      selection,
    };
    state.pending = pending;

    void this.#transport.requestSuggestion(request).then(
      (response) => {
        this.#receive(field, state, pending, response);
      },
      () => {
        if (state.pending?.request.requestId === request.requestId) {
          state.pending = null;
        }
      },
    );
  }

  #receive(
    field: EditableField,
    state: FieldState,
    pending: PendingSuggestion,
    response: SuggestionResponse,
  ): void {
    const { request } = pending;
    if (state.pending?.request.requestId !== request.requestId) {
      return;
    }
    state.pending = null;
    if (
      this.#paused ||
      this.#document.visibilityState !== "visible" ||
      !this.#document.hasFocus() ||
      this.#activeField !== field ||
      this.#document.activeElement !== field ||
      !field.isConnected ||
      state.focusEpoch !== request.focusEpoch ||
      state.revision !== request.revision ||
      response.requestId !== request.requestId ||
      response.sessionId !== request.sessionId ||
      response.focusEpoch !== request.focusEpoch ||
      response.revision !== request.revision ||
      response.fingerprint !== request.context.fingerprint ||
      field.value !== pending.value
    ) {
      return;
    }
    const selection = readSelection(field);
    if (
      selection === null ||
      selection.start !== selection.end ||
      !selectionsEqual(selection, pending.selection)
    ) {
      return;
    }
    const decision = evaluateField(field);
    if (!decision.allowed) {
      return;
    }
    const revalidatedContext = captureContextOrNull({
      field,
      purpose: decision.purpose,
      selection,
      composing: state.composing,
      activation: request.context.activation,
      explicit: request.context.explicit,
      fingerprintSalt: this.#fingerprintSalt,
    });
    if (
      revalidatedContext === null ||
      revalidatedContext.fingerprint !== request.context.fingerprint
    ) {
      return;
    }

    const text = sanitizeSuggestion(response.suggestion ?? "");
    const preferredWord =
      response.acceptWord === null ? null : sanitizeSuggestion(response.acceptWord);
    if (
      text === null ||
      (response.acceptWord !== null && preferredWord === null) ||
      (text !== null &&
        response.acceptWord !== null &&
        preferredWord !== nextSuggestionWord(text)) ||
      response.suggestionId === null ||
      (response.ttlMs !== null &&
        (!Number.isInteger(response.ttlMs) || response.ttlMs < 1 || response.ttlMs > 600))
    ) {
      this.#clearSuggestion();
      return;
    }

    const sourceAddress: SuggestionAddress = {
      requestId: request.requestId,
      sessionId: request.sessionId,
      focusEpoch: request.focusEpoch,
      revision: request.revision,
      monotonicMs: Math.max(0, Math.floor(this.#now())),
      fingerprint: request.context.fingerprint,
      suggestionId: response.suggestionId,
    };
    this.#showSuggestion({
      field,
      focusEpoch: state.focusEpoch,
      revision: state.revision,
      value: field.value,
      selection,
      text,
      request,
      suggestionId: response.suggestionId,
      preferredWord,
      expiresAt: this.#now() + (response.ttlMs ?? 600),
      brokerBound: true,
      sourceAddress,
    });
  }

  #accept(mode: "word" | "all"): void {
    const visible = this.#visible;
    if (visible === null || !this.#view.visible) {
      this.#clearSuggestion();
      return;
    }
    if (!visible.brokerBound) {
      this.#clearSuggestion();
      this.#schedule(visible.field);
      return;
    }
    if (!this.#validateVisible(visible)) {
      this.#clearSuggestion();
      return;
    }
    if (this.#activeAuthorization?.visible === visible) {
      return;
    }

    const accepted =
      mode === "all"
        ? visible.text
        : (visible.preferredWord ?? nextSuggestionWord(visible.text));
    const remainder = visible.text.slice(accepted.length);
    if (!this.#insertionSatisfiesConstraints(visible.field, visible.selection, accepted)) {
      this.#clearSuggestion();
      return;
    }

    const authorizationRequest: CommitAuthorizationRequest = {
      ...this.#addressFor(visible),
      expectedText: accepted,
      acceptance: mode,
    };
    const pending: PendingAuthorization = {
      visible,
      request: authorizationRequest,
      remainder,
    };
    this.#activeAuthorization = pending;
    void this.#transport.authorizeCommit(authorizationRequest).then(
      (authorization) => this.#applyAuthorizedCommit(pending, authorization),
      () => {
        if (this.#activeAuthorization === pending) {
          this.#activeAuthorization = null;
          this.#clearSuggestion();
        }
      },
    );
  }

  #applyAuthorizedCommit(
    pending: PendingAuthorization,
    authorization: CommitAuthorization,
  ): void {
    const { visible, request, remainder } = pending;
    const addressMatches =
      authorization.requestId === request.requestId &&
      authorization.sessionId === request.sessionId &&
      authorization.focusEpoch === request.focusEpoch &&
      authorization.revision === request.revision &&
      authorization.fingerprint === request.fingerprint &&
      authorization.suggestionId === request.suggestionId &&
      authorization.text === request.expectedText &&
      authorization.acceptance === request.acceptance;
    const isCurrent = this.#activeAuthorization === pending;
    if (!addressMatches) {
      if (isCurrent) {
        this.#activeAuthorization = null;
        this.#clearSuggestion();
      }
      void Promise.resolve(
        this.#transport.reportCommit({ ...this.#addressFor(visible), status: "blocked" }),
      ).catch(() => undefined);
      return;
    }
    if (
      !isCurrent ||
      !this.#validateVisible(visible) ||
      !this.#insertionSatisfiesConstraints(
        visible.field,
        visible.selection,
        authorization.text,
      )
    ) {
      if (isCurrent) {
        this.#activeAuthorization = null;
        this.#clearSuggestion();
      }
      void Promise.resolve(
        this.#transport.reportCommit({ ...this.#addressFor(visible), status: "stale" }),
      ).catch(() => undefined);
      return;
    }

    const field = visible.field;
    const state = this.#stateFor(field);
    this.#activeAuthorization = null;

    // Authorization is complete; this is the final identity/value/selection/
    // revision/constraint gate immediately before the sole page mutation.
    if (
      !this.#validateVisible(visible) ||
      !this.#insertionSatisfiesConstraints(field, visible.selection, authorization.text)
    ) {
      this.#clearSuggestion();
      void Promise.resolve(
        this.#transport.reportCommit({ ...this.#addressFor(visible), status: "stale" }),
      ).catch(() => undefined);
      return;
    }
    this.#internalMutations.add(field);
    field.setRangeText(
      authorization.text,
      visible.selection.start,
      visible.selection.end,
      "end",
    );
    state.revision += 1;
    const updatedSelection = readSelection(field);
    state.lastSelection = updatedSelection;
    const expectedValue = field.value;

    let inputEvent: Event;
    try {
      inputEvent = new InputEvent("input", {
        bubbles: true,
        composed: true,
        inputType: "insertText",
        data: authorization.text,
      });
    } catch {
      inputEvent = new Event("input", { bubbles: true, composed: true });
    }
    field.dispatchEvent(inputEvent);
    this.#internalMutations.delete(field);

    const postDispatchSelection = readSelection(field);
    if (
      updatedSelection === null ||
      postDispatchSelection === null ||
      field.value !== expectedValue ||
      !field.isConnected ||
      this.#document.activeElement !== field ||
      !selectionsEqual(updatedSelection, postDispatchSelection)
    ) {
      state.revision += 1;
      state.lastSelection = postDispatchSelection;
      this.#clearSuggestion();
      void Promise.resolve(
        this.#transport.reportCommit({ ...this.#addressFor(visible), status: "stale" }),
      ).catch(() => undefined);
      this.#schedule(field);
      return;
    }

    const decision = evaluateField(field);
    const continuedContext = decision.allowed
      ? captureContextOrNull({
          field,
          purpose: decision.purpose,
          selection: updatedSelection,
          composing: false,
          activation: visible.request.context.activation,
          explicit: visible.request.context.explicit,
          fingerprintSalt: this.#fingerprintSalt,
        })
      : null;
    const continuedRequest: SuggestionRequest | null =
      decision.allowed && continuedContext !== null
      ? {
          ...visible.request,
          revision: state.revision,
          monotonicMs: Math.max(0, Math.floor(this.#now())),
          context: continuedContext,
        }
      : null;
    void Promise.resolve(
      this.#transport.reportCommit({
        ...this.#addressFor(visible),
        status: "dispatched-unverified",
        ...(continuedRequest === null
          ? {}
          : {
              newRevision: continuedRequest.revision,
              newFingerprint: continuedRequest.context.fingerprint,
            }),
      }),
    ).catch(() => undefined);

    this.#clearSuggestion();
    if (
      request.acceptance === "word" &&
      remainder.length > 0 &&
      continuedRequest !== null
    ) {
      // The broker clears the accepted suggestion. Locally advanced
      // coordinates are not eligible for another commit until a fresh
      // provider response binds the remainder to the new revision.
      this.#schedule(field);
    }
  }

  #validateVisible(visible: VisibleSuggestion): boolean {
    const field = visible.field;
    const state = this.#states.get(field);
    const selection = readSelection(field);
    if (
      this.#paused ||
      this.#document.visibilityState !== "visible" ||
      !this.#document.hasFocus() ||
      this.#visible !== visible ||
      !this.#view.visible ||
      state === undefined ||
      this.#activeField !== field ||
      this.#document.activeElement !== field ||
      !field.isConnected ||
      state.focusEpoch !== visible.focusEpoch ||
      state.revision !== visible.revision ||
      field.value !== visible.value ||
      selection === null ||
      selection.start !== selection.end ||
      !selectionsEqual(selection, visible.selection) ||
      this.#now() >= visible.expiresAt
    ) {
      return false;
    }
    const decision = evaluateField(field);
    if (!decision.allowed) {
      return false;
    }
    const context = captureContextOrNull({
      field,
      purpose: decision.purpose,
      selection,
      composing: state.composing,
      activation: visible.request.context.activation,
      explicit: visible.request.context.explicit,
      fingerprintSalt: this.#fingerprintSalt,
    });
    return (
      context !== null && context.fingerprint === visible.request.context.fingerprint
    );
  }

  #insertionSatisfiesConstraints(
    field: EditableField,
    selection: SelectionSnapshot,
    inserted: string,
  ): boolean {
    const candidate =
      field.value.slice(0, selection.start) +
      inserted +
      field.value.slice(selection.end);
    if (
      (field.maxLength >= 0 && candidate.length > field.maxLength) ||
      (field.minLength >= 0 && candidate.length < field.minLength) ||
      (field.required && candidate.length === 0) ||
      field.validity.customError
    ) {
      return false;
    }
    const probe = field.cloneNode(false) as EditableField;
    probe.value = candidate;
    return probe.value === candidate && probe.checkValidity();
  }

  #reconcileTypeThrough(
    field: EditableField,
    state: FieldState,
    event: Event,
    visible: VisibleSuggestion,
  ): boolean {
    if (
      !(event instanceof InputEvent) ||
      event.inputType !== "insertText" ||
      event.data === null ||
      event.data.length === 0 ||
      !this.#view.visible ||
      this.#paused ||
      this.#document.visibilityState !== "visible" ||
      visible.field !== field ||
      this.#activeField !== field ||
      this.#document.activeElement !== field ||
      !field.isConnected ||
      state.focusEpoch !== visible.focusEpoch ||
      state.revision !== visible.revision + 1 ||
      this.#now() >= visible.expiresAt
    ) {
      return false;
    }
    const decision = evaluateField(field);
    const selection = readSelection(field);
    if (!decision.allowed || selection === null) {
      return false;
    }

    const typedScalars = Array.from(event.data);
    const suggestionScalars = Array.from(visible.text);
    if (
      typedScalars.length > suggestionScalars.length ||
      suggestionScalars.slice(0, typedScalars.length).join("") !== event.data
    ) {
      return false;
    }
    const expectedValue =
      visible.value.slice(0, visible.selection.start) +
      event.data +
      visible.value.slice(visible.selection.end);
    const expectedCaret = visible.selection.start + event.data.length;
    if (
      field.value !== expectedValue ||
      selection.start !== expectedCaret ||
      selection.end !== expectedCaret
    ) {
      return false;
    }

    const remainder = suggestionScalars.slice(typedScalars.length).join("");
    if (remainder.length === 0) {
      this.#clearSuggestion();
      this.#schedule(field);
      return true;
    }
    const continuedContext = captureContextOrNull({
      field,
      purpose: decision.purpose,
      selection,
      composing: false,
      activation: visible.request.context.activation,
      explicit: visible.request.context.explicit,
      fingerprintSalt: this.#fingerprintSalt,
    });
    if (continuedContext === null) {
      return false;
    }
    const continuedRequest: SuggestionRequest = {
      ...visible.request,
      revision: state.revision,
      monotonicMs: Math.max(0, Math.floor(this.#now())),
      context: continuedContext,
    };
    const reconciled: VisibleSuggestion = {
      field,
      focusEpoch: state.focusEpoch,
      revision: state.revision,
      value: field.value,
      selection,
      text: remainder,
      request: continuedRequest,
      suggestionId: visible.suggestionId,
      preferredWord:
        visible.preferredWord !== null && visible.preferredWord.startsWith(event.data)
          ? visible.preferredWord.slice(event.data.length) || null
          : null,
      expiresAt: visible.expiresAt,
      brokerBound: false,
      sourceAddress: visible.sourceAddress,
    };
    this.#showSuggestion(reconciled);
    this.#schedule(field);

    // Page listeners run after our capture listener and may transform the value.
    // Revalidate after the complete event dispatch before retaining the suffix.
    queueMicrotask(() => {
      if (this.#visible === reconciled && !this.#validateVisible(reconciled)) {
        this.#clearSuggestion();
        this.#schedule(field);
      }
    });
    return true;
  }

  #cancelActiveWork(): void {
    if (this.#activeField !== null) {
      this.#cancelStateWork(this.#stateFor(this.#activeField));
    }
  }

  #invalidateActiveState(): void {
    this.#cancelActiveWork();
    this.#clearSuggestion();
  }

  #cancelStateWork(state: FieldState): void {
    if (state.debounceTimer !== null) {
      clearTimeout(state.debounceTimer);
      state.debounceTimer = null;
    }
    const pending = state.pending;
    state.pending = null;
    if (pending !== null) {
      void Promise.resolve(this.#transport.cancelSuggestion(pending.request)).catch(
        () => undefined,
      );
    }
  }

  #clearSuggestion(): void {
    if (this.#expiryTimer !== null) {
      clearTimeout(this.#expiryTimer);
      this.#expiryTimer = null;
    }
    this.#activeAuthorization = null;
    this.#visible = null;
    this.#view.hide();
  }

  #addressFor(visible: VisibleSuggestion) {
    return {
      ...visible.sourceAddress,
      monotonicMs: Math.max(0, Math.floor(this.#now())),
    } as const;
  }

  #showSuggestion(visible: VisibleSuggestion): void {
    if (this.#expiryTimer !== null) clearTimeout(this.#expiryTimer);
    this.#visible = visible;
    const remainingMs = Math.max(0, visible.expiresAt - this.#now());
    if (remainingMs === 0) {
      this.#clearSuggestion();
      return;
    }
    this.#view.show(visible.field, visible.text);
    this.#expiryTimer = setTimeout(() => {
      if (this.#visible === visible) this.#clearSuggestion();
    }, remainingMs);
  }

  #notifyDismissal(visible: VisibleSuggestion): void {
    void Promise.resolve(
      this.#transport.dismissSuggestion?.(this.#addressFor(visible)),
    ).catch(() => undefined);
  }
}
