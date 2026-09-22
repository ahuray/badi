import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AnchoredGhostView } from "../src/content/ghost-view";

let root: ShadowRoot;
let field: HTMLTextAreaElement;
let view: AnchoredGhostView;
let caretX = 180;
let caretY = 60;
let suffixWidth = 120;
let suffixLines = 1;
let panelHeight = 64;
let fieldRect: DOMRect;

const rectangle = (left: number, top: number, width: number, height: number): DOMRect =>
  new DOMRect(left, top, width, height);
const host = (): HTMLElement => {
  const value = document.querySelector<HTMLElement>("[data-badi-owned]");
  if (value === null) throw new Error("Missing ghost host");
  return value;
};

beforeEach(() => {
  document.body.replaceChildren();
  caretX = 180;
  caretY = 60;
  suffixWidth = 120;
  suffixLines = 1;
  panelHeight = 64;
  fieldRect = rectangle(20, 30, 500, 200);
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 1024 });
  Object.defineProperty(window, "innerHeight", { configurable: true, value: 768 });
  const attach = HTMLElement.prototype.attachShadow;
  vi.spyOn(HTMLElement.prototype, "attachShadow").mockImplementation(function (this: HTMLElement, options) {
    root = attach.call(this, options);
    return root;
  });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    if (this === field) return fieldRect;
    if (this.className === "caret-marker") return rectangle(caretX, caretY - field.scrollTop, 0, 24);
    if (this.className === "suffix-marker") return rectangle(caretX, caretY - field.scrollTop, suffixWidth, 24);
    const owner = this.hasAttribute("data-badi-owned") ? this : (this.getRootNode() as ShadowRoot).host as HTMLElement | undefined;
    if (owner?.hasAttribute("data-badi-owned")) return rectangle(Number.parseFloat(owner.style.left) || 0,
      Number.parseFloat(owner.style.top) || 0, Number.parseFloat(owner.style.width) || 500,
      root.querySelector(".inline") ? 24 : panelHeight);
    return rectangle(0, 0, 0, 0);
  });
  vi.spyOn(HTMLElement.prototype, "getClientRects").mockImplementation(function (this: HTMLElement) {
    return Array.from({ length: this.className === "suffix-marker" ? suffixLines : 1 }, () => this.getBoundingClientRect()) as unknown as DOMRectList;
  });
  field = document.createElement("textarea");
  field.value = "A useful prefix";
  field.style.cssText = "font: 20px/24px monospace; padding: 12px; border: 2px solid; direction: ltr; color: rgb(30, 40, 50)";
  Object.defineProperty(field, "clientWidth", { configurable: true, value: 496 });
  Object.defineProperty(field, "clientHeight", { configurable: true, value: 196 });
  document.body.append(field);
  field.setSelectionRange(field.value.length, field.value.length);
  view = new AnchoredGhostView(document, true);
});

afterEach(() => {
  view.dispose();
  document.querySelectorAll("[data-badi-owned]").forEach(element => element.remove());
  vi.restoreAllMocks();
});

describe("ordinary field suggestion geometry", () => {
  it("anchors a readable RTL callout to the measured caret and clears private mirror text", () => {
    field.value = "از همکاری شما بسیار";
    field.style.direction = "rtl";
    field.setSelectionRange(field.value.length, field.value.length);
    const original = { value: field.value, style: field.style.cssText, start: field.selectionStart, end: field.selectionEnd };
    view.show(field, " ممنون هستیم");
    expect(host().style.top).toBe("90px");
    expect(root.querySelector(".panel")?.classList.contains("inline")).toBe(false);
    const text = root.querySelector<HTMLSpanElement>(".suggestion");
    expect(text?.style.fontSize).toBe("20px");
    expect(text?.dir).toBe("auto");
    expect(text?.textContent).toBe(" ممنون هستیم");
    expect(root.querySelector(".hint")?.textContent).toContain("Tab next word");
    expect(root.querySelector(".measurement")?.childNodes.length).toBe(0);
    expect(host().shadowRoot).toBeNull();
    expect({ value: field.value, style: field.style.cssText, start: field.selectionStart, end: field.selectionEnd }).toEqual(original);
    expect(view.visible).toBe(true);
  });

  it("uses inline only when a whole LTR suffix fits without wrapping or moving the caret", () => {
    view.show(field, " can continue");
    expect(root.querySelector(".panel")?.classList.contains("inline")).toBe(true);
    expect(host().style.left).toBe("180px");
    expect(host().style.top).toBe("60px");
    expect(root.querySelector<HTMLSpanElement>(".suggestion")?.style.color).toBe("rgb(30, 40, 50)");
    expect(view.visible).toBe(true);
    suffixLines = 2;
    view.show(field, " a long complete suggestion");
    expect(root.querySelector(".panel")?.classList.contains("inline")).toBe(false);
    expect(root.querySelector(".suggestion")?.textContent).toBe(" a long complete suggestion");
    expect(host().style.top).toBe("90px");
  });

  it("keeps mid-word and mixed-direction suffixes in a callout", () => {
    view.show(field, "ation");
    expect(root.querySelector(".inline")).toBeNull();
    view.show(field, " متن فارسی");
    expect(root.querySelector(".inline")).toBeNull();
    suffixWidth = 800;
    view.show(field, " wider than the field");
    expect(root.querySelector(".inline")).toBeNull();
    expect(root.querySelector(".suggestion")?.textContent).toBe(" wider than the field");
  });

  it("uses a field callout when transforms make mirror coordinates unreliable", () => {
    field.style.transform = "scale(1.2)";
    view.show(field, " still readable");
    expect(root.querySelector(".inline")).toBeNull();
    expect(host().style.top).toBe("236px");
    expect(root.querySelector(".measurement")?.textContent).toBe("");
  });

  it("places the full callout above a low caret and hides if the full content cannot fit", () => {
    Object.defineProperty(window, "innerHeight", { configurable: true, value: 640 });
    fieldRect = rectangle(20, 100, 500, 520);
    caretY = 590;
    view.show(field, " متن کامل");
    expect(host().style.top).toBe("520px");
    expect(view.visible).toBe(true);
    panelHeight = 635;
    view.show(field, " متن کامل");
    expect(host().hidden).toBe(true);
    expect(view.visible).toBe(false);
    expect(root.querySelector(".suggestion")?.textContent).toBe("");
  });

  it("tracks scrolling and style changes, then disconnects and clears on hide/dispose", async () => {
    view.show(field, " متن تازه");
    field.scrollTop = 10;
    field.dispatchEvent(new Event("scroll"));
    expect(host().style.top).toBe("80px");
    field.style.font = "28px/32px monospace";
    await Promise.resolve();
    expect(root.querySelector<HTMLSpanElement>(".suggestion")?.style.fontSize).toBe("28px");
    expect(root.querySelector(".measurement")?.textContent).toBe("");
    view.hide();
    expect(root.querySelector(".suggestion")?.textContent).toBe("");
    expect(view.visible).toBe(false);
    view.dispose();
    field.dispatchEvent(new Event("scroll"));
    expect(document.querySelector("[data-badi-owned]")).toBeNull();
  });

  it("does not measure a field after it becomes disabled and rejects hidden/clipped hosts", async () => {
    view.show(field, " متن تازه");
    host().style.opacity = "0";
    expect(view.visible).toBe(false);
    host().style.opacity = "1";
    host().style.overflow = "hidden";
    expect(view.visible).toBe(false);
    host().style.overflow = "";
    host().style.top = "-20px";
    expect(view.visible).toBe(false);
    field.disabled = true;
    await Promise.resolve();
    expect(host().hidden).toBe(true);
    expect(root.querySelector(".measurement")?.textContent).toBe("");
  });

  it("preserves the historical fixture shortcut hint", () => {
    view.dispose();
    view = new AnchoredGhostView(document, false);
    view.show(field, " completion");
    expect(root.querySelector(".hint")?.textContent).toBe("Tab accept all · Ctrl/⌘ + → accept word · Esc dismiss");
  });
});
