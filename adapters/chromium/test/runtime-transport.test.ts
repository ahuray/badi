import { describe, expect, it } from "vitest";
import { RuntimeSuggestionTransport } from "../src/content/runtime-transport";
import type { SuggestionRequest, TargetPolicy } from "../src/shared/model";

const SESSION_ID = "0198f215-3ec0-7000-8000-000000000001";

function policy(paused = true): TargetPolicy {
  return {
    authorityEpoch: 4,
    settingsRevision: 2,
    paused,
    activation: paused ? "never" : "always",
    contextAllowed: !paused,
    displayAllowed: !paused,
    suggestionsAllowed: !paused,
    learningAllowed: false,
    reason: paused ? "global_disabled" : "matched_rule",
  };
}

class DeferredReply {
  readonly promise: Promise<unknown>;
  resolve: (value: unknown) => void = () => undefined;

  constructor() {
    this.promise = new Promise((resolve) => {
      this.resolve = resolve;
    });
  }
}

function request(): SuggestionRequest {
  return {
    requestId: "request-1",
    sessionId: SESSION_ID,
    origin: "http://localhost:4173",
    focusEpoch: 2,
    revision: 3,
    monotonicMs: 1_000,
    context: {
      fingerprint: "0123456789abcdef",
      before: "Hello",
      after: "",
      selection: { start: 5, end: 5, direction: "none" },
      field: {
        purpose: "normal",
        editable: true,
        multiline: true,
        composing: false,
        sensitive: false,
        identityKnown: true,
      },
      activation: "always",
      explicit: false,
    },
  };
}

describe("RuntimeSuggestionTransport", () => {
  it("serializes a rapid blur close before the refocused session's next request", async () => {
    const messages: unknown[] = [];
    const closeReply = new DeferredReply();
    const transport = new RuntimeSuggestionTransport({
      sendMessage: (message) => {
        messages.push(message);
        if (
          typeof message === "object" &&
          message !== null &&
          "kind" in message &&
          message.kind === "badi.session.close.v1"
        ) {
          return closeReply.promise;
        }
        const sent = request();
        return Promise.resolve({
          ok: true,
          response: {
            requestId: sent.requestId,
            sessionId: sent.sessionId,
            focusEpoch: sent.focusEpoch,
            revision: sent.revision,
            fingerprint: sent.context.fingerprint,
            suggestion: " world",
            suggestionId: "suggestion-1",
            acceptWord: " world",
            ttlMs: 600,
          },
        });
      },
    });

    const close = transport.closeSession(SESSION_ID);
    const suggestion = transport.requestSuggestion(request());
    await Promise.resolve();
    expect(messages).toEqual([{ kind: "badi.session.close.v1", sessionId: SESSION_ID }]);

    closeReply.resolve({ ok: true });
    await close;
    await expect(suggestion).resolves.toMatchObject({ suggestion: " world" });
    expect(messages).toHaveLength(2);
    expect(messages[1]).toMatchObject({ kind: "badi.suggest.v1" });
  });

  it("returns the broker's content-free paused bootstrap state", async () => {
    const messages: unknown[] = [];
    const transport = new RuntimeSuggestionTransport({
      sendMessage: (message) => {
        messages.push(message);
        return Promise.resolve({ ok: true, paused: true, policy: policy() });
      },
    });

    await expect(transport.bootstrap(SESSION_ID)).resolves.toEqual({
      paused: true,
      policy: policy(),
    });
    expect(messages).toEqual([
      { kind: "badi.bootstrap.v1", sessionId: SESSION_ID },
    ]);
  });
});
