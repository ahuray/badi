import type { EditableField, SuggestionView } from "../shared/model";

const HOST_ATTRIBUTE = "data-badi-owned";

export class AnchoredGhostView implements SuggestionView {
  readonly #document: Document;
  #host: HTMLDivElement | null = null;
  #panel: HTMLDivElement | null = null;
  #text: HTMLSpanElement | null = null;
  #field: EditableField | null = null;
  #visible = false;

  readonly #reposition = (): void => {
    if (!this.#visible || this.#host === null || this.#field === null) {
      return;
    }
    if (!this.#field.isConnected) {
      this.hide();
      return;
    }

    const rect = this.#field.getBoundingClientRect();
    const viewportWidth = this.#document.defaultView?.innerWidth ?? 1024;
    const viewportHeight = this.#document.defaultView?.innerHeight ?? 768;
    const estimatedWidth = Math.min(Math.max(rect.width, 240), 560);
    const left = Math.max(8, Math.min(rect.left, viewportWidth - estimatedWidth - 8));
    const below = rect.bottom + 6;
    const top = below + 96 < viewportHeight ? below : Math.max(8, rect.top - 102);

    this.#host.style.left = `${Math.round(left)}px`;
    this.#host.style.top = `${Math.round(top)}px`;
    this.#host.style.width = `${Math.round(estimatedWidth)}px`;
  };

  constructor(document: Document = globalThis.document) {
    this.#document = document;
  }

  get visible(): boolean {
    const host = this.#host;
    const panel = this.#panel;
    if (
      !this.#visible ||
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
        panelRect.height > 0
      );
    } catch {
      return false;
    }
  }

  show(field: EditableField, text: string): void {
    this.#ensureMounted();
    this.#field = field;
    this.#visible = true;
    if (this.#text !== null) {
      this.#text.textContent = text;
    }
    if (this.#host !== null) {
      this.#host.hidden = false;
    }
    this.#reposition();
  }

  hide(): void {
    this.#visible = false;
    this.#field = null;
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
    this.#host?.remove();
    this.#host = null;
    this.#panel = null;
    this.#text = null;
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
      .panel {
        box-sizing: border-box;
        overflow: hidden;
        border: 1px solid color-mix(in srgb, CanvasText 18%, transparent);
        border-radius: 10px;
        padding: 9px 11px 8px;
        background: color-mix(in srgb, Canvas 94%, transparent);
        box-shadow: 0 10px 28px rgb(0 0 0 / 22%);
        color: color-mix(in srgb, CanvasText 74%, transparent);
        font: 500 13px/1.45 ui-sans-serif, system-ui, sans-serif;
        backdrop-filter: blur(12px);
      }
      .suggestion {
        display: block;
        overflow-wrap: anywhere;
        white-space: pre-wrap;
      }
      .hint {
        display: block;
        margin-top: 5px;
        color: color-mix(in srgb, CanvasText 42%, transparent);
        font-size: 10px;
        letter-spacing: 0.01em;
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
    hint.textContent = "Tab accept all · Ctrl/⌘ + → accept word · Esc dismiss";
    panel.append(suggestion, hint);
    shadow.append(style, panel);
    this.#document.documentElement.append(host);

    this.#host = host;
    this.#panel = panel;
    this.#text = suggestion;
    const window = this.#document.defaultView;
    window?.addEventListener("resize", this.#reposition);
    window?.addEventListener("scroll", this.#reposition, true);
  }
}
