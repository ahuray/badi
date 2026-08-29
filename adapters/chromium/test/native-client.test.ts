import { describe, expect, it } from "vitest";
import { NativeBrokerClient, type NativeEvent, type NativePortLike } from "../src/background/native-client";
import { NATIVE_HOST_NAME } from "../src/background/protocol-mapper";
import type { SuggestionRequest } from "../src/shared/model";

class FakeEvent<T> implements NativeEvent<T> {
  readonly listeners = new Set<(value: T) => void>();

  addListener(listener: (value: T) => void): void {
    this.listeners.add(listener);
  }

  removeListener(listener: (value: T) => void): void {
    this.listeners.delete(listener);
  }

  emit(value: T): void {
    for (const listener of this.listeners) listener(value);
  }
}

class FakeNativePort implements NativePortLike {
  readonly onMessage = new FakeEvent<unknown>();
  readonly onDisconnect = new FakeEvent<NativePortLike>();
  readonly posted: unknown[] = [];
  disconnected = false;

  postMessage(message: unknown): void {
    this.posted.push(message);
  }

  disconnect(): void {
    this.disconnected = true;
  }
}

function request(
  requestId = "request-1",
  revision = 9,
  fingerprint = "0123456789abcdef",
): SuggestionRequest {
  return {
    requestId,
    sessionId: "0198f215-3ec0-7000-8000-000000000001",
    origin: "https://fixture.test:8443",
    focusEpoch: 7,
    revision,
    monotonicMs: 1_000,
    context: {
      fingerprint,
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

function helloAck(paused = false): unknown {
  return {
    v: 1,
    id: "chromium.hello",
    type: "hello.ack",
    mono_ms: 1_000,
    payload: {
      selected_v: 1,
      connection_id: "c:test-connection",
      enabled_capabilities: [
        "context",
        "suggestion",
        "commit.dispatched_unverified",
        "control",
        "health",
      ],
      max_frame_bytes: 65_536,
      max_before_chars: 512,
      max_after_chars: 128,
      max_suggestion_chars: 64,
      max_suggestion_words: 8,
      paused,
    },
  };
}

describe("NativeBrokerClient", () => {
  it("does not become ready from a partial hello acknowledgment", async () => {
    const port = new FakeNativePort();
    const client = new NativeBrokerClient(
      { connectNative: () => port },
      { now: () => 1_000, handshakeTimeoutMs: 10_000 },
    );
    const pending = client.requestSuggestion(request());
    port.onMessage.emit({
      v: 1,
      id: "chromium.hello",
      type: "hello.ack",
      mono_ms: 1_000,
      payload: { selected_v: 1 },
    });
    await Promise.resolve();
    expect(port.posted).toHaveLength(1);

    port.onMessage.emit(helloAck(true));
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    expect(
      port.posted.some(
        (message) =>
          typeof message === "object" &&
          message !== null &&
          "type" in message &&
          message.type === "suggest.request",
      ),
    ).toBe(true);
    port.onMessage.emit({
      v: 1,
      id: "request-1",
      type: "error",
      mono_ms: 1_001,
      payload: { code: "paused" },
    });
    await expect(pending).rejects.toThrow("Broker rejected suggestion request");
    client.dispose();
  });

  it.each([
    ["session.open", "request-1.open"],
    ["context.changed", "request-1.context"],
    ["suggest.request", "request-1"],
  ])("settles a pending suggestion on a content-free %s broker error", async (_phase, id) => {
    const port = new FakeNativePort();
    const client = new NativeBrokerClient(
      { connectNative: () => port },
      { now: () => 1_000, handshakeTimeoutMs: 10_000 },
    );
    const pending = client.requestSuggestion(request());
    port.onMessage.emit(helloAck());
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    port.onMessage.emit({
      v: 1,
      id,
      type: "error",
      mono_ms: 1_001,
      payload: { code: "paused" },
    });

    await expect(pending).rejects.toThrow("Broker rejected suggestion request");
    client.dispose();
  });

  it("does not reject a different pending request on another request's error", async () => {
    const port = new FakeNativePort();
    const client = new NativeBrokerClient(
      { connectNative: () => port },
      { now: () => 1_000, handshakeTimeoutMs: 10_000 },
    );
    const first = client.requestSuggestion(request());
    port.onMessage.emit(helloAck());
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    const secondRequest = request("request-2", 10, "fedcba9876543210");
    let secondSettled = false;
    const second = client.requestSuggestion(secondRequest).then((response) => {
      secondSettled = true;
      return response;
    });
    await new Promise<void>((resolve) => setTimeout(resolve, 0));

    port.onMessage.emit({
      v: 1,
      id: "request-1.context",
      type: "error",
      mono_ms: 1_001,
      payload: { code: "no_context" },
    });
    await expect(first).rejects.toThrow("Broker rejected suggestion request");
    await Promise.resolve();
    expect(secondSettled).toBe(false);

    port.onMessage.emit({
      v: 1,
      id: "request-2",
      type: "suggestion.show",
      session_id: secondRequest.sessionId,
      focus_epoch: secondRequest.focusEpoch,
      revision: secondRequest.revision,
      mono_ms: 1_002,
      payload: {
        fingerprint: secondRequest.context.fingerprint,
        suggestion_id: "suggestion-2",
        text: " safe",
        accept_word: " safe",
        ttl_ms: 600,
        provider: "phrase_v1",
      },
    });
    await expect(second).resolves.toMatchObject({ suggestionId: "suggestion-2" });
    client.dispose();
  });

  it("targets only the registered host and maps request/reply without a live daemon", async () => {
    const port = new FakeNativePort();
    const connectedHosts: string[] = [];
    const client = new NativeBrokerClient(
      {
        connectNative: (hostName) => {
          connectedHosts.push(hostName);
          return port;
        },
      },
      { now: () => 1_000, handshakeTimeoutMs: 10_000 },
    );

    const pending = client.requestSuggestion(request());
    expect(connectedHosts).toEqual([NATIVE_HOST_NAME]);
    expect(port.posted).toHaveLength(1);
    expect(port.posted[0]).toMatchObject({ type: "hello", v: 1 });

    port.onMessage.emit(helloAck());
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    expect(port.posted.slice(1)).toMatchObject([
      { type: "session.open", session_id: request().sessionId },
      { type: "context.changed", payload: { before: "Hello" } },
      { type: "suggest.request", id: "request-1" },
    ]);

    port.onMessage.emit({
      v: 1,
      id: "request-1",
      type: "suggestion.show",
      session_id: request().sessionId,
      focus_epoch: 7,
      revision: 9,
      mono_ms: 1_001,
      payload: {
        fingerprint: "0123456789abcdef",
        suggestion_id: "suggestion-1",
        text: " world",
        accept_word: " world",
        ttl_ms: 600,
        provider: "phrase_v1",
      },
    });

    await expect(pending).resolves.toMatchObject({
      requestId: "request-1",
      suggestion: " world",
      suggestionId: "suggestion-1",
    });

    const authorization = client.authorizeCommit({
      requestId: "request-1",
      sessionId: request().sessionId,
      focusEpoch: 7,
      revision: 9,
      monotonicMs: 1_002,
      fingerprint: "0123456789abcdef",
      suggestionId: "suggestion-1",
      expectedText: " world",
      acceptance: "all",
    });
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    const outboundTypes = port.posted.map((message) =>
      typeof message === "object" && message !== null && "type" in message
        ? message.type
        : null,
    );
    expect(outboundTypes).toContain("control.request");
    expect(outboundTypes).not.toContain("commit.prepare");
    port.onMessage.emit({
      v: 1,
      id: "request-1.9.accept_all",
      type: "commit.prepare",
      session_id: request().sessionId,
      focus_epoch: 7,
      revision: 9,
      mono_ms: 1_003,
      payload: {
        fingerprint: "0123456789abcdef",
        suggestion_id: "suggestion-1",
        text: " world",
        acceptance: "all",
      },
    });
    await expect(authorization).resolves.toMatchObject({
      text: " world",
      acceptance: "all",
    });

    const pauseResult = client.globalControl("pause_toggle");
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    const pauseRequest = [...port.posted].reverse().find(
      (message) =>
        typeof message === "object" &&
        message !== null &&
        "type" in message &&
        message.type === "control.request" &&
        "session_id" in message === false,
    );
    if (
      typeof pauseRequest !== "object" ||
      pauseRequest === null ||
      !("id" in pauseRequest) ||
      typeof pauseRequest.id !== "string"
    ) {
      throw new Error("Global pause request missing");
    }
    port.onMessage.emit({
      v: 1,
      id: pauseRequest.id,
      type: "control.result",
      mono_ms: 1_004,
      payload: {
        action: "pause_toggle",
        accepted: true,
        reason: "accepted",
        paused: true,
      },
    });
    await expect(pauseResult).resolves.toBe(true);
    client.dispose();
    expect(port.disconnected).toBe(true);
  });

  it("ignores a reply with stale coordinates even when the id matches", async () => {
    const port = new FakeNativePort();
    const client = new NativeBrokerClient(
      { connectNative: () => port },
      { now: () => 1_000, handshakeTimeoutMs: 10_000 },
    );
    let settled = false;
    const pending = client.requestSuggestion(request()).then(() => {
      settled = true;
    });
    port.onMessage.emit(helloAck());
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    expect(port.posted.some((message) =>
      typeof message === "object" &&
      message !== null &&
      "type" in message &&
      message.type === "suggest.request"
    )).toBe(true);
    port.onMessage.emit({
      v: 1,
      id: "request-1",
      type: "suggestion.show",
      session_id: request().sessionId,
      focus_epoch: 7,
      revision: 8,
      mono_ms: 1_001,
      payload: {
        fingerprint: "0123456789abcdef",
        suggestion_id: "stale",
        text: " stale",
        accept_word: " stale",
        ttl_ms: 600,
        provider: "phrase_v1",
      },
    });
    await Promise.resolve();
    expect(settled).toBe(false);
    client.dispose();
    await expect(pending).rejects.toThrow("disposed");
  });

  it("treats a same-id suggestion.clear as revocation during the grant fence", async () => {
    const port = new FakeNativePort();
    const client = new NativeBrokerClient(
      { connectNative: () => port },
      { now: () => 1_000, handshakeTimeoutMs: 10_000 },
    );
    const commitRequest = {
      requestId: "request-revoked",
      sessionId: request().sessionId,
      focusEpoch: 7,
      revision: 9,
      monotonicMs: 1_010,
      fingerprint: "0123456789abcdef",
      suggestionId: "suggestion-revoked",
      expectedText: " blocked",
      acceptance: "all" as const,
    };
    const authorization = client.authorizeCommit(commitRequest);
    port.onMessage.emit(helloAck());
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    const correlationId = "request-revoked.9.accept_all";
    port.onMessage.emit({
      v: 1,
      id: correlationId,
      type: "commit.prepare",
      session_id: request().sessionId,
      focus_epoch: 7,
      revision: 9,
      mono_ms: 1_011,
      payload: {
        fingerprint: "0123456789abcdef",
        suggestion_id: "suggestion-revoked",
        text: " blocked",
        acceptance: "all",
      },
    });
    port.onMessage.emit({
      v: 1,
      id: correlationId,
      type: "suggestion.clear",
      session_id: request().sessionId,
      focus_epoch: 7,
      revision: 9,
      mono_ms: 1_012,
      payload: {
        fingerprint: "0123456789abcdef",
        suggestion_id: "suggestion-revoked",
        reason: "paused",
      },
    });
    await expect(authorization).rejects.toThrow("revoked");
    client.dispose();
  });

  it("delivers addressed broker clears after the original show promise settled", async () => {
    const port = new FakeNativePort();
    const client = new NativeBrokerClient(
      { connectNative: () => port },
      { now: () => 1_000, handshakeTimeoutMs: 10_000 },
    );
    const clears: unknown[] = [];
    client.setSuggestionClearHandler((event) => clears.push(event));
    const pending = client.requestSuggestion(request());
    port.onMessage.emit(helloAck());
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    port.onMessage.emit({
      v: 1,
      id: "request-1",
      type: "suggestion.show",
      session_id: request().sessionId,
      focus_epoch: 7,
      revision: 9,
      mono_ms: 1_001,
      payload: {
        fingerprint: "0123456789abcdef",
        suggestion_id: "suggestion-1",
        text: " world",
        accept_word: " world",
        ttl_ms: 600,
        provider: "phrase_v1",
      },
    });
    await expect(pending).resolves.toMatchObject({ suggestionId: "suggestion-1" });

    port.onMessage.emit({
      v: 1,
      type: "suggestion.clear",
      session_id: request().sessionId,
      focus_epoch: 7,
      revision: 9,
      mono_ms: 1_100,
      payload: {
        fingerprint: "0123456789abcdef",
        suggestion_id: "suggestion-1",
        reason: "expired",
      },
    });
    expect(clears).toEqual([
      {
        requestId: null,
        sessionId: request().sessionId,
        focusEpoch: 7,
        revision: 9,
        monotonicMs: 1_100,
        fingerprint: "0123456789abcdef",
        suggestionId: "suggestion-1",
        reason: "expired",
      },
    ]);
    client.dispose();
  });

  it("rejects an unaccepted shortcut control instead of treating paused as new state", async () => {
    const port = new FakeNativePort();
    const client = new NativeBrokerClient(
      { connectNative: () => port },
      { now: () => 1_000, handshakeTimeoutMs: 10_000 },
    );
    const result = client.globalControl("pause_toggle");
    port.onMessage.emit(helloAck());
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    const control = [...port.posted].reverse().find(
      (message) =>
        typeof message === "object" &&
        message !== null &&
        "type" in message &&
        message.type === "control.request",
    );
    if (
      typeof control !== "object" ||
      control === null ||
      !("id" in control) ||
      typeof control.id !== "string"
    ) {
      throw new Error("Global control request missing");
    }
    port.onMessage.emit({
      v: 1,
      id: control.id,
      type: "control.result",
      mono_ms: 1_001,
      payload: {
        action: "pause_toggle",
        accepted: false,
        reason: "paused",
        paused: true,
      },
    });
    await expect(result).rejects.toThrow("rejected global control");
    client.dispose();
  });

  it("notifies the worker when an established native port disconnects", async () => {
    const port = new FakeNativePort();
    const client = new NativeBrokerClient(
      { connectNative: () => port },
      { now: () => 1_000, handshakeTimeoutMs: 10_000 },
    );
    let disconnects = 0;
    client.setDisconnectHandler(() => {
      disconnects += 1;
    });
    const pending = client.requestSuggestion(request());
    port.onMessage.emit(helloAck());
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    port.onMessage.emit({
      v: 1,
      id: "request-1",
      type: "suggestion.show",
      session_id: request().sessionId,
      focus_epoch: 7,
      revision: 9,
      mono_ms: 1_001,
      payload: {
        fingerprint: "0123456789abcdef",
        suggestion_id: "suggestion-1",
        text: " shown",
        accept_word: " shown",
        ttl_ms: 600,
        provider: "phrase_v1",
      },
    });
    await expect(pending).resolves.toMatchObject({ suggestionId: "suggestion-1" });

    port.onDisconnect.emit(port);
    expect(disconnects).toBe(1);
  });
});
