import { readdir, readFile, stat } from "node:fs/promises";
import path from "node:path";
import process from "node:process";

const repository = path.resolve(import.meta.dirname, "..");
const skippedDirectories = new Set([
  ".git",
  "coverage",
  "dist",
  "node_modules",
  "target",
]);

async function markdownFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const nested = await Promise.all(
    entries.map(async (entry) => {
      const absolute = path.join(directory, entry.name);
      if (entry.isDirectory() && !skippedDirectories.has(entry.name)) {
        return markdownFiles(absolute);
      }
      return entry.isFile() && entry.name.endsWith(".md") ? [absolute] : [];
    }),
  );
  return nested.flat();
}

function linkTargets(markdown) {
  const targets = [];
  const inline = /!?\[[^\]\n]*\]\((<[^>]+>|[^)\s]+)(?:\s+["'][^)]*)?\)/gu;
  const reference = /^\s*\[[^\]]+\]:\s*(?:<([^>]+)>|(\S+))/gmu;

  for (const match of markdown.matchAll(inline)) {
    targets.push(match[1]);
  }
  for (const match of markdown.matchAll(reference)) {
    targets.push(match[1] ?? match[2]);
  }
  return targets;
}

function localPath(source, rawTarget) {
  const unwrapped = rawTarget.startsWith("<") && rawTarget.endsWith(">")
    ? rawTarget.slice(1, -1)
    : rawTarget;
  if (
    unwrapped.startsWith("#") ||
    /^[a-z][a-z0-9+.-]*:/iu.test(unwrapped)
  ) {
    return null;
  }

  const pathOnly = unwrapped.split(/[?#]/u, 1)[0];
  if (pathOnly.length === 0) {
    return null;
  }
  let decoded;
  try {
    decoded = decodeURIComponent(pathOnly);
  } catch {
    decoded = pathOnly;
  }
  return path.isAbsolute(decoded)
    ? decoded
    : path.resolve(path.dirname(source), decoded);
}

async function exists(target) {
  try {
    await stat(target);
    return true;
  } catch (error) {
    if (error?.code === "ENOENT" || error?.code === "ENOTDIR") {
      return false;
    }
    throw error;
  }
}

const failures = [];
let checked = 0;
for (const file of await markdownFiles(repository)) {
  const markdown = await readFile(file, "utf8");
  for (const rawTarget of linkTargets(markdown)) {
    const target = localPath(file, rawTarget);
    if (target === null) {
      continue;
    }
    checked += 1;
    if (!(await exists(target))) {
      failures.push(
        `${path.relative(repository, file)} -> ${rawTarget}`,
      );
    }
  }
}

if (failures.length > 0) {
  process.stderr.write(`Broken local Markdown links:\n${failures.join("\n")}\n`);
  process.exitCode = 1;
} else {
  process.stdout.write(`Checked ${checked} local Markdown links.\n`);
}
