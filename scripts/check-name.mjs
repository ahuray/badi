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
const files = execFileSync("git", ["ls-files", "--cached", "--others", "--exclude-standard", "-z"], {
  cwd: repository,
  encoding: "utf8",
})
  .split("\0")
  .filter(Boolean);

const violations = [];
for (const file of files) {
  let content;
  try {
    content = await readFile(path.join(repository, file), "utf8");
  } catch (error) {
    if (error.code === "ENOENT") continue;
    throw error;
  }
  for (const value of banned) {
    if (file.includes(value)) violations.push(`${file}: path contains ${value}`);
  }
  for (const value of banned) {
    const actualCount = content.split(value).length - 1;
    const expectedCount = 0;
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
  process.stdout.write(`Checked Badi naming in the working tree.\n`);
}
