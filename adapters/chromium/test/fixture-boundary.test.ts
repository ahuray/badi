import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  EXPECTED_FIXTURE_URL,
  isTrustedFixtureBootstrapSender,
  isTrustedFixtureSender,
} from "../src/background/fixture-boundary";
import { SessionRouteRegistry } from "../src/background/session-routes";
import {
  EXPECTED_FIXTURE_ORIGIN,
  isExpectedFixtureDocument,
} from "../src/shared/fixture-document";
import {
  isContentControlMessage,
  isRuntimeCommand,
  parseRuntimeBootstrapReply,
} from "../src/shared/runtime-messages";
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
  it("recognizes only the exact top-level fixture document", () => {
    const location = {
      origin: EXPECTED_FIXTURE_ORIGIN,
      pathname: "/chromium.html",
      href: EXPECTED_FIXTURE_URL,
    };
    const window = { location } as unknown as Window;
    Object.defineProperty(window, "top", { value: window });
    const fixtureDocument = { defaultView: window } as unknown as Document;

    expect(isExpectedFixtureDocument(fixtureDocument)).toBe(true);
    location.href = `${EXPECTED_FIXTURE_URL}?route=other`;
    expect(isExpectedFixtureDocument(fixtureDocument)).toBe(false);
  });

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

  it("allows only content-free bootstrap from an inactive exact document", () => {
    const inactive = {
      ...trustedSender(),
      tab: { ...trustedSender().tab, active: false } as chrome.tabs.Tab,
    };
    expect(isTrustedFixtureBootstrapSender(inactive, EXTENSION_ID)).toBe(true);
    expect(isTrustedFixtureSender(inactive, EXTENSION_ID)).toBe(false);
    expect(
      isTrustedFixtureBootstrapSender(
        { ...inactive, url: `${EXPECTED_FIXTURE_URL}#other` },
        EXTENSION_ID,
      ),
    ).toBe(false);
    const { active: _active, ...withoutActive } = inactive.tab!;
    expect(
      isTrustedFixtureBootstrapSender(
        { ...inactive, tab: withoutActive as chrome.tabs.Tab },
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

  it("never resubscribes one content session across tabs or documents", () => {
    const routes = new SessionRouteRegistry();
    const original = { ...trustedSender(), documentId: "document-a" };
    const otherTab = {
      ...trustedSender(),
      documentId: "document-b",
      tab: { id: 8 } as chrome.tabs.Tab,
    };
    expect(routes.subscribe("session-a", original)).toEqual({
      displacedSessionIds: [],
    });
    expect(routes.subscribe("session-a", original)).toEqual({
      displacedSessionIds: [],
    });
    expect(routes.subscribe("session-a", otherTab)).toBeNull();
    expect(routes.matches("session-a", otherTab)).toBe(false);
    expect(routes.get("session-a")).toEqual({
      tabId: 7,
      frameId: 0,
      documentId: "document-a",
      origin: EXPECTED_FIXTURE_ORIGIN,
    });
    expect(routes.deleteTab(7)).toEqual(["session-a"]);
    expect(routes.get("session-a")).toBeNull();
  });

  it("enforces one session per document and displaces old documents", () => {
    const routes = new SessionRouteRegistry();
    const firstDocument = { ...trustedSender(), documentId: "document-a" };
    const replacementDocument = { ...trustedSender(), documentId: "document-b" };
    const otherTab = {
      ...trustedSender(),
      documentId: "document-c",
      tab: { ...trustedSender().tab, id: 8 } as chrome.tabs.Tab,
    };

    expect(routes.subscribe("session-a", firstDocument)).toEqual({
      displacedSessionIds: [],
    });
    expect(routes.subscribe("session-a2", firstDocument)).toEqual({
      displacedSessionIds: ["session-a"],
    });
    expect(routes.subscribe("session-c", otherTab)).toEqual({
      displacedSessionIds: [],
    });

    // The route remains addressable until an explicit document replacement;
    // broker-session close is deliberately not a registry operation.
    expect(routes.matches("session-a", firstDocument)).toBe(false);
    const replacement = routes.subscribe("session-b", replacementDocument);
    if (replacement === null) throw new Error("Replacement route was rejected");
    expect(replacement).toEqual({
      displacedSessionIds: ["session-a2"],
    });
    expect(Object.isFrozen(replacement)).toBe(true);
    expect(Object.isFrozen(replacement.displacedSessionIds)).toBe(true);
    expect(routes.get("session-a")).toBeNull();
    expect(routes.get("session-a2")).toBeNull();
    expect(routes.matches("session-b", replacementDocument)).toBe(true);
    expect(routes.matches("session-c", otherTab)).toBe(true);

    expect(routes.subscribe("session-b", otherTab)).toBeNull();
    expect(routes.matches("session-b", replacementDocument)).toBe(true);
  });

  it("refuses to subscribe a route without a nonempty document identity", () => {
    const routes = new SessionRouteRegistry();
    const { documentId: _documentId, ...withoutDocumentId } = trustedSender();
    expect(routes.subscribe("session-a", withoutDocumentId)).toBeNull();
    expect(
      routes.subscribe("session-a", { ...trustedSender(), documentId: "" }),
    ).toBeNull();
    expect(routes.get("session-a")).toBeNull();
  });

  it("deletes only the exact registered route when explicitly retired", () => {
    const routes = new SessionRouteRegistry();
    expect(routes.subscribe("session-a", trustedSender())).not.toBeNull();
    const route = routes.get("session-a");
    if (route === null) throw new Error("Route missing");
    expect(
      routes.delete("session-a", { ...route, documentId: "different-document" }),
    ).toBe(false);
    expect(routes.get("session-a")).toEqual(route);
    expect(routes.delete("session-a", route)).toBe(true);
    expect(routes.get("session-a")).toBeNull();
  });

  it("retires every route bound to an exact disconnected document", () => {
    const routes = new SessionRouteRegistry();
    const document = trustedSender();
    expect(routes.subscribe("session-a", document)).not.toBeNull();
    expect(
      routes.deleteDocument({ ...document, documentId: "other-document" }),
    ).toEqual([]);
    expect(routes.matches("session-a", document)).toBe(true);
    expect(routes.deleteDocument(document)).toEqual(["session-a"]);
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
    expect(routes.subscribe("session-a", first)).not.toBeNull();
    expect(routes.subscribe("session-b", first)).toEqual({
      displacedSessionIds: ["session-a"],
    });
    expect(routes.subscribe("session-c", second)).not.toBeNull();

    const snapshot = routes.snapshot();
    expect(snapshot).toEqual([
      {
        tabId: 7,
        frameId: 0,
        documentId: "document-a",
        origin: EXPECTED_FIXTURE_ORIGIN,
      },
      {
        tabId: 8,
        frameId: 0,
        documentId: "document-b",
        origin: EXPECTED_FIXTURE_ORIGIN,
      },
    ]);
    expect(Object.isFrozen(snapshot)).toBe(true);
    expect(snapshot.every((route) => Object.isFrozen(route))).toBe(true);

    expect(routes.deleteTab(7)).toEqual(["session-b"]);
    expect(snapshot).toHaveLength(2);
    expect(routes.snapshot()).toEqual([
      {
        tabId: 8,
        frameId: 0,
        documentId: "document-b",
        origin: EXPECTED_FIXTURE_ORIGIN,
      },
    ]);
  });

  it("accepts only exact content-control runtime message shapes", () => {
    expect(
      isContentControlMessage({ kind: "badi.transport.disconnected.v1" }),
    ).toBe(true);
    expect(
      isContentControlMessage({
        kind: "badi.transport.disconnected.v1",
        leakedDetail: "native error contents",
      }),
    ).toBe(false);
    expect(
      isContentControlMessage({ kind: "badi.control.v1", action: "pause" }),
    ).toBe(true);
    expect(
      isContentControlMessage({
        kind: "badi.control.v1",
        action: "pause",
        leakedDetail: "native error contents",
      }),
    ).toBe(false);
    expect(isContentControlMessage({ kind: "badi.control.v1" })).toBe(false);
    expect(
      isContentControlMessage({ kind: "badi.control.v1", action: "unknown" }),
    ).toBe(false);
  });

  it("accepts only typed content-free bootstrap and bound-session close shapes", () => {
    expect(
      isRuntimeCommand({ kind: "badi.bootstrap.v1", sessionId: "session-a" }),
    ).toBe(true);
    for (const malformed of [
      { kind: "badi.bootstrap.v1" },
      { kind: "badi.bootstrap.v1", sessionId: "" },
      { kind: "badi.bootstrap.v1", sessionId: 7 },
      { kind: "badi.bootstrap.v1", sessionId: "session-a", leaked: true },
      ["badi.bootstrap.v1", "session-a"],
      null,
    ]) {
      expect(isRuntimeCommand(malformed)).toBe(false);
    }
    expect(
      isRuntimeCommand({ kind: "badi.session.close.v1", sessionId: "session-a" }),
    ).toBe(true);
    expect(isRuntimeCommand({ kind: "badi.session.close.v1", sessionId: "" })).toBe(false);
    const policy = {
      authorityEpoch: 4,
      settingsRevision: 2,
      paused: true,
      activation: "never",
      contextAllowed: false,
      displayAllowed: false,
      suggestionsAllowed: false,
      learningAllowed: false,
      reason: "global_disabled",
    } as const;
    expect(
      parseRuntimeBootstrapReply({ ok: true, paused: true, policy }),
    ).toEqual({ paused: true, policy });
    expect(
      isContentControlMessage({
        kind: "badi.policy.v1",
        policy: { ...policy, activation: "always" },
      }),
    ).toBe(false);
    expect(
      isContentControlMessage({
        kind: "badi.policy.v1",
        policy: { ...policy, leaked: true },
      }),
    ).toBe(false);
    expect(() => parseRuntimeBootstrapReply({ ok: true })).toThrow("bootstrap response");
    expect(() =>
      parseRuntimeBootstrapReply({ ok: true, paused: true, leaked: "content" }),
    ).toThrow("bootstrap response");
    expect(() => parseRuntimeBootstrapReply({ ok: true, paused: "true" })).toThrow(
      "bootstrap response",
    );
  });
});
