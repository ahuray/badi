import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  captureContext,
  nextSuggestionWord,
  sanitizeSuggestion,
} from "../src/content/context";

describe("bounded context", () => {
  it("caps before/after by Unicode scalar without splitting surrogate pairs", () => {
    const field = document.createElement("textarea");
    field.id = "long-draft";
    Object.defineProperty(field, "checkVisibility", { value: () => true });
    field.getBoundingClientRect = () => new DOMRect(0, 0, 320, 40);
    document.body.append(field);
    field.value = `${"🙂".repeat(600)}|${"界".repeat(200)}`;
    const caret = "🙂".repeat(600).length + 1;
    field.setSelectionRange(caret, caret);
    const context = captureContext({
      field,
      purpose: "normal",
      selection: { start: caret, end: caret, direction: "none" },
      composing: false,
      activation: "always",
      explicit: false,
      fingerprintSalt: "session-salt-a",
    });

    expect(Array.from(context.before)).toHaveLength(512);
    expect(Array.from(context.after)).toHaveLength(128);
    expect(context.fingerprint).toMatch(/^[a-f0-9]{32}$/u);

    const otherSession = captureContext({
      field,
      purpose: "normal",
      selection: { start: caret, end: caret, direction: "none" },
      composing: false,
      activation: "always",
      explicit: false,
      fingerprintSalt: "session-salt-b",
    });
    expect(otherSession.fingerprint).not.toBe(context.fingerprint);
  });

  it("captures only a canonical declared language and binds it to the fingerprint", () => {
    document.documentElement.lang = "en-us";
    const field = document.createElement("textarea");
    field.id = "localized-draft";
    Object.defineProperty(field, "checkVisibility", { value: () => true });
    field.getBoundingClientRect = () => new DOMRect(0, 0, 320, 40);
    document.body.append(field);
    field.value = "Thank you";
    field.setSelectionRange(field.value.length, field.value.length);
    const input = {
      field,
      purpose: "normal" as const,
      selection: { start: field.value.length, end: field.value.length, direction: "none" as const },
      composing: false,
      activation: "always" as const,
      explicit: false,
      fingerprintSalt: "session-salt",
    };

    const english = captureContext(input);
    expect(english.language).toBe("en-US");

    document.documentElement.lang = "de";
    const german = captureContext(input);
    expect(german.language).toBe("de");
    expect(german.fingerprint).not.toBe(english.fingerprint);

    field.lang = "not_a_language";
    const invalid = captureContext(input);
    expect(invalid.language).toBeUndefined();
    expect(invalid.fingerprint).not.toBe(german.fingerprint);
  });

  it("accepts bounded plain text without rewriting it", () => {
    const safe = " one two three four five six seven eight";
    expect(sanitizeSuggestion(safe)).toBe(safe);
    expect(sanitizeSuggestion("🙂".repeat(64))).toBe("🙂".repeat(64));
  });

  it.each([
    ["NUL", "safe\u0000hostile"],
    ["C0 newline", "safe\nhostile"],
    ["tab", "safe\thostile"],
    ["C1", "safe\u0085hostile"],
    ["DEL", "safe\u007fhostile"],
    ["bidi override", "safe\u202ehostile"],
    ["zero-width control", "safe\u200bhostile"],
    ["unpaired high surrogate", "safe\ud800hostile"],
    ["unpaired low surrogate", "safe\udc00hostile"],
  ])("rejects hostile provider output: %s", (_label, value) => {
    expect(sanitizeSuggestion(value)).toBeNull();
  });

  it("matches the broker's strict scalar and Unicode-word limits", () => {
    expect(sanitizeSuggestion("x".repeat(65))).toBeNull();
    expect(sanitizeSuggestion("one two three four five six seven eight nine")).toBeNull();
    expect(sanitizeSuggestion("valid ")).toBeNull();
    expect(sanitizeSuggestion("\u00a0world")).toBeNull();
    expect(sanitizeSuggestion(" world\u00a0again")).toBeNull();
    expect(sanitizeSuggestion(" world  again")).toBeNull();
    expect(sanitizeSuggestion("one two three four five six seven can't nine")).toBe(
      null,
    );
    expect(sanitizeSuggestion("one two three four five six seven can't")).toBe(
      "one two three four five six seven can't",
    );
  });

  it("consumes the shared protocol accept-word fixtures directly", async () => {
    const fixturePath = resolve(process.cwd(), "../../protocol/v1/accept-word-fixtures.json");
    const fixture = JSON.parse(await readFile(fixturePath, "utf8")) as {
      readonly v: number;
      readonly cases: ReadonlyArray<{
        readonly input: string;
        readonly accepted: string;
        readonly remainder: string;
      }>;
    };
    expect(fixture.v).toBe(1);
    for (const testCase of fixture.cases) {
      const accepted = nextSuggestionWord(testCase.input);
      expect(accepted).toBe(testCase.accepted);
      expect(testCase.input.slice(accepted.length)).toBe(testCase.remainder);
    }
    expect(nextSuggestionWord(" can't wait")).toBe(" can't");
    expect(nextSuggestionWord(" 3.14 seconds")).toBe(" 3.14");
  });
});
