#!/usr/bin/env python3
"""Tests for scripts/formal-sync-check.py."""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts" / "formal-sync-check.py"
spec = importlib.util.spec_from_file_location("formal_sync_check", SCRIPT)
sync = importlib.util.module_from_spec(spec)
assert spec.loader is not None
sys.modules[spec.name] = sync
spec.loader.exec_module(sync)

SOURCE = textwrap.dedent(
    """\
    impl Broker {
        /// Doc comments are not code.
        pub async fn set_paused(&self, paused: bool) -> bool {
            let open = "{"; // a brace in a string
            let close = '}';
            let raw = r#"fn set_paused() { "#;
            self.state.lock().await.paused = paused; /* nested /* comment */ { */
            paused
        }

        fn other<'a>(&'a self) -> &'a str {
            "other"
        }
    }

    trait Pause {
        fn set_paused(&self, paused: bool) -> bool;
    }
    """
)


def run_script(root: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--root", str(root), *args],
        capture_output=True,
        text=True,
        check=False,
    )


class Extraction(unittest.TestCase):
    def test_every_match_is_one_whole_item_without_comments(self) -> None:
        items = sync.functions(SOURCE, "set_paused")
        self.assertEqual(len(items), 2)
        body, declaration = items
        self.assertTrue(body.startswith("fn set_paused(&self, paused: bool) -> bool {"))
        self.assertTrue(body.endswith("paused }"))
        self.assertIn('r#"fn set_paused() { "#', body)
        self.assertIn("'}'", body)
        self.assertNotIn("brace in a string", body)
        self.assertNotIn("nested", body)
        self.assertNotIn("Doc comments", body)
        self.assertEqual(declaration, "fn set_paused(&self, paused: bool) -> bool;")

    def test_a_lifetime_is_not_a_character(self) -> None:
        self.assertEqual(sync.functions(SOURCE, "other"), ["fn other<'a>(&'a self) -> &'a str { \"other\" }"])

    def test_formatting_and_comments_do_not_change_the_text(self) -> None:
        reflowed = SOURCE.replace("paused: bool) -> bool {", "paused: bool)\n        -> bool\n    {  // reflowed")
        self.assertEqual(sync.functions(reflowed, "set_paused")[0], sync.functions(SOURCE, "set_paused")[0])

    def test_a_code_change_changes_the_text(self) -> None:
        changed = SOURCE.replace(".paused = paused;", ".paused = !paused;")
        self.assertNotEqual(sync.functions(changed, "set_paused")[0], sync.functions(SOURCE, "set_paused")[0])

    def test_a_missing_name_finds_nothing(self) -> None:
        self.assertEqual(sync.functions(SOURCE, "set_pause"), [])


class Check(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / "src").mkdir()
        (self.root / "src/pause.rs").write_text(SOURCE, encoding="utf-8")
        self.skill = self.root / ".cursor/skills/formal-pause/SKILL.md"
        self.skill.parent.mkdir(parents=True)
        self.skill.write_text(
            "---\nname: formal-pause\ndescription: test\ncovered:\n  - src/pause.rs set_paused other\n---\n\n# Body\n",
            encoding="utf-8",
        )
        unrelated = self.root / ".cursor/skills/plain/SKILL.md"
        unrelated.parent.mkdir(parents=True)
        unrelated.write_text("---\nname: plain\ndescription: no model\n---\n", encoding="utf-8")

    def tearDown(self) -> None:
        self.temp.cleanup()

    def test_an_unrecorded_hash_fails_until_updated(self) -> None:
        first = run_script(self.root)
        self.assertEqual(first.returncode, 1)
        self.assertIn("--update", first.stderr)
        updated = run_script(self.root, "--update")
        self.assertEqual(updated.returncode, 0, updated.stderr)
        self.assertRegex(self.skill.read_text(encoding="utf-8"), r"\ncovered_sha256: [0-9a-f]{16}\n---\n")
        passed = run_script(self.root)
        self.assertEqual(passed.returncode, 0, passed.stderr)
        self.assertIn("1 skill", passed.stdout)

    def test_a_comment_passes_and_a_code_change_fails(self) -> None:
        run_script(self.root, "--update")
        pause = self.root / "src/pause.rs"
        pause.write_text(SOURCE.replace("paused\n    }", "paused // still paused\n    }"), encoding="utf-8")
        self.assertEqual(run_script(self.root).returncode, 0)
        pause.write_text(SOURCE + "\nfn unrelated() { changed() }\n", encoding="utf-8")
        self.assertEqual(run_script(self.root).returncode, 0, "code outside the covered functions")
        pause.write_text(SOURCE.replace(".paused = paused;", ".paused = false;"), encoding="utf-8")
        failed = run_script(self.root)
        self.assertEqual(failed.returncode, 1)
        self.assertIn(".cursor/skills/formal-pause/SKILL.md", failed.stderr)
        self.assertIn("git log -p -- src/pause.rs", failed.stderr)

    def test_a_renamed_function_fails_even_after_update(self) -> None:
        run_script(self.root, "--update")
        (self.root / "src/pause.rs").write_text(SOURCE.replace("fn other", "fn renamed"), encoding="utf-8")
        for args in ((), ("--update",)):
            failed = run_script(self.root, *args)
            self.assertEqual(failed.returncode, 1)
            self.assertIn("`fn other`", failed.stderr)


class Repository(unittest.TestCase):
    # npm run formal:check runs the script on this tree after these tests.
    def test_the_withhold_skill_is_found(self) -> None:
        names = [skill.path.parent.name for skill in sync.formal_skills(ROOT)]
        self.assertIn("verify-badi-formal-withhold", names)


if __name__ == "__main__":
    unittest.main()
