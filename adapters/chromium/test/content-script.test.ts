// @vitest-environment-options {"url":"http://localhost:4173/chromium.html"}

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

interface MockController {
  readonly pause: ReturnType<typeof vi.fn>;
  readonly resume: ReturnType<typeof vi.fn>;
  readonly start: ReturnType<typeof vi.fn>;
}

const harness = vi.hoisted(() => ({
  bootstrap: vi.fn<(sessionId: string) => Promise<boolean>>(),
  controllerOptions: [] as Array<Record<string, unknown>>,
  controllers: [] as MockController[],
}));

vi.mock("../src/content/runtime-transport", () => ({
  RuntimeSuggestionTransport: class MockRuntimeSuggestionTransport {
    bootstrap(sessionId: string): Promise<boolean> {
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

    constructor(options: Record<string, unknown>) {
      harness.controllerOptions.push(options);
      harness.controllers.push(this);
    }
  },
}));

type ContentListener = (message: unknown) => void;

function installChromeRuntime(): ContentListener[] {
  const listeners: ContentListener[] = [];
  Object.defineProperty(globalThis, "chrome", {
    configurable: true,
    value: {
      runtime: {
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
    history.replaceState(null, "", "/chromium.html");
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    delete (globalThis as { chrome?: unknown }).chrome;
  });

  it("bootstraps and constructs the controller with one ID, resolving queued controls", async () => {
    let resolveBootstrap!: (paused: boolean) => void;
    harness.bootstrap.mockImplementation(
      () =>
        new Promise<boolean>((resolve) => {
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
    resolveBootstrap(true);
    await flushPromises();

    expect(harness.controllerOptions).toHaveLength(1);
    expect(harness.controllerOptions[0]?.["sessionId"]).toBe(sessionId);
    const controller = harness.controllers[0];
    if (controller === undefined) throw new Error("Controller was not constructed");
    expect(controller.pause).not.toHaveBeenCalled();
    expect(controller.start).toHaveBeenCalledTimes(1);

    listener({ kind: "badi.control.v1", action: "pause" });
    listener({ kind: "badi.control.v1", action: "resume" });
    expect(controller.pause).toHaveBeenCalledTimes(1);
    expect(controller.resume).toHaveBeenCalledTimes(1);
  });

  it("retains a completed bootstrap until the exact route returns, then resumes", async () => {
    let resolveBootstrap!: (paused: boolean) => void;
    harness.bootstrap.mockImplementation(
      () =>
        new Promise<boolean>((resolve) => {
          resolveBootstrap = resolve;
        }),
    );
    const listeners = installChromeRuntime();

    await import("../src/content/content-script");
    history.pushState(null, "", "/chromium.html?ineligible=1");
    resolveBootstrap(true);
    await flushPromises();
    expect(harness.controllers).toHaveLength(0);

    history.replaceState(null, "", "/chromium.html");
    window.dispatchEvent(new FocusEvent("focus"));
    await flushPromises();
    const controller = harness.controllers[0];
    const listener = listeners[0];
    if (controller === undefined || listener === undefined) {
      throw new Error("Controller or content listener missing");
    }
    expect(controller.pause).toHaveBeenCalledTimes(1);
    const pauseOrder = controller.pause.mock.invocationCallOrder[0];
    const startOrder = controller.start.mock.invocationCallOrder[0];
    if (pauseOrder === undefined || startOrder === undefined) {
      throw new Error("Controller startup calls missing");
    }
    expect(pauseOrder).toBeLessThan(startOrder);

    listener({ kind: "badi.control.v1", action: "resume" });
    expect(controller.resume).toHaveBeenCalledTimes(1);
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
      .mockResolvedValueOnce(false);
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
});
