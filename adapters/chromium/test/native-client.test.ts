import { describe, expect, it, vi } from "vitest";
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

function commitRequest() {
  return {
    requestId: "request-1",
    sessionId: request().sessionId,
    focusEpoch: 7,
    revision: 9,
    monotonicMs: 1_002,
    fingerprint: "0123456789abcdef",
    suggestionId: "suggestion-1",
    expectedText: " world",
    acceptance: "all" as const,
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
        "policy",
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

function policyStatus(
  id: string,
  paused = false,
  authorityEpoch = 4,
  settingsRevision = 2,
): unknown {
  return {
    v: 1,
    id,
    type: "policy.status",
    mono_ms: 1_001,
    payload: {
      authority_epoch: authorityEpoch,
      settings_revision: settingsRevision,
      paused,
      activation: paused ? "never" : "always",
      context_allowed: !paused,
      display_allowed: !paused,
      suggestions_allowed: !paused,
      learning_allowed: false,
      reason: paused ? "global_disabled" : "matched_rule",
    },
  };
}

function authorityChanged(
  authorityEpoch: number,
  paused = false,
  settingsRevision = 2,
): unknown {
  return {
    v: 1,
    type: "authority.changed",
    mono_ms: 1_002,
    payload: {
      authority_epoch: authorityEpoch,
      settings_revision: settingsRevision,
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
    const pending = client.bootstrap();
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
    await expect(pending).resolves.toBe(true);
    expect(port.posted).toHaveLength(1);
    client.dispose();
  });

  it("does not transmit session context when the hello state is already paused", async () => {
    const port = new FakeNativePort();
    const client = new NativeBrokerClient(
      { connectNative: () => port },
      { now: () => 1_000, handshakeTimeoutMs: 10_000 },
    );
    const pending = client.requestSuggestion(request());
    port.onMessage.emit(helloAck(true));

    await expect(pending).rejects.toThrow("Broker is paused");
    expect(port.posted).toEqual([expect.objectContaining({ type: "hello" })]);
    client.dispose();
  });

  it("resolves trusted document policy before returning bootstrap state", async () => {
    const port = new FakeNativePort();
    const client = new NativeBrokerClient(
      { connectNative: () => port },
      { now: () => 1_000, handshakeTimeoutMs: 10_000 },
    );
    const pending = client.bootstrap(request().sessionId, request().origin);
    port.onMessage.emit(helloAck());
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    const query = port.posted.find(
      (message) =>
        typeof message === "object" &&
        message !== null &&
        "type" in message &&
        message.type === "policy.query",
    );
    if (
      typeof query !== "object" ||
      query === null ||
      !("id" in query) ||
      typeof query.id !== "string"
    ) {
      throw new Error("Policy query missing");
    }
    expect(query).toMatchObject({
      payload: {
        target: {
          kind: "browser",
          app_id: "chromium",
          target_id: request().sessionId,
          origin: { scheme: "https", host: "fixture.test", port: 8443 },
        },
      },
    });
    port.onMessage.emit(policyStatus(query.id));
    await expect(pending).resolves.toMatchObject({
      paused: false,
      policy: { authorityEpoch: 4, reason: "matched_rule" },
    });
    client.dispose();
  });

  it("handles an authority event even when its policy response arrived first", async () => {
    const port = new FakeNativePort();
    const client = new NativeBrokerClient(
      { connectNative: () => port },
      { now: () => 1_000, handshakeTimeoutMs: 10_000 },
    );
    const ready = client.bootstrap();
    port.onMessage.emit(helloAck());
    await ready;

    const earlyPolicy = client.resolvePolicy(request().sessionId, request().origin);
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    const query = [...port.posted].reverse().find(
      (message) =>
        typeof message === "object" &&
        message !== null &&
        "type" in message &&
        message.type === "policy.query",
    );
    if (
      typeof query !== "object" ||
      query === null ||
      !("id" in query) ||
      typeof query.id !== "string"
    ) {
      throw new Error("Policy query missing");
    }
    port.onMessage.emit(policyStatus(query.id, true, 5, 3));
    await expect(earlyPolicy).resolves.toMatchObject({ authorityEpoch: 5 });

    let finishRefresh!: () => void;
    const refresh = new Promise<void>((resolve) => {
      finishRefresh = resolve;
    });
    client.setAuthorityChangedHandler(() => refresh);
    port.onMessage.emit({
      v: 1,
      type: "authority.changed",
      mono_ms: 1_002,
      payload: { authority_epoch: 5, settings_revision: 3, paused: true },
    });
    await Promise.resolve();
    expect(
      port.posted.some(
        (message) =>
          typeof message === "object" &&
          message !== null &&
          "type" in message &&
          message.type === "authority.ack",
      ),
    ).toBe(false);

    finishRefresh();
    await refresh;
    await Promise.resolve();
    expect(port.posted.at(-1)).toMatchObject({
      type: "authority.ack",
      payload: { authority_epoch: 5 },
    });
    client.dispose();
  });

  it("does not dispatch context until queued authority handling is acknowledged", async () => {
    const port = new FakeNativePort();
    const client = new NativeBrokerClient(
      { connectNative: () => port },
      { now: () => 1_000, handshakeTimeoutMs: 10_000 },
    );
    const ready = client.bootstrap();
    port.onMessage.emit(helloAck());
    await ready;

    let finishAuthority!: () => void;
    const authorityGate = new Promise<void>((resolve) => {
      finishAuthority = resolve;
    });
    client.setAuthorityChangedHandler(() => authorityGate);
    port.onMessage.emit(authorityChanged(5));

    const suggestion = client.requestSuggestion(request());
    await Promise.resolve();
    expect(
      port.posted.some(
        (message) =>
          typeof message === "object" &&
          message !== null &&
          "type" in message &&
          message.type === "context.changed",
      ),
    ).toBe(false);

    finishAuthority();
    await authorityGate;
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    const types = port.posted.map((message) =>
      typeof message === "object" && message !== null && "type" in message
        ? message.type
        : null,
    );
    expect(types.indexOf("authority.ack")).toBeGreaterThan(-1);
    expect(types.indexOf("context.changed")).toBeGreaterThan(types.indexOf("authority.ack"));

    client.dispose();
    await expect(suggestion).rejects.toThrow("disposed");
  });

  it("accepts a fresh epoch after reconnect and never ACKs old authority on the new port", async () => {
    const firstPort = new FakeNativePort();
    const secondPort = new FakeNativePort();
    const ports = [firstPort, secondPort];
    const client = new NativeBrokerClient(
      {
        connectNative: () => {
          const port = ports.shift();
          if (port === undefined) throw new Error("Unexpected native reconnect");
          return port;
        },
      },
      { now: () => 1_000, handshakeTimeoutMs: 10_000 },
    );
    const firstReady = client.bootstrap();
    firstPort.onMessage.emit(helloAck());
    await firstReady;

    let finishFirst!: () => void;
    const firstGate = new Promise<void>((resolve) => {
      finishFirst = resolve;
    });
    let authorityCalls = 0;
    client.setAuthorityChangedHandler(() => {
      authorityCalls += 1;
      return authorityCalls === 1 ? firstGate : Promise.resolve();
    });
    firstPort.onMessage.emit(authorityChanged(5, true, 3));
    await Promise.resolve();
    firstPort.onDisconnect.emit(firstPort);

    const secondReady = client.bootstrap();
    secondPort.onMessage.emit(helloAck());
    await secondReady;
    secondPort.onMessage.emit(authorityChanged(0, false, 3));
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    expect(secondPort.posted).toContainEqual(
      expect.objectContaining({
        type: "authority.ack",
        payload: expect.objectContaining({ authority_epoch: 0 }),
      }),
    );

    finishFirst();
    await firstGate;
    await Promise.resolve();
    expect(
      firstPort.posted.some(
        (message) =>
          typeof message === "object" &&
          message !== null &&
          "type" in message &&
          message.type === "authority.ack",
      ),
    ).toBe(false);
    expect(
      secondPort.posted.filter(
        (message) =>
          typeof message === "object" &&
          message !== null &&
          "type" in message &&
          message.type === "authority.ack",
      ),
    ).toHaveLength(1);
    client.dispose();
  });

  it("disconnects the native port when the hello handshake times out", async () => {
    vi.useFakeTimers();
    try {
      const port = new FakeNativePort();
      const client = new NativeBrokerClient(
        { connectNative: () => port },
        { now: () => 1_000, handshakeTimeoutMs: 20 },
      );
      const pending = client.bootstrap();
      const rejected = expect(pending).rejects.toThrow("handshake timed out");
      await vi.advanceTimersByTimeAsync(20);

      expect(port.disconnected).toBe(true);
      await rejected;
    } finally {
      vi.useRealTimers();
    }
  });

  it("bounds a silent post-handshake suggestion operation", async () => {
    vi.useFakeTimers();
    try {
      const port = new FakeNativePort();
      const client = new NativeBrokerClient(
        { connectNative: () => port },
        {
          now: () => 1_000,
          handshakeTimeoutMs: 10_000,
          operationTimeoutMs: 20,
        },
      );
      const ready = client.bootstrap();
      port.onMessage.emit(helloAck());
      await ready;

      const pending = client.requestSuggestion(request());
      const rejected = pending.catch((error: unknown) => error);
      await vi.advanceTimersByTimeAsync(0);
      expect(vi.getTimerCount()).toBe(1);
      await vi.advanceTimersByTimeAsync(20);

      const error = await rejected;
      expect(error).toBeInstanceOf(Error);
      expect((error as Error).message).toContain("suggestion operation timed out");
      expect(vi.getTimerCount()).toBe(0);
      client.dispose();
    } finally {
      vi.useRealTimers();
    }
  });

  it("settles a terminal suggestion clear without waiting for the operation timeout", async () => {
    vi.useFakeTimers();
    try {
      const port = new FakeNativePort();
      const client = new NativeBrokerClient(
        { connectNative: () => port },
        {
          now: () => 1_000,
          handshakeTimeoutMs: 10_000,
          operationTimeoutMs: 3_000,
        },
      );
      const ready = client.bootstrap();
      port.onMessage.emit(helloAck());
      await ready;

      const pending = client.requestSuggestion(request());
      await vi.advanceTimersByTimeAsync(0);
      expect(vi.getTimerCount()).toBe(1);
      port.onMessage.emit({
        v: 1,
        id: "request-1",
        type: "suggestion.clear",
        session_id: request().sessionId,
        focus_epoch: 7,
        revision: 9,
        mono_ms: 1_001,
        payload: {
          fingerprint: "0123456789abcdef",
          reason: "invalid_output",
        },
      });

      await expect(pending).resolves.toMatchObject({ suggestion: null, suggestionId: null });
      expect(vi.getTimerCount()).toBe(0);
      client.dispose();
    } finally {
      vi.useRealTimers();
    }
  });

  it("bounds a silent post-handshake commit operation", async () => {
    vi.useFakeTimers();
    try {
      const port = new FakeNativePort();
      const client = new NativeBrokerClient(
        { connectNative: () => port },
        {
          now: () => 1_000,
          handshakeTimeoutMs: 10_000,
          operationTimeoutMs: 20,
        },
      );
      const ready = client.bootstrap();
      port.onMessage.emit(helloAck());
      await ready;

      const pending = client.authorizeCommit(commitRequest());
      const rejected = pending.catch((error: unknown) => error);
      await vi.advanceTimersByTimeAsync(0);
      expect(vi.getTimerCount()).toBe(1);
      await vi.advanceTimersByTimeAsync(20);

      const error = await rejected;
      expect(error).toBeInstanceOf(Error);
      expect((error as Error).message).toContain("commit operation timed out");
      expect(vi.getTimerCount()).toBe(0);
      client.dispose();
    } finally {
      vi.useRealTimers();
    }
  });

  it("clears operation deadlines when suggestion and commit replies settle", async () => {
    vi.useFakeTimers();
    try {
      const port = new FakeNativePort();
      const client = new NativeBrokerClient(
        { connectNative: () => port },
        {
          now: () => 1_000,
          handshakeTimeoutMs: 10_000,
          operationTimeoutMs: 20,
        },
      );
      const ready = client.bootstrap();
      port.onMessage.emit(helloAck());
      await ready;

      const suggestion = client.requestSuggestion(request());
      await vi.advanceTimersByTimeAsync(0);
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
      await expect(suggestion).resolves.toMatchObject({ suggestionId: "suggestion-1" });
      expect(vi.getTimerCount()).toBe(0);

      const authorization = client.authorizeCommit(commitRequest());
      await vi.advanceTimersByTimeAsync(0);
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
      await vi.advanceTimersByTimeAsync(0);

      await expect(authorization).resolves.toMatchObject({ acceptance: "all" });
      expect(vi.getTimerCount()).toBe(0);
      client.dispose();
    } finally {
      vi.useRealTimers();
    }
  });

  it("clears operation deadlines when a session closes and when the client resets", async () => {
    vi.useFakeTimers();
    try {
      const port = new FakeNativePort();
      const client = new NativeBrokerClient(
        { connectNative: () => port },
        {
          now: () => 1_000,
          handshakeTimeoutMs: 10_000,
          operationTimeoutMs: 20,
        },
      );
      const ready = client.bootstrap();
      port.onMessage.emit(helloAck());
      await ready;

      const closing = client.requestSuggestion(request());
      const closed = closing.catch((error: unknown) => error);
      await vi.advanceTimersByTimeAsync(0);
      await client.closeSession(request().sessionId);
      const closeError = await closed;
      expect(closeError).toBeInstanceOf(Error);
      expect((closeError as Error).message).toContain("session closed");
      await vi.advanceTimersByTimeAsync(0);
      expect(vi.getTimerCount()).toBe(0);

      const nextRequest = request("request-2", 10, "fedcba9876543210");
      const suggestion = client.requestSuggestion(nextRequest);
      const commit = client.authorizeCommit({
        ...commitRequest(),
        requestId: nextRequest.requestId,
        revision: nextRequest.revision,
        fingerprint: nextRequest.context.fingerprint,
      });
      const suggestionRejected = suggestion.catch((error: unknown) => error);
      const commitRejected = commit.catch((error: unknown) => error);
      await vi.advanceTimersByTimeAsync(0);
      expect(vi.getTimerCount()).toBe(2);

      client.dispose();
      const suggestionError = await suggestionRejected;
      const commitError = await commitRejected;
      expect(suggestionError).toBeInstanceOf(Error);
      expect((suggestionError as Error).message).toContain("disposed");
      expect(commitError).toBeInstanceOf(Error);
      expect((commitError as Error).message).toContain("disposed");
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("coalesces identical global controls, rejects incompatible overlap, and bounds silence", async () => {
    vi.useFakeTimers();
    try {
      const port = new FakeNativePort();
      const client = new NativeBrokerClient(
        { connectNative: () => port },
        {
          now: () => 1_000,
          handshakeTimeoutMs: 10_000,
          operationTimeoutMs: 20,
        },
      );
      const ready = client.bootstrap();
      port.onMessage.emit(helloAck());
      await ready;

      const first = client.globalControl("pause");
      const duplicate = client.globalControl("pause");
      const incompatible = client.globalControl("resume");
      const incompatibleRejected = incompatible.catch((error: unknown) => error);
      const firstRejected = first.catch((error: unknown) => error);
      const duplicateRejected = duplicate.catch((error: unknown) => error);
      await vi.advanceTimersByTimeAsync(0);

      const incompatibleError = await incompatibleRejected;
      expect(incompatibleError).toBeInstanceOf(Error);
      expect((incompatibleError as Error).message).toContain(
        "pause is already in progress",
      );
      expect(
        port.posted.filter(
          (message) =>
            typeof message === "object" &&
            message !== null &&
            "type" in message &&
            message.type === "control.request",
        ),
      ).toHaveLength(1);
      expect(vi.getTimerCount()).toBe(1);

      await vi.advanceTimersByTimeAsync(20);
      const firstError = await firstRejected;
      const duplicateError = await duplicateRejected;
      expect(firstError).toBeInstanceOf(Error);
      expect((firstError as Error).message).toContain(
        "global control operation timed out",
      );
      expect(duplicateError).toBeInstanceOf(Error);
      expect((duplicateError as Error).message).toContain(
        "global control operation timed out",
      );
      expect(vi.getTimerCount()).toBe(0);

      const retry = client.globalControl("resume");
      await vi.advanceTimersByTimeAsync(0);
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
        throw new Error("Global control retry missing");
      }
      port.onMessage.emit({
        v: 1,
        id: control.id,
        type: "control.result",
        mono_ms: 1_001,
        payload: {
          action: "resume",
          accepted: true,
          reason: "accepted",
          paused: false,
        },
      });

      await expect(retry).resolves.toBe(false);
      expect(vi.getTimerCount()).toBe(0);
      client.dispose();
    } finally {
      vi.useRealTimers();
    }
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

  it("closes an opened session and emits a fresh open when that session is reused", async () => {
    const port = new FakeNativePort();
    const client = new NativeBrokerClient(
      { connectNative: () => port },
      { now: () => 1_000, handshakeTimeoutMs: 10_000 },
    );
    const first = client.requestSuggestion(request());
    port.onMessage.emit(helloAck());
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    const firstRejected = expect(first).rejects.toThrow("session closed");

    await client.closeSession(request().sessionId);
    await firstRejected;
    expect(port.posted.at(-1)).toMatchObject({
      type: "session.close",
      session_id: request().sessionId,
      focus_epoch: 7,
      revision: 9,
      payload: { reason: "session_closed" },
    });

    const secondRequest = request("request-2", 10, "fedcba9876543210");
    const second = client.requestSuggestion(secondRequest);
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    const closeIndex = port.posted.findIndex(
      (message) =>
        typeof message === "object" &&
        message !== null &&
        "type" in message &&
        message.type === "session.close",
    );
    const reopenedIndex = port.posted.findIndex(
      (message, index) =>
        index > closeIndex &&
        typeof message === "object" &&
        message !== null &&
        "type" in message &&
        message.type === "session.open",
    );
    expect(reopenedIndex).toBeGreaterThan(closeIndex);

    port.onMessage.emit({
      v: 1,
      id: secondRequest.requestId,
      type: "suggestion.show",
      session_id: secondRequest.sessionId,
      focus_epoch: secondRequest.focusEpoch,
      revision: secondRequest.revision,
      mono_ms: 1_010,
      payload: {
        fingerprint: secondRequest.context.fingerprint,
        suggestion_id: "suggestion-2",
        text: " reopened",
        accept_word: " reopened",
        ttl_ms: 600,
        provider: "phrase_v1",
      },
    });
    await expect(second).resolves.toMatchObject({ suggestion: " reopened" });
    client.dispose();
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
