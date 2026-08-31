import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

describe("product live diagnostic privacy", () => {
  it("does not serialize Monaco document text into its final records", async () => {
    const sources = await Promise.all(
      ["run-product-live.mjs", "run-product-monaco-proof.mjs"].map((name) =>
        readFile(resolve(process.cwd(), "live", name), "utf8"),
      ),
    );
    for (const source of sources) {
      for (const forbiddenKey of [
        "observed_editor_value",
        "accepted_value",
        "undo_value",
        "redo_value",
      ]) {
        expect(source).not.toContain(`${forbiddenKey}:`);
      }
    }

    const transactionRecord = sources[0]?.match(
      /transaction: \{(?<body>[\s\S]*?)\n\s*\},\n\s*revocation:/u,
    )?.groups?.["body"];
    expect(transactionRecord).toBeDefined();
    expect(transactionRecord).not.toMatch(/\b(?:trigger|completion)\b\s*[,.:]/u);
    expect(transactionRecord).not.toContain(".value");
  });
});
