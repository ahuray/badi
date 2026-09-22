import { describe, expect, it } from "vitest";
import { isProductBridgeCommand } from "../src/product/messages";

const applyCommand = {
  kind: "badi.product.monaco.apply.v1",
  sessionId: "session-a",
  expected: {
    modelUri: "inmemory://model/1",
    languageId: "markdown",
    versionId: 7,
    valueLength: 9,
    offset: 9,
    lineNumber: 1,
    column: 10,
    before: "thank you",
    after: "",
  },
  authorization: {
    requestId: "request-a",
    sessionId: "session-a",
    focusEpoch: 1,
    revision: 2,
    monotonicMs: 3,
    fingerprint: "fingerprint-a",
    suggestionId: "suggestion-a",
    text: " for your time",
    acceptance: "all",
  },
} as const;

describe("product bridge messages", () => {
  it("keeps correction spacing bound to its exact snapshot suffix", () => {
    const command = { ...applyCommand,
      expected: { ...applyCommand.expected, before: "This is teh ", valueLength: 12, offset: 12, column: 13 },
      authorization: { ...applyCommand.authorization, text: "the ", replaceBefore: "teh " },
    };
    expect(isProductBridgeCommand(command)).toBe(true);
    for (const text of ["the", "the  ", "the\n", "teh "]) {
      expect(isProductBridgeCommand({ ...command, authorization: { ...command.authorization, text } })).toBe(false);
    }
    expect(isProductBridgeCommand({ ...command, expected: { ...command.expected, before: "Thisisteh " } })).toBe(false);
    expect(isProductBridgeCommand({ ...command, expected: { ...command.expected, after: "next" } })).toBe(false);
  });
  it("requires the complete exact all-acceptance authorization at apply", () => {
    expect(isProductBridgeCommand(applyCommand)).toBe(true);
    expect(
      isProductBridgeCommand({
        kind: applyCommand.kind,
        sessionId: applyCommand.sessionId,
        expected: applyCommand.expected,
        text: applyCommand.authorization.text,
      }),
    ).toBe(false);
    expect(
      isProductBridgeCommand({
        ...applyCommand,
        authorization: { ...applyCommand.authorization, sessionId: "session-b" },
      }),
    ).toBe(false);
    expect(
      isProductBridgeCommand({
        ...applyCommand,
        authorization: { ...applyCommand.authorization, acceptance: "word" },
      }),
    ).toBe(false);
  });
});
