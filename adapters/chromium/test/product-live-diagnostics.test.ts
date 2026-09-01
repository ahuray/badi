import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  createBrowserLifecycle,
  interactiveEvidenceDeltas,
  requireInteractiveEvidence,
} from "../live/run-product-live.mjs";

const status = (overrides: Record<string, number> = {}) => ({
  metrics: {
    context_updates: 0,
    provider_calls: 0,
    suggestions_shown: 0,
    commits_prepared: 0,
    commits_applied: 0,
    ...overrides,
  },
});

const testRoot = dirname(fileURLToPath(import.meta.url));
const liveSource = (name: string) => readFile(resolve(testRoot, "../live", name), "utf8");

describe("product live diagnostic privacy", () => {
  it("uses the current settings contract in both isolated live runners", async () => {
    const sources = await Promise.all(
      ["run-live.mjs", "run-product-live.mjs"].map((name) => liveSource(name)),
    );
    for (const source of sources) {
      expect(source).toContain('schema: "badi.settings.v2"');
      expect(source).not.toContain('schema: "badi.settings.v1"');
    }
  });

  it("does not serialize Monaco document text into its final records", async () => {
    const sources = await Promise.all(
      ["run-product-live.mjs", "run-product-monaco-proof.mjs"].map((name) =>
        liveSource(name),
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
      /transaction: \{(?<body>[\s\S]*?)\n\s*\},\n\s*browser_diagnostics:/u,
    )?.groups?.["body"];
    expect(transactionRecord).toBeDefined();
    expect(transactionRecord).not.toMatch(/\b(?:trigger|completion)\b\s*[,.:]/u);
    expect(transactionRecord).not.toContain(".value");
  });

  it("makes the destructive editor reset and verified terminal handshake explicit", async () => {
    const source = await liveSource("run-product-live.mjs");

    expect(source).toContain("Press Ctrl+A to select the entire existing document.");
    expect(source).toContain("press Enter. The runner will verify content-free broker metrics");
    expect(source).toContain(
      'attachExtensionDiagnostics(dillingerPage, browserDiagnostics, "dillinger")',
    );
    expect(source).toContain('acceptance_trigger: "extension-owned-content-control"');
    expect(source).toContain("chrome.tabs.sendMessage");
    expect(source).toContain("acceptanceControl?.armed === true");
  });
});

describe("product interactive live evidence", () => {
  it("requires positive context, provider, suggestion, and applied-commit deltas", () => {
    const before = status();
    const after = status({
      context_updates: 1,
      provider_calls: 1,
      suggestions_shown: 1,
      commits_prepared: 1,
      commits_applied: 1,
    });

    expect(requireInteractiveEvidence(before, after)).toEqual({
      context_updates: 1,
      provider_calls: 1,
      suggestions_shown: 1,
      commits_prepared: 1,
      commits_applied: 1,
    });
  });

  it("reports the first missing stage with a concrete recovery action", () => {
    expect(() =>
      requireInteractiveEvidence(
        status(),
        status({
          context_updates: 1,
          provider_calls: 1,
          suggestions_shown: 1,
        }),
      ),
    ).toThrow(/\[interactive-commit-prepare\].*Ctrl\+Shift\+Y/u);
  });

  it("rejects malformed or decreasing broker counters", () => {
    expect(() =>
      interactiveEvidenceDeltas(status({ provider_calls: 2 }), status({ provider_calls: 1 })),
    ).toThrow(/\[interactive-evidence\].*provider_calls.*moved backwards/u);
  });
});

describe("product browser lifecycle", () => {
  it("interrupts an active wait immediately with an actionable lifecycle error", async () => {
    const lifecycle = createBrowserLifecycle();
    const pending = lifecycle.race(new Promise(() => undefined));

    lifecycle.fail("browser-lifecycle", "Chromium closed.", "Re-run and keep it open.");

    await expect(pending).rejects.toMatchObject({
      stage: "browser-lifecycle",
      action: "Re-run and keep it open.",
    });
  });

  it("ignores expected close events after cleanup starts", async () => {
    const lifecycle = createBrowserLifecycle();
    lifecycle.beginCleanup();
    lifecycle.fail("browser-lifecycle", "Chromium closed.", "Re-run.");

    await expect(lifecycle.race(Promise.resolve("clean"))).resolves.toBe("clean");
  });
});
