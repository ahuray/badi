import type { CommitAuthorization } from "../shared/model";
import type { TrustedSessionRoute } from "../background/session-routes";

interface CommitGrant {
  readonly authorization: CommitAuthorization;
  readonly route: TrustedSessionRoute;
  readonly epoch: number;
  readonly expiresAt: number;
}

const PRODUCT_COMMIT_GRANT_TTL_MS = 1_000;

function authorizationsEqual(
  left: CommitAuthorization,
  right: CommitAuthorization,
): boolean {
  return (
    left.requestId === right.requestId &&
    left.sessionId === right.sessionId &&
    left.focusEpoch === right.focusEpoch &&
    left.revision === right.revision &&
    left.monotonicMs === right.monotonicMs &&
    left.fingerprint === right.fingerprint &&
    left.suggestionId === right.suggestionId &&
    left.text === right.text &&
    left.acceptance === right.acceptance
  );
}

function routesEqual(left: TrustedSessionRoute, right: TrustedSessionRoute): boolean {
  return (
    left.tabId === right.tabId &&
    left.frameId === right.frameId &&
    left.documentId === right.documentId &&
    left.origin === right.origin
  );
}

/**
 * One-shot product commit grants. `invoke` consumes the grant and calls the
 * mutation callback in the same JavaScript turn, with no await or event-loop
 * handoff between the final epoch check and chrome.scripting.executeScript.
 */
export class ProductCommitLinearizer {
  readonly #grants = new Map<string, CommitGrant>();
  readonly #now: () => number;
  readonly #ttlMs: number;
  #epoch = 0;

  constructor(options: { readonly now?: () => number; readonly ttlMs?: number } = {}) {
    this.#now = options.now ?? (() => performance.now());
    this.#ttlMs = options.ttlMs ?? PRODUCT_COMMIT_GRANT_TTL_MS;
  }

  get epoch(): number {
    return this.#epoch;
  }

  issue(authorization: CommitAuthorization, route: TrustedSessionRoute): void {
    this.#grants.set(authorization.sessionId, {
      authorization: Object.freeze({ ...authorization }),
      route: Object.freeze({ ...route }),
      epoch: this.#epoch,
      expiresAt: this.#now() + this.#ttlMs,
    });
  }

  invoke<T>(
    authorization: CommitAuthorization,
    route: TrustedSessionRoute,
    expectedEpoch: number,
    mutation: () => Promise<T>,
  ): Promise<T> | null {
    const grant = this.#grants.get(authorization.sessionId);
    if (grant !== undefined && this.#now() >= grant.expiresAt) {
      this.#grants.delete(authorization.sessionId);
      return null;
    }
    if (
      expectedEpoch !== this.#epoch ||
      grant === undefined ||
      grant.epoch !== this.#epoch ||
      !authorizationsEqual(grant.authorization, authorization) ||
      !routesEqual(grant.route, route)
    ) {
      return null;
    }
    this.#grants.delete(authorization.sessionId);
    return mutation();
  }

  revokeSession(sessionId: string): void {
    this.#epoch += 1;
    this.#grants.delete(sessionId);
    // Epoch changes invalidate every grant; eagerly clear rather than retain
    // unreachable tokens from another route.
    this.#grants.clear();
  }

  revokeAll(): void {
    this.#epoch += 1;
    this.#grants.clear();
  }
}
