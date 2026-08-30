import { execFileSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import path from "node:path";

const repository = path.resolve(import.meta.dirname, "..");
const banned = [
  "Oma" + "type",
  "oma" + "type",
  "OMA" + "TYPE",
  ["io", "badi", "broker"].join("."),
];
const historicalContentCounts = new Map([
  [
    "docs/delivery/2026-08-30-independent-adversarial-audit.md",
    new Map([
      ["Oma" + "type", 2],
      ["oma" + "type", 1],
    ]),
  ],
]);
const files = execFileSync("git", ["ls-files", "-z"], {
  cwd: repository,
  encoding: "utf8",
})
  .split("\0")
  .filter(Boolean);

const violations = [];
for (const file of files) {
  for (const value of banned) {
    if (file.includes(value)) violations.push(`${file}: path contains ${value}`);
  }
  const content = await readFile(path.join(repository, file), "utf8");
  const expectedCounts = historicalContentCounts.get(file);
  for (const value of banned) {
    const actualCount = content.split(value).length - 1;
    const expectedCount = expectedCounts?.get(value) ?? 0;
    if (actualCount !== expectedCount) {
      violations.push(
        `${file}: content contains ${value} ${actualCount} time(s); expected ${expectedCount}`,
      );
    }
  }
}

if (violations.length > 0) {
  process.stderr.write(`${violations.join("\n")}\n`);
  process.exitCode = 1;
} else {
  process.stdout.write(`Checked Badi naming across ${files.length} tracked files.\n`);
}
