import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  EXPECTED_FIXTURE_URL,
  isTrustedFixtureSender,
} from "../src/background/fixture-boundary";
import { SessionRouteRegistry } from "../src/background/session-routes";
import { isContentControlMessage } from "../src/shared/runtime-messages";

const EXTENSION_ID = "abcdefghijklmnopabcdefghijklmnop";

function trustedSender(): chrome.runtime.MessageSender {
  return {
    id: EXTENSION_ID,
    frameId: 0,
    origin: "http://localhost:4173",
    url: EXPECTED_FIXTURE_URL,
    documentLifecycle: "active",
    tab: { id: 7 } as chrome.tabs.Tab,
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
    const { tab: _tab, ...withoutTab } = trustedSender();
    expect(isTrustedFixtureSender(withoutTab, EXTENSION_ID)).toBe(false);
    expect(isTrustedFixtureSender(trustedSender(), "wrong-extension")).toBe(false);
  });

  it("keeps exact least-privilege manifest permissions and a top-frame localhost grant", async () => {
    const manifest = JSON.parse(
      await readFile(resolve(process.cwd(), "manifest.json"), "utf8"),
    ) as {
      readonly permissions: readonly string[];
      readonly content_scripts: ReadonlyArray<{
        readonly matches: readonly string[];
        readonly all_frames: boolean;
      }>;
    };
    expect(manifest.permissions).toEqual(["nativeMessaging"]);
    expect(manifest.content_scripts).toHaveLength(1);
    expect(manifest.content_scripts[0]?.matches).toEqual(["http://localhost/*"]);
    expect(manifest.content_scripts[0]?.all_frames).toBe(false);
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
