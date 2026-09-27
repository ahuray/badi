import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";

const repository = path.resolve(import.meta.dirname, "..");

// Check exactly what a clean checkout contains: tracked files plus untracked
// files that are not ignored. Links into ignored paths pass locally but break CI.
const checkoutFiles = new Set(
  execFileSync("git", ["ls-files", "--cached", "--others", "--exclude-standard", "-z"], {
    cwd: repository,
    encoding: "utf8",
  })
    .split("\0")
    .filter(Boolean)
    .map((file) => path.join(repository, file))
    .filter((file) => existsSync(file)),
);
const checkoutDirectories = new Set();
for (const file of checkoutFiles) {
  for (let directory = path.dirname(file); directory.startsWith(repository); directory = path.dirname(directory)) {
    if (checkoutDirectories.has(directory)) break;
    checkoutDirectories.add(directory);
  }
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

function decodeComponent(value) {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

function localTarget(source, rawTarget) {
  const unwrapped = rawTarget.startsWith("<") && rawTarget.endsWith(">")
    ? rawTarget.slice(1, -1)
    : rawTarget;
  if (/^[a-z][a-z0-9+.-]*:/iu.test(unwrapped)) {
    return null;
  }

  const hashIndex = unwrapped.indexOf("#");
  const withoutFragment = hashIndex === -1
    ? unwrapped
    : unwrapped.slice(0, hashIndex);
  const pathOnly = withoutFragment.split("?", 1)[0];
  const decodedPath = decodeComponent(pathOnly);
  const absolute = decodedPath.length === 0
    ? source
    : path.isAbsolute(decodedPath)
      ? decodedPath
      : path.resolve(path.dirname(source), decodedPath);
  return {
    absolute,
    fragment: hashIndex === -1
      ? null
      : decodeComponent(unwrapped.slice(hashIndex + 1)),
  };
}

function exists(target) {
  return checkoutFiles.has(target) || checkoutDirectories.has(target);
}

function lineCount(value) {
  if (value.length === 0) return 0;
  return value.split("\n").length - Number(value.endsWith("\n"));
}

function headingTextToSlug(value) {
  return value
    .replace(/<[^>]*>/gu, "")
    .replace(/!\[([^\]]*)\]\([^)]*\)/gu, "$1")
    .replace(/\[([^\]]+)\]\([^)]*\)/gu, "$1")
    .replace(/[`*_~]/gu, "")
    .toLocaleLowerCase("en-US")
    .replace(/[^\p{Letter}\p{Number}\s_-]/gu, "")
    .trim()
    .replace(/\s+/gu, "-");
}

function markdownAnchors(markdown) {
  const anchors = new Set();
  const counts = new Map();
  let fence = null;
  for (const line of markdown.split("\n")) {
    const fenceMatch = /^\s*(`{3,}|~{3,})/u.exec(line);
    if (fenceMatch !== null) {
      const marker = fenceMatch[1][0];
      if (fence === null) fence = marker;
      else if (fence === marker) fence = null;
      continue;
    }
    if (fence !== null) continue;
    const heading = /^\s{0,3}#{1,6}\s+(.+?)\s*#*\s*$/u.exec(line);
    if (heading === null) continue;
    const base = headingTextToSlug(heading[1]);
    if (base.length === 0) continue;
    const duplicate = counts.get(base) ?? 0;
    counts.set(base, duplicate + 1);
    anchors.add(duplicate === 0 ? base : `${base}-${duplicate}`);
  }
  return anchors;
}

async function fragmentExists(target, fragment) {
  if (fragment === null || fragment.length === 0) return true;
  const contents = await readFile(target, "utf8");
  const line = /^L([1-9][0-9]*)(?:-L?([1-9][0-9]*))?$/u.exec(fragment);
  if (line !== null) {
    const first = Number(line[1]);
    const last = Number(line[2] ?? line[1]);
    return first <= last && last <= lineCount(contents);
  }
  if (path.extname(target).toLowerCase() !== ".md") return true;
  return markdownAnchors(contents).has(fragment);
}

const failures = [];
let checked = 0;
for (const file of [...checkoutFiles].filter((file) => file.endsWith(".md"))) {
  const markdown = await readFile(file, "utf8");
  for (const rawTarget of linkTargets(markdown)) {
    const target = localTarget(file, rawTarget);
    if (target === null) {
      continue;
    }
    checked += 1;
    if (
      !exists(target.absolute) ||
      !(await fragmentExists(target.absolute, target.fragment))
    ) {
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
