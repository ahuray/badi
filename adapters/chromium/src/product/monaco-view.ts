import type { MonacoGeometry } from "./monaco-main-world";

const HOST_ATTRIBUTE = "data-badi-dillinger-preview";
const VIEWPORT_MARGIN_PX = 8;

export interface MonacoSuggestionView {
  readonly visible: boolean;
  show(text: string, geometry: MonacoGeometry): void;
  hide(): void;
  dispose(): void;
}

export class MonacoGhostView implements MonacoSuggestionView {
  readonly #document: Document;
  #host: HTMLDivElement | null = null;
  #text: HTMLSpanElement | null = null;
  #visible = false;

  constructor(document: Document = globalThis.document) {
    this.#document = document;
  }

  get visible(): boolean {
    if (!this.#visible || !this.#layoutIsProvenVisible()) return false;
    return true;
  }

  show(text: string, geometry: MonacoGeometry): void {
    this.#ensureMounted();
    if (this.#host === null || this.#text === null || text.length === 0) {
      this.hide();
      return;
    }
    this.#visible = false;
    this.#text.textContent = text;
    this.#text.style.lineHeight = `${String(geometry.height)}px`;
    this.#host.hidden = false;
    this.#host.style.visibility = "hidden";
    this.#host.style.left = "0px";
    this.#host.style.top = "0px";
    const view = this.#document.defaultView;
    const measured = this.#host.getBoundingClientRect();
    if (
      view === null ||
      ![geometry.left, geometry.top, geometry.height, measured.width, measured.height].every(
        Number.isFinite,
      ) ||
      geometry.height <= 0 ||
      measured.width <= 0 ||
      measured.height <= 0 ||
      measured.width > view.innerWidth - VIEWPORT_MARGIN_PX * 2 ||
      measured.height > view.innerHeight - VIEWPORT_MARGIN_PX * 2
    ) {
      this.hide();
      return;
    }
    const left = Math.min(
      Math.max(VIEWPORT_MARGIN_PX, geometry.left),
      view.innerWidth - VIEWPORT_MARGIN_PX - measured.width,
    );
    const top = Math.min(
      Math.max(VIEWPORT_MARGIN_PX, geometry.top),
      view.innerHeight - VIEWPORT_MARGIN_PX - measured.height,
    );
    this.#host.style.left = `${String(left)}px`;
    this.#host.style.top = `${String(top)}px`;
    this.#host.style.visibility = "visible";
    this.#visible = true;
    if (!this.#layoutIsProvenVisible()) this.hide();
  }

  hide(): void {
    this.#visible = false;
    if (this.#host !== null) {
      this.#host.hidden = true;
      this.#host.style.visibility = "hidden";
    }
    if (this.#text !== null) this.#text.textContent = "";
  }

  dispose(): void {
    this.#host?.remove();
    this.#host = null;
    this.#text = null;
    this.#visible = false;
  }

  #ensureMounted(): void {
    if (this.#host?.isConnected === true) return;
    const host = this.#document.createElement("div");
    host.setAttribute(HOST_ATTRIBUTE, "");
    host.hidden = true;
    host.style.cssText = [
      "all:initial",
      "position:fixed",
      "display:block",
      "width:max-content",
      "z-index:2147483647",
      "pointer-events:none",
      "box-sizing:border-box",
      "max-width:min(420px,calc(100vw - 24px))",
    ].join(";");
    const shadow = host.attachShadow({ mode: "closed" });
    const style = this.#document.createElement("style");
    style.textContent = `
      .suggestion {
        display: block;
        overflow: hidden;
        overflow-wrap: anywhere;
        white-space: pre-wrap;
        color: color-mix(in srgb, CanvasText 42%, transparent);
        font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
        font-size: 14px;
        font-weight: 400;
      }
    `;
    const suggestion = this.#document.createElement("span");
    suggestion.className = "suggestion";
    suggestion.setAttribute("role", "status");
    shadow.append(style, suggestion);
    this.#document.documentElement.append(host);
    this.#host = host;
    this.#text = suggestion;
  }

  #layoutIsProvenVisible(): boolean {
    const host = this.#host;
    const view = this.#document.defaultView;
    if (
      host === null ||
      view === null ||
      !host.isConnected ||
      host.hidden ||
      this.#document.visibilityState !== "visible" ||
      !this.#document.hasFocus()
    ) {
      return false;
    }
    const style = view.getComputedStyle(host);
    const rect = host.getBoundingClientRect();
    if (
      style.display === "none" ||
      style.visibility !== "visible" ||
      Number(style.opacity) <= 0 ||
      ![rect.left, rect.top, rect.right, rect.bottom, rect.width, rect.height].every(
        Number.isFinite,
      ) ||
      rect.width <= 0 ||
      rect.height <= 0 ||
      rect.left < 0 ||
      rect.top < 0 ||
      rect.right > view.innerWidth ||
      rect.bottom > view.innerHeight
    ) {
      return false;
    }
    if (typeof this.#document.elementsFromPoint !== "function") return false;
    const inset = 1;
    const points: ReadonlyArray<readonly [number, number]> = [
      [rect.left + inset, rect.top + inset],
      [rect.right - inset, rect.top + inset],
      [rect.left + inset, rect.bottom - inset],
      [rect.right - inset, rect.bottom - inset],
      [rect.left + rect.width / 2, rect.top + rect.height / 2],
    ];
    const pointerEvents = host.style.pointerEvents;
    host.style.pointerEvents = "auto";
    try {
      return points.every(([x, y]) => this.#document.elementsFromPoint(x, y)[0] === host);
    } finally {
      host.style.pointerEvents = pointerEvents;
    }
  }
}
