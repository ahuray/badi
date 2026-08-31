import { describe, expect, it, vi } from "vitest";
import type { CommitAuthorization } from "../src/shared/model";
import type { TrustedSessionRoute } from "../src/background/session-routes";
import { ProductCommitLinearizer } from "../src/product/commit-linearizer";

const AUTHORIZATION: CommitAuthorization = {
  requestId: "request-a",
  sessionId: "session-a",
  focusEpoch: 1,
  revision: 2,
  monotonicMs: 3,
  fingerprint: "fingerprint-a",
  suggestionId: "suggestion-a",
  text: " for your time",
  acceptance: "all",
};

const ROUTE: TrustedSessionRoute = {
  tabId: 7,
  frameId: 0,
  documentId: "document-a",
  origin: "https://dillinger.io",
};

describe("product commit linearization", () => {
  it("does not invoke MAIN-world mutation after a revocation epoch", () => {
    const linearizer = new ProductCommitLinearizer();
    const epoch = linearizer.epoch;
    linearizer.issue(AUTHORIZATION, ROUTE);
    linearizer.revokeSession(AUTHORIZATION.sessionId);
    const mutation = vi.fn(async () => true);

    expect(linearizer.invoke(AUTHORIZATION, ROUTE, epoch, mutation)).toBeNull();
    expect(mutation).not.toHaveBeenCalled();
  });

  it("rejects a stale document and consumes an exact grant only once", async () => {
    const linearizer = new ProductCommitLinearizer();
    linearizer.issue(AUTHORIZATION, ROUTE);
    const mutation = vi.fn(async () => true);

    expect(
      linearizer.invoke(
        AUTHORIZATION,
        { ...ROUTE, documentId: "document-b" },
        linearizer.epoch,
        mutation,
      ),
    ).toBeNull();
    expect(mutation).not.toHaveBeenCalled();

    const execution = linearizer.invoke(
      AUTHORIZATION,
      ROUTE,
      linearizer.epoch,
      mutation,
    );
    expect(execution).not.toBeNull();
    await expect(execution).resolves.toBe(true);
    expect(mutation).toHaveBeenCalledTimes(1);
    expect(
      linearizer.invoke(AUTHORIZATION, ROUTE, linearizer.epoch, mutation),
    ).toBeNull();
  });

  it("invokes the mutation synchronously at the one-shot boundary", async () => {
    const linearizer = new ProductCommitLinearizer();
    linearizer.issue(AUTHORIZATION, ROUTE);
    let resolveMutation!: (value: boolean) => void;
    let invoked = false;
    const execution = linearizer.invoke(
      AUTHORIZATION,
      ROUTE,
      linearizer.epoch,
      () => {
        invoked = true;
        return new Promise<boolean>((resolve) => {
          resolveMutation = resolve;
        });
      },
    );

    expect(invoked).toBe(true);
    expect(execution).not.toBeNull();
    linearizer.revokeAll();
    resolveMutation(true);
    await expect(execution).resolves.toBe(true);
  });

  it("expires an unused grant without invoking mutation", () => {
    let now = 100;
    const linearizer = new ProductCommitLinearizer({ now: () => now, ttlMs: 25 });
    linearizer.issue(AUTHORIZATION, ROUTE);
    now = 125;
    const mutation = vi.fn(async () => true);

    expect(
      linearizer.invoke(AUTHORIZATION, ROUTE, linearizer.epoch, mutation),
    ).toBeNull();
    expect(mutation).not.toHaveBeenCalled();
  });
});
