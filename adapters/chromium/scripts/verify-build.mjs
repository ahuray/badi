import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const buildManifestPath = resolve(packageRoot, "dist/BUILD_MANIFEST.json");

async function runBuild() {
  await new Promise((resolvePromise, reject) => {
    const child = spawn(process.execPath, ["scripts/build.mjs"], {
      cwd: packageRoot,
      stdio: "inherit",
    });
    child.once("error", reject);
    child.once("exit", (code) => {
      if (code === 0) resolvePromise();
      else reject(new Error(`Build exited with status ${String(code)}`));
    });
  });
  return readFile(buildManifestPath, "utf8");
}

const first = await runBuild();
const second = await runBuild();
if (first !== second) {
  throw new Error("Chromium build is not byte-deterministic");
}
process.stdout.write("Deterministic build verified across two clean builds\n");
