import { execFile } from "node:child_process";
import path from "node:path";
import process from "node:process";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const repository = path.resolve(import.meta.dirname, "..");
const arguments_ = process.argv.slice(2);
let base = process.env.CAPABILITY_BASE_SHA || "HEAD^";
const zeroObjectId = /^0{40}$/u;

if (arguments_.length > 0) {
  if (arguments_.length !== 2 || arguments_[0] !== "--base") {
    process.stderr.write(
      "Usage: node scripts/check-capability-immutability.mjs [--base GIT_REF]\n",
    );
    process.exit(2);
  }
  base = arguments_[1];
}

if (
  typeof base !== "string" ||
  base.length === 0 ||
  base.startsWith("-") ||
  !/^[A-Za-z0-9_./^~-]+$/u.test(base)
) {
  process.stderr.write("Capability immutability base is not a safe Git ref.\n");
  process.exit(2);
}
if (zeroObjectId.test(base)) {
  process.stderr.write(
    "Capability immutability base cannot be GitHub's all-zero push sentinel.\n",
  );
  process.exit(2);
}

try {
  const [{ stdout: baseCommit }, { stdout: headCommit }] = await Promise.all([
    execFileAsync("git", ["rev-parse", "--verify", `${base}^{commit}`], {
      cwd: repository,
      encoding: "utf8",
      timeout: 5_000,
    }),
    execFileAsync("git", ["rev-parse", "--verify", "HEAD^{commit}"], {
      cwd: repository,
      encoding: "utf8",
      timeout: 5_000,
    }),
  ]);
  if (baseCommit.trim() === headCommit.trim()) {
    process.stderr.write(
      "Capability immutability base must identify a commit before HEAD.\n",
    );
    process.exit(1);
  }
  const resolvedBase = baseCommit.trim();
  const resolvedHead = headCommit.trim();
  try {
    await execFileAsync(
      "git",
      ["merge-base", "--is-ancestor", resolvedBase, resolvedHead],
      { cwd: repository, timeout: 5_000 },
    );
  } catch {
    process.stderr.write(
      `Capability immutability base is not an ancestor of HEAD: ${base}\n`,
    );
    process.exit(1);
  }
  const { stdout } = await execFileAsync(
    "git",
    [
      "diff",
      "--name-only",
      "--diff-filter=DMRT",
      "-z",
      `${resolvedBase}...${resolvedHead}`,
      "--",
      ":(glob)capabilities/*.json",
      ":(glob)capabilities/evidence/*.json",
      ":(glob)capabilities/v*/*.json",
      ":(glob)capabilities/v*/manifest-policy.mjs",
      ":(glob)capabilities/v*/policy.mjs",
      ":(glob)capabilities/v*/validator.mjs",
    ],
    { cwd: repository, encoding: "utf8", timeout: 5_000 },
  );
  const changed = stdout.split("\0").filter((value) => value.length > 0);
  if (changed.length > 0) {
    process.stderr.write(
      "Committed capability identities are immutable; create new files instead:\n" +
        `${changed.map((file) => `- ${file}`).join("\n")}\n`,
    );
    process.exit(1);
  }
  process.stdout.write(`Capability evidence is immutable relative to ${base}.\n`);
} catch (error) {
  if (typeof error === "object" && error !== null && "code" in error && error.code === 1) {
    throw error;
  }
  process.stderr.write(
    `Unable to validate capability immutability against ${base}; ensure the Git object is available.\n`,
  );
  process.exit(1);
}
