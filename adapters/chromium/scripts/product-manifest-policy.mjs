import { isDeepStrictEqual } from "node:util";

export const CHROMIUM_PRODUCT_MANIFEST_TOP_LEVEL_KEYS = Object.freeze([
  "action",
  "background",
  "commands",
  "description",
  "incognito",
  "key",
  "manifest_version",
  "minimum_chrome_version",
  "name",
  "optional_host_permissions",
  "permissions",
  "version",
]);

export const DILLINGER_OPTIONAL_HOST_PERMISSION = "https://dillinger.io:443/*";

function isRecord(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Rejects permission or injection drift in the separate product proof. */
export function assertExactChromiumProductManifest(manifest) {
  if (!isRecord(manifest)) {
    throw new Error("Chromium product manifest must be an object");
  }
  const keys = Object.keys(manifest).sort();
  if (!isDeepStrictEqual(keys, CHROMIUM_PRODUCT_MANIFEST_TOP_LEVEL_KEYS)) {
    throw new Error("Chromium product manifest top-level surface is not exact");
  }
  if (
    manifest.manifest_version !== 3 ||
    manifest.minimum_chrome_version !== "132" ||
    manifest.incognito !== "not_allowed" ||
    !isDeepStrictEqual(manifest.permissions, ["nativeMessaging", "scripting"]) ||
    !isDeepStrictEqual(manifest.optional_host_permissions, [
      DILLINGER_OPTIONAL_HOST_PERMISSION,
    ]) ||
    !isDeepStrictEqual(manifest.background, {
      service_worker: "product-service-worker.js",
    }) ||
    !isDeepStrictEqual(manifest.action, {
      default_title: "Badi Dillinger access",
      default_popup: "product-access.html",
    }) ||
    !isDeepStrictEqual(manifest.commands, {
      "accept-dillinger-suggestion": {
        suggested_key: { default: "Ctrl+Shift+Y" },
        description:
          "Accept the visible Badi suggestion in the exact Dillinger document",
      },
    }) ||
    typeof manifest.name !== "string" ||
    manifest.name.length === 0 ||
    typeof manifest.version !== "string" ||
    manifest.version.length === 0 ||
    typeof manifest.key !== "string" ||
    manifest.key.length === 0 ||
    typeof manifest.description !== "string" ||
    manifest.description.length === 0
  ) {
    throw new Error("Chromium product manifest policy values are not exact");
  }
}
