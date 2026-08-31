import type { CommitAuthorization } from "../shared/model";
import type { MonacoSnapshot, MonacoSnapshotGuard } from "./monaco-main-world";

export type ProductBridgeCommand =
  | {
      readonly kind: "badi.product.monaco.snapshot.v1";
      readonly sessionId: string;
    }
  | {
      readonly kind: "badi.product.monaco.apply.v1";
      readonly sessionId: string;
      readonly expected: MonacoSnapshotGuard;
      readonly authorization: CommitAuthorization;
    };

export type ProductExtensionCommand =
  | { readonly kind: "badi.product.activate-current.v1" }
  | { readonly kind: "badi.product.permission-state.v1" };

export type ProductBridgeReply =
  | { readonly ok: true; readonly snapshot: MonacoSnapshot | null }
  | { readonly ok: true; readonly applied: boolean }
  | { readonly ok: false; readonly error: string };

export type ProductExtensionReply =
  | { readonly ok: true; readonly granted: boolean }
  | { readonly ok: false; readonly error: string };

export type ProductControlMessage =
  | { readonly kind: "badi.product.accept-all.v1" }
  | { readonly kind: "badi.product.disable.v1" }
  | { readonly kind: "badi.product.worker-restarted.v1" };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasExactKeys(
  value: Record<string, unknown>,
  expected: readonly string[],
): boolean {
  const actual = Object.keys(value);
  return actual.length === expected.length && actual.every((key) => expected.includes(key));
}

function isCounter(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function isCommitAuthorization(value: unknown): value is CommitAuthorization {
  return (
    isRecord(value) &&
    hasExactKeys(value, [
      "requestId",
      "sessionId",
      "focusEpoch",
      "revision",
      "monotonicMs",
      "fingerprint",
      "suggestionId",
      "text",
      "acceptance",
    ]) &&
    typeof value["requestId"] === "string" &&
    value["requestId"].length > 0 &&
    typeof value["sessionId"] === "string" &&
    value["sessionId"].length > 0 &&
    isCounter(value["focusEpoch"]) &&
    isCounter(value["revision"]) &&
    isCounter(value["monotonicMs"]) &&
    typeof value["fingerprint"] === "string" &&
    value["fingerprint"].length > 0 &&
    typeof value["suggestionId"] === "string" &&
    value["suggestionId"].length > 0 &&
    typeof value["text"] === "string" &&
    value["text"].length > 0 &&
    value["acceptance"] === "all"
  );
}

export function isMonacoSnapshotGuard(value: unknown): value is MonacoSnapshotGuard {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "modelUri",
      "languageId",
      "versionId",
      "valueLength",
      "offset",
      "lineNumber",
      "column",
      "before",
      "after",
    ])
  ) {
    return false;
  }
  return (
    typeof value["modelUri"] === "string" &&
    value["modelUri"].length > 0 &&
    value["languageId"] === "markdown" &&
    isCounter(value["versionId"]) &&
    isCounter(value["valueLength"]) &&
    isCounter(value["offset"]) &&
    value["offset"] <= value["valueLength"] &&
    isCounter(value["lineNumber"]) &&
    value["lineNumber"] > 0 &&
    isCounter(value["column"]) &&
    value["column"] > 0 &&
    typeof value["before"] === "string" &&
    Array.from(value["before"]).length <= 512 &&
    typeof value["after"] === "string" &&
    Array.from(value["after"]).length <= 128
  );
}

export function isProductBridgeCommand(value: unknown): value is ProductBridgeCommand {
  if (!isRecord(value) || typeof value["kind"] !== "string") return false;
  if (value["kind"] === "badi.product.monaco.snapshot.v1") {
    return (
      hasExactKeys(value, ["kind", "sessionId"]) &&
      typeof value["sessionId"] === "string" &&
      value["sessionId"].length > 0
    );
  }
  if (value["kind"] === "badi.product.monaco.apply.v1") {
    return (
      hasExactKeys(value, ["kind", "sessionId", "expected", "authorization"]) &&
      typeof value["sessionId"] === "string" &&
      value["sessionId"].length > 0 &&
      isMonacoSnapshotGuard(value["expected"]) &&
      isCommitAuthorization(value["authorization"]) &&
      value["authorization"].sessionId === value["sessionId"]
    );
  }
  return false;
}

export function isProductExtensionCommand(value: unknown): value is ProductExtensionCommand {
  return (
    isRecord(value) &&
    hasExactKeys(value, ["kind"]) &&
    (value["kind"] === "badi.product.activate-current.v1" ||
      value["kind"] === "badi.product.permission-state.v1")
  );
}

export function isProductControlMessage(value: unknown): value is ProductControlMessage {
  return (
    isRecord(value) &&
    hasExactKeys(value, ["kind"]) &&
    (value["kind"] === "badi.product.accept-all.v1" ||
      value["kind"] === "badi.product.disable.v1" ||
      value["kind"] === "badi.product.worker-restarted.v1")
  );
}

export function parseSnapshotReply(value: unknown): MonacoSnapshot | null {
  if (!isRecord(value) || value["ok"] !== true || !("snapshot" in value)) {
    throw new Error(
      isRecord(value) && typeof value["error"] === "string"
        ? value["error"]
        : "Invalid Monaco snapshot reply",
    );
  }
  const snapshot = value["snapshot"];
  if (snapshot === null) return null;
  const geometry = isRecord(snapshot) ? snapshot["geometry"] : null;
  if (
    !isRecord(snapshot) ||
    !hasExactKeys(snapshot, [
      "modelUri",
      "languageId",
      "versionId",
      "valueLength",
      "offset",
      "lineNumber",
      "column",
      "before",
      "after",
      "geometry",
    ]) ||
    !isMonacoSnapshotGuard({
      modelUri: snapshot["modelUri"],
      languageId: snapshot["languageId"],
      versionId: snapshot["versionId"],
      valueLength: snapshot["valueLength"],
      offset: snapshot["offset"],
      lineNumber: snapshot["lineNumber"],
      column: snapshot["column"],
      before: snapshot["before"],
      after: snapshot["after"],
    }) ||
    !isRecord(geometry) ||
    !hasExactKeys(geometry, ["left", "top", "height"]) ||
    !["left", "top", "height"].every(
      (key) => typeof geometry[key] === "number" && Number.isFinite(geometry[key]),
    ) ||
    (geometry["left"] as number) < 0 ||
    (geometry["top"] as number) < 0 ||
    (geometry["height"] as number) <= 0
  ) {
    throw new Error("Malformed Monaco snapshot from MAIN world");
  }
  return snapshot as unknown as MonacoSnapshot;
}

export function parseApplyReply(value: unknown): boolean {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ["ok", "applied"]) ||
    value["ok"] !== true ||
    typeof value["applied"] !== "boolean"
  ) {
    throw new Error(
      isRecord(value) && typeof value["error"] === "string"
        ? value["error"]
        : "Invalid Monaco apply reply",
    );
  }
  return value["applied"];
}
