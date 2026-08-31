import { isDeepStrictEqual } from "node:util";

const TOP_LEVEL_KEYS = Object.freeze([
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

const CONTENT_SCRIPT = Object.freeze({
  matches: Object.freeze(["http://localhost:4173/chromium.html"]),
  js: Object.freeze(["content-script.js"]),
  run_at: "document_idle",
  all_frames: false,
});

const BACKGROUND = Object.freeze({
  service_worker: "service-worker.js",
});

const COMMANDS = Object.freeze({
  "toggle-pause": Object.freeze({
    suggested_key: Object.freeze({
      default: "Alt+Shift+P",
    }),
    description: "Toggle broker pause state for trusted Badi fixture controllers",
  }),
});

function isRecord(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Frozen validator for the historical Chromium M2A/V2 localhost manifest.
 * Product manifests use their own versioned policy and must never change how
 * an already-committed V2 receipt is interpreted.
 */
export function assertHistoricalV2ChromiumManifest(manifest) {
  if (!isRecord(manifest)) {
    throw new Error("historical V2 Chromium manifest must be an object");
  }
  if (!isDeepStrictEqual(Object.keys(manifest).sort(), TOP_LEVEL_KEYS)) {
    throw new Error("historical V2 Chromium manifest top-level surface is not exact");
  }
  if (
    manifest.manifest_version !== 3 ||
    manifest.minimum_chrome_version !== "132" ||
    manifest.incognito !== "not_allowed" ||
    !isDeepStrictEqual(manifest.permissions, ["nativeMessaging"]) ||
    !isDeepStrictEqual(manifest.background, BACKGROUND) ||
    !isDeepStrictEqual(manifest.content_scripts, [CONTENT_SCRIPT]) ||
    !isDeepStrictEqual(manifest.commands, COMMANDS) ||
    typeof manifest.name !== "string" ||
    manifest.name.length === 0 ||
    typeof manifest.version !== "string" ||
    manifest.version.length === 0 ||
    typeof manifest.key !== "string" ||
    manifest.key.length === 0 ||
    typeof manifest.description !== "string" ||
    manifest.description.length === 0
  ) {
    throw new Error("historical V2 Chromium manifest policy values are not exact");
  }
}
