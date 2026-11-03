import assert from "node:assert/strict";
import { test } from "node:test";
import { changelogProblems, diffAddsUnderNewestHeading, isProductPath, parallelDocuments, pushRange, vocabularyProblems } from "./check-agent-rules.mjs";

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
