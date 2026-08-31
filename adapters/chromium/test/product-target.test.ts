import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  CHROMIUM_PRODUCT_MANIFEST_TOP_LEVEL_KEYS,
  DILLINGER_OPTIONAL_HOST_PERMISSION,
  assertExactChromiumProductManifest,
} from "../scripts/product-manifest-policy.mjs";
import {
  DILLINGER_ORIGIN,
  DILLINGER_URL,
  isExactDillingerUrl,
  isTrustedDillingerSender,
} from "../src/product/target";

const EXTENSION_ID = "abcdefghijklmnopabcdefghijklmnop";

function trustedSender(): chrome.runtime.MessageSender {
  return {
    id: EXTENSION_ID,
    frameId: 0,
    documentId: "document-a",
    documentLifecycle: "active",
    origin: DILLINGER_ORIGIN,
    url: DILLINGER_URL,
    tab: {
      id: 7,
      index: 0,
      highlighted: true,
      active: true,
      selected: true,
      pinned: false,
      incognito: false,
      discarded: false,
      frozen: false,
      autoDiscardable: true,
      groupId: -1,
      windowId: 3,
      lastAccessed: 1,
    },
  };
}

describe("Dillinger product target boundary", () => {
  it("accepts only the exact root URL and exact active top document", () => {
    expect(isExactDillingerUrl(DILLINGER_URL)).toBe(true);
    for (const url of [
      "https://dillinger.io/?draft=1",
      "https://dillinger.io/#draft",
      "https://dillinger.io/editor",
      "http://dillinger.io/",
      "https://www.dillinger.io/",
      undefined,
    ]) {
      expect(isExactDillingerUrl(url)).toBe(false);
    }

    expect(isTrustedDillingerSender(trustedSender(), EXTENSION_ID)).toBe(true);
    const mutations: chrome.runtime.MessageSender[] = [
      { ...trustedSender(), id: "other-extension" },
      { ...trustedSender(), frameId: 1 },
      { ...trustedSender(), documentId: "" },
      { ...trustedSender(), documentLifecycle: "cached" },
      { ...trustedSender(), origin: "https://example.com" },
      { ...trustedSender(), url: `${DILLINGER_URL}?draft=1` },
      { ...trustedSender(), tab: { ...trustedSender().tab!, active: false } },
      { ...trustedSender(), tab: { ...trustedSender().tab!, incognito: true } },
      { ...trustedSender(), tab: { ...trustedSender().tab!, discarded: true } },
      { ...trustedSender(), tab: { ...trustedSender().tab!, frozen: true } },
    ];
    for (const sender of mutations) {
      expect(isTrustedDillingerSender(sender, EXTENSION_ID)).toBe(false);
    }
  });

  it("freezes a separate exact optional-permission product manifest", async () => {
    const manifest = JSON.parse(
      await readFile(resolve(process.cwd(), "manifest.product.json"), "utf8"),
    ) as Record<string, unknown>;
    expect(() => assertExactChromiumProductManifest(manifest)).not.toThrow();
    expect(Object.keys(manifest).sort()).toEqual(
      CHROMIUM_PRODUCT_MANIFEST_TOP_LEVEL_KEYS,
    );
    expect(manifest["permissions"]).toEqual(["nativeMessaging", "scripting"]);
    expect(manifest["optional_host_permissions"]).toEqual([
      DILLINGER_OPTIONAL_HOST_PERMISSION,
    ]);
    expect(manifest).not.toHaveProperty("host_permissions");
    expect(manifest).not.toHaveProperty("content_scripts");
    expect(manifest["permissions"]).not.toContain("tabs");
    expect(manifest["permissions"]).not.toContain("activeTab");

    for (const mutate of [
      (candidate: Record<string, unknown>) => {
        candidate["optional_host_permissions"] = ["https://*/*"];
      },
      (candidate: Record<string, unknown>) => {
        candidate["host_permissions"] = [DILLINGER_OPTIONAL_HOST_PERMISSION];
      },
      (candidate: Record<string, unknown>) => {
        candidate["content_scripts"] = [];
      },
      (candidate: Record<string, unknown>) => {
        candidate["permissions"] = ["nativeMessaging", "scripting", "tabs"];
      },
      (candidate: Record<string, unknown>) => {
        candidate["commands"] = {};
      },
    ]) {
      const candidate = structuredClone(manifest);
      mutate(candidate);
      expect(() => assertExactChromiumProductManifest(candidate)).toThrow();
    }
  });
});
