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
    Object.defineProperty(globalThis, "chrome", {
      configurable: true,
      value: {
        runtime: {
          id: "ckkiehcjbclcjckkkajohopoikeejkoa",
          connect: vi.fn(() => port),
          sendMessage: vi.fn(() => new Promise(() => undefined)),
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
  });
});
