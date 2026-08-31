import { createHash } from "node:crypto";
import { mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const product = process.argv.includes("--product");
const outputRoot = join(packageRoot, product ? "dist-product" : "dist");

await rm(outputRoot, { recursive: true, force: true });
await mkdir(outputRoot, { recursive: true });

await build({
  absWorkingDir: packageRoot,
  entryPoints: product
    ? {
        "product-access": "src/product/access-popup.ts",
        "product-content-script": "src/product/content-script.ts",
        "product-service-worker": "src/product/service-worker.ts",
      }
    : {
        "content-script": "src/content/content-script.ts",
        "service-worker": "src/background/service-worker.ts",
      },
  outdir: outputRoot,
  bundle: true,
  charset: "utf8",
  format: "iife",
  platform: "browser",
  target: ["chrome132"],
  legalComments: "none",
  logLevel: "silent",
  minify: false,
  sourcemap: false,
  treeShaking: true,
});

const sourceManifest = JSON.parse(
  await readFile(join(packageRoot, product ? "manifest.product.json" : "manifest.json"), "utf8"),
);
await writeFile(
  join(outputRoot, "manifest.json"),
  `${JSON.stringify(sourceManifest, null, 2)}\n`,
  "utf8",
);
if (product) {
  await writeFile(
    join(outputRoot, "product-access.html"),
    await readFile(join(packageRoot, "product-access.html"), "utf8"),
    "utf8",
  );
}

async function filesBelow(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries.sort((left, right) => left.name.localeCompare(right.name))) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) files.push(...(await filesBelow(path)));
    else files.push(path);
  }
  return files;
}

const artifacts = [];
for (const path of await filesBelow(outputRoot)) {
  if (path.endsWith("BUILD_MANIFEST.json")) continue;
  const contents = await readFile(path);
  artifacts.push({
    path: relative(outputRoot, path).replaceAll("\\", "/"),
    bytes: contents.byteLength,
    sha256: createHash("sha256").update(contents).digest("hex"),
  });
}

const buildManifest = {
  schema: 1,
  package: "@badi/chromium",
  version: sourceManifest.version,
  target: product ? "chrome132-mv3-dillinger-product" : "chrome132-mv3",
  native_host: "io.github.ahuray.badi",
  artifacts,
};
await writeFile(
  join(outputRoot, "BUILD_MANIFEST.json"),
  `${JSON.stringify(buildManifest, null, 2)}\n`,
  "utf8",
);

process.stdout.write(
  `Built ${artifacts.length} deterministic ${product ? "product" : "fixture"} files in ${outputRoot}\n`,
);
