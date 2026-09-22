export {
  EXPECTED_FIXTURE_ORIGIN,
  EXPECTED_FIXTURE_URL,
} from "../shared/fixture-document";
import {
  EXPECTED_FIXTURE_ORIGIN,
  EXPECTED_FIXTURE_URL,
} from "../shared/fixture-document";

export function isTrustedFixtureSender(
  sender: chrome.runtime.MessageSender,
  extensionId: string,
): boolean {
  return (
    isTrustedFixtureBootstrapSender(sender, extensionId) &&
    sender.tab?.active === true
  );
}

export function isFocusedFixtureTab(
  sender: chrome.runtime.MessageSender,
  tab: chrome.tabs.Tab,
  window: chrome.windows.Window,
  extensionId: string,
): boolean {
  // MessageSender supplies the exact sending document URL and document ID.
  // tabs.get omits URL under this fixture's nativeMessaging-only manifest.
  // Recheck current tab/window activity without requiring a broader grant.
  return (
    isTrustedFixtureSender(sender, extensionId) &&
    tab.id === sender.tab?.id &&
    tab.windowId === sender.tab?.windowId &&
    window.id === tab.windowId &&
    tab.active === true &&
    tab.incognito === false &&
    tab.discarded === false &&
    tab.frozen === false &&
    (tab.url === undefined || tab.url === EXPECTED_FIXTURE_URL) &&
    window.focused === true
  );
}

/**
 * The content-free bootstrap may subscribe an inactive exact document so it
 * can receive pause/resume before it ever acquires text. Content-bearing and
 * action messages must continue to use isTrustedFixtureSender.
 */
export function isTrustedFixtureBootstrapSender(
  sender: chrome.runtime.MessageSender,
  extensionId: string,
): boolean {
  return (
    sender.id === extensionId &&
    sender.frameId === 0 &&
    sender.tab?.id !== undefined &&
    typeof sender.tab.active === "boolean" &&
    sender.tab.incognito === false &&
    sender.tab.discarded === false &&
    sender.tab.frozen === false &&
    typeof sender.tab.windowId === "number" &&
    typeof sender.documentId === "string" &&
    sender.documentId.length > 0 &&
    sender.origin === EXPECTED_FIXTURE_ORIGIN &&
    sender.url === EXPECTED_FIXTURE_URL &&
    sender.documentLifecycle === "active"
  );
}
