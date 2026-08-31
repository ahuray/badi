import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { MonacoGhostView } from "../src/product/monaco-view";

const originalElementsFromPoint = document.elementsFromPoint;
let measuredWidth = 120;
let measuredHeight = 40;
let occluded = false;

function rect(left: number, top: number, width: number, height: number): DOMRect {
  return {
    x: left,
    y: top,
    left,
    top,
    right: left + width,
    bottom: top + height,
    width,
    height,
    toJSON: () => ({}),
  };
}

beforeEach(() => {
  measuredWidth = 120;
  measuredHeight = 40;
  occluded = false;
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  vi.spyOn(window, "getComputedStyle").mockImplementation(
    (element) =>
      ({
        display: element instanceof HTMLElement ? element.style.display || "block" : "block",
        visibility:
          element instanceof HTMLElement ? element.style.visibility || "visible" : "visible",
        opacity: "1",
      }) as CSSStyleDeclaration,
  );
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement,
  ) {
    if (!this.hasAttribute("data-badi-dillinger-preview")) return rect(0, 0, 1, 1);
    return rect(
      Number.parseFloat(this.style.left) || 0,
      Number.parseFloat(this.style.top) || 0,
      measuredWidth,
      measuredHeight,
    );
  });
  Object.defineProperty(document, "elementsFromPoint", {
    configurable: true,
    value: vi.fn(() => {
      const host = document.querySelector<HTMLElement>("[data-badi-dillinger-preview]");
      if (host === null) return [];
      return occluded ? [document.body, host] : [host, document.documentElement];
    }),
  });
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 300 });
  Object.defineProperty(window, "innerHeight", { configurable: true, value: 200 });
});

afterEach(() => {
  document.querySelector("[data-badi-dillinger-preview]")?.remove();
  Object.defineProperty(document, "elementsFromPoint", {
    configurable: true,
    value: originalElementsFromPoint,
  });
  vi.restoreAllMocks();
});

describe("Dillinger Monaco preview geometry", () => {
  it("ships only the calm suggestion ghost without card or shortcut hint chrome", async () => {
    const source = await readFile(
      resolve(process.cwd(), "src/product/monaco-view.ts"),
      "utf8",
    );
    expect(source).not.toContain("Ctrl + Shift");
    expect(source).not.toContain(".panel");
    expect(source).not.toContain(".hint");
    expect(source).not.toContain("border:");
    expect(source).not.toContain("background:");
  });

  it("keeps a suggestion-only ghost caret-relative while clamping both viewport edges", () => {
    const view = new MonacoGhostView(document);

    view.show("fixed suggestion", { left: 280, top: 170, height: 20 });

    const host = document.querySelector<HTMLElement>("[data-badi-dillinger-preview]");
    expect(host?.style.left).toBe("172px");
    expect(host?.style.top).toBe("152px");
    expect(view.visible).toBe(true);
    view.dispose();
  });

  it("fails closed when the panel fits neither above nor below", () => {
    measuredHeight = 190;
    const view = new MonacoGhostView(document);

    view.show("fixed suggestion", { left: 100, top: 90, height: 18 });

    expect(view.visible).toBe(false);
    expect(
      document.querySelector<HTMLElement>("[data-badi-dillinger-preview]")?.hidden,
    ).toBe(true);
    view.dispose();
  });

  it("fails closed when a hostile page layer occludes the preview", () => {
    const view = new MonacoGhostView(document);
    occluded = true;

    view.show("fixed suggestion", { left: 40, top: 40, height: 18 });

    expect(view.visible).toBe(false);
    expect(
      document.querySelector<HTMLElement>("[data-badi-dillinger-preview]")?.hidden,
    ).toBe(true);
    view.dispose();
  });
});
