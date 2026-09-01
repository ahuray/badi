// @vitest-environment-options {"url":"https://dillinger.io/"}

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  CommitAuthorization,
  CommitAuthorizationRequest,
  SuggestionRequest,
  SuggestionResponse,
  SuggestionTransport,
} from "../src/shared/model";
import { MonacoController } from "../src/product/monaco-controller";
import type { MonacoBridge } from "../src/product/monaco-runtime-bridge";
import type { MonacoSnapshot } from "../src/product/monaco-main-world";
import type { MonacoSuggestionView } from "../src/product/monaco-view";

const SNAPSHOT: MonacoSnapshot = {
  modelUri: "inmemory://model/1",
  languageId: "markdown",
  versionId: 7,
  valueLength: 9,
  offset: 9,
  lineNumber: 1,
  column: 10,
  before: "thank you",
  after: "",
  geometry: { left: 120, top: 80, height: 18 },
};

interface Harness {
  readonly bridge: MonacoBridge & {
    snapshot: ReturnType<typeof vi.fn>;
    apply: ReturnType<typeof vi.fn>;
  };
  readonly transport: SuggestionTransport & {
    requestSuggestion: ReturnType<typeof vi.fn>;
    cancelSuggestion: ReturnType<typeof vi.fn>;
    authorizeCommit: ReturnType<typeof vi.fn>;
    reportCommit: ReturnType<typeof vi.fn>;
    closeSession: ReturnType<typeof vi.fn>;
  };
  readonly view: MonacoSuggestionView & {
    show: ReturnType<typeof vi.fn>;
    hide: ReturnType<typeof vi.fn>;
    dispose: ReturnType<typeof vi.fn>;
  };
}

function responseFor(request: SuggestionRequest): SuggestionResponse {
  return {
    requestId: request.requestId,
    sessionId: request.sessionId,
    focusEpoch: request.focusEpoch,
    revision: request.revision,
    fingerprint: request.context.fingerprint,
    suggestion: " for your time",
    suggestionId: "suggestion-a",
    acceptWord: " for",
    ttlMs: 500,
  };
}

function authorizationFor(request: CommitAuthorizationRequest): CommitAuthorization {
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
  };
}

function harness(): Harness {
  let visible = false;
  const view = {
    get visible() {
      return visible;
    },
    show: vi.fn(() => {
      visible = true;
    }),
    hide: vi.fn(() => {
      visible = false;
    }),
    dispose: vi.fn(() => {
      visible = false;
    }),
  };
  return {
    bridge: {
      snapshot: vi.fn().mockResolvedValue(SNAPSHOT),
      apply: vi.fn().mockResolvedValue(true),
    },
    transport: {
      requestSuggestion: vi.fn(async (request: SuggestionRequest) => responseFor(request)),
      cancelSuggestion: vi.fn().mockResolvedValue(undefined),
      closeSession: vi.fn().mockResolvedValue(undefined),
      dismissSuggestion: vi.fn().mockResolvedValue(undefined),
      authorizeCommit: vi.fn(
        async (request: CommitAuthorizationRequest) => authorizationFor(request),
      ),
      reportCommit: vi.fn().mockResolvedValue(undefined),
      dispose: vi.fn(),
    },
    view,
  };
}

function controllerFor(
  state: Harness,
  isCurrentDocument: () => boolean = () => true,
  sessionId = "session-a",
): MonacoController {
  return new MonacoController({
    transport: state.transport,
    bridge: state.bridge,
    view: state.view,
    document,
    sessionId,
    origin: "https://dillinger.io",
    isCurrentDocument,
    debounceMs: 0,
    now: () => 100,
    idFactory: () => "request-a",
  });
}

async function settle(): Promise<void> {
  for (let index = 0; index < 8; index += 1) await Promise.resolve();
}

async function showSuggestion(controller: MonacoController): Promise<void> {
  controller.start();
  controller.resume();
  await vi.advanceTimersByTimeAsync(0);
  await settle();
  expect(controller.suggestionVisible).toBe(true);
}

describe("Dillinger Monaco controller", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  it("previews and applies one broker-authorized target-native transaction", async () => {
    const state = harness();
    const controller = controllerFor(state);
    await showSuggestion(controller);

    expect(state.view.show).toHaveBeenCalledWith(" for your time", SNAPSHOT.geometry);
    const suggestionRequest = state.transport.requestSuggestion.mock.calls[0]?.[0] as SuggestionRequest;
    expect(suggestionRequest.origin).toBe("https://dillinger.io");
    expect(suggestionRequest.context.before).toBe("thank you");
    expect(suggestionRequest.context.language).toBe("en");
    expect(suggestionRequest.context.fingerprint).toMatch(/^[a-f0-9]{32}$/u);

    expect(controller.acceptAll()).toBe(true);
    await settle();
    expect(state.bridge.apply).toHaveBeenCalledWith(
      "session-a",
      SNAPSHOT,
      expect.objectContaining({ text: " for your time", acceptance: "all" }),
    );
    expect(state.transport.reportCommit).toHaveBeenCalledWith(
      expect.objectContaining({ status: "applied", suggestionId: "suggestion-a" }),
    );
    expect(controller.suggestionVisible).toBe(false);
    controller.dispose();
  });

  it("keeps repeated acceptance idempotent while authorization is pending", async () => {
    const state = harness();
    let resolveAuthorization!: (authorization: CommitAuthorization) => void;
    state.transport.authorizeCommit.mockImplementation(
      (request: CommitAuthorizationRequest) =>
        new Promise<CommitAuthorization>((resolve) => {
          resolveAuthorization = () => resolve(authorizationFor(request));
        }),
    );
    const controller = controllerFor(state);
    await showSuggestion(controller);
    expect(controller.acceptAll()).toBe(true);
    expect(state.transport.authorizeCommit).toHaveBeenCalledTimes(1);
    expect(controller.acceptAll()).toBe(true);
    expect(state.transport.authorizeCommit).toHaveBeenCalledTimes(1);
    expect(controller.suggestionVisible).toBe(true);

    resolveAuthorization(authorizationFor(
      state.transport.authorizeCommit.mock.calls[0]?.[0] as CommitAuthorizationRequest,
    ));
    await settle();

    expect(state.bridge.apply).toHaveBeenCalledTimes(1);
    expect(state.transport.reportCommit).toHaveBeenCalledTimes(1);
    expect(state.transport.reportCommit).toHaveBeenCalledWith(
      expect.objectContaining({ status: "applied", suggestionId: "suggestion-a" }),
    );
    controller.dispose();
  });

  it("salts protocol-valid fingerprints per session", async () => {
    const first = harness();
    const second = harness();
    const firstController = controllerFor(first, () => true, "session-a");
    const secondController = controllerFor(second, () => true, "session-b");
    await showSuggestion(firstController);
    await showSuggestion(secondController);

    const firstRequest = first.transport.requestSuggestion.mock.calls[0]?.[0] as SuggestionRequest;
    const secondRequest = second.transport.requestSuggestion.mock.calls[0]?.[0] as SuggestionRequest;
    expect(firstRequest.context.fingerprint).toMatch(/^[a-f0-9]{32}$/u);
    expect(secondRequest.context.fingerprint).toMatch(/^[a-f0-9]{32}$/u);
    expect(secondRequest.context.fingerprint).not.toBe(firstRequest.context.fingerprint);

    firstController.dispose();
    secondController.dispose();
  });

  it("does not mutate when the exact document becomes stale during authorization", async () => {
    const state = harness();
    let current = true;
    let resolveAuthorization!: (authorization: CommitAuthorization) => void;
    state.transport.authorizeCommit.mockImplementation(
      (request: CommitAuthorizationRequest) =>
        new Promise<CommitAuthorization>((resolve) => {
          resolveAuthorization = () => resolve(authorizationFor(request));
        }),
    );
    const controller = controllerFor(state, () => current);
    await showSuggestion(controller);

    expect(controller.acceptAll()).toBe(true);
    current = false;
    resolveAuthorization(authorizationFor(
      state.transport.authorizeCommit.mock.calls[0]?.[0] as CommitAuthorizationRequest,
    ));
    await settle();

    expect(state.bridge.apply).not.toHaveBeenCalled();
    expect(state.transport.reportCommit).toHaveBeenCalledWith(
      expect.objectContaining({ status: "stale" }),
    );
    expect(controller.suggestionVisible).toBe(false);
    controller.dispose();
  });

  it("clears an exact revoked suggestion before acceptance", async () => {
    const state = harness();
    const controller = controllerFor(state);
    await showSuggestion(controller);
    const request = state.transport.requestSuggestion.mock.calls[0]?.[0] as SuggestionRequest;
    controller.revokeCommit({
      requestId: request.requestId,
      sessionId: request.sessionId,
      focusEpoch: request.focusEpoch,
      revision: request.revision,
      monotonicMs: 100,
      fingerprint: request.context.fingerprint,
      suggestionId: "suggestion-a",
    });
    expect(controller.acceptAll()).toBe(false);

    expect(controller.suggestionVisible).toBe(false);
    expect(state.transport.authorizeCommit).not.toHaveBeenCalled();
    expect(state.bridge.apply).not.toHaveBeenCalled();
    controller.dispose();
  });

  it("does not arm acceptance when the preview cannot prove visibility", async () => {
    const state = harness();
    state.view.show.mockImplementation(() => undefined);
    const controller = controllerFor(state);
    controller.start();
    controller.resume();
    await vi.advanceTimersByTimeAsync(0);
    await settle();

    expect(state.view.show).toHaveBeenCalledTimes(1);
    expect(controller.suggestionVisible).toBe(false);
    expect(controller.acceptAll()).toBe(false);
    expect(state.transport.authorizeCommit).not.toHaveBeenCalled();
    expect(state.bridge.apply).not.toHaveBeenCalled();
    controller.dispose();
  });

  it("reports an already-linearized delayed apply honestly after local revocation", async () => {
    const state = harness();
    let resolveApply!: (applied: boolean) => void;
    state.bridge.apply.mockImplementation(
      () =>
        new Promise<boolean>((resolve) => {
          resolveApply = resolve;
        }),
    );
    const controller = controllerFor(state);
    await showSuggestion(controller);
    const request = state.transport.requestSuggestion.mock.calls[0]?.[0] as SuggestionRequest;

    controller.acceptAll();
    await settle();
    expect(state.bridge.apply).toHaveBeenCalledTimes(1);
    controller.revokeCommit({
      requestId: request.requestId,
      sessionId: request.sessionId,
      focusEpoch: request.focusEpoch,
      revision: request.revision,
      monotonicMs: 100,
      fingerprint: request.context.fingerprint,
      suggestionId: "suggestion-a",
    });
    resolveApply(true);
    await settle();

    expect(state.transport.reportCommit).toHaveBeenCalledWith(
      expect.objectContaining({ status: "applied", suggestionId: "suggestion-a" }),
    );
    expect(controller.suggestionVisible).toBe(false);
    controller.dispose();
  });

  it("cancels pending work and removes preview state on cleanup", async () => {
    const state = harness();
    let resolveSuggestion!: (response: SuggestionResponse) => void;
    state.transport.requestSuggestion.mockImplementation(
      () =>
        new Promise<SuggestionResponse>((resolve) => {
          resolveSuggestion = resolve;
        }),
    );
    const controller = controllerFor(state);
    controller.start();
    controller.resume();
    await vi.advanceTimersByTimeAsync(0);
    await settle();
    const request = state.transport.requestSuggestion.mock.calls[0]?.[0] as SuggestionRequest;

    controller.dispose(true);
    resolveSuggestion(responseFor(request));
    await settle();

    expect(state.transport.cancelSuggestion).toHaveBeenCalledWith(request);
    expect(state.transport.closeSession).toHaveBeenCalledWith("session-a");
    expect(state.view.dispose).toHaveBeenCalledTimes(1);
    expect(state.view.show).not.toHaveBeenCalled();
    expect(controller.suggestionVisible).toBe(false);
  });
});
