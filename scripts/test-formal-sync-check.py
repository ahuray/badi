"""The formal sync gate fails when covered code moves without its spec."""

import importlib.util
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = Path(__file__).with_name("formal-sync-check.py")
SPEC = importlib.util.spec_from_file_location("formal_sync_check", SCRIPT)
formal_sync_check = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(formal_sync_check)

CODE = "fn set_runtime_pause() {\n" + "".join(f"    let hold_{index} = {index};\n" for index in range(30)) + "}\n"


class FormalSyncCheckTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.git("init", "-b", "main")
        self.git("config", "user.email", "formal-sync@example.com")
        self.git("config", "user.name", "Formal Sync Test")
        (self.root / "code.rs").write_text(CODE)
        self.commit("add the covered function")
        self.base = self.head()
        self.write_skill(self.base)
        (self.root / "model.tla").write_text("---- MODULE model ----\n====\n")
        check = self.root / ".cursor/skills/sample-formal/check.sh"
        check.write_text("#!/bin/sh\nexit 0\n")
        check.chmod(0o755)
        self.commit("add the formal skill")

    def tearDown(self):
        self.temporary.cleanup()

    def git(self, *args, check=True):
        result = subprocess.run(
            ["git", "-C", str(self.root), "-c", "core.quotepath=false", *args],
            check=False,
            capture_output=True,
            text=True,
        )
        if check and result.returncode != 0:
            raise AssertionError(f"git {args}\n{result.stderr}")
        return result

    def commit(self, message):
        self.git("add", "-A")
        self.git("commit", "-m", message)

    def head(self):
        return self.git("rev-parse", "HEAD").stdout.strip()

    def write_skill(self, sync, covered=("code.rs",), spec=("model.tla",), name="sample-formal", marker=False):
        covered_lines = "\n".join(f"  - {path}" for path in covered)
        spec_lines = "\n".join(f"  - {path}" for path in spec)
        marker_line = "formal: true\n" if marker else ""
        directory = self.root / ".cursor" / "skills" / name
        directory.mkdir(parents=True, exist_ok=True)
        (directory / "SKILL.md").write_text(
            "---\n"
            f"name: {name}\n"
            "description: Sample formal skill for the sync gate.\n"
            f"{marker_line}"
            f"sync_commit: {sync}\n"
            "covered:\n"
            f"{covered_lines}\n"
            "spec:\n"
            f"{spec_lines}\n"
            "---\n"
            "\n# Sample\n"
        )

    def run_check(self, *args):
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--root", str(self.root), *args],
            check=False,
            capture_output=True,
            text=True,
        )
        return result.returncode, result.stdout, result.stderr

    def touch_function(self):
        path = self.root / "code.rs"
        path.write_text(path.read_text().replace(
            "fn set_runtime_pause() {\n",
            "fn set_runtime_pause() {\n    let touched = true;\n",
            1,
        ))

    def test_matching_tree_passes(self):
        code, out, err = self.run_check()
        self.assertEqual(code, 0, err)
        self.assertEqual(out, "formal-sync-check: ok, 1 formal skill\n")

    def test_covered_function_fails_until_the_sync_commit_moves(self):
        self.touch_function()
        self.commit("touch set_runtime_pause")
        code, out, err = self.run_check()
        self.assertEqual(code, 1, out)
        self.assertIn("sample-formal", err)
        self.assertIn("code.rs (modified)", err)
        self.assertIn(self.head(), err)
        self.assertIn("follow /maintain-formal-verification-skill, then move the sync commit", err)
        self.assertIn(".cursor/skills/sample-formal/check.sh", err)
        self.assertIn(".cursor/skills/sample-formal/SKILL.md", err)
        confirmed = self.head()
        self.write_skill(confirmed)
        self.commit("move sync commit")
        code, out, err = self.run_check()
        self.assertEqual(code, 0, err)
        self.assertEqual(out, "formal-sync-check: ok, 1 formal skill\n")

    def test_deleted_covered_file_fails(self):
        self.git("rm", "code.rs")
        self.commit("delete the covered file")
        code, _out, err = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn("code.rs (deleted)", err)

    def test_renamed_covered_file_fails(self):
        self.git("mv", "code.rs", "moved.rs")
        self.commit("rename the covered file")
        code, _out, err = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn("code.rs (renamed to moved.rs)", err)

    def test_same_commit_spec_update_is_excused_and_schedules_the_check(self):
        parent = self.head()
        self.touch_function()
        model = self.root / "model.tla"
        model.write_text(model.read_text() + "\\* updated\n")
        self.commit("update the code and the model together")
        code, out, err = self.run_check()
        self.assertEqual(code, 0, err)
        self.assertIn("excused because each of those commits also updates a spec file", out)
        code, out, err = self.run_check("--changed-since", parent)
        self.assertEqual(code, 0, err)
        self.assertEqual(out, ".cursor/skills/sample-formal/check.sh\n")

    def test_later_spec_commit_does_not_excuse_an_earlier_code_change(self):
        self.touch_function()
        self.commit("touch the covered function only")
        model = self.root / "model.tla"
        model.write_text(model.read_text() + "\\* later\n")
        self.commit("update the model afterwards")
        code, _out, err = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn("code.rs (modified)", err)
        self.assertNotIn("excused", err)

    def test_runner_edit_does_not_excuse_covered_code_but_schedules_a_rerun(self):
        parent = self.head()
        self.touch_function()
        check = self.root / ".cursor/skills/sample-formal/check.sh"
        check.write_text(check.read_text() + "\n# rerun\n")
        self.commit("code and runner")
        code, out, err = self.run_check()
        self.assertEqual(code, 1, out)
        self.assertIn("code.rs (modified)", err)
        code, out, err = self.run_check("--changed-since", parent)
        self.assertEqual(code, 0, err)
        self.assertIn("check.sh", out)

    def test_runner_only_change_passes_and_schedules_the_check(self):
        parent = self.head()
        check = self.root / ".cursor/skills/sample-formal/check.sh"
        check.write_text(check.read_text() + "\n# again\n")
        self.commit("runner only")
        code, out, err = self.run_check()
        self.assertEqual(code, 0, err)
        self.assertNotIn("excused", out)
        code, out, err = self.run_check("--changed-since", parent)
        self.assertEqual(code, 0, err)
        self.assertEqual(out, ".cursor/skills/sample-formal/check.sh\n")

    def test_working_tree_edit_fails_before_commit(self):
        self.touch_function()
        code, _out, err = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn("code.rs (modified)", err)
        self.assertIn("working tree", err)

    def test_uncovered_file_is_ignored(self):
        parent = self.head()
        (self.root / "other.rs").write_text("fn elsewhere() {}\n")
        self.commit("edit a file the model does not describe")
        code, out, err = self.run_check()
        self.assertEqual(code, 0, err)
        code, out, err = self.run_check("--changed-since", parent)
        self.assertEqual(code, 0, err)
        self.assertEqual(out, "")

    def test_frontmatter_marker_opts_in_a_skill_outside_the_name_pattern(self):
        (self.root / "opt.rs").write_text(CODE)
        self.commit("add opt-in code")
        opted = self.head()
        directory = self.root / ".cursor" / "skills" / "opt-in"
        directory.mkdir(parents=True)
        (directory / "check.sh").write_text("#!/bin/sh\nexit 0\n")
        (self.root / "opt.tla").write_text("---- MODULE opt ----\n====\n")
        skill = (
            "---\n"
            "name: opt-in\n"
            "description: Opted in with the frontmatter marker.\n"
            "formal: true\n"
            f"sync_commit: {opted}\n"
            "covered:\n"
            "  - opt.rs\n"
            "spec:\n"
            "  - opt.tla\n"
            "---\n"
        )
        (directory / "SKILL.md").write_text(skill)
        self.commit("add the opted-in skill")
        opt = self.root / "opt.rs"
        opt.write_text(opt.read_text() + "\nfn extra() {}\n")
        self.commit("touch opted-in code")
        code, _out, err = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn("opt-in", err)
        self.assertIn("opt.rs (modified)", err)
        self.assertNotIn("sample-formal drifted", err)

    def test_plain_skill_is_not_a_formal_skill(self):
        plain = self.root / ".cursor" / "skills" / "plain"
        plain.mkdir(parents=True)
        (plain / "SKILL.md").write_text(
            "---\nname: plain\ndescription: Not formal.\ncovered:\n  - missing.rs\n---\n"
        )
        self.commit("add a broken skill that is not formal")
        code, out, err = self.run_check()
        self.assertEqual(code, 0, err)
        self.assertEqual(out, "formal-sync-check: ok, 1 formal skill\n")

    def test_escaped_covered_path_fails(self):
        self.write_skill(self.base, covered=("../secret",))
        self.commit("list a path outside the repository")
        code, _out, err = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn("escapes the repository", err)

    def test_all_lists_the_check(self):
        code, out, err = self.run_check("--changed-since", "ALL")
        self.assertEqual(code, 0, err)
        self.assertEqual(out, ".cursor/skills/sample-formal/check.sh\n")


class BadiRepositoryTests(unittest.TestCase):
    def run_check(self, *args):
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--root", str(ROOT), *args],
            check=False,
            capture_output=True,
            text=True,
        )
        return result.returncode, result.stdout, result.stderr

    def skill(self):
        skills, errors = formal_sync_check.load_skills(ROOT)
        self.assertEqual(errors, [])
        matched = [item for item in skills if item.name == "verify-badi-formal-withhold"]
        self.assertEqual(len(matched), 1)
        return matched[0]

    def test_repository_matches_its_sync_commit(self):
        skill = self.skill()
        code, out, err = self.run_check()
        self.assertEqual(err, "")
        self.assertEqual(code, 0)
        self.assertEqual(out, "formal-sync-check: ok, 1 formal skill\n")
        for path in (
            "broker/src/engine/authority.rs",
            "broker/src/engine/settings.rs",
            "broker/src/engine/mod.rs",
            "broker/src/engine/desktop.rs",
            "broker/src/engine/session.rs",
            "broker/src/engine/generation.rs",
            "broker/src/engine/snooze.rs",
            "broker/src/protocol.rs",
            "broker/src/settings/policy.rs",
        ):
            self.assertIn(path, skill.covered)
        diff = subprocess.run(
            ["git", "-C", str(ROOT), "diff", "--name-only", skill.sync_commit, "HEAD", "--", *skill.covered],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertEqual(diff.stdout, "")

    def test_withhold_tests_are_not_covered_files(self):
        skill = self.skill()
        self.assertNotIn("broker/src/engine/tests/withhold.rs", skill.covered)
        self.assertNotIn("broker/src/engine/tests/mod.rs", skill.covered)

    def test_coverage_table_paths_are_in_the_frontmatter_list(self):
        text = (ROOT / ".cursor/skills/verify-badi-formal-withhold/SKILL.md").read_text(encoding="utf-8")
        coverage = text.split("## Coverage", 1)[1].split("\n## ", 1)[0]
        paths = set(re.findall(r"(?:broker|adapters|protocol|scripts)/[A-Za-z0-9_./-]+\.[A-Za-z0-9]+", coverage))
        skill = self.skill()
        self.assertTrue(paths)
        self.assertLessEqual(paths, set(skill.covered))
        self.assertEqual(skill.spec, (
            ".cursor/skills/verify-badi-formal-withhold/Withhold.tla",
            ".cursor/skills/verify-badi-formal-withhold/Withhold.cfg",
        ))

    def test_clean_head_schedules_nothing_and_all_lists_the_check(self):
        code, out, err = self.run_check("--changed-since", "HEAD")
        self.assertEqual((code, out, err), (0, "", ""))
        code, out, err = self.run_check("--changed-since", "ALL")
        self.assertEqual(code, 0, err)
        self.assertEqual(out, ".cursor/skills/verify-badi-formal-withhold/check.sh\n")


if __name__ == "__main__":
    unittest.main()
