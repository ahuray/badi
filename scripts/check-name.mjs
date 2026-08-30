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
  for (const value of banned) {
    if (content.includes(value)) violations.push(`${file}: content contains ${value}`);
  }
}

if (violations.length > 0) {
  process.stderr.write(`${violations.join("\n")}\n`);
  process.exitCode = 1;
} else {
  process.stdout.write(`Checked Badi naming across ${files.length} tracked files.\n`);
}
