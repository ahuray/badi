// @vitest-environment-options {"url":"http://localhost:4173/chromium.html"}

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FieldController } from "../src/content/field-controller";
import { AnchoredGhostView } from "../src/content/ghost-view";
import {
  EXPECTED_FIXTURE_ORIGIN,
  isExpectedFixtureDocument,
} from "../src/shared/fixture-document";
import type {
  CommitAuthorization,
  CommitAuthorizationRequest,
  CommitResultNotice,
  EditableField,
  SuggestionAddress,
  SuggestionRequest,
  SuggestionResponse,
  SuggestionTransport,
  SuggestionView,
} from "../src/shared/model";

interface Deferred {
  readonly request: SuggestionRequest;
  readonly resolve: (response: SuggestionResponse) => void;
  readonly reject: (error: Error) => void;
}

interface DeferredAuthorization {
  readonly request: CommitAuthorizationRequest;
  readonly resolve: (authorization: CommitAuthorization) => void;
  readonly reject: (error: Error) => void;
}

class FakeTransport implements SuggestionTransport {
  readonly requests: SuggestionRequest[] = [];
  readonly cancellations: SuggestionRequest[] = [];
  readonly dismissals: SuggestionAddress[] = [];
  readonly authorizationRequests: CommitAuthorizationRequest[] = [];
  readonly deferredAuthorizations: DeferredAuthorization[] = [];
  readonly results: CommitResultNotice[] = [];
  readonly deferred: Deferred[] = [];
  readonly closedSessions: string[] = [];
  autoAuthorize = true;

  requestSuggestion(request: SuggestionRequest): Promise<SuggestionResponse> {
    this.requests.push(request);
    return new Promise((resolve, reject) => {
      this.deferred.push({ request, resolve, reject });
    });
  }

  cancelSuggestion(request: SuggestionRequest): void {
    this.cancellations.push(request);
  }

  closeSession(sessionId: string): void {
    this.closedSessions.push(sessionId);
  }

  dismissSuggestion(address: SuggestionAddress): void {
    this.dismissals.push(address);
  }

  authorizeCommit(request: CommitAuthorizationRequest): Promise<CommitAuthorization> {
    this.authorizationRequests.push(request);
    if (this.autoAuthorize) {
      return Promise.resolve(this.authorizationFor(request));
    }
    return new Promise((resolve, reject) => {
      this.deferredAuthorizations.push({ request, resolve, reject });
    });
  }

  reportCommit(notice: CommitResultNotice): void {
    this.results.push(notice);
  }


  resolve(
    index: number,
    suggestion: string,
    acceptWord: string | null = null,
    ttlMs: number | null = null,
  ): void {
    const deferred = this.deferred[index];
    if (deferred === undefined) {
      throw new Error(`No deferred request at index ${index}`);
    }
    const { request } = deferred;
    deferred.resolve({
      requestId: request.requestId,
      sessionId: request.sessionId,
      focusEpoch: request.focusEpoch,
      revision: request.revision,
      fingerprint: request.context.fingerprint,
      suggestion,
      suggestionId: `suggestion-${index}`,
      acceptWord,
      ttlMs,
    });
  }

  authorizationFor(
    request: CommitAuthorizationRequest,
    overrides: Partial<CommitAuthorization> = {},
  ): CommitAuthorization {
    return {
      requestId: request.requestId,
      sessionId: request.sessionId,
      focusEpoch: request.focusEpoch,
      revision: request.revision,
      monotonicMs: request.monotonicMs,
      fingerprint: request.fingerprint,
      suggestionId: request.suggestionId,
      text: request.expectedText,
      acceptance: request.acceptance,
      ...overrides,
    };
  }
}

class RecordingView implements SuggestionView {
  visible = false;
  readonly shown: string[] = [];
  current = "";
  visibleHideTransitions = 0;

  show(_field: EditableField, text: string): void {
    this.visible = true;
    this.current = text;
    this.shown.push(text);
  }

  hide(): void {
    if (this.visible) this.visibleHideTransitions += 1;
    this.visible = false;
    this.current = "";
  }

  dispose(): void {
    this.hide();
  }
}

const SESSION_ID = "0198f215-3ec0-7000-8000-000000000001";

interface SilentPolicyMutation {
  readonly name: string;
  readonly mutate: (field: HTMLInputElement, ancestor: HTMLDivElement) => void;
  readonly expectedRequestCount?: number;
}

const SILENT_POLICY_MUTATIONS: readonly SilentPolicyMutation[] = [
  { name: "readonly", mutate: (field) => { field.readOnly = true; } },
  { name: "disabled", mutate: (field) => { field.disabled = true; } },
  { name: "type", mutate: (field) => { field.type = "password"; } },
  {
    name: "autocomplete",
    mutate: (field) => { field.autocomplete = "one-time-code"; },
  },
  // A changed constraint invalidates the old suggestion but leaves the field
  // eligible for a fresh, constraint-aware result.
  {
    name: "constraint",
    mutate: (field) => { field.maxLength = field.value.length; },
    expectedRequestCount: 2,
  },
  { name: "ancestor hidden", mutate: (_field, ancestor) => { ancestor.hidden = true; } },
  {
    name: "ancestor opt-out",
    mutate: (_field, ancestor) => { ancestor.setAttribute("data-badi", "off"); },
  },
  {
    name: "ancestor style",
    mutate: (field, ancestor) => {
      ancestor.style.display = "none";
      // jsdom has no layout engine; model Chromium's checkVisibility result.
      Object.defineProperty(field, "checkVisibility", {
        configurable: true,
        value: () => false,
      });
    },
  },
  {
    name: "ancestor inert",
    mutate: (_field, ancestor) => { ancestor.setAttribute("inert", ""); },
  },
  {
    name: "ancestor aria-hidden",
    mutate: (_field, ancestor) => { ancestor.setAttribute("aria-hidden", "true"); },
  },
];

function nextIdFactory(): () => string {
  let next = 0;
  return () => `request-${++next}`;
}

function setValue(field: EditableField, value: string): void {
  field.value = value;
  field.setSelectionRange(value.length, value.length, "none");
  field.dispatchEvent(
    new InputEvent("input", {
      bubbles: true,
      inputType: "insertText",
      data: value.at(-1) ?? null,
    }),
  );
}

function typeText(field: EditableField, value: string): void {
  const start = field.selectionStart ?? field.value.length;
  const end = field.selectionEnd ?? start;
  field.setRangeText(value, start, end, "end");
  field.dispatchEvent(
    new InputEvent("input", {
      bubbles: true,
      inputType: "insertText",
      data: value,
    }),
  );
}

async function dispatchRequest(debounceMs = 5): Promise<void> {
  await vi.advanceTimersByTimeAsync(debounceMs);
  await Promise.resolve();
}

async function deliverMutationObserver(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
}

describe("FieldController", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    history.replaceState(null, "", "/chromium.html");
    document.body.replaceChildren();
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    Object.defineProperty(HTMLElement.prototype, "checkVisibility", {
      configurable: true,
      value: () => true,
    });
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(
      new DOMRect(0, 0, 320, 40),
    );
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    delete (HTMLElement.prototype as { checkVisibility?: unknown }).checkVisibility;
    document.body.replaceChildren();
    document.querySelectorAll("[data-badi-owned]").forEach((node) => node.remove());
  });

  it("makes zero outbound context/provider requests for locally denied fields", async () => {
    document.body.innerHTML = `
      <input id="password" type="password" value="never send me">
      <input id="hidden" type="hidden" value="never send me either">
      <input id="otp" type="text" autocomplete="one-time-code" value="123456">
      <textarea id="payment" autocomplete="cc-number">4111111111111111</textarea>
      <input id="search" type="search" value="private query">
      <input id="url" type="url" value="https://private.example">
      <input id="tel" type="tel" value="+49 000 000000">
      <input id="email" type="email" value="private@example.test">
      <textarea id="anonymous-secret-like">will lose its id</textarea>
    `;
    const transport = new FakeTransport();
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view: new RecordingView(),
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();

    for (const id of [
      "password",
      "hidden",
      "otp",
      "payment",
      "search",
      "url",
      "tel",
      "email",
    ]) {
      const field = document.querySelector<HTMLInputElement | HTMLTextAreaElement>(`#${id}`);
      if (field === null) throw new Error(`Fixture field ${id} missing`);
      field.focus();
      field.dispatchEvent(new InputEvent("input", { bubbles: true, data: "x" }));
      await dispatchRequest();
    }

    for (const [id, value, caret] of [
      ["unpaired-high", `safe\uD800tail`, 9],
      ["unpaired-low", `safe\uDFFFtail`, 9],
      ["split-valid-pair", "safe🙂tail", 5],
    ] as const) {
      const field = document.createElement("textarea");
      field.id = id;
      field.value = value;
      document.body.append(field);
      field.setSelectionRange(caret, caret);
      field.focus();
      await dispatchRequest();
    }

    const anonymous = document.querySelector<HTMLTextAreaElement>("#anonymous-secret-like");
    if (anonymous === null) throw new Error("Anonymous fixture missing");
    anonymous.removeAttribute("id");
    let valueReads = 0;
    Object.defineProperty(anonymous, "value", {
      configurable: true,
      get: () => {
        valueReads += 1;
        return "must stay local";
      },
      set: () => undefined,
    });
    anonymous.focus();
    anonymous.dispatchEvent(new InputEvent("input", { bubbles: true, data: "x" }));
    await dispatchRequest();

    expect(transport.requests).toHaveLength(0);
    expect(transport.cancellations).toHaveLength(0);
    expect(valueReads).toBe(0);
    controller.dispose();
  });

  it("does not acquire text from a focused field outside the viewport", async () => {
    const field = document.createElement("textarea");
    field.id = "offscreen-field";
    document.body.append(field);
    let fieldRect = new DOMRect(-500, -500, 200, 40);
    field.getBoundingClientRect = () => fieldRect;
    let valueReads = 0;
    Object.defineProperty(field, "value", {
      configurable: true,
      get: () => {
        valueReads += 1;
        return "must remain local";
      },
      set: () => undefined,
    });
    const transport = new FakeTransport();
    const controller = new FieldController({
      transport,
      view: new RecordingView(),
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();

    field.focus();
    await dispatchRequest();

    expect(valueReads).toBe(0);
    expect(transport.requests).toHaveLength(0);

    fieldRect = new DOMRect(20, 20, 200, 40);
    field.dispatchEvent(
      new InputEvent("input", {
        bubbles: true,
        inputType: "insertText",
        data: "x",
      }),
    );
    await dispatchRequest();
    expect(valueReads).toBeGreaterThan(0);
    expect(transport.requests).toHaveLength(1);
    controller.dispose();
  });

  it("does not acquire text through filtered, clipped, masked, or overflow-clipped fields", async () => {
    const transport = new FakeTransport();
    const controller = new FieldController({
      transport,
      view: new RecordingView(),
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    let valueReads = 0;

    const addField = (id: string): HTMLTextAreaElement => {
      const field = document.createElement("textarea");
      field.id = id;
      Object.defineProperty(field, "value", {
        configurable: true,
        get: () => {
          valueReads += 1;
          return "must remain local";
        },
        set: () => undefined,
      });
      document.body.append(field);
      return field;
    };

    const filtered = addField("filtered-field");
    filtered.style.filter = "opacity(0)";
    const clipped = addField("clipped-field");
    clipped.style.clipPath = "inset(50%)";
    const masked = addField("masked-field");
    masked.style.maskImage = "linear-gradient(transparent, transparent)";
    const transformed = addField("transformed-field");
    transformed.style.transform = "scale(0)";

    const clippingAncestor = document.createElement("div");
    clippingAncestor.style.overflow = "hidden";
    clippingAncestor.getBoundingClientRect = () => new DOMRect(0, 0, 10, 10);
    const overflowClipped = document.createElement("textarea");
    overflowClipped.id = "overflow-clipped-field";
    overflowClipped.getBoundingClientRect = () => new DOMRect(20, 20, 200, 40);
    Object.defineProperty(overflowClipped, "value", {
      configurable: true,
      get: () => {
        valueReads += 1;
        return "must also remain local";
      },
      set: () => undefined,
    });
    clippingAncestor.append(overflowClipped);
    document.body.append(clippingAncestor);

    const paintContainer = document.createElement("div");
    paintContainer.style.contain = "paint";
    paintContainer.getBoundingClientRect = () => new DOMRect(0, 0, 10, 10);
    const paintClipped = document.createElement("textarea");
    paintClipped.id = "paint-clipped-field";
    paintClipped.getBoundingClientRect = () => new DOMRect(20, 20, 200, 40);
    Object.defineProperty(paintClipped, "value", {
      configurable: true,
      get: () => {
        valueReads += 1;
        return "paint-contained secret";
      },
      set: () => undefined,
    });
    paintContainer.append(paintClipped);
    document.body.append(paintContainer);

    for (const field of [
      filtered,
      clipped,
      masked,
      transformed,
      overflowClipped,
      paintClipped,
    ]) {
      field.focus();
      await dispatchRequest();
    }

    expect(valueReads).toBe(0);
    expect(transport.requests).toHaveLength(0);
    controller.dispose();
  });

  it("does not acquire context from synthetic focus events or an unfocused document", async () => {
    const field = document.createElement("textarea");
    field.id = "synthetic-focus-target";
    const focusSink = document.createElement("button");
    document.body.append(field, focusSink);
    focusSink.focus();
    let valueReads = 0;
    Object.defineProperty(field, "value", {
      configurable: true,
      get: () => {
        valueReads += 1;
        return "must remain unread";
      },
      set: () => undefined,
    });
    const transport = new FakeTransport();
    const controller = new FieldController({
      transport,
      view: new RecordingView(),
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();

    field.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
    field.dispatchEvent(new InputEvent("input", { bubbles: true, data: "x" }));
    await dispatchRequest(50);
    expect(transport.requests).toHaveLength(0);
    expect(valueReads).toBe(0);

    vi.mocked(document.hasFocus).mockReturnValue(false);
    field.focus();
    field.dispatchEvent(new InputEvent("input", { bubbles: true, data: "y" }));
    await dispatchRequest(50);
    expect(transport.requests).toHaveLength(0);
    expect(valueReads).toBe(0);
    controller.dispose();
  });

  it("shows, dismisses, accepts a Unicode word, accepts all, and pauses/resumes", async () => {
    const field = document.createElement("textarea");
    field.id = "draft";
    field.value = "Hello";
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    const transport = new FakeTransport();
    const view = new RecordingView();
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view,
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
      now: () => 1_000,
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    transport.resolve(0, " schön weiter", " schön");
    await Promise.resolve();

    expect(view.current).toBe(" schön weiter");
    const dismiss = new KeyboardEvent("keydown", {
      key: "Escape",
      bubbles: true,
      cancelable: true,
    });
    field.dispatchEvent(dismiss);
    expect(dismiss.defaultPrevented).toBe(true);
    expect(view.visible).toBe(false);
    expect(transport.dismissals).toHaveLength(1);

    const unusedEscape = new KeyboardEvent("keydown", {
      key: "Escape",
      bubbles: true,
      cancelable: true,
    });
    field.dispatchEvent(unusedEscape);
    expect(unusedEscape.defaultPrevented).toBe(false);

    setValue(field, "Hallo");
    await dispatchRequest();
    transport.resolve(1, " élève encore", " élève");
    await Promise.resolve();
    let observedInputEvents = 0;
    field.addEventListener("input", () => {
      observedInputEvents += 1;
    });

    const acceptWord = new KeyboardEvent("keydown", {
      key: "ArrowRight",
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    });
    field.dispatchEvent(acceptWord);
    await Promise.resolve();
    await Promise.resolve();
    expect(acceptWord.defaultPrevented).toBe(true);
    expect(field.value).toBe("Hallo élève");
    expect(view.visible).toBe(false);
    expect(observedInputEvents).toBe(1);

    // The old suggestion was cleared by the broker-side commit. A fresh
    // provider response must bind the remainder to the new DOM coordinates.
    await dispatchRequest();
    transport.resolve(2, " encore", " encore");
    await Promise.resolve();
    expect(view.current).toBe(" encore");

    const acceptAll = new KeyboardEvent("keydown", {
      key: "Tab",
      bubbles: true,
      cancelable: true,
    });
    field.dispatchEvent(acceptAll);
    await Promise.resolve();
    await Promise.resolve();
    expect(acceptAll.defaultPrevented).toBe(true);
    expect(field.value).toBe("Hallo élève encore");
    expect(view.visible).toBe(false);
    expect(observedInputEvents).toBe(2);
    expect(transport.authorizationRequests.map((notice) => notice.acceptance)).toEqual([
      "word",
      "all",
    ]);
    expect(transport.results).toHaveLength(2);

    controller.pause();
    expect(controller.paused).toBe(true);
    setValue(field, `${field.value}!`);
    await dispatchRequest(50);
    expect(transport.requests).toHaveLength(3);
    controller.resume();
    await dispatchRequest();
    expect(controller.paused).toBe(false);
    expect(transport.requests).toHaveLength(4);
    controller.dispose();
  });

  it("preserves Shift+Tab as normal page navigation", async () => {
    const field = document.createElement("textarea");
    field.id = "shift-tab-draft";
    field.value = "Keep";
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    const transport = new FakeTransport();
    const view = new RecordingView();
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view,
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    transport.resolve(0, " unchanged", " unchanged");
    await Promise.resolve();

    const shiftTab = new KeyboardEvent("keydown", {
      key: "Tab",
      shiftKey: true,
      bubbles: true,
      cancelable: true,
    });
    field.dispatchEvent(shiftTab);

    expect(shiftTab.defaultPrevented).toBe(false);
    expect(field.value).toBe("Keep");
    expect(view.visible).toBe(true);
    expect(transport.authorizationRequests).toHaveLength(0);
    controller.dispose();
  });

  it("ignores an accept key already consumed by the page", async () => {
    const field = document.createElement("textarea");
    field.id = "page-consumed-tab";
    field.value = "Keep";
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    const transport = new FakeTransport();
    const view = new RecordingView();
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view,
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    transport.resolve(0, " unchanged", " unchanged");
    await Promise.resolve();

    const consumedTab = new KeyboardEvent("keydown", {
      key: "Tab",
      bubbles: true,
      cancelable: true,
    });
    consumedTab.preventDefault();
    field.dispatchEvent(consumedTab);

    expect(consumedTab.defaultPrevented).toBe(true);
    expect(field.value).toBe("Keep");
    expect(view.visible).toBe(true);
    expect(transport.authorizationRequests).toHaveLength(0);
    controller.dispose();
  });

  it("adopts an eligible already-focused field without acquiring while initially paused", async () => {
    const field = document.createElement("textarea");
    field.id = "autofocus-draft";
    field.value = "Ready";
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    field.focus();
    const transport = new FakeTransport();
    const controller = new FieldController({
      transport,
      view: new RecordingView(),
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.pause();
    controller.start();

    await dispatchRequest(50);
    expect(transport.requests).toHaveLength(0);
    controller.resume();
    await dispatchRequest();
    expect(transport.requests).toHaveLength(1);
    controller.dispose();
  });

  it("advances authority coordinates when an unchanged field pauses and resumes", async () => {
    const field = document.createElement("textarea");
    field.id = "pause-resume-draft";
    field.value = "Unchanged";
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    const transport = new FakeTransport();
    const controller = new FieldController({
      transport,
      view: new RecordingView(),
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    const beforePause = transport.requests[0];
    if (beforePause === undefined) throw new Error("Initial request missing");

    controller.pause();
    controller.resume();
    await dispatchRequest();
    const afterResume = transport.requests[1];
    if (afterResume === undefined) throw new Error("Resumed request missing");

    expect(transport.cancellations).toEqual([beforePause]);
    expect(afterResume.sessionId).toBe(beforePause.sessionId);
    expect(afterResume.focusEpoch).toBe(beforePause.focusEpoch);
    expect(afterResume.revision).toBeGreaterThan(beforePause.revision);
    expect(afterResume.context.before).toBe(beforePause.context.before);
    controller.dispose();
  });

  it("adopts a background document's active field only after the window gains focus", async () => {
    const field = document.createElement("textarea");
    field.id = "background-bootstrap-draft";
    field.value = "Ready";
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    field.focus();
    vi.mocked(document.hasFocus).mockReturnValue(false);
    const transport = new FakeTransport();
    const controller = new FieldController({
      transport,
      view: new RecordingView(),
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    await dispatchRequest(50);
    expect(transport.requests).toHaveLength(0);
    expect(document.activeElement).toBe(field);

    vi.mocked(document.hasFocus).mockReturnValue(true);
    const ownerWindow = document.defaultView;
    if (ownerWindow === null) throw new Error("Document window missing");
    ownerWindow.dispatchEvent(new ownerWindow.FocusEvent("focus"));
    await dispatchRequest();
    expect(transport.requests).toHaveLength(1);
    controller.dispose();
  });

  it("closes a session on focusout and page lifecycle changes, then reopens cleanly", async () => {
    const field = document.createElement("textarea");
    field.id = "lifecycle-draft";
    field.value = "Ready";
    const sink = document.createElement("button");
    document.body.append(field, sink);
    field.setSelectionRange(field.value.length, field.value.length);
    const transport = new FakeTransport();
    const controller = new FieldController({
      transport,
      view: new RecordingView(),
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    field.focus();
    await dispatchRequest();

    sink.focus();
    expect(transport.closedSessions).toEqual([SESSION_ID]);
    field.focus();
    await dispatchRequest();
    expect(transport.requests).toHaveLength(2);

    window.dispatchEvent(new Event("pagehide"));
    expect(transport.closedSessions).toEqual([SESSION_ID, SESSION_ID]);
    window.dispatchEvent(new Event("pagehide"));
    expect(transport.closedSessions).toEqual([SESSION_ID, SESSION_ID]);
    window.dispatchEvent(new Event("pageshow"));
    await dispatchRequest();
    expect(transport.requests).toHaveLength(3);

    controller.dispose();
    expect(transport.closedSessions).toEqual([SESSION_ID, SESSION_ID, SESSION_ID]);
  });

  it("drops a response after a pushState route transition and closes its session", async () => {
    const field = document.createElement("textarea");
    field.id = "push-state-draft";
    field.value = "Stay";
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    let exactDocument = true;
    const transport = new FakeTransport();
    const view = new RecordingView();
    const controller = new FieldController({
      transport,
      view,
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
      isCurrentDocument: () => exactDocument,
    });
    controller.start();
    field.focus();
    await dispatchRequest();

    history.pushState({ route: "other" }, "");
    exactDocument = false;
    transport.resolve(0, " must not display");
    await Promise.resolve();

    expect(view.shown).toEqual([]);
    expect(transport.closedSessions).toEqual([SESSION_ID]);
    controller.dispose();
  });

  it("uses the production exact-document predicate on a real query route transition", async () => {
    expect(isExpectedFixtureDocument(document)).toBe(true);
    const field = document.createElement("textarea");
    field.id = "exact-route-draft";
    field.value = "Stay";
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    const transport = new FakeTransport();
    const view = new RecordingView();
    const controller = new FieldController({
      transport,
      view,
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: EXPECTED_FIXTURE_ORIGIN,
      isCurrentDocument: () => isExpectedFixtureDocument(document),
    });
    controller.start();
    field.focus();
    await dispatchRequest();

    history.pushState({ route: "other" }, "", "/chromium.html?route=other");
    expect(isExpectedFixtureDocument(document)).toBe(false);
    window.dispatchEvent(new PopStateEvent("popstate"));
    transport.resolve(0, " must not display");
    await Promise.resolve();

    expect(view.shown).toEqual([]);
    expect(transport.closedSessions).toEqual([SESSION_ID]);
    controller.dispose();
  });

  it("rechecks exact-document authority after authorization and before mutation", async () => {
    const field = document.createElement("textarea");
    field.id = "replace-state-draft";
    field.value = "Stay";
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    let exactDocument = true;
    const transport = new FakeTransport();
    transport.autoAuthorize = false;
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view: new RecordingView(),
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
      isCurrentDocument: () => exactDocument,
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    transport.resolve(0, " blocked");
    await Promise.resolve();
    field.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }),
    );
    const pending = transport.deferredAuthorizations[0];
    if (pending === undefined) throw new Error("Authorization was not deferred");

    history.replaceState({ route: "other" }, "");
    exactDocument = false;
    pending.resolve(transport.authorizationFor(pending.request));
    await Promise.resolve();
    await Promise.resolve();

    expect(field.value).toBe("Stay");
    expect(transport.results.at(-1)?.status).toBe("stale");
    expect(transport.closedSessions).toContain(SESSION_ID);
    controller.dispose();
  });

  it("ignores page-authored untrusted accept and dismiss keyboard events", async () => {
    const field = document.createElement("textarea");
    field.id = "hostile-keyboard-draft";
    field.value = "Stable";
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    const transport = new FakeTransport();
    const view = new RecordingView();
    const controller = new FieldController({
      transport,
      view,
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
      now: () => 1_000,
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    transport.resolve(0, " safe", " safe");
    await Promise.resolve();

    for (const event of [
      new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }),
      new KeyboardEvent("keydown", {
        key: "ArrowRight",
        ctrlKey: true,
        bubbles: true,
        cancelable: true,
      }),
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }),
    ]) {
      expect(event.isTrusted).toBe(false);
      field.dispatchEvent(event);
      expect(event.defaultPrevented).toBe(false);
    }
    await Promise.resolve();

    expect(field.value).toBe("Stable");
    expect(view.visible).toBe(true);
    expect(transport.authorizationRequests).toHaveLength(0);
    expect(transport.dismissals).toHaveLength(0);
    controller.dispose();
  });

  it("waits for one broker authorization, blocks double-accept, and handles denial", async () => {
    const field = document.createElement("textarea");
    field.id = "authorization-draft";
    field.value = "Wait";
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    const transport = new FakeTransport();
    transport.autoAuthorize = false;
    const view = new RecordingView();
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view,
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    transport.resolve(0, " for permission");
    await Promise.resolve();

    const accept = (): KeyboardEvent => {
      const event = new KeyboardEvent("keydown", {
        key: "Tab",
        bubbles: true,
        cancelable: true,
      });
      field.dispatchEvent(event);
      return event;
    };
    expect(accept().defaultPrevented).toBe(true);
    // A second native shortcut is not consumed while the first authorization
    // is unresolved; no second wire request is created.
    expect(accept().defaultPrevented).toBe(false);
    expect(transport.authorizationRequests).toHaveLength(1);
    expect(field.value).toBe("Wait");

    const first = transport.deferredAuthorizations[0];
    if (first === undefined) throw new Error("Authorization was not deferred");
    first.resolve(transport.authorizationFor(first.request));
    await Promise.resolve();
    await Promise.resolve();
    expect(field.value).toBe("Wait for permission");
    expect(transport.results.at(-1)?.status).toBe("dispatched-unverified");

    setValue(field, "Denied");
    await dispatchRequest();
    transport.resolve(1, " insertion");
    await Promise.resolve();
    const deniedKey = accept();
    // Async MV3 denial is fail-closed: the trusted key was already consumed,
    // but no synthetic Tab or insertion is attempted.
    expect(deniedKey.defaultPrevented).toBe(true);
    const second = transport.deferredAuthorizations[1];
    if (second === undefined) throw new Error("Second authorization was not deferred");
    second.reject(new Error("policy paused"));
    await Promise.resolve();
    await Promise.resolve();
    expect(field.value).toBe("Denied");
    expect(view.visible).toBe(false);
    controller.dispose();
  });

  it("reports stale and never mutates when the DOM changes during authorization", async () => {
    const field = document.createElement("textarea");
    field.id = "authorization-race";
    field.value = "A";
    document.body.append(field);
    field.setSelectionRange(1, 1);
    const transport = new FakeTransport();
    transport.autoAuthorize = false;
    const view = new RecordingView();
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view,
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    transport.resolve(0, " authorized");
    await Promise.resolve();
    field.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Tab",
        bubbles: true,
        cancelable: true,
      }),
    );
    const pending = transport.deferredAuthorizations[0];
    if (pending === undefined) throw new Error("Authorization was not deferred");

    typeText(field, "x");
    pending.resolve(transport.authorizationFor(pending.request));
    await Promise.resolve();
    await Promise.resolve();
    expect(field.value).toBe("Ax");
    expect(field.value).not.toContain("authorized");
    expect(transport.results.at(-1)?.status).toBe("stale");
    controller.dispose();
  });

  it("honors a broker revocation that arrives before the authorized mutation", async () => {
    const field = document.createElement("textarea");
    field.id = "authorization-revocation";
    field.value = "Stay";
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    const transport = new FakeTransport();
    transport.autoAuthorize = false;
    const view = new RecordingView();
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view,
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    transport.resolve(0, " unchanged");
    await Promise.resolve();
    field.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Tab",
        bubbles: true,
        cancelable: true,
      }),
    );
    const pending = transport.deferredAuthorizations[0];
    if (pending === undefined) throw new Error("Authorization was not deferred");
    controller.revokeCommit(pending.request);
    pending.resolve(transport.authorizationFor(pending.request));
    await Promise.resolve();
    await Promise.resolve();

    expect(field.value).toBe("Stay");
    expect(view.visible).toBe(false);
    expect(transport.results.at(-1)?.status).toBe("stale");
    controller.dispose();
  });

  it("blocks an authorization whose exact accepted chunk diverges", async () => {
    const field = document.createElement("textarea");
    field.id = "authorization-mismatch";
    field.value = "Exact";
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    const transport = new FakeTransport();
    transport.autoAuthorize = false;
    const view = new RecordingView();
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view,
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    transport.resolve(0, " expected");
    await Promise.resolve();
    field.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Tab",
        bubbles: true,
        cancelable: true,
      }),
    );
    const pending = transport.deferredAuthorizations[0];
    if (pending === undefined) throw new Error("Authorization was not deferred");
    pending.resolve(transport.authorizationFor(pending.request, { text: " mismatch" }));
    await Promise.resolve();
    await Promise.resolve();

    expect(field.value).toBe("Exact");
    expect(transport.results.at(-1)?.status).toBe("blocked");
    controller.dispose();
  });

  it("suppresses non-collapsed ambient selections and impossible maxlength insertion", async () => {
    const field = document.createElement("textarea");
    field.id = "selection-contract";
    field.value = "replace me";
    document.body.append(field);
    field.setSelectionRange(0, 7);
    const transport = new FakeTransport();
    const view = new RecordingView();
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view,
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    expect(transport.requests).toHaveLength(0);

    field.setSelectionRange(field.value.length, field.value.length);
    field.dispatchEvent(new Event("select", { bubbles: true }));
    await dispatchRequest();
    expect(transport.requests).toHaveLength(1);
    transport.resolve(0, " too long");
    await Promise.resolve();
    field.maxLength = field.value.length + 2;
    const rejectedTab = new KeyboardEvent("keydown", {
      key: "Tab",
      bubbles: true,
      cancelable: true,
    });
    field.dispatchEvent(rejectedTab);
    await Promise.resolve();
    expect(rejectedTab.defaultPrevented).toBe(false);
    expect(transport.authorizationRequests).toHaveLength(0);
    expect(field.value).toBe("replace me");
    controller.dispose();
  });

  it("does not consume the word shortcut when local insertion constraints fail", async () => {
    const field = document.createElement("textarea");
    field.id = "word-constraint";
    field.value = "Exact";
    field.maxLength = field.value.length + 1;
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    const transport = new FakeTransport();
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view: new RecordingView(),
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    transport.resolve(0, " word remainder");
    await Promise.resolve();

    const rejectedWord = new KeyboardEvent("keydown", {
      key: "ArrowRight",
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    });
    field.dispatchEvent(rejectedWord);
    await Promise.resolve();

    expect(rejectedWord.defaultPrevented).toBe(false);
    expect(transport.authorizationRequests).toHaveLength(0);
    expect(field.value).toBe("Exact");
    controller.dispose();
  });

  it("reconciles exact Unicode type-through without flicker and clears contradictions", async () => {
    const field = document.createElement("textarea");
    field.id = "type-through";
    field.value = "A";
    document.body.append(field);
    field.setSelectionRange(1, 1);
    const transport = new FakeTransport();
    const view = new RecordingView();
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view,
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    transport.resolve(0, " 🙂über weiter", " 🙂");
    await Promise.resolve();
    expect(view.visible).toBe(true);
    const hidesBeforeTyping = view.visibleHideTransitions;

    typeText(field, " ");
    await Promise.resolve();
    expect(field.value).toBe("A ");
    expect(view.current).toBe("🙂über weiter");
    expect(view.visibleHideTransitions).toBe(hidesBeforeTyping);

    const original = transport.requests[0];
    if (original === undefined) throw new Error("Original request missing");
    controller.clearFromBroker({
      requestId: original.requestId,
      sessionId: original.sessionId,
      focusEpoch: original.focusEpoch,
      revision: original.revision,
      monotonicMs: 2_000,
      fingerprint: original.context.fingerprint,
      suggestionId: "suggestion-0",
      reason: "superseded",
    });
    expect(view.current).toBe("🙂über weiter");
    expect(view.visible).toBe(true);

    typeText(field, "🙂");
    await Promise.resolve();
    expect(field.value).toBe("A 🙂");
    expect(view.current).toBe("über weiter");
    expect(view.visibleHideTransitions).toBe(hidesBeforeTyping);

    const staleShortcut = new KeyboardEvent("keydown", {
      key: "Tab",
      bubbles: true,
      cancelable: true,
    });
    field.dispatchEvent(staleShortcut);
    expect(staleShortcut.defaultPrevented).toBe(false);
    expect(field.value).toBe("A 🙂");
    expect(view.visible).toBe(true);
    expect(transport.authorizationRequests).toHaveLength(0);

    const staleWordShortcut = new KeyboardEvent("keydown", {
      key: "ArrowRight",
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    });
    field.dispatchEvent(staleWordShortcut);
    expect(staleWordShortcut.defaultPrevented).toBe(false);
    expect(view.visible).toBe(true);
    expect(transport.authorizationRequests).toHaveLength(0);

    await dispatchRequest();
    transport.resolve(1, "über weiter", "über");
    await Promise.resolve();
    expect(view.current).toBe("über weiter");
    typeText(field, "x");
    expect(view.visible).toBe(false);
    expect(view.visibleHideTransitions).toBe(hidesBeforeTyping + 1);
    expect(field.value).toBe("A 🙂x");
    controller.dispose();
  });

  it("hides at the receiver-local TTL even when the page stays idle", async () => {
    vi.setSystemTime(1_000);
    const field = document.createElement("textarea");
    field.id = "ttl-draft";
    field.value = "Idle";
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    const transport = new FakeTransport();
    const view = new RecordingView();
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view,
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
      now: () => Date.now(),
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    transport.resolve(0, " expires", " expires", 20);
    await Promise.resolve();

    expect(view.visible).toBe(true);
    await vi.advanceTimersByTimeAsync(19);
    expect(view.visible).toBe(true);
    await vi.advanceTimersByTimeAsync(1);
    expect(view.visible).toBe(false);

    const afterExpiry = new KeyboardEvent("keydown", {
      key: "Tab",
      bubbles: true,
      cancelable: true,
    });
    field.dispatchEvent(afterExpiry);
    expect(afterExpiry.defaultPrevented).toBe(false);
    expect(transport.authorizationRequests).toHaveLength(0);
    controller.dispose();
  });

  it("immediately applies only a broker clear addressed to the visible session", async () => {
    const field = document.createElement("textarea");
    field.id = "broker-clear";
    field.value = "Visible";
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    const transport = new FakeTransport();
    const view = new RecordingView();
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view,
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    transport.resolve(0, " clear me", " clear");
    await Promise.resolve();
    const sent = transport.requests[0];
    if (sent === undefined) throw new Error("Suggestion request missing");

    controller.clearFromBroker({
      requestId: null,
      sessionId: "0198f215-3ec0-7000-8000-000000000099",
      focusEpoch: sent.focusEpoch,
      revision: sent.revision,
      monotonicMs: 2_000,
      fingerprint: sent.context.fingerprint,
      suggestionId: "suggestion-0",
      reason: "paused",
    });
    expect(view.visible).toBe(true);

    controller.clearFromBroker({
      requestId: null,
      sessionId: sent.sessionId,
      focusEpoch: sent.focusEpoch,
      revision: sent.revision,
      monotonicMs: 2_001,
      fingerprint: sent.context.fingerprint,
      suggestionId: null,
      reason: "expired",
    });
    expect(view.visible).toBe(false);
    controller.dispose();
  });

  it.each(SILENT_POLICY_MUTATIONS)(
    "clears before acceptance after a silent $name mutation",
    async ({ mutate, expectedRequestCount = 1 }) => {
      const ancestor = document.createElement("div");
      const field = document.createElement("input");
      field.id = "silently-mutated-field";
      field.type = "text";
      field.value = "Keep";
      ancestor.append(field);
      document.body.append(ancestor);
      field.setSelectionRange(field.value.length, field.value.length);
      const transport = new FakeTransport();
      const view = new RecordingView();
      const controller = new FieldController({
        allowUntrustedKeyboardForTesting: true,
        transport,
        view,
        debounceMs: 5,
        sessionId: SESSION_ID,
        idFactory: nextIdFactory(),
        origin: "https://fixture.test",
      });
      controller.start();
      field.focus();
      await dispatchRequest();
      transport.resolve(0, " blocked", " blocked");
      await Promise.resolve();
      expect(view.visible).toBe(true);

      mutate(field, ancestor);
      await deliverMutationObserver();
      await dispatchRequest(50);
      expect(view.visible).toBe(false);
      const accept = new KeyboardEvent("keydown", {
        key: "Tab",
        bubbles: true,
        cancelable: true,
      });
      field.dispatchEvent(accept);
      expect(accept.defaultPrevented).toBe(false);
      expect(field.value).toBe("Keep");
      expect(transport.requests).toHaveLength(expectedRequestCount);
      expect(transport.authorizationRequests).toHaveLength(0);
      controller.dispose();
    },
  );

  it("cancels a pending rebind when silent invalidation follows type-through", async () => {
    const ancestor = document.createElement("div");
    const field = document.createElement("textarea");
    field.id = "pending-invalidation";
    field.value = "A";
    ancestor.append(field);
    document.body.append(ancestor);
    field.setSelectionRange(1, 1);
    const transport = new FakeTransport();
    const view = new RecordingView();
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view,
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    transport.resolve(0, " next", " next");
    await Promise.resolve();
    typeText(field, " ");
    await dispatchRequest();
    expect(transport.requests).toHaveLength(2);
    expect(view.current).toBe("next");

    ancestor.setAttribute("aria-hidden", "true");
    await deliverMutationObserver();
    expect(view.visible).toBe(false);
    expect(transport.cancellations.map((request) => request.requestId)).toContain(
      transport.requests[1]?.requestId,
    );
    transport.resolve(1, " stale");
    await Promise.resolve();
    expect(view.visible).toBe(false);
    expect(field.value).toBe("A ");
    controller.dispose();
  });

  it("keeps irrelevant DOM mutations text-free and request-free", async () => {
    const field = document.createElement("textarea");
    field.id = "irrelevant-mutation";
    field.value = "Stable";
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    const transport = new FakeTransport();
    const view = new RecordingView();
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view,
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    transport.resolve(0, " visible", " visible");
    await Promise.resolve();

    const valueDescriptor = Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      "value",
    );
    if (valueDescriptor?.get === undefined || valueDescriptor.set === undefined) {
      throw new Error("Textarea value accessor missing");
    }
    let reads = 0;
    Object.defineProperty(field, "value", {
      configurable: true,
      get: () => {
        reads += 1;
        return valueDescriptor.get?.call(field) as string;
      },
      set: (value: string) => valueDescriptor.set?.call(field, value),
    });
    const irrelevant = document.createElement("div");
    irrelevant.className = "unrelated";
    document.body.append(irrelevant);
    await deliverMutationObserver();
    expect(reads).toBe(0);
    expect(transport.requests).toHaveLength(1);
    expect(view.visible).toBe(true);

    const duplicate = document.createElement("textarea");
    duplicate.id = field.id;
    document.body.append(duplicate);
    await deliverMutationObserver();
    expect(view.visible).toBe(false);
    expect(reads).toBe(0);
    controller.dispose();
  });

  it("re-adopts once after a benign observed mutation invalidates the field", async () => {
    const field = document.createElement("textarea");
    field.id = "benign-mutation";
    field.value = "Stable";
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    const transport = new FakeTransport();
    const controller = new FieldController({
      transport,
      view: new RecordingView(),
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    const first = transport.requests[0];
    if (first === undefined) throw new Error("Initial request missing");

    field.classList.add("layout-only-change");
    await deliverMutationObserver();
    await dispatchRequest();
    const recovered = transport.requests[1];
    if (recovered === undefined) throw new Error("Recovery request missing");

    expect(transport.closedSessions).toEqual([SESSION_ID]);
    expect(recovered.sessionId).toBe(first.sessionId);
    expect(recovered.revision).toBeGreaterThan(first.revision);
    await dispatchRequest(50);
    expect(transport.requests).toHaveLength(2);
    controller.dispose();
  });

  it("recovers when a focused denied field becomes eligible in a later task", async () => {
    const field = document.createElement("textarea");
    field.id = "restored-policy";
    field.value = "Stable";
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    const transport = new FakeTransport();
    const controller = new FieldController({
      transport,
      view: new RecordingView(),
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    expect(transport.requests).toHaveLength(1);

    field.readOnly = true;
    await deliverMutationObserver();
    await dispatchRequest();
    expect(transport.requests).toHaveLength(1);

    field.readOnly = false;
    await deliverMutationObserver();
    await dispatchRequest();
    expect(transport.requests).toHaveLength(2);
    expect(transport.closedSessions).toEqual([SESSION_ID]);
    controller.dispose();
  });

  it("clears and blocks acceptance when the document becomes hidden", async () => {
    const field = document.createElement("textarea");
    field.id = "hidden-document";
    field.value = "Stay";
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    const transport = new FakeTransport();
    const view = new RecordingView();
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view,
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    const originalVisibility = Object.getOwnPropertyDescriptor(
      document,
      "visibilityState",
    );
    controller.start();
    field.focus();
    await dispatchRequest();
    transport.resolve(0, " hidden", " hidden");
    await Promise.resolve();
    expect(view.visible).toBe(true);

    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: "hidden",
    });
    document.dispatchEvent(new Event("visibilitychange"));
    expect(view.visible).toBe(false);
    field.dispatchEvent(
      new InputEvent("input", {
        bubbles: true,
        inputType: "insertText",
        data: "x",
      }),
    );
    await dispatchRequest(50);
    expect(transport.requests).toHaveLength(1);
    const accept = new KeyboardEvent("keydown", {
      key: "Tab",
      bubbles: true,
      cancelable: true,
    });
    field.dispatchEvent(accept);
    expect(accept.defaultPrevented).toBe(false);
    expect(field.value).toBe("Stay");
    expect(transport.authorizationRequests).toHaveLength(0);
    controller.dispose();
    if (originalVisibility === undefined) {
      delete (document as { visibilityState?: string }).visibilityState;
    } else {
      Object.defineProperty(document, "visibilityState", originalVisibility);
    }
  });

  it("clears on native transport invalidation and re-adopts the eligible field once", async () => {
    const field = document.createElement("textarea");
    field.id = "native-disconnect";
    field.value = "Stay";
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    const transport = new FakeTransport();
    const view = new RecordingView();
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view,
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    transport.resolve(0, " disconnected", " disconnected");
    await Promise.resolve();
    controller.invalidateTransport();
    controller.invalidateTransport();

    expect(view.visible).toBe(false);
    const accept = new KeyboardEvent("keydown", {
      key: "Tab",
      bubbles: true,
      cancelable: true,
    });
    field.dispatchEvent(accept);
    expect(accept.defaultPrevented).toBe(false);
    expect(field.value).toBe("Stay");
    expect(transport.authorizationRequests).toHaveLength(0);
    await Promise.resolve();
    await dispatchRequest();
    expect(transport.requests).toHaveLength(2);
    await dispatchRequest(50);
    expect(transport.requests).toHaveLength(2);
    controller.dispose();
  });

  it.each(["compositionstart", "focusout"] as const)(
    "clears and blocks acceptance on post-display %s",
    async (eventType) => {
      const field = document.createElement("textarea");
      field.id = `post-display-${eventType}`;
      field.value = "Stay";
      document.body.append(field);
      field.setSelectionRange(field.value.length, field.value.length);
      const transport = new FakeTransport();
      const view = new RecordingView();
      const controller = new FieldController({
        allowUntrustedKeyboardForTesting: true,
        transport,
        view,
        debounceMs: 5,
        sessionId: SESSION_ID,
        idFactory: nextIdFactory(),
        origin: "https://fixture.test",
      });
      controller.start();
      field.focus();
      await dispatchRequest();
      transport.resolve(0, " blocked", " blocked");
      await Promise.resolve();
      expect(view.visible).toBe(true);

      field.dispatchEvent(
        eventType === "compositionstart"
          ? new CompositionEvent(eventType, { bubbles: true })
          : new FocusEvent(eventType, { bubbles: true }),
      );
      expect(view.visible).toBe(false);
      const accept = new KeyboardEvent("keydown", {
        key: "Tab",
        bubbles: true,
        cancelable: true,
      });
      field.dispatchEvent(accept);
      expect(accept.defaultPrevented).toBe(false);
      expect(field.value).toBe("Stay");
      expect(transport.authorizationRequests).toHaveLength(0);
      controller.dispose();
    },
  );

  it("does not retain composing state across blur and a fresh focus epoch", async () => {
    const field = document.createElement("textarea");
    field.id = "composition-refocus";
    field.value = "Before";
    const sink = document.createElement("button");
    document.body.append(field, sink);
    field.setSelectionRange(field.value.length, field.value.length);
    const transport = new FakeTransport();
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view: new RecordingView(),
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    expect(transport.requests).toHaveLength(1);

    field.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    sink.focus();
    field.focus();
    setValue(field, "After");
    await dispatchRequest();

    expect(transport.requests).toHaveLength(2);
    expect(transport.requests[1]?.context.field.composing).toBe(false);
    controller.dispose();
  });

  it("never carries a visible suggestion across DOM field identity replacement", async () => {
    const field = document.createElement("textarea");
    field.id = "identity-source";
    field.value = "original";
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    const transport = new FakeTransport();
    const view = new RecordingView();
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view,
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    transport.resolve(0, " stale identity");
    await Promise.resolve();
    expect(view.visible).toBe(true);

    const replacement = field.cloneNode() as HTMLTextAreaElement;
    replacement.value = field.value;
    field.replaceWith(replacement);
    await deliverMutationObserver();
    expect(view.visible).toBe(false);
    replacement.focus();
    replacement.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Tab",
        bubbles: true,
        cancelable: true,
      }),
    );
    expect(replacement.value).toBe("original");
    expect(view.visible).toBe(false);
    controller.dispose();
  });

  it("drops 100 delayed stale replies with zero stale display or insertion", async () => {
    const field = document.createElement("textarea");
    field.id = "race-draft";
    document.body.append(field);
    const transport = new FakeTransport();
    const view = new RecordingView();
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view,
      debounceMs: 1,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    field.focus();

    // 101 requests create exactly 100 superseded, still-resolving races.
    for (let index = 0; index <= 100; index += 1) {
      setValue(field, `draft ${index}`);
      await dispatchRequest(1);
    }
    expect(transport.requests).toHaveLength(101);
    expect(transport.cancellations).toHaveLength(100);

    for (let index = 99; index >= 0; index -= 1) {
      transport.resolve(index, ` STALE-${index}`);
      await Promise.resolve();
    }
    expect(view.shown).toEqual([]);
    expect(field.value).toBe("draft 100");

    transport.resolve(100, " CURRENT");
    await Promise.resolve();
    expect(view.shown).toEqual([" CURRENT"]);
    field.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Tab",
        bubbles: true,
        cancelable: true,
      }),
    );
    await Promise.resolve();
    await Promise.resolve();
    expect(field.value).toBe("draft 100 CURRENT");
    expect(field.value).not.toContain("STALE");
    expect(view.shown).toEqual([" CURRENT"]);
    controller.dispose();
  });

  it("mounts an isolated extension-owned ghost anchored to the active field", async () => {
    const field = document.createElement("input");
    field.id = "anchor-input";
    field.type = "text";
    field.value = "Anchor";
    field.getBoundingClientRect = () => new DOMRect(20, 30, 320, 40);
    document.body.append(field);
    field.setSelectionRange(field.value.length, field.value.length);
    const transport = new FakeTransport();
    const view = new AnchoredGhostView(document);
    const controller = new FieldController({
      allowUntrustedKeyboardForTesting: true,
      transport,
      view,
      debounceMs: 5,
      sessionId: SESSION_ID,
      idFactory: nextIdFactory(),
      origin: "https://fixture.test",
    });
    controller.start();
    field.focus();
    await dispatchRequest();
    transport.resolve(0, " point");
    await Promise.resolve();

    const host = document.querySelector<HTMLElement>("[data-badi-owned]");
    expect(host).not.toBeNull();
    expect(host?.hidden).toBe(false);
    expect(host?.style.left).toBe("20px");
    expect(host?.style.top).toBe("76px");
    expect(host?.shadowRoot).toBeNull();
    if (host === null) throw new Error("Ghost host missing");
    host.style.opacity = "0";
    const acceptWhileInvisible = new KeyboardEvent("keydown", {
      key: "Tab",
      bubbles: true,
      cancelable: true,
    });
    field.dispatchEvent(acceptWhileInvisible);
    expect(acceptWhileInvisible.defaultPrevented).toBe(false);
    expect(transport.authorizationRequests).toHaveLength(0);
    expect(field.value).toBe("Anchor");
    host.style.opacity = "";
    host.style.filter = "opacity(0)";
    const acceptWhileFiltered = new KeyboardEvent("keydown", {
      key: "Tab",
      bubbles: true,
      cancelable: true,
    });
    field.dispatchEvent(acceptWhileFiltered);
    expect(acceptWhileFiltered.defaultPrevented).toBe(false);
    expect(transport.authorizationRequests).toHaveLength(0);
    expect(field.value).toBe("Anchor");
    host.style.filter = "";
    host.style.height = "1px";
    host.style.overflow = "hidden";
    const acceptWhileClipped = new KeyboardEvent("keydown", {
      key: "Tab",
      bubbles: true,
      cancelable: true,
    });
    field.dispatchEvent(acceptWhileClipped);
    expect(acceptWhileClipped.defaultPrevented).toBe(false);
    expect(transport.authorizationRequests).toHaveLength(0);
    expect(field.value).toBe("Anchor");
    host.style.height = "";
    host.style.overflow = "";
    host?.remove();
    const acceptAfterRemoval = new KeyboardEvent("keydown", {
      key: "Tab",
      bubbles: true,
      cancelable: true,
    });
    field.dispatchEvent(acceptAfterRemoval);
    expect(acceptAfterRemoval.defaultPrevented).toBe(false);
    expect(field.value).toBe("Anchor");
    controller.dispose();
  });
});
