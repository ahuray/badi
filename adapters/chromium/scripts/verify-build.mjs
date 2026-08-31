import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
function buildManifestPath(product) {
  return resolve(packageRoot, product ? "dist-product/BUILD_MANIFEST.json" : "dist/BUILD_MANIFEST.json");
}

async function runBuild(product) {
  await new Promise((resolvePromise, reject) => {
    const child = spawn(process.execPath, [
      "scripts/build.mjs",
      ...(product ? ["--product"] : []),
    ], {
      cwd: packageRoot,
      stdio: "inherit",
    });
    child.once("error", reject);
    child.once("exit", (code) => {
      if (code === 0) resolvePromise();
      else reject(new Error(`Build exited with status ${String(code)}`));
    });
  });
  return readFile(buildManifestPath(product), "utf8");
}

for (const product of [false, true]) {
  const first = await runBuild(product);
  const second = await runBuild(product);
  if (first !== second) {
    throw new Error(
      `Chromium ${product ? "product" : "fixture"} build is not byte-deterministic`,
    );
  }
}
process.stdout.write("Deterministic fixture and product builds verified across two clean builds\n");
