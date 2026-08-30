export const CHROMIUM_MANIFEST_TOP_LEVEL_KEYS: readonly string[];

export const CHROMIUM_CONTENT_SCRIPT: Readonly<{
  matches: readonly ["http://localhost:4173/chromium.html"];
  js: readonly ["content-script.js"];
  run_at: "document_idle";
  all_frames: false;
}>;

export function assertExactChromiumManifest(manifest: unknown): void;
