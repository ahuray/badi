import type {
  CommitAuthorization,
  CommitAuthorizationRequest,
  CommitResultNotice,
  SuggestionAddress,
  SuggestionClearEvent,
  SuggestionRequest,
  SuggestionResponse,
} from "../shared/model";

export const NATIVE_HOST_NAME = "io.github.ahuray.badi";

const BROWSER_CAPABILITIES = [
  "context",
  "suggestion",
  "commit.dispatched_unverified",
  "control",
  "health",
] as const;

type Coordinates = Pick<
  SuggestionRequest,
  "sessionId" | "focusEpoch" | "revision" | "monotonicMs"
>;

interface WireBase {
  readonly v: 1;
  readonly id?: string;
  readonly type: string;
  readonly mono_ms: number;
  readonly payload: Readonly<Record<string, unknown>>;
}

export interface WireSessionEnvelope extends WireBase {
  readonly session_id: string;
  readonly focus_epoch: number;
  readonly revision: number;
}

export type WireEnvelope = WireBase | WireSessionEnvelope;

function withCoordinates(
  coordinates: Coordinates,
  type: string,
  payload: Readonly<Record<string, unknown>>,
  id?: string,
): WireSessionEnvelope {
  return {
    v: 1,
    ...(id === undefined ? {} : { id }),
    type,
    session_id: coordinates.sessionId,
    focus_epoch: coordinates.focusEpoch,
    revision: coordinates.revision,
    mono_ms: coordinates.monotonicMs,
    payload,
  };
}

function parseOrigin(origin: string): Readonly<Record<string, unknown>> {
  const parsed = new URL(origin);
  const scheme = parsed.protocol.slice(0, -1);
  if (scheme !== "http" && scheme !== "https") {
    throw new Error(`Unsupported page origin scheme: ${scheme}`);
  }
  const port = parsed.port.length === 0 ? undefined : Number(parsed.port);
  return {
    scheme,
    host: parsed.hostname,
    ...(port === undefined ? {} : { port }),
  };
}

export function helloEnvelope(monotonicMs: number): WireEnvelope {
  return {
    v: 1,
    id: "chromium.hello",
    type: "hello",
    mono_ms: monotonicMs,
    payload: {
      min_v: 1,
      max_v: 1,
      adapter: { kind: "browser", name: "badi-chromium", version: "0.1.0" },
      capabilities: [...BROWSER_CAPABILITIES],
    },
  };
}

export function sessionOpenEnvelope(request: SuggestionRequest): WireSessionEnvelope {
  return withCoordinates(
    request,
    "session.open",
    {
      target: {
        kind: "browser",
        app_id: "chromium",
        target_id: request.sessionId,
        origin: parseOrigin(request.origin),
      },
      activation: request.context.activation,
    },
    `${request.requestId}.open`,
  );
}

export function sessionCloseEnvelope(request: SuggestionRequest): WireSessionEnvelope {
  return withCoordinates(
    { ...request, monotonicMs: Math.max(0, Math.floor(performance.now())) },
    "session.close",
    { reason: "session_closed" },
    `${request.requestId}.${request.revision}.close`,
  );
}

export function suggestionRequestEnvelopes(
  request: SuggestionRequest,
): readonly [WireSessionEnvelope, WireSessionEnvelope] {
  const { selection } = request.context;
  const anchor = selection.direction === "backward" ? selection.end : selection.start;
  const head = selection.direction === "backward" ? selection.start : selection.end;
  const context = withCoordinates(
    request,
    "context.changed",
    {
      fingerprint: request.context.fingerprint,
      before: request.context.before,
      after: request.context.after,
      selection: { anchor, head, unit: "utf16_code_units" },
      field: {
        purpose: request.context.field.purpose,
        editable: request.context.field.editable,
        multiline: request.context.field.multiline,
        composing: request.context.field.composing,
        sensitive: request.context.field.sensitive,
        identity_known: request.context.field.identityKnown,
        focused: true,
        lock_screen: false,
      },
      activation: request.context.activation,
      explicit: request.context.explicit,
    },
    `${request.requestId}.context`,
  );
  const suggest = withCoordinates(
    request,
    "suggest.request",
    {
      fingerprint: request.context.fingerprint,
      explicit: request.context.explicit,
    },
    request.requestId,
  );
  return [context, suggest];
}

export function cancelEnvelope(request: SuggestionRequest): WireSessionEnvelope {
  return withCoordinates(
    { ...request, monotonicMs: Math.max(0, Math.floor(performance.now())) },
    "suggest.cancel",
    { fingerprint: request.context.fingerprint, reason: "superseded" },
    `${request.requestId}.cancel`,
  );
}

export function dismissEnvelope(address: SuggestionAddress): WireSessionEnvelope {
  return withCoordinates(
    address,
    "control.request",
    {
      action: "dismiss",
      fingerprint: address.fingerprint,
      suggestion_id: address.suggestionId,
    },
    `${address.requestId}.${address.revision}.dismiss`,
  );
}

export function commitControlId(request: CommitAuthorizationRequest): string {
  const action = request.acceptance === "word" ? "accept_word" : "accept_all";
  return `${request.requestId}.${request.revision}.${action}`;
}

export function acceptanceControlEnvelope(
  request: CommitAuthorizationRequest,
): WireSessionEnvelope {
  const action = request.acceptance === "word" ? "accept_word" : "accept_all";
  return withCoordinates(
    request,
    "control.request",
    {
      action,
      fingerprint: request.fingerprint,
      suggestion_id: request.suggestionId,
    },
    commitControlId(request),
  );
}

export function commitResultEnvelope(notice: CommitResultNotice): WireSessionEnvelope {
  return withCoordinates(
    notice,
    "commit.result",
    {
      fingerprint: notice.fingerprint,
      suggestion_id: notice.suggestionId,
      status: notice.status,
      ...(notice.newRevision === undefined
        ? {}
        : {
            new_revision: notice.newRevision,
            new_fingerprint: notice.newFingerprint,
          }),
    },
    `${notice.requestId}.${notice.revision}.result`,
  );
}

export function globalControlEnvelope(
  action: "pause" | "resume" | "pause_toggle",
  monotonicMs: number,
  correlationId = `chromium.${action}.${monotonicMs}`,
): WireEnvelope {
  return {
    v: 1,
    id: correlationId,
    type: "control.request",
    mono_ms: monotonicMs,
    payload: { action },
  };
}

export function parseGlobalControlResult(
  value: unknown,
  correlationId: string,
  action: "pause" | "resume" | "pause_toggle",
): boolean | null {
  if (
    !isRecord(value) ||
    value["v"] !== 1 ||
    value["id"] !== correlationId ||
    value["type"] !== "control.result" ||
    !isRecord(value["payload"]) ||
    value["payload"]["action"] !== action ||
    value["payload"]["accepted"] !== true ||
    typeof value["payload"]["paused"] !== "boolean"
  ) {
    return null;
  }
  return value["payload"]["paused"];
}

export function isGlobalControlRejection(
  value: unknown,
  correlationId: string,
  action: "pause" | "resume" | "pause_toggle",
): boolean {
  return (
    isRecord(value) &&
    value["v"] === 1 &&
    value["id"] === correlationId &&
    value["type"] === "control.result" &&
    isRecord(value["payload"]) &&
    value["payload"]["action"] === action &&
    value["payload"]["accepted"] === false &&
    typeof value["payload"]["paused"] === "boolean"
  );
}

export function isGlobalControlError(value: unknown, correlationId: string): boolean {
  return (
    isRecord(value) &&
    value["v"] === 1 &&
    value["id"] === correlationId &&
    value["type"] === "error"
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isOpaqueId(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length >= 1 &&
    value.length <= 128 &&
    /^[A-Za-z0-9._:-]+$/u.test(value)
  );
}

function isCounter(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function hasExactKeys(
  value: Record<string, unknown>,
  expected: readonly string[],
): boolean {
  const actual = Object.keys(value);
  return actual.length === expected.length && actual.every((key) => expected.includes(key));
}

function isSessionId(value: unknown): value is string {
  return (
    typeof value === "string" &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u.test(
      value,
    )
  );
}

function isFingerprint(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length >= 16 &&
    value.length <= 128 &&
    /^[A-Za-z0-9_-]+$/u.test(value)
  );
}

const REASON_CODES = new Set([
  "accepted",
  "ambiguous_session",
  "cancelled",
  "dismissed",
  "expired",
  "field_ambiguous",
  "field_not_editable",
  "field_sensitive",
  "focus_changed",
  "invalid_capability",
  "invalid_frame",
  "invalid_message",
  "invalid_output",
  "manual_required",
  "no_context",
  "no_suggestion",
  "paused",
  "policy_never",
  "provider_error",
  "provider_timeout",
  "session_closed",
  "stale",
  "superseded",
  "unknown_session",
  "unsupported_version",
]);

export function messageId(value: unknown): string | null {
  return isRecord(value) && typeof value["id"] === "string" ? value["id"] : null;
}

export function suggestionErrorMatchesRequest(
  value: unknown,
  request: SuggestionRequest,
): boolean {
  if (
    !isRecord(value) ||
    value["v"] !== 1 ||
    value["type"] !== "error" ||
    !isCounter(value["mono_ms"]) ||
    !isRecord(value["payload"]) ||
    Object.keys(value).some(
      (key) => !["v", "id", "type", "mono_ms", "payload"].includes(key),
    ) ||
    Object.keys(value["payload"]).length !== 1 ||
    typeof value["payload"]["code"] !== "string" ||
    !REASON_CODES.has(value["payload"]["code"])
  ) {
    return false;
  }
  const id = value["id"];
  return (
    id === request.requestId ||
    id === `${request.requestId}.open` ||
    id === `${request.requestId}.context`
  );
}

export function parseSuggestionClearEvent(value: unknown): SuggestionClearEvent | null {
  if (
    !isRecord(value) ||
    value["v"] !== 1 ||
    value["type"] !== "suggestion.clear" ||
    !isSessionId(value["session_id"]) ||
    !isCounter(value["focus_epoch"]) ||
    !isCounter(value["revision"]) ||
    !isCounter(value["mono_ms"]) ||
    !isRecord(value["payload"])
  ) {
    return null;
  }
  const payload = value["payload"];
  if (
    !isFingerprint(payload["fingerprint"]) ||
    typeof payload["reason"] !== "string" ||
    !REASON_CODES.has(payload["reason"]) ||
    (payload["suggestion_id"] !== undefined && !isOpaqueId(payload["suggestion_id"]))
  ) {
    return null;
  }
  const id = value["id"];
  if (id !== undefined && !isOpaqueId(id)) {
    return null;
  }
  return {
    requestId: typeof id === "string" ? id : null,
    sessionId: value["session_id"],
    focusEpoch: value["focus_epoch"],
    revision: value["revision"],
    monotonicMs: value["mono_ms"],
    fingerprint: payload["fingerprint"],
    suggestionId:
      typeof payload["suggestion_id"] === "string" ? payload["suggestion_id"] : null,
    reason: payload["reason"],
  };
}

export function isCommitRejection(
  value: unknown,
  request: CommitAuthorizationRequest,
): boolean {
  if (!isRecord(value) || value["id"] !== commitControlId(request)) {
    return false;
  }
  if (value["type"] === "error") {
    return true;
  }
  return (
    value["type"] === "control.result" &&
    isRecord(value["payload"]) &&
    value["payload"]["accepted"] === false
  );
}

export function isCommitRevocation(
  value: unknown,
  request: CommitAuthorizationRequest,
): boolean {
  if (
    !isRecord(value) ||
    value["v"] !== 1 ||
    value["id"] !== commitControlId(request) ||
    value["type"] !== "suggestion.clear" ||
    value["session_id"] !== request.sessionId ||
    value["focus_epoch"] !== request.focusEpoch ||
    value["revision"] !== request.revision ||
    !isRecord(value["payload"])
  ) {
    return false;
  }
  const payload = value["payload"];
  return (
    payload["fingerprint"] === request.fingerprint &&
    payload["suggestion_id"] === request.suggestionId
  );
}

export function parseCommitAuthorization(
  value: unknown,
  request: CommitAuthorizationRequest,
): CommitAuthorization | null {
  if (
    !isRecord(value) ||
    value["v"] !== 1 ||
    value["id"] !== commitControlId(request) ||
    value["type"] !== "commit.prepare" ||
    value["session_id"] !== request.sessionId ||
    value["focus_epoch"] !== request.focusEpoch ||
    value["revision"] !== request.revision ||
    !isRecord(value["payload"])
  ) {
    return null;
  }
  const payload = value["payload"];
  if (
    payload["fingerprint"] !== request.fingerprint ||
    payload["suggestion_id"] !== request.suggestionId ||
    payload["text"] !== request.expectedText ||
    payload["acceptance"] !== request.acceptance
  ) {
    return null;
  }
  return {
    requestId: request.requestId,
    sessionId: request.sessionId,
    focusEpoch: request.focusEpoch,
    revision: request.revision,
    monotonicMs:
      typeof value["mono_ms"] === "number" ? value["mono_ms"] : request.monotonicMs,
    fingerprint: request.fingerprint,
    suggestionId: request.suggestionId,
    text: request.expectedText,
    acceptance: request.acceptance,
  };
}

function sameCoordinates(value: Record<string, unknown>, request: SuggestionRequest): boolean {
  return (
    value["session_id"] === request.sessionId &&
    value["focus_epoch"] === request.focusEpoch &&
    value["revision"] === request.revision
  );
}

export function parseSuggestionReply(
  value: unknown,
  request: SuggestionRequest,
): SuggestionResponse | null {
  if (!isRecord(value) || value["v"] !== 1 || !sameCoordinates(value, request)) {
    return null;
  }
  const payload = value["payload"];
  if (!isRecord(payload) || payload["fingerprint"] !== request.context.fingerprint) {
    return null;
  }

  if (value["type"] === "suggestion.clear") {
    return {
      requestId: request.requestId,
      sessionId: request.sessionId,
      focusEpoch: request.focusEpoch,
      revision: request.revision,
      fingerprint: request.context.fingerprint,
      suggestion: null,
      suggestionId: null,
      acceptWord: null,
      ttlMs: null,
    };
  }
  if (
    value["type"] !== "suggestion.show" ||
    typeof payload["text"] !== "string" ||
    !isOpaqueId(payload["suggestion_id"]) ||
    typeof payload["accept_word"] !== "string" ||
    typeof payload["ttl_ms"] !== "number" ||
    (payload["provider"] !== "phrase_v1" && payload["provider"] !== "local_model")
  ) {
    return null;
  }
  return {
    requestId: request.requestId,
    sessionId: request.sessionId,
    focusEpoch: request.focusEpoch,
    revision: request.revision,
    fingerprint: request.context.fingerprint,
    suggestion: payload["text"],
    suggestionId: payload["suggestion_id"],
    acceptWord: payload["accept_word"],
    ttlMs: payload["ttl_ms"],
  };
}

export function replyMatchesRequest(value: unknown, request: SuggestionRequest): boolean {
  if (!isRecord(value) || !sameCoordinates(value, request)) {
    return false;
  }
  const id = value["id"];
  const payload = value["payload"];
  return (
    (id === undefined || id === request.requestId) &&
    isRecord(payload) &&
    payload["fingerprint"] === request.context.fingerprint
  );
}

export function parseHelloAckPaused(value: unknown): boolean | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ["v", "id", "type", "mono_ms", "payload"]) ||
    value["v"] !== 1 ||
    value["id"] !== "chromium.hello" ||
    value["type"] !== "hello.ack" ||
    !isCounter(value["mono_ms"]) ||
    !isRecord(value["payload"])
  ) {
    return null;
  }
  const payload = value["payload"];
  if (
    !hasExactKeys(payload, [
      "selected_v",
      "connection_id",
      "enabled_capabilities",
      "max_frame_bytes",
      "max_before_chars",
      "max_after_chars",
      "max_suggestion_chars",
      "max_suggestion_words",
      "paused",
    ]) ||
    payload["selected_v"] !== 1 ||
    !isOpaqueId(payload["connection_id"]) ||
    payload["max_frame_bytes"] !== 65_536 ||
    payload["max_before_chars"] !== 512 ||
    payload["max_after_chars"] !== 128 ||
    payload["max_suggestion_chars"] !== 64 ||
    payload["max_suggestion_words"] !== 8 ||
    typeof payload["paused"] !== "boolean" ||
    !Array.isArray(payload["enabled_capabilities"])
  ) {
    return null;
  }
  const capabilities = payload["enabled_capabilities"];
  const capabilitiesMatch =
    capabilities.length === BROWSER_CAPABILITIES.length &&
    new Set(capabilities).size === BROWSER_CAPABILITIES.length &&
    BROWSER_CAPABILITIES.every((capability) => capabilities.includes(capability));
  return capabilitiesMatch ? payload["paused"] : null;
}

export function isHelloAck(value: unknown): boolean {
  return parseHelloAckPaused(value) !== null;
}
