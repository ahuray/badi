export interface TrustedSessionRoute {
  readonly tabId: number;
  readonly frameId: number;
  readonly documentId: string;
}

function routeFromSender(
  sender: chrome.runtime.MessageSender,
): TrustedSessionRoute | null {
  const tabId = sender.tab?.id;
  const frameId = sender.frameId;
  const documentId = sender.documentId;
  if (
    tabId === undefined ||
    frameId === undefined ||
    typeof documentId !== "string" ||
    documentId.length === 0
  ) {
    return null;
  }
  return Object.freeze({
    tabId,
    frameId,
    documentId,
  });
}

function routesEqual(left: TrustedSessionRoute, right: TrustedSessionRoute): boolean {
  return (
    left.tabId === right.tabId &&
    left.frameId === right.frameId &&
    left.documentId === right.documentId
  );
}

/**
 * Binds each opaque session to its first independently trusted content sender.
 * A session id can never migrate to another tab, frame, or document.
 */
export class SessionRouteRegistry {
  readonly #routes = new Map<string, TrustedSessionRoute>();

  bind(sessionId: string, sender: chrome.runtime.MessageSender): boolean {
    const candidate = routeFromSender(sender);
    if (candidate === null) return false;
    const existing = this.#routes.get(sessionId);
    if (existing !== undefined) return routesEqual(existing, candidate);
    this.#routes.set(sessionId, candidate);
    return true;
  }

  matches(sessionId: string, sender: chrome.runtime.MessageSender): boolean {
    const candidate = routeFromSender(sender);
    const existing = this.#routes.get(sessionId);
    return candidate !== null && existing !== undefined && routesEqual(existing, candidate);
  }

  get(sessionId: string): TrustedSessionRoute | null {
    return this.#routes.get(sessionId) ?? null;
  }

  snapshot(): readonly TrustedSessionRoute[] {
    const unique: TrustedSessionRoute[] = [];
    for (const route of this.#routes.values()) {
      if (!unique.some((candidate) => routesEqual(candidate, route))) {
        unique.push(Object.freeze({ ...route }));
      }
    }
    return Object.freeze(unique);
  }

  deleteTab(tabId: number): void {
    for (const [sessionId, route] of this.#routes) {
      if (route.tabId === tabId) this.#routes.delete(sessionId);
    }
  }
}
