export const EXPECTED_FIXTURE_ORIGIN = "http://localhost:4173";
export const EXPECTED_FIXTURE_URL = `${EXPECTED_FIXTURE_ORIGIN}/chromium.html`;

export function isTrustedFixtureSender(
  sender: chrome.runtime.MessageSender,
  extensionId: string,
): boolean {
  return (
    sender.id === extensionId &&
    sender.frameId === 0 &&
    sender.tab?.id !== undefined &&
    sender.origin === EXPECTED_FIXTURE_ORIGIN &&
    sender.url === EXPECTED_FIXTURE_URL &&
    (sender.documentLifecycle === undefined || sender.documentLifecycle === "active")
  );
}
