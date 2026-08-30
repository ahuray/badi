export interface TrustedSessionRoute {
  readonly tabId: number;
  readonly frameId: number;
  readonly documentId: string;
  readonly origin: string;
}

export interface SessionRouteSubscription {
  readonly displacedSessionIds: readonly string[];
}

export interface TrustedSessionRouteEntry {
  readonly sessionId: string;
  readonly route: TrustedSessionRoute;
}

function routeFromSender(
  sender: chrome.runtime.MessageSender,
): TrustedSessionRoute | null {
  const tabId = sender.tab?.id;
  const frameId = sender.frameId;
  const documentId = sender.documentId;
  const origin = sender.origin;
  if (
    tabId === undefined ||
    frameId === undefined ||
    typeof documentId !== "string" ||
    documentId.length === 0 ||
    typeof origin !== "string" ||
    origin.length === 0
  ) {
    return null;
  }
  return Object.freeze({
    tabId,
    frameId,
    documentId,
    origin,
  });
}

function routesEqual(left: TrustedSessionRoute, right: TrustedSessionRoute): boolean {
  return (
    left.tabId === right.tabId &&
    left.frameId === right.frameId &&
    left.documentId === right.documentId &&
    left.origin === right.origin
  );
}

/** Registers opaque sessions against independently trusted content senders. */
export class SessionRouteRegistry {
  readonly #routes = new Map<string, TrustedSessionRoute>();

  /**
   * Subscribes one exact document before it can send content. A new session
   * in the same tab/frame displaces the previous subscription so its native
   * session can be retired. A session id itself never migrates.
   */
  subscribe(
    sessionId: string,
    sender: chrome.runtime.MessageSender,
  ): SessionRouteSubscription | null {
    const candidate = routeFromSender(sender);
    if (candidate === null) return null;
    const existing = this.#routes.get(sessionId);
    if (existing !== undefined && !routesEqual(existing, candidate)) return null;

    const displacedSessionIds: string[] = [];
    for (const [otherSessionId, route] of this.#routes) {
      if (
        otherSessionId === sessionId ||
        route.tabId !== candidate.tabId ||
        route.frameId !== candidate.frameId
      ) {
        continue;
      }
      this.#routes.delete(otherSessionId);
      displacedSessionIds.push(otherSessionId);
    }
    this.#routes.set(sessionId, candidate);
    return Object.freeze({
      displacedSessionIds: Object.freeze(displacedSessionIds),
    });
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

  entries(): readonly TrustedSessionRouteEntry[] {
    return Object.freeze(
      [...this.#routes].map(([sessionId, route]) =>
        Object.freeze({ sessionId, route: Object.freeze({ ...route }) }),
      ),
    );
  }

  delete(sessionId: string, expected?: TrustedSessionRoute): boolean {
    const current = this.#routes.get(sessionId);
    if (current === undefined || (expected !== undefined && !routesEqual(current, expected))) {
      return false;
    }
    return this.#routes.delete(sessionId);
  }

  deleteDocument(sender: chrome.runtime.MessageSender): readonly string[] {
    const candidate = routeFromSender(sender);
    if (candidate === null) return Object.freeze([]);
    const deleted: string[] = [];
    for (const [sessionId, route] of this.#routes) {
      if (!routesEqual(route, candidate)) continue;
      this.#routes.delete(sessionId);
      deleted.push(sessionId);
    }
    return Object.freeze(deleted);
  }

  deleteTab(tabId: number): readonly string[] {
    const deleted: string[] = [];
    for (const [sessionId, route] of this.#routes) {
      if (route.tabId !== tabId) continue;
      this.#routes.delete(sessionId);
      deleted.push(sessionId);
    }
    return Object.freeze(deleted);
  }
}
