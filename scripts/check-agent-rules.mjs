// Checks for mistakes agents repeated here.
//
//   changelog    a behavior commit updates CHANGELOG.md itself
//   documents    future plans.md stays the only backlog
//   vocabulary   the Lab calls badi_broker::vocabulary::complete
//   notices      THIRD-PARTY-NOTICES.txt lists the Cargo.lock versions
//   feature-map  verify-badi quotes panel copy the QML actually shows
//   arch         CI installs Arch packages through scripts/ci-arch-snapshot.sh
//
//   node scripts/check-agent-rules.mjs [--only changelog|documents|vocabulary|notices|feature-map|arch] [--rev SHA]
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import process from "node:process";

const repository = path.resolve(import.meta.dirname, "..");
const CHECKS = ["changelog", "documents", "vocabulary", "notices", "feature-map", "arch"];
const ARCH_SCRIPT = "scripts/ci-arch-snapshot.sh";
const ARCH_KEY = "  BADI_ARCH_SNAPSHOT: |";
const FEATURE_MAP = ".cursor/skills/verify-badi/";
const NOTICES_FIX = "Run python3 scripts/third-party-notices.py and commit THIRD-PARTY-NOTICES.txt.";
const LAB_SRC = "evaluation/writing/lab-worker/src/";

const SKIP_PRODUCT = /(?:^|\/)(?:tests?|examples)\//u;
const SKIP_FILE = /(?:\.md|\.test\.mjs|\/tests\.rs)$/u;
const SKIP_PY = /(?:^|\/)test[-_.].+\.py$/u;

export function isProductPath(file) {
  if (SKIP_PRODUCT.test(file) || SKIP_FILE.test(file) || SKIP_PY.test(file) || file.endsWith("/live.mjs")) {
    return false;
  }
  if (file.startsWith("broker/src/") && file.endsWith(".rs")) return true;
  if (file.startsWith("broker/schemas/") && file.endsWith(".json")) return true;
  if (file.startsWith("broker/data/")) return true;
  if (file.startsWith("adapters/") || file.startsWith("ui/")) return true;
  // .SRCINFO is makepkg's printout of the PKGBUILD. The PKGBUILD commit is the behavior change.
  if (file.startsWith("packaging/") && file !== "packaging/aur/.SRCINFO") return true;
  if (file.startsWith("protocol/") && file.endsWith(".json")) return true;
  return /^scripts\/(?:badi-desktop|badi_install|install-[^/]+)\.py$/u.test(file);
}

export function parallelDocuments(files) {
  return files.filter((file) => {
    const base = file.split("/").pop().toLowerCase();
    return file === "next steps.md"
      || file === "docs/parity-status.md"
      || file.startsWith("docs/reports/")
      || file.startsWith("docs/plan/")
      || file.startsWith("docs/delivery/")
      || base.includes("handoff");
  });
}

function codeOnly(text) {
  return text.replace(/\/\*[\s\S]*?\*\//gu, "").replace(/\/\/.*$/gmu, "");
}

export function vocabularyProblems(files) {
  const problems = [];
  let callsBroker = false;
  for (const file of files) {
    if (!file.path.startsWith(LAB_SRC) || !file.path.endsWith(".rs")) continue;
    if (file.path.includes("/spelling/") || file.path.endsWith("/spelling.rs")) continue;
    const code = codeOnly(file.text);
    if (code.includes("unicode_word_indices")) {
      problems.push(
        `${file.path} scans words with unicode_word_indices. Call badi_broker::vocabulary::complete and delete the local scan.`,
      );
    }
    if (/(?:badi_broker::)?vocabulary::complete\s*\(/u.test(code)) callsBroker = true;
  }
  const looked = files.some((file) => file.path.startsWith(LAB_SRC) && file.path.endsWith(".rs"));
  if (looked && !callsBroker) {
    problems.push(
      `${LAB_SRC} does not call badi_broker::vocabulary::complete. Finish words through that function.`,
    );
  }
  return problems;
}

export function changelogProblems(sets) {
  const problems = [];
  for (const { label, paths, changelog } of sets) {
    const product = paths.filter(isProductPath);
    if (product.length === 0 || changelog) continue;
    problems.push(
      `${label} edits behavior and does not add a CHANGELOG.md line:\n  ${product.join("\n  ")}\nAdd one line under the newest heading in this commit. A later commit that only edits the changelog leaves this one failing.`,
    );
  }
  return problems;
}

function git(args, allowFailure = false) {
  const result = spawnSync("git", ["-c", "core.quotepath=false", ...args], {
    cwd: repository,
    encoding: "utf8",
  });
  if (result.status !== 0 && !allowFailure) {
    throw new Error((result.stderr || `git ${args.join(" ")} failed`).trim());
  }
  return result;
}

function lines(args) {
  return git([...args, "-z"]).stdout.split("\0").filter(Boolean);
}

/// Whether `diff` (a -U0 unified diff of CHANGELOG.md) adds a non-empty
/// line under the newest heading of the resulting text: `## Unreleased`
/// when present, else the first `## ` heading. A line appended under an
/// older release does not count.
export function diffAddsUnderNewestHeading(diff, text) {
  const fileLines = text.split("\n");
  let heading = fileLines.findIndex((line) => line.trim() === "## Unreleased");
  if (heading === -1) heading = fileLines.findIndex((line) => line.startsWith("## "));
  if (heading === -1) return false;
  let next = fileLines.findIndex((line, index) => index > heading && line.startsWith("## "));
  if (next === -1) next = fileLines.length;
  let newLine = 0;
  for (const line of diff.split("\n")) {
    const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/u.exec(line);
    if (hunk) {
      newLine = Number(hunk[1]);
    } else if (line.startsWith("+") && !line.startsWith("+++")) {
      if (line.slice(1).trim() !== "" && newLine > heading + 1 && newLine <= next) return true;
      newLine++;
    } else if (!line.startsWith("-") && !line.startsWith("\\")) {
      newLine++;
    }
  }
  return false;
}

function changelogAdded(args, text) {
  const diff = git(["diff", "-U0", ...args, "--", "CHANGELOG.md"], true);
  if (diff.status !== 0) return false;
  return diffAddsUnderNewestHeading(diff.stdout, text);
}

function changelogTextAt(rev) {
  const shown = git(["show", `${rev}:CHANGELOG.md`], true);
  return shown.status === 0 ? shown.stdout : "";
}

function ensureHistory() {
  if (git(["rev-parse", "--is-shallow-repository"]).stdout.trim() !== "true") return;
  git(["fetch", "--unshallow", "origin"]);
}

function event() {
  if (process.env.GITHUB_ACTIONS !== "true" || !process.env.GITHUB_EVENT_PATH) return null;
  return JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, "utf8"));
}

// The commits a push brought: from the old tip, or only the pushed commit when
// the push replaced history (forced, or the old tip is not an ancestor).
// A new branch (zero old tip) falls back to its merge base with main.
export function pushRange(payload, head, isAncestor) {
  if (!/^[0-9a-f]{40}$/u.test(payload.before ?? "") || /^0+$/u.test(payload.before)) return null;
  if (payload.forced || !isAncestor(payload.before, head)) return { only: head };
  return { base: payload.before, head };
}

function rangeEnds() {
  const payload = event();
  if (payload && process.env.GITHUB_EVENT_NAME === "pull_request") {
    const number = payload.pull_request.number;
    const baseRef = payload.pull_request.base.ref;
    if (!Number.isInteger(number) || !/^[A-Za-z0-9._/-]+$/u.test(baseRef)) {
      throw new Error("The pull request event did not name a branch.");
    }
    // Named refs, not FETCH_HEAD: a later fetch would overwrite FETCH_HEAD.
    git(["fetch", "origin", `pull/${number}/head:refs/review/pr-head`]);
    if (git(["rev-parse", "--verify", "--quiet", `origin/${baseRef}`], true).status !== 0) {
      git(["fetch", "origin", `${baseRef}:refs/review/pr-base`]);
    }
    const baseName = git(["rev-parse", "--verify", "--quiet", `origin/${baseRef}`], true).status === 0
      ? `origin/${baseRef}`
      : "refs/review/pr-base";
    return {
      base: git(["merge-base", baseName, "refs/review/pr-head"]).stdout.trim(),
      head: "refs/review/pr-head",
    };
  }
  if (payload && process.env.GITHUB_EVENT_NAME === "push") {
    const head = /^[0-9a-f]{40}$/u.test(payload.after) ? payload.after : git(["rev-parse", "HEAD"]).stdout.trim();
    const range = pushRange(payload, head, (older, newer) =>
      git(["merge-base", "--is-ancestor", older, newer], true).status === 0);
    if (range) return range;
  }
  if (git(["rev-parse", "--verify", "--quiet", "origin/main"], true).status !== 0) {
    git(["fetch", "origin", "main"]);
  }
  const head = git(["rev-parse", "HEAD"]).stdout.trim();
  return { base: git(["merge-base", "origin/main", head]).stdout.trim(), head };
}

function commitPaths(rev) {
  return lines(["diff-tree", "-r", "--root", "--no-renames", "--name-only", "--no-commit-id", rev]);
}

function commitSet(rev) {
  // A root commit is compared with the empty tree.
  const parent = git(["rev-parse", "--verify", "--quiet", `${rev}^`], true).status === 0
    ? `${rev}^`
    : git(["hash-object", "-t", "tree", "/dev/null"]).stdout.trim();
  return { label: rev, paths: commitPaths(rev), changelog: changelogAdded([parent, rev], changelogTextAt(rev)) };
}

function setsForRev(rev) {
  const parents = git(["rev-list", "--parents", "-n", "1", rev]).stdout.trim().split(/\s+/u).slice(1);
  if (parents.length < 2) return [commitSet(rev)];
  // A merge is the commits it brought in, not one squashed diff against the first parent.
  return git(["rev-list", "--no-merges", `${parents[0]}..${rev}`]).stdout
    .split("\n")
    .filter(Boolean)
    .map(commitSet);
}

function rangeSets() {
  ensureHistory();
  const { base, head, only } = rangeEnds();
  const commits = only
    ? setsForRev(only)
    : git(["rev-list", "--no-merges", `${base}..${head}`]).stdout.split("\n").filter(Boolean).map(commitSet);
  const staged = lines(["diff", "--cached", "--name-only"]);
  if (staged.length > 0) {
    commits.push({ label: "staged changes", paths: staged, changelog: changelogAdded(["--cached"], changelogTextAt("")) });
  }
  const unstaged = [...new Set([
    ...lines(["diff", "--name-only"]),
    ...lines(["ls-files", "--others", "--exclude-standard"]),
  ])];
  if (unstaged.length > 0) {
    const worktree = existsSync(path.join(repository, "CHANGELOG.md"))
      ? readFileSync(path.join(repository, "CHANGELOG.md"), "utf8") : "";
    commits.push({ label: "unstaged changes", paths: unstaged, changelog: changelogAdded([], worktree) });
  }
  return commits;
}

function treeFiles(rev) {
  const found = rev
    ? lines(["ls-tree", "-r", "--name-only", rev])
    : lines(["ls-files", "--cached", "--others", "--exclude-standard"]);
  if (rev) return found;
  return found.filter((file) => existsSync(path.join(repository, file)));
}

function readAt(rev, file) {
  if (!rev) return readFileSync(path.join(repository, file), "utf8");
  return git(["show", `${rev}:${file}`]).stdout;
}

function vocabularyFiles(rev) {
  if (!rev) {
    const root = path.join(repository, LAB_SRC);
    const found = [];
    const walk = (directory) => {
      for (const entry of readdirSync(directory, { withFileTypes: true })) {
        const full = path.join(directory, entry.name);
        if (entry.isDirectory()) walk(full);
        else if (entry.name.endsWith(".rs")) found.push(path.relative(repository, full));
      }
    };
    walk(root);
    return found.map((file) => ({ path: file, text: readAt(null, file) }));
  }
  return treeFiles(rev)
    .filter((file) => file.startsWith(LAB_SRC) && file.endsWith(".rs"))
    .map((file) => ({ path: file, text: readAt(rev, file) }));
}

export function cargoLockVersions(text) {
  const versions = new Map();
  for (const match of text.matchAll(/\[\[package\]\]\nname = "([^"]+)"\nversion = "([^"]+)"/gu)) {
    const found = versions.get(match[1]) ?? new Set();
    found.add(match[2]);
    versions.set(match[1], found);
  }
  return versions;
}

// Cargo.lock is the source of the versions linked into the binaries.
// THIRD-PARTY-NOTICES.txt is generated from it. Dependabot and agents
// updated the lock and left the notices file behind (PRs 155, 156, 178).
export function noticesProblems(lockText, noticesText) {
  const versions = cargoLockVersions(lockText);
  const marker = "Components\n----------\n";
  const start = noticesText.indexOf(marker);
  const end = noticesText.indexOf("\nLicence texts\n");
  if (start === -1 || end === -1 || end < start) {
    return [`THIRD-PARTY-NOTICES.txt has no Components section. ${NOTICES_FIX}`];
  }
  const problems = [];
  for (const line of noticesText.slice(start + marker.length, end).split("\n")) {
    if (!line.trim() || line.includes("compiled into the Fcitx addon")) continue;
    const match = /^(\S+) (\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?)/u.exec(line);
    if (!match) {
      problems.push(`THIRD-PARTY-NOTICES.txt has an unreadable component line: ${line}\n${NOTICES_FIX}`);
      continue;
    }
    const name = match[1];
    const version = match[2];
    const have = versions.get(name);
    if (!have || !have.has(version)) {
      const listed = have ? [...have].sort().join(", ") : "nothing";
      problems.push(`THIRD-PARTY-NOTICES.txt lists ${name} ${version} but Cargo.lock has ${listed}. ${NOTICES_FIX}`);
    }
  }
  return problems;
}

// The panel hero is one string in SettingsView.qml. The feature map copied
// a different one (`HELD · SCREEN SHARED`) through three upkeep passes
// (PRs 153, 154, 161) until PR 179. A backtick containing "·" has to be
// that source, not a paraphrase.
export function featureMapProblems(markdownFiles, sourceText) {
  const problems = [];
  for (const file of markdownFiles) {
    if (!file.path.startsWith(FEATURE_MAP) || !file.path.endsWith(".md")) continue;
    for (const match of file.text.matchAll(/`([^`\n]*·[^`\n]*)`/gu)) {
      const copy = match[1];
      if (!sourceText.includes(copy)) {
        problems.push(
          `${file.path} quotes \`${copy}\`, which is not the panel or status text. Copy it from ui/omarchy-plugin/*.qml or scripts/badi-desktop.py.`,
        );
      }
    }
  }
  return problems;
}

function textAt(rev, file) {
  if (!rev) {
    const full = path.join(repository, file);
    if (!existsSync(full)) return null;
    return readFileSync(full, "utf8");
  }
  const shown = git(["show", `${rev}:${file}`], true);
  return shown.status === 0 ? shown.stdout : null;
}

function featureMapAt(rev) {
  const names = treeFiles(rev);
  const markdown = names
    .filter((file) => file.startsWith(FEATURE_MAP) && file.endsWith(".md"))
    .map((file) => ({ path: file, text: textAt(rev, file) ?? "" }));
  const sources = names.filter((file) =>
    (file.startsWith("ui/omarchy-plugin/") && file.endsWith(".qml")) || file === "scripts/badi-desktop.py");
  const sourceText = sources.map((file) => textAt(rev, file) ?? "").join("\n");
  return featureMapProblems(markdown, sourceText);
}

function noticesAt(rev) {
  const lock = textAt(rev, "Cargo.lock");
  const notices = textAt(rev, "THIRD-PARTY-NOTICES.txt");
  if (lock === null || notices === null) {
    return [`Cargo.lock or THIRD-PARTY-NOTICES.txt is missing. ${NOTICES_FIX}`];
  }
  return noticesProblems(lock, notices);
}

function normalizeScript(text) {
  return `${text.replace(/\s+$/u, "")}\n`;
}

// The snapshot helper is inlined into the workflow because the Arch image
// has no git until pacman has run. The block is the lines indented under
// BADI_ARCH_SNAPSHOT, which is the script with a four-space indent.
export function embeddedSnapshot(ciText) {
  const lines = ciText.split("\n");
  const start = lines.indexOf(ARCH_KEY);
  if (start === -1) return null;
  const body = [];
  for (let index = start + 1; index < lines.length; index++) {
    const line = lines[index];
    if (line === "") {
      body.push("");
      continue;
    }
    if (!line.startsWith("    ")) break;
    body.push(line.slice(4));
  }
  while (body.length > 0 && body[body.length - 1] === "") body.pop();
  return body.length === 0 ? null : `${body.join("\n")}\n`;
}

export function barePacmanLines(ciText) {
  const lines = ciText.split("\n");
  const start = lines.indexOf(ARCH_KEY);
  let end = start;
  if (start !== -1) {
    end = start + 1;
    for (; end < lines.length; end++) {
      const line = lines[end];
      if (line !== "" && !line.startsWith("    ")) break;
    }
  }
  const found = [];
  lines.forEach((line, index) => {
    if (start !== -1 && index > start && index < end) return;
    if (/pacman\s+-S/u.test(line)) found.push(`${index + 1}: ${line.trim()}`);
  });
  return found;
}

export function archSnapshotProblems(ciText, scriptText) {
  const problems = [];
  const bare = barePacmanLines(ciText);
  if (bare.length > 0) {
    problems.push(
      `.github/workflows/ci.yml installs packages with pacman directly:\n  ${bare.join("\n  ")}\narchive.archlinux.org stalls these jobs ("Operation too slow"; runs 38242538936, 38684819408, 39558825776). Call ${ARCH_SCRIPT}. Before checkout, pipe BADI_ARCH_SNAPSHOT, which must be a copy of that script. It sets DisableDownloadTimeout and retries.`,
    );
  }
  if (scriptText === null) return problems;
  const embedded = embeddedSnapshot(ciText);
  if (embedded === null) {
    problems.push(
      `.github/workflows/ci.yml has no BADI_ARCH_SNAPSHOT block. Paste ${ARCH_SCRIPT} there. The Arch image has no git until that script has run.`,
    );
  } else if (embedded !== normalizeScript(scriptText)) {
    problems.push(
      `BADI_ARCH_SNAPSHOT in .github/workflows/ci.yml is not ${ARCH_SCRIPT}. Paste the script into that block. The job container cannot read the file until pacman has installed git.`,
    );
  }
  if (!scriptText.includes("DisableDownloadTimeout") || !scriptText.includes("pacman_retry")) {
    problems.push(`${ARCH_SCRIPT} must set DisableDownloadTimeout and retry pacman.`);
  }
  return problems;
}

function archAt(rev) {
  const ci = textAt(rev, ".github/workflows/ci.yml");
  const script = textAt(rev, ARCH_SCRIPT);
  if (ci === null) return [".github/workflows/ci.yml is missing."];
  return archSnapshotProblems(ci, script);
}

function selected() {
  const only = process.argv.indexOf("--only");
  if (only === -1) return CHECKS;
  const name = process.argv[only + 1];
  if (!CHECKS.includes(name)) throw new Error(`--only must be one of ${CHECKS.join(", ")}`);
  return [name];
}

function revision() {
  const index = process.argv.indexOf("--rev");
  if (index === -1) return null;
  const rev = process.argv[index + 1];
  if (!rev) throw new Error("--rev needs a commit");
  return git(["rev-parse", "--verify", rev]).stdout.trim();
}

function run() {
  const want = selected();
  const rev = revision();
  const problems = [];
  if (want.includes("changelog")) problems.push(...changelogProblems(rev ? setsForRev(rev) : rangeSets()));
  if (want.includes("documents")) {
    const found = parallelDocuments(treeFiles(rev));
    if (found.length > 0) {
      problems.push(
        `Parallel planning or report document:\n  ${found.join("\n  ")}\nfuture plans.md is the only backlog. Put the decision there, the contract in the runbook, and the trial in the pull request, then delete the file.`,
      );
    }
  }
  if (want.includes("vocabulary")) problems.push(...vocabularyProblems(vocabularyFiles(rev)));
  if (want.includes("notices")) problems.push(...noticesAt(rev));
  if (want.includes("feature-map")) problems.push(...featureMapAt(rev));
  if (want.includes("arch")) problems.push(...archAt(rev));
  if (problems.length > 0) {
    process.stderr.write(`${problems.join("\n\n")}\n`);
    process.exitCode = 1;
    return;
  }
  const where = rev ? ` at ${rev.slice(0, 12)}` : "";
  process.stdout.write(`Agent rules passed (${want.join(", ")})${where}.\n`);
}

if (process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url) run();
