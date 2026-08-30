import type {
  CommitAuthorization,
  CommitAuthorizationRequest,
  CommitResultNotice,
  SuggestionAddress,
  SuggestionClearEvent,
  SuggestionRequest,
  SuggestionResponse,
} from "./model";

export type RuntimeCommand =
  | { readonly kind: "badi.suggest.v1"; readonly request: SuggestionRequest }
  | { readonly kind: "badi.cancel.v1"; readonly request: SuggestionRequest }
  | { readonly kind: "badi.dismiss.v1"; readonly address: SuggestionAddress }
  | {
      readonly kind: "badi.commit.authorize.v1";
      readonly request: CommitAuthorizationRequest;
    }
  | { readonly kind: "badi.commit.result.v1"; readonly notice: CommitResultNotice };

export type RuntimeReply =
  | {
      readonly ok: true;
      readonly response?: SuggestionResponse | CommitAuthorization;
    }
  | { readonly ok: false; readonly error: string };

export type ContentControlMessage =
  | {
      readonly kind: "badi.control.v1";
      readonly action:
        | "pause"
        | "resume"
        | "accept_word"
        | "accept_all"
        | "dismiss";
    }
  | {
      readonly kind: "badi.commit.revoke.v1";
      readonly address: SuggestionAddress;
    }
  | {
      readonly kind: "badi.suggestion.clear.v1";
      readonly event: SuggestionClearEvent;
    }
  | {
      readonly kind: "badi.transport.disconnected.v1";
    };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isCounter(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function hasCoordinates(value: Record<string, unknown>): boolean {
  return (
    typeof value["sessionId"] === "string" &&
    isCounter(value["focusEpoch"]) &&
    isCounter(value["revision"]) &&
    isCounter(value["monotonicMs"])
  );
}

export function isSuggestionRequest(value: unknown): value is SuggestionRequest {
  if (!isRecord(value) || !hasCoordinates(value) || typeof value["requestId"] !== "string") {
    return false;
  }
  if (typeof value["origin"] !== "string" || !isRecord(value["context"])) {
    return false;
  }
  const context = value["context"];
  return (
    typeof context["fingerprint"] === "string" &&
    typeof context["before"] === "string" &&
    typeof context["after"] === "string" &&
    isRecord(context["selection"]) &&
    isRecord(context["field"])
  );
}

function isAddress(value: unknown): value is SuggestionAddress {
  return (
    isRecord(value) &&
    hasCoordinates(value) &&
    typeof value["requestId"] === "string" &&
    typeof value["fingerprint"] === "string" &&
    typeof value["suggestionId"] === "string"
  );
}

function isSuggestionClearEvent(value: unknown): value is SuggestionClearEvent {
  return (
    isRecord(value) &&
    hasCoordinates(value) &&
    (value["requestId"] === null || typeof value["requestId"] === "string") &&
    typeof value["fingerprint"] === "string" &&
    (value["suggestionId"] === null || typeof value["suggestionId"] === "string") &&
    typeof value["reason"] === "string"
  );
}

export function isRuntimeCommand(value: unknown): value is RuntimeCommand {
  if (!isRecord(value) || typeof value["kind"] !== "string") {
    return false;
  }
  switch (value["kind"]) {
    case "badi.suggest.v1":
    case "badi.cancel.v1":
      return isSuggestionRequest(value["request"]);
    case "badi.dismiss.v1":
      return isAddress(value["address"]);
    case "badi.commit.authorize.v1": {
      const notice = value["request"];
      return (
        isRecord(notice) &&
        isAddress(notice) &&
        typeof notice["expectedText"] === "string" &&
        (notice["acceptance"] === "word" || notice["acceptance"] === "all")
      );
    }
    case "badi.commit.result.v1": {
      const notice = value["notice"];
      return isRecord(notice) && isAddress(notice) && typeof notice["status"] === "string";
    }
    default:
      return false;
  }
}

export function isContentControlMessage(value: unknown): value is ContentControlMessage {
  if (!isRecord(value)) {
    return false;
  }
  if (value["kind"] === "badi.transport.disconnected.v1") {
    return Object.keys(value).length === 1;
  }
  if (value["kind"] === "badi.commit.revoke.v1") {
    return isAddress(value["address"]);
  }
  if (value["kind"] === "badi.suggestion.clear.v1") {
    return isSuggestionClearEvent(value["event"]);
  }
  if (value["kind"] !== "badi.control.v1") return false;
  return (
    value["action"] === "pause" ||
    value["action"] === "resume" ||
    value["action"] === "accept_word" ||
    value["action"] === "accept_all" ||
    value["action"] === "dismiss"
  );
}

export function parseRuntimeSuggestionReply(value: unknown): SuggestionResponse {
  if (!isRecord(value) || value["ok"] !== true || !isRecord(value["response"])) {
    const error = isRecord(value) && typeof value["error"] === "string"
      ? value["error"]
      : "Invalid extension service-worker response";
    throw new Error(error);
  }
  const response = value["response"];
  if (
    typeof response["requestId"] !== "string" ||
    typeof response["sessionId"] !== "string" ||
    typeof response["focusEpoch"] !== "number" ||
    typeof response["revision"] !== "number" ||
    typeof response["fingerprint"] !== "string" ||
    (response["suggestion"] !== null && typeof response["suggestion"] !== "string") ||
    (response["suggestionId"] !== null && typeof response["suggestionId"] !== "string") ||
    (response["acceptWord"] !== null && typeof response["acceptWord"] !== "string") ||
    (response["ttlMs"] !== null && typeof response["ttlMs"] !== "number")
  ) {
    throw new Error("Malformed suggestion response from extension service worker");
  }
  return response as unknown as SuggestionResponse;
}

export function parseRuntimeCommitAuthorization(value: unknown): CommitAuthorization {
  if (!isRecord(value) || value["ok"] !== true || !isRecord(value["response"])) {
    const error =
      isRecord(value) && typeof value["error"] === "string"
        ? value["error"]
        : "Commit authorization was denied";
    throw new Error(error);
  }
  const response = value["response"];
  if (
    !isAddress(response) ||
    typeof response["text"] !== "string" ||
    (response["acceptance"] !== "word" && response["acceptance"] !== "all")
  ) {
    throw new Error("Malformed commit authorization from extension service worker");
  }
  return response as unknown as CommitAuthorization;
}
