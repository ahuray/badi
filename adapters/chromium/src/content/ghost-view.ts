import type { EditableField, SuggestionView } from "../shared/model";

const HOST_ATTRIBUTE = "data-badi-owned";
const TYPOGRAPHY = ["font-family", "font-size", "font-weight", "font-style", "font-stretch",
  "font-variant", "font-feature-settings", "font-variation-settings", "letter-spacing", "word-spacing",
  "text-transform", "text-rendering", "tab-size"] as const;
const EDGE = 8;
const rtlText = /[\u0590-\u08ff\ufb1d-\ufdff\ufe70-\ufeff]/u;
const pixel = (value: string): number => Number.parseFloat(value) || 0;

interface CaretMeasurement {
  readonly caret: DOMRect;
  readonly suggestion: DOMRect | null;
}

export class AnchoredGhostView implements SuggestionView {
  readonly #document: Document;
  readonly #tabAcceptsWord: boolean;
  #host: HTMLDivElement | null = null;
  #panel: HTMLDivElement | null = null;
  #text: HTMLSpanElement | null = null;
  #hint: HTMLSpanElement | null = null;
  #mirror: HTMLDivElement | null = null;
  #styleObserver: MutationObserver | null = null;
  #sizeObserver: ResizeObserver | null = null;
  #field: EditableField | null = null;
  #visible = false;

  readonly #reposition = (): void => {
    if (!this.#visible || this.#host === null || this.#field === null) {
      return;
    }
    const field = this.#field;
    if (!field.isConnected || field.disabled || field.readOnly ||
        (field.tagName === "INPUT" && field.type !== "text")) {
      this.hide();
      return;
    }

    const window = this.#document.defaultView;
    if (window === null || this.#panel === null || this.#text === null || this.#hint === null) return;
    const rect = field.getBoundingClientRect();
    const viewportWidth = window.innerWidth;
    const viewportHeight = window.innerHeight;
    const fieldStyle = window.getComputedStyle(field);
    if (rect.width <= 0 || rect.height <= 0 || fieldStyle.display === "none" ||
        ["hidden", "collapse"].includes(fieldStyle.visibility)) {
      this.hide();
      return;
    }
    for (const property of TYPOGRAPHY) this.#text.style.setProperty(property, fieldStyle.getPropertyValue(property));
    this.#text.style.fontSize = `${Math.max(14, pixel(fieldStyle.fontSize) || 16)}px`;
    this.#text.style.lineHeight = "1.4";
    this.#text.dir = "auto";
    this.#panel.classList.remove("inline");
    this.#text.style.color = "";
    this.#hint.style.cssText = "";
    const measurement = this.#measure(field, fieldStyle);

    // A separate overlay cannot reproduce arbitrary bidi or mid-word shaping.
    // Inline is limited to a fully measured suffix in a simple LTR text run.
    if (measurement?.suggestion !== null && measurement?.suggestion !== undefined &&
        field.tagName === "TEXTAREA" && viewportWidth >= 360 &&
        fieldStyle.direction !== "rtl" && !rtlText.test(field.value + (this.#text.textContent ?? "")) &&
        /^\s/u.test(this.#text.textContent ?? "") &&
        this.#inside(measurement.suggestion, rect, viewportWidth, viewportHeight)) {
      const suffix = measurement.suggestion;
      this.#panel.classList.add("inline");
      this.#text.style.fontSize = fieldStyle.fontSize;
      this.#text.style.lineHeight = `${suffix.height}px`;
      // Inherit the field's readable contrast instead of assuming a white page.
      this.#text.style.color = fieldStyle.color;
      this.#host.style.left = `${suffix.left}px`;
      this.#host.style.top = `${suffix.top}px`;
      this.#host.style.width = `${Math.ceil(suffix.width + 2)}px`;
      const hintWidth = Math.min(420, viewportWidth - EDGE * 2);
      this.#hint.style.position = "fixed";
      this.#hint.style.width = `${hintWidth}px`;
      this.#hint.style.left = `${Math.max(EDGE, Math.min(suffix.left, viewportWidth - hintWidth - EDGE))}px`;
      this.#hint.style.top = `${Math.min(viewportHeight - 44, suffix.bottom + 4)}px`;
      return;
    }

    const estimatedWidth = Math.min(Math.max(rect.width, 240), 560);
    const width = Math.max(1, Math.min(estimatedWidth, viewportWidth - EDGE * 2));
    const caret = measurement?.caret;
    const rtl = fieldStyle.direction === "rtl" || rtlText.test(this.#text.textContent ?? "");
    const anchor = caret === undefined ? rect.left : rtl ? caret.right - width : caret.left;
    const left = Math.max(EDGE, Math.min(anchor, viewportWidth - width - EDGE));
    this.#host.style.width = `${Math.round(width)}px`;
    const measuredHeight = this.#panel.getBoundingClientRect().height;
    const height = measuredHeight > 0 ? measuredHeight : 96;
    const below = (caret?.bottom ?? rect.bottom) + 6;
    const above = (caret?.top ?? rect.top) - height - 6;
    const top = below + height <= viewportHeight - EDGE ? below : above;
    // Never present a clipped tail which could later be accepted invisibly.
    if (height > viewportHeight - EDGE * 2 || (top < EDGE && measuredHeight > 0)) {
      this.hide();
      return;
    }

    this.#host.style.left = `${Math.round(left)}px`;
    this.#host.style.top = `${Math.round(Math.max(EDGE, top))}px`;
  };

  #inside(suffix: DOMRect, field: DOMRect, width: number, height: number): boolean {
    const target = this.#field;
    const window = this.#document.defaultView;
    if (target === null || window === null) return false;
    const style = window.getComputedStyle(target);
    return suffix.width > 0 && suffix.height > 0 && suffix.left >= Math.max(EDGE, field.left + pixel(style.paddingLeft) + pixel(style.borderLeftWidth)) &&
      suffix.right <= Math.min(width - EDGE, field.left + pixel(style.borderLeftWidth) + target.clientWidth - pixel(style.paddingRight)) &&
      suffix.top >= Math.max(EDGE, field.top + pixel(style.borderTopWidth)) &&
      suffix.bottom <= Math.min(height - 48, field.top + pixel(style.borderTopWidth) + target.clientHeight - pixel(style.paddingBottom));
  }

  #measure(field: EditableField, style: CSSStyleDeclaration): CaretMeasurement | null {
    const mirror = this.#mirror;
    const window = this.#document.defaultView;
    const end = field.selectionEnd;
    if (mirror === null || window === null || end === null || field.selectionStart !== end ||
        end !== field.value.length || field.value.length > 16384 || field.clientWidth === 0 ||
        !["", "horizontal-tb"].includes(style.writingMode)) return null;
    for (let node: Element | null = field; node !== null; node = node.parentElement) {
      const ancestor = window.getComputedStyle(node);
      if (!["", "none"].includes(ancestor.transform) ||
          !["", "none", "1"].includes(ancestor.getPropertyValue("scale")) ||
          !["", "normal", "1"].includes(ancestor.getPropertyValue("zoom"))) return null;
    }
    try {
      const rect = field.getBoundingClientRect();
      mirror.style.cssText = "position:fixed;visibility:hidden;pointer-events:none;box-sizing:border-box;overflow:hidden;contain:layout style;";
      for (const property of [...TYPOGRAPHY, "line-height", "padding-top", "padding-bottom", "padding-left", "padding-right",
        "border-top-width", "border-bottom-width", "border-left-width", "border-right-width", "text-align", "direction", "text-indent", "word-break", "overflow-wrap"]) {
        mirror.style.setProperty(property, style.getPropertyValue(property));
      }
      mirror.style.borderStyle = "solid";
      mirror.style.borderColor = "transparent";
      mirror.style.left = `${rect.left}px`;
      mirror.style.top = `${rect.top}px`;
      mirror.style.width = `${field.clientWidth + pixel(style.borderLeftWidth) + pixel(style.borderRightWidth)}px`;
      mirror.style.height = `${field.clientHeight + pixel(style.borderTopWidth) + pixel(style.borderBottomWidth)}px`;
      mirror.style.whiteSpace = field.tagName === "INPUT" || field.getAttribute("wrap") === "off" ? "pre" : "pre-wrap";
      const marker = this.#document.createElement("span");
      marker.className = "caret-marker";
      marker.textContent = "\u200b";
      mirror.append(this.#document.createTextNode(field.value), marker);
      if (field.tagName === "INPUT") {
        const lineHeight = pixel(style.lineHeight) || marker.getBoundingClientRect().height;
        const freeHeight = field.clientHeight - pixel(style.paddingTop) - pixel(style.paddingBottom) - lineHeight;
        mirror.style.paddingTop = `${pixel(style.paddingTop) + Math.max(0, freeHeight / 2)}px`;
      }
      mirror.scrollTop = field.scrollTop;
      mirror.scrollLeft = field.scrollLeft;
      const caret = marker.getBoundingClientRect();
      if (caret.height <= 0 || caret.bottom < rect.top || caret.top > rect.bottom ||
          caret.left < rect.left || caret.right > rect.right) return null;
      const suffix = this.#document.createElement("span");
      suffix.className = "suffix-marker";
      suffix.textContent = this.#text?.textContent ?? "";
      mirror.append(suffix);
      const moved = marker.getBoundingClientRect();
      const bounds = suffix.getBoundingClientRect();
      const oneLine = suffix.getClientRects().length === 1 && Math.abs(moved.left - caret.left) < .5 &&
        Math.abs(moved.top - caret.top) < .5 && Math.abs(bounds.top - caret.top) < .5;
      return { caret, suggestion: oneLine ? bounds : null };
    } finally {
      // Measurement text never persists in the shadow tree between events.
      mirror.replaceChildren();
    }
  }

  constructor(document: Document = globalThis.document, tabAcceptsWord = false) {
    this.#document = document;
    this.#tabAcceptsWord = tabAcceptsWord;
  }

  get visible(): boolean {
    const host = this.#host;
    const panel = this.#panel;
    if (
      !this.#visible ||
      this.#field?.isConnected !== true ||
      host === null ||
      panel === null ||
      !host.isConnected ||
      host.hidden
    ) {
      return false;
    }
    const window = this.#document.defaultView;
    if (window === null) return false;
    try {
      const style = window.getComputedStyle(host);
      const opacity = style.opacity === "" ? 1 : Number.parseFloat(style.opacity);
      const scale = style.getPropertyValue("scale");
      const clip = style.getPropertyValue("clip");
      const contain = style.getPropertyValue("contain");
      const rect = host.getBoundingClientRect();
      const panelRect = panel.getBoundingClientRect();
      return (
        style.display !== "none" &&
        style.visibility !== "hidden" &&
        style.visibility !== "collapse" &&
        style.getPropertyValue("content-visibility") !== "hidden" &&
        (style.filter === "" || style.filter === "none") &&
        (style.clipPath === "" || style.clipPath === "none") &&
        (clip === "" || clip === "auto") &&
        (style.getPropertyValue("mask-image") === "" ||
          style.getPropertyValue("mask-image") === "none") &&
        (style.transform === "" || style.transform === "none") &&
        (scale === "" || scale === "none" || scale === "1") &&
        (style.overflow === "" || style.overflow === "visible") &&
        (style.overflowX === "" || style.overflowX === "visible") &&
        (style.overflowY === "" || style.overflowY === "visible") &&
        !/(?:^|\s)(?:content|paint|strict)(?:\s|$)/u.test(contain) &&
        Number.isFinite(opacity) &&
        opacity > 0 &&
        rect.width > 0 &&
        rect.height > 0 &&
        rect.right > 0 &&
        rect.bottom > 0 &&
        rect.left < window.innerWidth &&
        rect.top < window.innerHeight &&
        panelRect.width > 0 &&
        panelRect.height > 0 &&
        panelRect.left >= 0 && panelRect.top >= 0 &&
        panelRect.right <= window.innerWidth && panelRect.bottom <= window.innerHeight
      );
    } catch {
      return false;
    }
  }

  show(field: EditableField, text: string): void {
    this.#ensureMounted();
    this.#styleObserver?.disconnect();
    this.#sizeObserver?.disconnect();
    this.#field = field;
    this.#visible = true;
    if (this.#text !== null) {
      this.#text.textContent = text;
    }
    if (this.#host !== null) {
      this.#host.hidden = false;
    }
    for (let node: Element | null = field; node !== null; node = node.parentElement) {
      this.#styleObserver?.observe(node, { attributes: true,
        attributeFilter: ["style", "class", "dir", "hidden", "type", "disabled", "readonly", "wrap"] });
    }
    this.#sizeObserver?.observe(field);
    this.#reposition();
  }

  hide(): void {
    this.#visible = false;
    this.#field = null;
    this.#styleObserver?.disconnect();
    this.#sizeObserver?.disconnect();
    this.#mirror?.replaceChildren();
    if (this.#host !== null) {
      this.#host.hidden = true;
    }
    if (this.#text !== null) {
      this.#text.textContent = "";
    }
  }

  dispose(): void {
    const window = this.#document.defaultView;
    window?.removeEventListener("resize", this.#reposition);
    window?.removeEventListener("scroll", this.#reposition, true);
    this.#document.fonts?.removeEventListener("loadingdone", this.#reposition);
    this.#styleObserver?.disconnect();
    this.#sizeObserver?.disconnect();
    this.#host?.remove();
    this.#host = null;
    this.#panel = null;
    this.#text = null;
    this.#hint = null;
    this.#mirror = null;
    this.#styleObserver = null;
    this.#sizeObserver = null;
    this.#field = null;
    this.#visible = false;
  }

  #ensureMounted(): void {
    if (this.#host?.isConnected === true) {
      return;
    }
    this.#host = null;
    this.#panel = null;
    this.#text = null;
    this.#hint = null;
    this.#mirror = null;

    const host = this.#document.createElement("div");
    host.setAttribute(HOST_ATTRIBUTE, "");
    host.setAttribute("aria-live", "polite");
    host.hidden = true;
    const shadow = host.attachShadow({ mode: "closed" });

    const style = this.#document.createElement("style");
    style.textContent = `
      :host {
        all: initial;
        position: fixed;
        z-index: 2147483647;
        pointer-events: none;
        box-sizing: border-box;
        color-scheme: light dark;
      }
      :host([hidden]) { display: none !important; }
      .panel {
        box-sizing: border-box;
        overflow: visible;
        border: 1px solid color-mix(in srgb, CanvasText 18%, transparent);
        border-radius: 10px;
        padding: 9px 11px 8px;
        background: Canvas;
        box-shadow: 0 10px 28px rgb(0 0 0 / 22%);
        color: CanvasText;
        font: 500 13px/1.45 ui-sans-serif, system-ui, sans-serif;
      }
      .suggestion {
        display: block;
        overflow-wrap: anywhere;
        white-space: pre-wrap;
        text-align: start;
      }
      .hint {
        display: block;
        margin-top: 5px;
        color: CanvasText;
        font: 11px/1.4 ui-sans-serif, system-ui, sans-serif;
        direction: ltr;
        letter-spacing: 0.01em;
      }
      .panel.inline { padding: 0; border: 0; background: transparent; box-shadow: none; border-radius: 0; }
      .inline .suggestion {
        white-space: pre;
        text-decoration: underline dotted;
        text-underline-offset: 3px;
      }
      .inline .hint {
        box-sizing: border-box;
        margin: 0;
        padding: 5px 8px;
        border: 1px solid color-mix(in srgb, CanvasText 20%, Canvas);
        border-radius: 6px;
        background: Canvas;
      }
      kbd {
        font: inherit;
        border: 1px solid color-mix(in srgb, CanvasText 16%, transparent);
        border-radius: 4px;
        padding: 0 3px;
      }
    `;

    const panel = this.#document.createElement("div");
    panel.className = "panel";
    panel.setAttribute("role", "status");
    const suggestion = this.#document.createElement("span");
    suggestion.className = "suggestion";
    const hint = this.#document.createElement("span");
    hint.className = "hint";
    hint.textContent = this.#tabAcceptsWord
      ? "Tab next word · Ctrl/⌘ + → accept all · Esc dismiss"
      : "Tab accept all · Ctrl/⌘ + → accept word · Esc dismiss";
    panel.append(suggestion, hint);
    const mirror = this.#document.createElement("div");
    mirror.className = "measurement";
    mirror.setAttribute("aria-hidden", "true");
    shadow.append(style, panel, mirror);
    this.#document.documentElement.append(host);

    this.#host = host;
    this.#panel = panel;
    this.#text = suggestion;
    this.#hint = hint;
    this.#mirror = mirror;
    const window = this.#document.defaultView;
    this.#styleObserver ??= new MutationObserver(this.#reposition);
    if (window?.ResizeObserver !== undefined) this.#sizeObserver ??= new window.ResizeObserver(this.#reposition);
    window?.addEventListener("resize", this.#reposition);
    window?.addEventListener("scroll", this.#reposition, true);
    this.#document.fonts?.addEventListener("loadingdone", this.#reposition);
  }
}
