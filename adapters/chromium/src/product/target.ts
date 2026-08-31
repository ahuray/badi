export const DILLINGER_URL = "https://dillinger.io/";
export const DILLINGER_ORIGIN = "https://dillinger.io";
export const DILLINGER_PERMISSION_PATTERN = "https://dillinger.io:443/*";
export const DILLINGER_CONTENT_MATCH = "https://dillinger.io:443/";
export const PRODUCT_LIFETIME_PORT = "badi-dillinger-lifetime-v1";

export function isExactDillingerUrl(value: string | undefined): boolean {
  return value === DILLINGER_URL;
}

export function isExactDillingerDocument(document: Document): boolean {
  return document.location.href === DILLINGER_URL;
}

export function isTrustedDillingerSender(
  sender: chrome.runtime.MessageSender,
  extensionId: string,
): boolean {
  return (
    sender.id === extensionId &&
    sender.frameId === 0 &&
    sender.tab?.id !== undefined &&
    sender.tab.active === true &&
    sender.tab.incognito === false &&
    sender.tab.discarded === false &&
    sender.tab.frozen === false &&
    typeof sender.tab.windowId === "number" &&
    typeof sender.documentId === "string" &&
    sender.documentId.length > 0 &&
    sender.origin === DILLINGER_ORIGIN &&
    sender.url === DILLINGER_URL &&
    sender.documentLifecycle === "active"
  );
}
