import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { test } from "node:test";
import { archSnapshotProblems, changelogProblems, cargoLockVersions, diffAddsUnderNewestHeading, featureMapProblems, isProductPath, noticesProblems, parallelDocuments, pushRange, vocabularyProblems } from "./check-agent-rules.mjs";

const lookup = "evaluation/writing/lab-worker/src/context_lookup.rs";

test("product paths are behavior, not tests or notes", () => {
  assert.equal(isProductPath("broker/src/policy.rs"), true);
  assert.equal(isProductPath("broker/schemas/badi.settings.v2.schema.json"), true);
  assert.equal(isProductPath("adapters/shell/preview.c"), true);
  assert.equal(isProductPath("adapters/neovim/lua/badi/init.lua"), true);
  assert.equal(isProductPath("adapters/libreoffice/extension/Jobs.xcu"), true);
  assert.equal(isProductPath("ui/omarchy-plugin/fcitx-theme.py"), true);
  assert.equal(isProductPath("packaging/systemd/badi-broker.service"), true);
  assert.equal(isProductPath("packaging/aur/PKGBUILD"), true);
  assert.equal(isProductPath("packaging/aur/.SRCINFO"), false);
  assert.equal(isProductPath("broker/src/engine/tests/spelling.rs"), false);
  assert.equal(isProductPath("broker/src/settings/tests.rs"), false);
  assert.equal(isProductPath("adapters/shell/test-preview.py"), false);
  assert.equal(isProductPath("adapters/shell/README.md"), false);
  assert.equal(isProductPath("README.md"), false);
  assert.equal(isProductPath("scripts/install-desktop.py"), true);
  assert.equal(isProductPath("scripts/test-install-desktop.py"), false);
  assert.equal(isProductPath("scripts/check-agent-rules.mjs"), false);
});

test("parallel documents are the extra backlog, not the runbooks", () => {
  assert.deepEqual(parallelDocuments([
    "next steps.md",
    "docs/reports/2026-10-13-overnight-audit.md",
    "docs/parity-status.md",
    "docs/handoff-preview.md",
    "future plans.md",
    "docs/decisions/0003-ime-parity-append-only.md",
    "AGENTS.md",
  ]), [
    "next steps.md",
    "docs/reports/2026-10-13-overnight-audit.md",
    "docs/parity-status.md",
    "docs/handoff-preview.md",
  ]);
});

test("a Lab word scan fails and a call to the broker passes", () => {
  assert.ok(vocabularyProblems([{ path: lookup, text: "fn stem() { text.unicode_word_indices() }" }]).length > 0);
  assert.deepEqual(vocabularyProblems([
    { path: lookup, text: "vocabulary::complete(&before, \"\", language, &words)" },
    { path: "evaluation/writing/lab-worker/src/lib.rs", text: "badi_broker::vocabulary::complete(&before, &after, language, vocabulary)" },
  ]), []);
  assert.equal(vocabularyProblems([
    { path: lookup, text: "// vocabulary::complete(\nfn local() { text.unicode_word_indices() }" },
  ]).length > 0, true);
});

test("changelog silence fails only when the commit edits behavior", () => {
  assert.equal(changelogProblems([{ label: "abc", paths: ["README.md"], changelog: false }]).length, 0);
  assert.equal(changelogProblems([{ label: "abc", paths: ["broker/src/policy.rs"], changelog: true }]).length, 0);
  assert.match(changelogProblems([{ label: "abc", paths: ["broker/src/policy.rs"], changelog: false }])[0], /CHANGELOG\.md/);
  assert.match(changelogProblems([{ label: "abc", paths: ["adapters/shell/preview.c"], changelog: false }])[0], /preview\.c/);
});

test("only a changelog line under the newest heading counts", () => {
  const text = "# Changelog\n\n## Unreleased\n\n- New thing.\n- Old thing.\n\n## 0.2.0\n\n- Released.\n- Sneaked in.\n";
  const underUnreleased = "@@ -4,0 +5 @@\n+- New thing.\n";
  const underRelease = "@@ -9,0 +11 @@\n+- Sneaked in.\n";
  const blankOnly = "@@ -4,0 +5 @@\n+\n";
  assert.equal(diffAddsUnderNewestHeading(underUnreleased, text), true);
  assert.equal(diffAddsUnderNewestHeading(underRelease, text), false, "a line under an old release is not the newest heading");
  assert.equal(diffAddsUnderNewestHeading(blankOnly, text), false);
  assert.equal(diffAddsUnderNewestHeading(underUnreleased, "# Changelog\n\n- New thing.\n"), false, "no heading, no credit");
});

test("notices follow Cargo.lock, and an unshipped crate is not part of the file", () => {
  const lock = [
    "[[package]]",
    'name = "tokio"',
    'version = "1.54.1"',
    "",
    "[[package]]",
    'name = "jsonschema"',
    'version = "0.60.2"',
    "",
  ].join("\n");
  const stale = "Components\n----------\n\ntokio 1.53.3  MIT\nnlohmann-json 3.12.0  MIT  compiled into the Fcitx addon\n\nLicence texts\n";
  assert.match(noticesProblems(lock, stale)[0], /tokio 1\.53\.3 but Cargo.lock has 1\.54\.1/);
  assert.match(noticesProblems(lock, stale)[0], /python3 scripts\/third-party-notices\.py/);
  assert.deepEqual(noticesProblems(lock, stale.replace("tokio 1.53.3", "tokio 1.54.1")), []);
  assert.deepEqual([...cargoLockVersions(lock).get("jsonschema")], ["0.60.2"]);
});

test("current main's notices match, and the jsonschema bump did not need a rewrite", () => {
  const main = "972c5a52900a8ceec57528664734c7ae0ee31180";
  const jsonschema = "4f79746206f136a763953da4bc77598b2308e84b";
  const fixed = "1f9d7cfcfca55afac6f9a4cc6bf68e5a98c18f21";
  for (const rev of [main, jsonschema, fixed]) {
    const run = spawnSync(process.execPath, ["scripts/check-agent-rules.mjs", "--only", "notices", "--rev", rev], { encoding: "utf8" });
    assert.equal(run.status, 0, run.stderr);
  }
});

test("a feature-map hero that the panel does not show fails", () => {
  const source = 'text: "Held · screen shared"';
  const wrong = [{ path: ".cursor/skills/verify-badi/features/screen-share-hold.md", text: "the hero reads `HELD · SCREEN SHARED`" }];
  const right = [{ path: wrong[0].path, text: "the hero reads `Held · screen shared`" }];
  assert.match(featureMapProblems(wrong, source)[0], /HELD · SCREEN SHARED/);
  assert.match(featureMapProblems(wrong, source)[0], /ui\/omarchy-plugin/);
  assert.deepEqual(featureMapProblems(right, source), []);
  assert.deepEqual(featureMapProblems([{ path: "README.md", text: "`HELD · SCREEN SHARED`" }], source), []);
});

test("the feature map before PR 179 fails, and current main passes", () => {
  const before = "23b9285b3101ada4e4ffad6e1f8b8e41037817bb";
  const main = "972c5a52900a8ceec57528664734c7ae0ee31180";
  const failed = spawnSync(process.execPath, ["scripts/check-agent-rules.mjs", "--only", "feature-map", "--rev", before], { encoding: "utf8" });
  assert.equal(failed.status, 1);
  assert.match(failed.stderr, /HELD · SCREEN SHARED/);
  const passed = spawnSync(process.execPath, ["scripts/check-agent-rules.mjs", "--only", "feature-map", "--rev", main], { encoding: "utf8" });
  assert.equal(passed.status, 0, passed.stderr);
});

test("the Arch snapshot helper retries, and a direct pacman install fails the rule", () => {
  const root = mkdtempSync(`${tmpdir()}/badi-arch-`);
  const bin = `${root}/bin`;
  const conf = `${root}/pacman.conf`;
  const mirror = `${root}/mirrorlist`;
  writeFileSync(conf, "[options]\nHoldPkg = pacman\n");
  spawnSync("mkdir", ["-p", bin]);
  writeFileSync(`${bin}/pacman`, "#!/bin/bash\nprintf '%s\\n' \"$*\" >> \"$BADI_PACMAN_LOG\"\ncount=$(wc -l < \"$BADI_PACMAN_LOG\")\nif [[ \"$count\" -lt 3 ]]; then echo 'Operation too slow' >&2; exit 1; fi\nexit 0\n");
  chmodSync(`${bin}/pacman`, 0o755);
  const env = {
    ...process.env,
    PATH: `${bin}:/usr/bin:/bin`,
    BADI_PACMAN_CONF: conf,
    BADI_MIRRORLIST: mirror,
    BADI_ARCH_RETRY_SLEEP: "0",
    BADI_PACMAN_LOG: `${root}/log`,
  };
  writeFileSync(env.BADI_PACMAN_LOG, "");
  const ok = spawnSync("bash", ["scripts/ci-arch-snapshot.sh", "2026/10/18", "git"], { env, encoding: "utf8" });
  assert.equal(ok.status, 0, ok.stderr);
  assert.match(readFileSync(conf, "utf8"), /^DisableDownloadTimeout$/m);
  const calls = readFileSync(env.BADI_PACMAN_LOG, "utf8").trim().split("\n");
  assert.equal(calls.length, 4, calls.join(" | "));
  assert.match(calls[2], /^-Syyu/);
  assert.match(calls[3], /^-S --needed --noconfirm git$/);

  writeFileSync(env.BADI_PACMAN_LOG, "");
  writeFileSync(`${bin}/pacman`, "#!/bin/bash\necho 'Failed to connect' >&2\nexit 1\n");
  chmodSync(`${bin}/pacman`, 0o755);
  const failed = spawnSync("bash", ["scripts/ci-arch-snapshot.sh", "2026/08/31", "git"], { env, encoding: "utf8" });
  assert.equal(failed.status, 1);
  assert.match(failed.stderr, /failed after 5 attempts/);

  const direct = "jobs:\n  arch:\n    steps:\n      - run: pacman -Syyu --noconfirm\n";
  assert.match(archSnapshotProblems(direct, null)[0], /pacman -Syyu/);
  assert.match(archSnapshotProblems(direct, null)[0], /scripts\/ci-arch-snapshot\.sh/);
});

test("the Arch workflow that timed out fails, and this tree's helper matches", () => {
  // The Actions runs that stalled (161351c and the others) predate the 0.2.0
  // root, so a normal clone has no such object. c948ffd is that root, and
  // 972c5a5 is current main: both still call pacman directly.
  for (const rev of ["c948ffd92efda219ad54d0ee3fbd879869db9ec1", "972c5a52900a8ceec57528664734c7ae0ee31180"]) {
    const run = spawnSync(process.execPath, ["scripts/check-agent-rules.mjs", "--only", "arch", "--rev", rev], { encoding: "utf8" });
    assert.equal(run.status, 1, `${rev}\n${run.stderr}`);
    assert.match(run.stderr, /pacman -S/);
  }
  const here = spawnSync(process.execPath, ["scripts/check-agent-rules.mjs", "--only", "arch"], { encoding: "utf8" });
  assert.equal(here.status, 0, here.stderr);
});

test("a push is checked from its old tip, or alone when it replaced history", () => {
  const before = "a".repeat(40);
  const head = "b".repeat(40);
  const ancestor = () => true;
  assert.deepEqual(pushRange({ before }, head, ancestor), { base: before, head });
  assert.deepEqual(pushRange({ before, forced: true }, head, ancestor), { only: head });
  assert.deepEqual(pushRange({ before }, head, () => false), { only: head });
  assert.equal(pushRange({ before: "0".repeat(40) }, head, ancestor), null);
  assert.equal(pushRange({}, head, ancestor), null);
});
