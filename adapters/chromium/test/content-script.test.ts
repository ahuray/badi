// @vitest-environment-options {"url":"http://localhost:4173/chromium.html"}

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BootstrapState, TargetPolicy } from "../src/shared/model";

interface MockController {
  readonly dispose: ReturnType<typeof vi.fn>;
  readonly invalidateTransport: ReturnType<typeof vi.fn>;
  readonly pause: ReturnType<typeof vi.fn>;
  readonly resume: ReturnType<typeof vi.fn>;
  readonly start: ReturnType<typeof vi.fn>;
}

const harness = vi.hoisted(() => ({
  bootstrap: vi.fn<(sessionId: string) => Promise<BootstrapState>>(),
  controllerOptions: [] as Array<Record<string, unknown>>,
  controllers: [] as MockController[],
  disconnectListeners: [] as Array<() => void>,
}));

vi.mock("../src/content/runtime-transport", () => ({
  RuntimeSuggestionTransport: class MockRuntimeSuggestionTransport {
    bootstrap(sessionId: string): Promise<BootstrapState> {
      return harness.bootstrap(sessionId);
    }
  },
}));

vi.mock("../src/content/field-controller", () => ({
  FieldController: class MockFieldController implements MockController {
    readonly pause = vi.fn();
    readonly resume = vi.fn();
    readonly start = vi.fn();
    readonly revokeCommit = vi.fn();
    readonly clearFromBroker = vi.fn();
    readonly invalidateTransport = vi.fn();
    readonly acceptWord = vi.fn();
    readonly acceptAll = vi.fn();
    readonly dismiss = vi.fn();
    readonly dispose = vi.fn();

    constructor(options: Record<string, unknown>) {
      harness.controllerOptions.push(options);
      harness.controllers.push(this);
    }
  },
}));

type ContentListener = (
  message: unknown,
  sender?: unknown,
  sendResponse?: (response: unknown) => void,
) => boolean | void;

function targetPolicy(
  paused: boolean,
  overrides: Partial<TargetPolicy> = {},
): TargetPolicy {
  return {
    authorityEpoch: 4,
    settingsRevision: 2,
    paused,
    activation: "always",
    contextAllowed: true,
    displayAllowed: true,
    suggestionsAllowed: true,
    learningAllowed: false,
    reason: "matched_rule",
    ...overrides,
  };
}

function bootstrapState(
  paused: boolean,
  overrides: Partial<TargetPolicy> = {},
): BootstrapState {
  return { paused, policy: targetPolicy(paused, overrides) };
}

function installChromeRuntime(): ContentListener[] {
  const listeners: ContentListener[] = [];
  Object.defineProperty(globalThis, "chrome", {
    configurable: true,
    value: {
      runtime: {
        connect(): object {
          return {
            onDisconnect: {
              addListener(listener: () => void): void {
                harness.disconnectListeners.push(listener);
              },
            },
          };
        },
        onMessage: {
          addListener(listener: ContentListener): void {
            listeners.push(listener);
          },
        },
      },
    },
  });
  return listeners;
}

async function flushPromises(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
}

describe("content-script bootstrap", () => {
  beforeEach(() => {
    vi.resetModules();
    harness.bootstrap.mockReset();
    harness.controllerOptions.length = 0;
    harness.controllers.length = 0;
    harness.disconnectListeners.length = 0;
    history.replaceState(null, "", "/chromium.html");
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    delete (globalThis as { chrome?: unknown }).chrome;
  });

  it("does not let an unversioned resume revive a bootstrap fenced by authority", async () => {
    let resolveBootstrap!: (state: BootstrapState) => void;
    harness.bootstrap.mockImplementation(
      () =>
        new Promise<BootstrapState>((resolve) => {
          resolveBootstrap = resolve;
        }),
    );
    const listeners = installChromeRuntime();

    await import("../src/content/content-script");
    expect(harness.bootstrap).toHaveBeenCalledTimes(1);
    const sessionId = harness.bootstrap.mock.calls[0]?.[0];
    expect(sessionId).toEqual(expect.any(String));
    expect(sessionId).not.toBe("");
    const listener = listeners[0];
    if (listener === undefined) throw new Error("Bootstrap listener missing");

    listener({ kind: "badi.control.v1", action: "pause" });
    listener({ kind: "badi.control.v1", action: "resume" });
    resolveBootstrap(bootstrapState(false));
    await flushPromises();

    expect(harness.controllerOptions).toHaveLength(0);
    listener({
      kind: "badi.policy.v1",
      policy: targetPolicy(false, { authorityEpoch: 5 }),
    });
    expect(harness.controllerOptions).toHaveLength(1);
    expect(harness.controllerOptions[0]?.["sessionId"]).toBe(sessionId);
    const controller = harness.controllers[0];
    if (controller === undefined) throw new Error("Controller was not constructed");
    expect(controller.pause).not.toHaveBeenCalled();
    expect(controller.resume).toHaveBeenCalledTimes(1);
    expect(controller.start).toHaveBeenCalledTimes(1);

    listener({ kind: "badi.control.v1", action: "pause" });
    expect(controller.pause).toHaveBeenCalledTimes(1);
    expect(controller.dispose).toHaveBeenCalledTimes(1);
    listener({ kind: "badi.control.v1", action: "resume" });
    expect(harness.controllers).toHaveLength(1);
  });

  it("retains denied bootstrap without observing fields, then starts after a newer allow", async () => {
    let resolveBootstrap!: (state: BootstrapState) => void;
    harness.bootstrap.mockImplementation(
      () =>
        new Promise<BootstrapState>((resolve) => {
          resolveBootstrap = resolve;
        }),
    );
    const listeners = installChromeRuntime();

    await import("../src/content/content-script");
    history.pushState(null, "", "/chromium.html?ineligible=1");
    resolveBootstrap(bootstrapState(true));
    await flushPromises();
    expect(harness.controllers).toHaveLength(0);

    history.replaceState(null, "", "/chromium.html");
    window.dispatchEvent(new FocusEvent("focus"));
    await flushPromises();
    const listener = listeners[0];
    if (listener === undefined) throw new Error("Content listener missing");
    expect(harness.controllers).toHaveLength(0);

    listener({
      kind: "badi.policy.v1",
      policy: targetPolicy(false, { authorityEpoch: 5 }),
    });
    const controller = harness.controllers[0];
    if (controller === undefined) throw new Error("Controller was not authorized");
    expect(controller.resume).toHaveBeenCalledTimes(1);
    expect(controller.start).toHaveBeenCalledTimes(1);
  });

  it("retries only on later eligible events and stops after the bounded attempts", async () => {
    harness.bootstrap.mockRejectedValue(new Error("transient bootstrap failure"));
    installChromeRuntime();

    await import("../src/content/content-script");
    await flushPromises();
    expect(harness.bootstrap).toHaveBeenCalledTimes(1);

    history.pushState(null, "", "/chromium.html?ineligible=1");
    window.dispatchEvent(new FocusEvent("focus"));
    await flushPromises();
    expect(harness.bootstrap).toHaveBeenCalledTimes(1);

    history.replaceState(null, "", "/chromium.html");
    for (let attempt = 0; attempt < 5; attempt += 1) {
      window.dispatchEvent(new FocusEvent("focus"));
      await flushPromises();
    }
    expect(harness.bootstrap).toHaveBeenCalledTimes(3);
    expect(harness.controllers).toHaveLength(0);
  });

  it("recovers a transient bootstrap failure on a later eligible focus", async () => {
    harness.bootstrap
      .mockRejectedValueOnce(new Error("transient bootstrap failure"))
      .mockResolvedValueOnce(bootstrapState(false));
    installChromeRuntime();

    await import("../src/content/content-script");
    await flushPromises();
    expect(harness.bootstrap).toHaveBeenCalledTimes(1);
    expect(harness.controllers).toHaveLength(0);

    history.pushState(null, "", "/chromium.html?ineligible=1");
    window.dispatchEvent(new FocusEvent("focus"));
    await flushPromises();
    expect(harness.bootstrap).toHaveBeenCalledTimes(1);

    history.replaceState(null, "", "/chromium.html");
    window.dispatchEvent(new FocusEvent("focus"));
    await flushPromises();
    expect(harness.bootstrap).toHaveBeenCalledTimes(2);
    expect(harness.controllers).toHaveLength(1);
  });

  it("pauses, disposes, and re-bootstraps after the MV3 lifetime port disconnects", async () => {
    harness.bootstrap.mockResolvedValue(bootstrapState(false));
    installChromeRuntime();

    await import("../src/content/content-script");
    await flushPromises();
    const first = harness.controllers[0];
    const disconnected = harness.disconnectListeners[0];
    if (first === undefined || disconnected === undefined) {
      throw new Error("Controller or lifetime listener missing");
    }

    disconnected();
    await flushPromises();
    expect(first.pause).toHaveBeenCalledTimes(1);
    expect(first.dispose).toHaveBeenCalledTimes(1);
    expect(harness.bootstrap).toHaveBeenCalledTimes(2);
    expect(harness.controllers).toHaveLength(2);
  });

  it("drops prior authority and bootstraps a fresh generation after native disconnect", async () => {
    harness.bootstrap.mockResolvedValue(bootstrapState(false));
    const listeners = installChromeRuntime();

    await import("../src/content/content-script");
    await flushPromises();
    const first = harness.controllers[0];
    const listener = listeners[0];
    if (first === undefined || listener === undefined) {
      throw new Error("Controller or content listener missing");
    }

    listener({ kind: "badi.transport.disconnected.v1" });
    await flushPromises();

    expect(first.pause).toHaveBeenCalledTimes(1);
    expect(first.dispose).toHaveBeenCalledTimes(1);
    expect(first.invalidateTransport).toHaveBeenCalledTimes(1);
    expect(harness.bootstrap).toHaveBeenCalledTimes(2);
    expect(harness.controllers).toHaveLength(2);
  });
});
