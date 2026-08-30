import { isDeepStrictEqual } from "node:util";

export const CHROMIUM_MANIFEST_TOP_LEVEL_KEYS = Object.freeze([
  "background",
  "commands",
  "content_scripts",
  "description",
  "incognito",
  "key",
  "manifest_version",
  "minimum_chrome_version",
  "name",
  "permissions",
  "version",
]);

export const CHROMIUM_CONTENT_SCRIPT = Object.freeze({
  matches: Object.freeze(["http://localhost:4173/chromium.html"]),
  js: Object.freeze(["content-script.js"]),
  run_at: "document_idle",
  all_frames: false,
});

const EXPECTED_BACKGROUND = Object.freeze({
  service_worker: "service-worker.js",
});

const EXPECTED_COMMANDS = Object.freeze({
  "toggle-pause": Object.freeze({
    suggested_key: Object.freeze({
      default: "Alt+Shift+P",
    }),
    description: "Toggle broker pause state for trusted Omatype fixture controllers",
  }),
});

function isRecord(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Rejects every manifest surface outside the frozen M2A development cell.
 * This is intentionally stricter than Chrome's manifest parser.
 */
export function assertExactChromiumManifest(manifest) {
  if (!isRecord(manifest)) {
    throw new Error("Chromium manifest must be an object");
  }
  const keys = Object.keys(manifest).sort();
  if (!isDeepStrictEqual(keys, CHROMIUM_MANIFEST_TOP_LEVEL_KEYS)) {
    throw new Error("Chromium manifest top-level surface is not exact");
  }
  if (
    manifest.manifest_version !== 3 ||
    manifest.minimum_chrome_version !== "132" ||
    manifest.incognito !== "not_allowed" ||
    !isDeepStrictEqual(manifest.permissions, ["nativeMessaging"]) ||
    !isDeepStrictEqual(manifest.background, EXPECTED_BACKGROUND) ||
    !isDeepStrictEqual(manifest.content_scripts, [CHROMIUM_CONTENT_SCRIPT]) ||
    !isDeepStrictEqual(manifest.commands, EXPECTED_COMMANDS) ||
    typeof manifest.name !== "string" ||
    manifest.name.length === 0 ||
    typeof manifest.version !== "string" ||
    manifest.version.length === 0 ||
    typeof manifest.key !== "string" ||
    manifest.key.length === 0 ||
    typeof manifest.description !== "string" ||
    manifest.description.length === 0
  ) {
    throw new Error("Chromium manifest policy values are not exact");
  }
}
