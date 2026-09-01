import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import Ajv2020 from "ajv/dist/2020.js";
import { beforeAll, describe, expect, it } from "vitest";
import {
  acceptanceControlEnvelope,
  authorityAckEnvelope,
  cancelEnvelope,
  commitResultEnvelope,
  dismissEnvelope,
  globalControlEnvelope,
  helloEnvelope,
  isHelloAck,
  parseAuthorityChanged,
  parseHelloAckPaused,
  parsePolicyStatus,
  policyQueryEnvelope,
  sessionCloseEnvelope,
  sessionOpenEnvelope,
  suggestionRequestEnvelopes,
} from "../src/background/protocol-mapper";
import { contextFingerprint } from "../src/content/context";
import type { SuggestionRequest } from "../src/shared/model";

let validate: ReturnType<Ajv2020["compile"]>;

function request(): SuggestionRequest {
  return {
    requestId: "request-1",
    sessionId: "0198f215-3ec0-7000-8000-000000000001",
    origin: "https://fixture.test:8443",
    focusEpoch: 7,
    revision: 9,
    monotonicMs: 1_000,
    context: {
      fingerprint: "0123456789abcdef",
      before: "Hello",
      after: " there",
      language: "en-US",
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

describe("protocol v1 mapper", () => {
  beforeAll(async () => {
    const schemaPath = resolve(process.cwd(), "../../protocol/v1/schema.json");
    const schema = JSON.parse(await readFile(schemaPath, "utf8"));
    validate = new Ajv2020({ allErrors: true, strict: false }).compile(schema);
  });

  it("emits only schema-valid strict envelopes for the complete browser flow", () => {
    const current = request();
    const address = {
      requestId: current.requestId,
      sessionId: current.sessionId,
      focusEpoch: current.focusEpoch,
      revision: current.revision,
      monotonicMs: 1_001,
      fingerprint: current.context.fingerprint,
      suggestionId: "suggestion-1",
    } as const;
    const frames = [
      helloEnvelope(1_000),
      policyQueryEnvelope(current.sessionId, current.origin, 1_000, "policy-1"),
      authorityAckEnvelope(4, 1_000),
      sessionOpenEnvelope(current),
      sessionCloseEnvelope(current),
      ...suggestionRequestEnvelopes(current),
      cancelEnvelope(current),
      dismissEnvelope(address),
      acceptanceControlEnvelope({
        ...address,
        expectedText: " world",
        acceptance: "word",
      }),
      commitResultEnvelope({
        ...address,
        status: "dispatched-unverified",
      }),
      globalControlEnvelope("pause_toggle", 1_002),
    ];

    for (const frame of frames) {
      expect(validate(frame), JSON.stringify(validate.errors)).toBe(true);
    }
    expect(frames.some((frame) => frame.type === "commit.prepare")).toBe(false);
    const context = frames.find((frame) => frame.type === "context.changed");
    expect(context?.payload["language"]).toBe("en-US");
  });

  it("strictly maps pre-acquisition policy and authority messages", () => {
    const status = {
      v: 1,
      id: "policy-1",
      type: "policy.status",
      mono_ms: 1_001,
      payload: {
        authority_epoch: 4,
        settings_revision: 2,
        paused: false,
        activation: "always",
        context_allowed: true,
        display_allowed: true,
        suggestions_allowed: true,
        learning_allowed: false,
        reason: "matched_rule",
      },
    };
    expect(validate(status), JSON.stringify(validate.errors)).toBe(true);
    expect(parsePolicyStatus(status, "policy-1")).toEqual({
      authorityEpoch: 4,
      settingsRevision: 2,
      paused: false,
      activation: "always",
      contextAllowed: true,
      displayAllowed: true,
      suggestionsAllowed: true,
      learningAllowed: false,
      reason: "matched_rule",
    });
    expect(parsePolicyStatus({ ...status, leaked: true }, "policy-1")).toBeNull();
    expect(
      parsePolicyStatus(
        {
          ...status,
          payload: { ...status.payload, context_allowed: false },
        },
        "policy-1",
      ),
    ).toBeNull();

    const changed = {
      v: 1,
      type: "authority.changed",
      mono_ms: 1_002,
      payload: {
        authority_epoch: 5,
        settings_revision: 3,
        paused: true,
      },
    };
    expect(validate(changed), JSON.stringify(validate.errors)).toBe(true);
    expect(parseAuthorityChanged(changed)).toEqual({
      authorityEpoch: 5,
      settingsRevision: 3,
      paused: true,
    });
    expect(parseAuthorityChanged({ ...changed, id: "unsolicited" })).toBeNull();
  });

  it("keeps full-target UTF-16 offsets independent of capped context strings", () => {
    const current = request();
    const [context] = suggestionRequestEnvelopes({
      ...current,
      context: {
        ...current.context,
        before: "🙂".repeat(512),
        after: "界".repeat(128),
        selection: { start: 1_205, end: 1_205, direction: "none" },
      },
    });
    expect(context.payload["selection"]).toEqual({
      anchor: 1_205,
      head: 1_205,
      unit: "utf16_code_units",
    });
    expect(validate(context), JSON.stringify(validate.errors)).toBe(true);
  });

  it("emits the shared context fingerprint within the wire contract", () => {
    const current = request();
    const fingerprint = contextFingerprint(
      `${current.sessionId}\u001finmemory://model/1\u001fthank you`,
    );
    const [context] = suggestionRequestEnvelopes({
      ...current,
      context: { ...current.context, fingerprint },
    });

    expect(fingerprint).toMatch(/^[a-f0-9]{32}$/u);
    expect(context.payload["fingerprint"]).toBe(fingerprint);
    expect(validate(context), JSON.stringify(validate.errors)).toBe(true);
    expect(
      validate({
        ...context,
        payload: { ...context.payload, fingerprint: fingerprint.slice(0, 8) },
      }),
    ).toBe(false);
  });

  it("rejects lone context surrogates while allowing controls and astral pairs", () => {
    const [base] = suggestionRequestEnvelopes(request());
    const withContext = (before: string, after: string) => ({
      ...base,
      payload: { ...base.payload, before, after },
    });

    expect(
      validate(withContext("line one\nline two\u0000🙂", "\tvalid")),
      JSON.stringify(validate.errors),
    ).toBe(true);
    expect(validate(withContext(`bad\uD800`, ""))).toBe(false);
    expect(validate(withContext("", `bad\uDFFF`))).toBe(false);
  });

  it("normatively rejects lone UTF-16 surrogates in suggestion output", () => {
    const show = (text: string) => ({
      v: 1,
      id: "request-1",
      type: "suggestion.show",
      session_id: request().sessionId,
      focus_epoch: 7,
      revision: 9,
      mono_ms: 1_001,
      payload: {
        fingerprint: request().context.fingerprint,
        suggestion_id: "suggestion-1",
        text,
        accept_word: text,
        ttl_ms: 600,
        provider: "phrase_v1",
      },
    });

    expect(validate(show(" safe")), JSON.stringify(validate.errors)).toBe(true);
    expect(validate(show("\uD800"))).toBe(false);
    expect(validate(show("\uDFFF"))).toBe(false);
  });

  it("accepts only the complete exact Chromium hello negotiation", () => {
    const acknowledgment = {
      v: 1,
      id: "chromium.hello",
      type: "hello.ack",
      mono_ms: 11,
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
        paused: true,
      },
    };
    expect(isHelloAck(acknowledgment)).toBe(true);
    expect(parseHelloAckPaused(acknowledgment)).toBe(true);
    expect(
      parseHelloAckPaused({
        ...acknowledgment,
        payload: { ...acknowledgment.payload, paused: false },
      }),
    ).toBe(false);
    expect(
      isHelloAck({ v: 1, type: "hello.ack", mono_ms: 11, payload: { selected_v: 1 } }),
    ).toBe(false);
    expect(
      isHelloAck({
        ...acknowledgment,
        payload: {
          ...acknowledgment.payload,
          enabled_capabilities: acknowledgment.payload.enabled_capabilities.slice(0, -1),
        },
      }),
    ).toBe(false);
    expect(
      isHelloAck({
        ...acknowledgment,
        payload: { ...acknowledgment.payload, max_before_chars: 511 },
      }),
    ).toBe(false);
    expect(
      isHelloAck({
        ...acknowledgment,
        payload: { ...acknowledgment.payload, unexpected: true },
      }),
    ).toBe(false);
    expect(isHelloAck({ ...acknowledgment, id: "other.hello" })).toBe(false);
    expect(
      isHelloAck({
        ...acknowledgment,
        payload: { ...acknowledgment.payload, connection_id: "not canonical/opaque" },
      }),
    ).toBe(false);
    expect(isHelloAck({ ...acknowledgment, mono_ms: Number.MAX_SAFE_INTEGER + 1 })).toBe(
      false,
    );
  });
});
