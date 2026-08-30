import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  EXPECTED_FIXTURE_URL,
  isTrustedFixtureSender,
} from "../src/background/fixture-boundary";
import { SessionRouteRegistry } from "../src/background/session-routes";
import { isContentControlMessage } from "../src/shared/runtime-messages";
import {
  assertExactChromiumManifest,
  CHROMIUM_MANIFEST_TOP_LEVEL_KEYS,
} from "../scripts/manifest-policy.mjs";

const EXTENSION_ID = "abcdefghijklmnopabcdefghijklmnop";
const DEVELOPMENT_EXTENSION_ID = "ckkiehcjbclcjckkkajohopoikeejkoa";

function extensionIdFromKey(key: string): string {
  const digest = createHash("sha256").update(Buffer.from(key, "base64")).digest();
  return [...digest.subarray(0, 16)]
    .flatMap((byte) => [byte >>> 4, byte & 0x0f])
    .map((nibble) => String.fromCharCode("a".charCodeAt(0) + nibble))
    .join("");
}

function trustedSender(): chrome.runtime.MessageSender {
  return {
    id: EXTENSION_ID,
    frameId: 0,
    origin: "http://localhost:4173",
    url: EXPECTED_FIXTURE_URL,
    documentLifecycle: "active",
    documentId: "document-a",
    tab: {
      id: 7,
      active: true,
      incognito: false,
      discarded: false,
      frozen: false,
      windowId: 3,
    } as chrome.tabs.Tab,
  };
}

describe("localhost fixture boundary", () => {
  it("requires the exact extension, origin, URL, active document, tab, and top frame", () => {
    expect(isTrustedFixtureSender(trustedSender(), EXTENSION_ID)).toBe(true);
    expect(isTrustedFixtureSender({ ...trustedSender(), frameId: 1 }, EXTENSION_ID)).toBe(false);
    expect(
      isTrustedFixtureSender(
        { ...trustedSender(), origin: "http://localhost:9999" },
        EXTENSION_ID,
      ),
    ).toBe(false);
    expect(
      isTrustedFixtureSender(
        { ...trustedSender(), url: `${EXPECTED_FIXTURE_URL}?unexpected=1` },
        EXTENSION_ID,
      ),
    ).toBe(false);
    expect(
      isTrustedFixtureSender(
        { ...trustedSender(), documentLifecycle: "prerender" },
        EXTENSION_ID,
      ),
    ).toBe(false);
    const { documentLifecycle: _lifecycle, ...withoutLifecycle } = trustedSender();
    expect(isTrustedFixtureSender(withoutLifecycle, EXTENSION_ID)).toBe(false);
    expect(
      isTrustedFixtureSender({ ...trustedSender(), documentId: "" }, EXTENSION_ID),
    ).toBe(false);
    const { documentId: _documentId, ...withoutDocumentId } = trustedSender();
    expect(isTrustedFixtureSender(withoutDocumentId, EXTENSION_ID)).toBe(false);
    const { tab: _tab, ...withoutTab } = trustedSender();
    expect(isTrustedFixtureSender(withoutTab, EXTENSION_ID)).toBe(false);
    expect(isTrustedFixtureSender(trustedSender(), "wrong-extension")).toBe(false);
    expect(
      isTrustedFixtureSender(
        { ...trustedSender(), tab: { ...trustedSender().tab, active: false } as chrome.tabs.Tab },
        EXTENSION_ID,
      ),
    ).toBe(false);
    expect(
      isTrustedFixtureSender(
        { ...trustedSender(), tab: { ...trustedSender().tab, incognito: true } as chrome.tabs.Tab },
        EXTENSION_ID,
      ),
    ).toBe(false);
    const { incognito: _incognito, ...withoutIncognito } = trustedSender().tab!;
    expect(
      isTrustedFixtureSender(
        { ...trustedSender(), tab: withoutIncognito as chrome.tabs.Tab },
        EXTENSION_ID,
      ),
    ).toBe(false);
    expect(
      isTrustedFixtureSender(
        { ...trustedSender(), tab: { ...trustedSender().tab, discarded: true } as chrome.tabs.Tab },
        EXTENSION_ID,
      ),
    ).toBe(false);
    const { discarded: _discarded, ...withoutDiscarded } = trustedSender().tab!;
    expect(
      isTrustedFixtureSender(
        { ...trustedSender(), tab: withoutDiscarded as chrome.tabs.Tab },
        EXTENSION_ID,
      ),
    ).toBe(false);
    expect(
      isTrustedFixtureSender(
        { ...trustedSender(), tab: { ...trustedSender().tab, frozen: true } as chrome.tabs.Tab },
        EXTENSION_ID,
      ),
    ).toBe(false);
    const { frozen: _frozen, ...withoutFrozen } = trustedSender().tab!;
    expect(
      isTrustedFixtureSender(
        { ...trustedSender(), tab: withoutFrozen as chrome.tabs.Tab },
        EXTENSION_ID,
      ),
    ).toBe(false);
  });

  it("keeps exact least-privilege permissions and a top-frame localhost grant", async () => {
    const manifest = JSON.parse(
      await readFile(resolve(process.cwd(), "manifest.json"), "utf8"),
    ) as {
      readonly key: string;
      readonly incognito: string;
      readonly minimum_chrome_version: string;
      readonly permissions: readonly string[];
      readonly content_scripts: ReadonlyArray<{
        readonly matches: readonly string[];
        readonly js: readonly string[];
        readonly run_at: string;
        readonly all_frames: boolean;
      }>;
    };
    expect(() => assertExactChromiumManifest(manifest)).not.toThrow();
    expect(Object.keys(manifest).sort()).toEqual(CHROMIUM_MANIFEST_TOP_LEVEL_KEYS);
    expect(manifest.incognito).toBe("not_allowed");
    expect(manifest.minimum_chrome_version).toBe("132");
    expect(manifest.permissions).toEqual(["nativeMessaging"]);
    expect(manifest.content_scripts).toHaveLength(1);
    expect(manifest.content_scripts[0]?.matches).toEqual([
      "http://localhost:4173/chromium.html",
    ]);
    expect(manifest.content_scripts[0]?.js).toEqual(["content-script.js"]);
    expect(manifest.content_scripts[0]?.run_at).toBe("document_idle");
    expect(manifest.content_scripts[0]?.all_frames).toBe(false);
    expect(manifest.permissions).not.toContain("tabs");
    expect(manifest.permissions).not.toContain("activeTab");
    expect(extensionIdFromKey(manifest.key)).toBe(DEVELOPMENT_EXTENSION_ID);

    for (const forbiddenKey of [
      "host_permissions",
      "optional_permissions",
      "optional_host_permissions",
      "externally_connectable",
    ]) {
      const candidate = structuredClone(manifest) as Record<string, unknown>;
      candidate[forbiddenKey] = [];
      expect(() => assertExactChromiumManifest(candidate)).toThrow();
    }

    for (const forbiddenKey of ["match_about_blank", "match_origin_as_fallback"]) {
      const candidate = structuredClone(manifest) as unknown as {
        content_scripts: Array<Record<string, unknown>>;
      };
      candidate.content_scripts[0]![forbiddenKey] = false;
      expect(() => assertExactChromiumManifest(candidate)).toThrow();
    }

    const extraScript = structuredClone(manifest) as unknown as {
      content_scripts: Array<Record<string, unknown>>;
    };
    extraScript.content_scripts.push({
      matches: ["http://localhost:4173/chromium.html"],
      js: ["content-script.js"],
      run_at: "document_idle",
      all_frames: false,
    });
    expect(() => assertExactChromiumManifest(extraScript)).toThrow();
  });

  it("never rebinds one content session across tabs or documents", () => {
    const routes = new SessionRouteRegistry();
    const original = { ...trustedSender(), documentId: "document-a" };
    const otherTab = {
      ...trustedSender(),
      documentId: "document-b",
      tab: { id: 8 } as chrome.tabs.Tab,
    };
    expect(routes.bind("session-a", original)).toBe(true);
    expect(routes.bind("session-a", original)).toBe(true);
    expect(routes.bind("session-a", otherTab)).toBe(false);
    expect(routes.matches("session-a", otherTab)).toBe(false);
    expect(routes.get("session-a")).toEqual({
      tabId: 7,
      frameId: 0,
      documentId: "document-a",
    });
    routes.deleteTab(7);
    expect(routes.get("session-a")).toBeNull();
  });

  it("refuses to bind a route without a nonempty document identity", () => {
    const routes = new SessionRouteRegistry();
    const { documentId: _documentId, ...withoutDocumentId } = trustedSender();
    expect(routes.bind("session-a", withoutDocumentId)).toBe(false);
    expect(routes.bind("session-a", { ...trustedSender(), documentId: "" })).toBe(false);
    expect(routes.get("session-a")).toBeNull();
  });

  it("enumerates an immutable deduplicated route snapshot and deletes by tab", () => {
    const routes = new SessionRouteRegistry();
    const first = { ...trustedSender(), documentId: "document-a" };
    const second = {
      ...trustedSender(),
      documentId: "document-b",
      tab: { id: 8 } as chrome.tabs.Tab,
    };
    expect(routes.bind("session-a", first)).toBe(true);
    expect(routes.bind("session-b", first)).toBe(true);
    expect(routes.bind("session-c", second)).toBe(true);

    const snapshot = routes.snapshot();
    expect(snapshot).toEqual([
      { tabId: 7, frameId: 0, documentId: "document-a" },
      { tabId: 8, frameId: 0, documentId: "document-b" },
    ]);
    expect(Object.isFrozen(snapshot)).toBe(true);
    expect(snapshot.every((route) => Object.isFrozen(route))).toBe(true);

    routes.deleteTab(7);
    expect(snapshot).toHaveLength(2);
    expect(routes.snapshot()).toEqual([
      { tabId: 8, frameId: 0, documentId: "document-b" },
    ]);
  });

  it("accepts only the exact content-safe native-disconnect runtime message", () => {
    expect(
      isContentControlMessage({ kind: "omatype.transport.disconnected.v1" }),
    ).toBe(true);
    expect(
      isContentControlMessage({
        kind: "omatype.transport.disconnected.v1",
        leakedDetail: "native error contents",
      }),
    ).toBe(false);
  });
});
