export const EXPECTED_FIXTURE_ORIGIN = "http://localhost:4173";
export const EXPECTED_FIXTURE_PATH = "/chromium.html";
export const EXPECTED_FIXTURE_URL = `${EXPECTED_FIXTURE_ORIGIN}${EXPECTED_FIXTURE_PATH}`;

export function isExpectedFixtureDocument(document: Document): boolean {
  const window = document.defaultView;
  return (
    window !== null &&
    window.top === window &&
    window.location.origin === EXPECTED_FIXTURE_ORIGIN &&
    window.location.pathname === EXPECTED_FIXTURE_PATH &&
    window.location.href === EXPECTED_FIXTURE_URL
  );
}
