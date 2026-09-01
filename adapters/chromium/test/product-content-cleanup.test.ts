// @vitest-environment-options {"url":"https://dillinger.io/"}

import { afterEach, describe, expect, it, vi } from "vitest";

describe("product content-script cleanup", () => {
  afterEach(() => {
    delete (globalThis as Record<string, unknown>)["chrome"];
    delete (globalThis as Record<string, unknown>)["__BADI_DILLINGER_PRODUCT_V1__"];
    vi.restoreAllMocks();
    vi.resetModules();
  });

  it("detaches its runtime and document listeners before same-document regrant", async () => {
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    const windowRemove = vi.spyOn(window, "removeEventListener");
    const documentAdd = vi.spyOn(document, "addEventListener");
    const documentRemove = vi.spyOn(document, "removeEventListener");
    type RuntimeListener = (
      message: unknown,
      sender: chrome.runtime.MessageSender,
      sendResponse: (response?: unknown) => void,
    ) => boolean;
    let runtimeListener: RuntimeListener | null = null;
    const removeRuntimeListener = vi.fn((listener) => {
      if (runtimeListener === listener) runtimeListener = null;
    });
    const port = {
      onDisconnect: { addListener: vi.fn() },
      disconnect: vi.fn(),
    };
    let resolveBootstrap!: (value: unknown) => void;
    const bootstrapReply = new Promise<unknown>((resolve) => {
      resolveBootstrap = resolve;
    });
    const sendMessage = vi.fn(() => bootstrapReply);
    Object.defineProperty(globalThis, "chrome", {
      configurable: true,
      value: {
        runtime: {
          id: "ckkiehcjbclcjckkkajohopoikeejkoa",
          connect: vi.fn(() => port),
          sendMessage,
          onMessage: {
            addListener: vi.fn((listener) => {
              runtimeListener = listener;
            }),
            removeListener: removeRuntimeListener,
          },
        },
      },
    });

    await import("../src/product/content-script");
    const installedListener = runtimeListener as unknown as RuntimeListener;
    expect(installedListener).not.toBeNull();
    const respond = vi.fn();
    installedListener(
      { kind: "badi.product.disable.v1" },
      { id: "ckkiehcjbclcjckkkajohopoikeejkoa" },
      respond,
    );

    expect(respond).toHaveBeenCalledWith({ applied: true });
    expect(removeRuntimeListener).toHaveBeenCalledWith(installedListener);
    expect(runtimeListener).toBeNull();
    for (const event of ["focus", "pageshow", "popstate", "hashchange", "blur", "pagehide"]) {
      expect(windowRemove).toHaveBeenCalledWith(event, expect.any(Function), true);
    }
    expect(documentRemove).toHaveBeenCalledWith(
      "visibilitychange",
      expect.any(Function),
      true,
    );
    expect(
      (globalThis as Record<string, unknown>)["__BADI_DILLINGER_PRODUCT_V1__"],
    ).toBe(false);

    resolveBootstrap({
      ok: true,
      paused: false,
      policy: {
        authorityEpoch: 4,
        settingsRevision: 2,
        paused: false,
        activation: "always",
        contextAllowed: true,
        displayAllowed: true,
        suggestionsAllowed: true,
        learningAllowed: false,
        reason: "matched_rule",
      },
    });
    await Promise.resolve();
    await Promise.resolve();
    expect(sendMessage).toHaveBeenCalledOnce();
    expect(documentAdd).not.toHaveBeenCalledWith("input", expect.any(Function), true);
  });

  it("retries a transient bootstrap failure and recovers with a bounded backoff", async () => {
    vi.useFakeTimers();
    try {
      vi.spyOn(document, "hasFocus").mockReturnValue(true);
      vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
      const warning = vi.spyOn(console, "warn").mockImplementation(() => undefined);
      const sendMessage = vi
        .fn()
        .mockRejectedValueOnce(new Error("Dillinger bootstrap route was displaced"))
        .mockResolvedValue({
          ok: true,
          paused: true,
          policy: {
            authorityEpoch: 4,
            settingsRevision: 2,
            paused: true,
            activation: "never",
            contextAllowed: false,
            displayAllowed: false,
            suggestionsAllowed: false,
            learningAllowed: false,
            reason: "global_disabled",
          },
        });
      const port = {
        onDisconnect: { addListener: vi.fn() },
        disconnect: vi.fn(),
      };
      Object.defineProperty(globalThis, "chrome", {
        configurable: true,
        value: {
          runtime: {
            id: "ckkiehcjbclcjckkkajohopoikeejkoa",
            connect: vi.fn(() => port),
            sendMessage,
            onMessage: { addListener: vi.fn(), removeListener: vi.fn() },
          },
        },
      });

      await import("../src/product/content-script");
      await vi.advanceTimersByTimeAsync(0);
      expect(sendMessage).toHaveBeenCalledTimes(1);
      expect(warning).toHaveBeenCalledWith(
        expect.stringContaining("retry 2/5 scheduled in 100 ms"),
      );

      await vi.advanceTimersByTimeAsync(100);
      expect(sendMessage).toHaveBeenCalledTimes(2);
      expect(sendMessage.mock.calls[1]?.[0]).toMatchObject({ kind: "badi.bootstrap.v1" });
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("stops retrying after five consecutive bootstrap failures", async () => {
    vi.useFakeTimers();
    try {
      vi.spyOn(document, "hasFocus").mockReturnValue(true);
      vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
      vi.spyOn(console, "warn").mockImplementation(() => undefined);
      const terminal = vi.spyOn(console, "error").mockImplementation(() => undefined);
      const sendMessage = vi.fn().mockRejectedValue(new Error("native host unavailable"));
      const port = {
        onDisconnect: { addListener: vi.fn() },
        disconnect: vi.fn(),
      };
      Object.defineProperty(globalThis, "chrome", {
        configurable: true,
        value: {
          runtime: {
            id: "ckkiehcjbclcjckkkajohopoikeejkoa",
            connect: vi.fn(() => port),
            sendMessage,
            onMessage: { addListener: vi.fn(), removeListener: vi.fn() },
          },
        },
      });

      await import("../src/product/content-script");
      await vi.advanceTimersByTimeAsync(10_000);

      expect(sendMessage).toHaveBeenCalledTimes(5);
      expect(terminal).toHaveBeenCalledOnce();
      expect(terminal).toHaveBeenCalledWith(
        expect.stringContaining("failed after 5 attempts"),
      );
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("cancels a scheduled bootstrap retry when the product is disabled", async () => {
    vi.useFakeTimers();
    try {
      vi.spyOn(document, "hasFocus").mockReturnValue(true);
      vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
      vi.spyOn(console, "warn").mockImplementation(() => undefined);
      const sendMessage = vi.fn().mockRejectedValue(new Error("transient failure"));
      type RuntimeListener = (
        message: unknown,
        sender: chrome.runtime.MessageSender,
        sendResponse: (response?: unknown) => void,
      ) => boolean;
      let runtimeListener: RuntimeListener | null = null;
      const port = {
        onDisconnect: { addListener: vi.fn() },
        disconnect: vi.fn(),
      };
      Object.defineProperty(globalThis, "chrome", {
        configurable: true,
        value: {
          runtime: {
            id: "ckkiehcjbclcjckkkajohopoikeejkoa",
            connect: vi.fn(() => port),
            sendMessage,
            onMessage: {
              addListener: vi.fn((listener) => {
                runtimeListener = listener;
              }),
              removeListener: vi.fn(),
            },
          },
        },
      });

      await import("../src/product/content-script");
      await vi.advanceTimersByTimeAsync(0);
      expect(vi.getTimerCount()).toBe(1);
      const installedListener = runtimeListener as unknown as RuntimeListener;
      installedListener(
        { kind: "badi.product.disable.v1" },
        { id: "ckkiehcjbclcjckkkajohopoikeejkoa" },
        vi.fn(),
      );
      await vi.advanceTimersByTimeAsync(10_000);

      expect(sendMessage).toHaveBeenCalledOnce();
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps one bounded backoff budget across repeated broker disconnects", async () => {
    vi.useFakeTimers();
    try {
      vi.spyOn(document, "hasFocus").mockReturnValue(true);
      vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
      vi.spyOn(console, "warn").mockImplementation(() => undefined);
      const terminal = vi.spyOn(console, "error").mockImplementation(() => undefined);
      const sendMessage = vi.fn(() => new Promise<unknown>(() => undefined));
      type RuntimeListener = (
        message: unknown,
        sender: chrome.runtime.MessageSender,
        sendResponse: (response?: unknown) => void,
      ) => boolean;
      let runtimeListener: RuntimeListener | null = null;
      const port = {
        onDisconnect: { addListener: vi.fn() },
        disconnect: vi.fn(),
      };
      Object.defineProperty(globalThis, "chrome", {
        configurable: true,
        value: {
          runtime: {
            id: "ckkiehcjbclcjckkkajohopoikeejkoa",
            connect: vi.fn(() => port),
            sendMessage,
            onMessage: {
              addListener: vi.fn((listener) => {
                runtimeListener = listener;
              }),
              removeListener: vi.fn(),
            },
          },
        },
      });

      await import("../src/product/content-script");
      const installedListener = runtimeListener as unknown as RuntimeListener;
      const disconnect = (): void => {
        installedListener(
          { kind: "badi.transport.disconnected.v1" },
          { id: "ckkiehcjbclcjckkkajohopoikeejkoa" },
          vi.fn(),
        );
      };
      expect(sendMessage).toHaveBeenCalledOnce();

      for (const delayMs of [100, 200, 400, 800]) {
        disconnect();
        await vi.advanceTimersByTimeAsync(delayMs - 1);
        expect(sendMessage).toHaveBeenCalledTimes(
          1 + [100, 200, 400, 800].indexOf(delayMs),
        );
        await vi.advanceTimersByTimeAsync(1);
      }
      expect(sendMessage).toHaveBeenCalledTimes(5);

      disconnect();
      await vi.advanceTimersByTimeAsync(10_000);
      expect(sendMessage).toHaveBeenCalledTimes(5);
      expect(terminal).toHaveBeenCalledOnce();
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });
});
