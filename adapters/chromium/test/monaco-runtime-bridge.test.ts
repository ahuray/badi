import { describe, expect, it, vi } from "vitest";
import type { CommitAuthorization } from "../src/shared/model";
import { isProductBridgeCommand } from "../src/product/messages";
import { RuntimeMonacoBridge } from "../src/product/monaco-runtime-bridge";
import type { MonacoSnapshot } from "../src/product/monaco-main-world";

const snapshot: MonacoSnapshot = {
  modelUri: "inmemory://model/1",
  languageId: "markdown",
  versionId: 7,
  valueLength: 9,
  offset: 9,
  lineNumber: 1,
  column: 10,
  before: "thank you",
  after: "",
  geometry: { left: 120, top: 80, height: 18 },
};

const authorization: CommitAuthorization = {
  requestId: "request-a",
  sessionId: "session-a",
  focusEpoch: 1,
  revision: 1,
  monotonicMs: 100,
  fingerprint: "0123456789abcdef0123456789abcdef",
  suggestionId: "suggestion-a",
  text: " for your time",
  acceptance: "all",
};

describe("runtime Monaco bridge", () => {
  it("projects a rich snapshot onto the exact mutation-guard wire shape", async () => {
    const sendMessage = vi.fn().mockResolvedValue({ ok: true, applied: true });
    const bridge = new RuntimeMonacoBridge({ sendMessage });

    await expect(bridge.apply("session-a", snapshot, authorization)).resolves.toBe(true);

    const command: unknown = sendMessage.mock.calls[0]?.[0];
    expect(isProductBridgeCommand(command)).toBe(true);
    if (!isProductBridgeCommand(command) || command.kind !== "badi.product.monaco.apply.v1") {
      throw new Error("Runtime bridge emitted a malformed apply command");
    }
    expect(command.expected).toEqual({
      modelUri: snapshot.modelUri,
      languageId: snapshot.languageId,
      versionId: snapshot.versionId,
      valueLength: snapshot.valueLength,
      offset: snapshot.offset,
      lineNumber: snapshot.lineNumber,
      column: snapshot.column,
      before: snapshot.before,
      after: snapshot.after,
    });
    expect(command.expected).not.toHaveProperty("geometry");
  });
});
