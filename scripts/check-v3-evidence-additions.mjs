import { execFile } from "node:child_process";
import { lstat, readFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { promisify } from "node:util";
import {
  assertV3ProductCellAppendOnly,
  assertV3ProductCellFileIdentity,
  looksLikeV3ProductCell,
} from "../capabilities/v3/validator.mjs";

const execFileAsync = promisify(execFile);
const repository = path.resolve(import.meta.dirname, "..");
const checker = path.join(repository, "scripts/check-capabilities.mjs");
const base = process.env.CAPABILITY_BASE_SHA;
const safeGitRef = /^(?!-)(?!.*(?:\.\.|\/\/))[A-Za-z0-9][A-Za-z0-9_./^~-]{0,255}$/u;
const zeroObjectId = /^0{40}$/u;

function fail(message) {
  throw new Error(message);
}

async function runGit(arguments_, label) {
  try {
    return await execFileAsync("git", arguments_, {
      cwd: repository,
      encoding: "utf8",
      maxBuffer: 1024 * 1024,
      timeout: 10_000,
    });
  } catch {
    fail(label);
  }
}

async function runChecker(receiptId, requireLive) {
  const arguments_ = [
    checker,
    "--require-current",
    "--receipt-id",
    receiptId,
  ];
  if (requireLive) arguments_.push("--require-live");
  try {
    await execFileAsync(process.execPath, arguments_, {
      cwd: repository,
      encoding: "utf8",
      maxBuffer: 16 * 1024 * 1024,
      timeout: 60_000,
    }).then(({ stdout, stderr }) => {
      process.stdout.write(stdout);
      process.stderr.write(stderr);
    });
  } catch (error) {
    if (typeof error === "object" && error !== null) {
      if (typeof error.stdout === "string") process.stdout.write(error.stdout);
      if (typeof error.stderr === "string") process.stderr.write(error.stderr);
    }
    fail(`V3 evidence validation failed for ${receiptId}.`);
  }
}

try {
  if (typeof base !== "string" || !safeGitRef.test(base)) {
    fail("CAPABILITY_BASE_SHA must be set to a safe Git ref.");
  }
  if (zeroObjectId.test(base)) {
    fail("CAPABILITY_BASE_SHA cannot be GitHub's all-zero push sentinel.");
  }
  const { stdout: baseCommit } = await runGit(
    ["rev-parse", "--verify", `${base}^{commit}`],
    `CAPABILITY_BASE_SHA does not resolve to a commit: ${base}`,
  );
  const { stdout: headCommit } = await runGit(
    ["rev-parse", "--verify", "HEAD^{commit}"],
    "HEAD does not resolve to a commit.",
  );
  if (baseCommit.trim() === headCommit.trim()) {
    fail("CAPABILITY_BASE_SHA must identify a commit before HEAD.");
  }
  const resolvedBase = baseCommit.trim();
  await runGit(
    ["merge-base", "--is-ancestor", resolvedBase, "HEAD"],
    `CAPABILITY_BASE_SHA is not an ancestor of HEAD: ${base}`,
  );
  const { stdout } = await runGit(
    [
      "diff",
      "--name-status",
      "-z",
      "--no-renames",
      `${resolvedBase}...HEAD`,
      "--",
      ":(top,glob)capabilities/*.json",
    ],
    `Unable to compare capability receipts with ${base}.`,
  );
  const fields = stdout.split("\0");
  if (fields.at(-1) === "") fields.pop();
  if (fields.length % 2 !== 0) {
    fail("Git returned an incomplete capability receipt diff.");
  }

  const receipts = [];
  for (let index = 0; index < fields.length; index += 2) {
    const status = fields[index];
    const relativePath = fields[index + 1];
    assertV3ProductCellAppendOnly(status, relativePath);
    if (status !== "A") {
      continue;
    }

    const absolutePath = path.join(repository, relativePath);
    const metadata = await lstat(absolutePath);
    if (!metadata.isFile() || metadata.isSymbolicLink()) {
      fail(`New capability receipt is not a regular file: ${relativePath}`);
    }
    let receipt;
    try {
      receipt = JSON.parse(await readFile(absolutePath, "utf8"));
    } catch {
      fail(`New capability receipt is not valid JSON: ${relativePath}`);
    }
    if (!looksLikeV3ProductCell(relativePath, receipt)) continue;
    assertV3ProductCellFileIdentity(relativePath, receipt);
    receipts.push({ id: receipt.id, live: receipt.status === "live" });
  }

  receipts.sort((left, right) => left.id.localeCompare(right.id));
  for (const receipt of receipts) {
    await runChecker(receipt.id, receipt.live);
  }
  process.stdout.write(
    `Validated ${receipts.length} new V3 product-cell receipt(s) relative to ${base}.\n`,
  );
} catch (error) {
  const detail = error instanceof Error ? error.message : String(error);
  process.stderr.write(`${detail}\n`);
  process.exitCode = 1;
}
