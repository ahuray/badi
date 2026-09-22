"""Lexicon generation boundary checks; the real expansion lane is optional."""

import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("build_writing_lexicon", ROOT / "scripts/build-writing-lexicon.py")
generator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(generator)


class LexiconTests(unittest.TestCase):
    def test_filter_rejects_flagged_nonword_and_capitalized_stems(self):
        flags = generator.excluded_flags("SET UTF-8\nNOSUGGEST !\nFORBIDDENWORD *\nONLYINCOMPOUND c\n")
        allowed, excluded, other = generator.partition_entries(
            "7\nordinary/S\nblocked/S!\nforbidden/S*\nfragment/c\nCapital/S\nnon-word/S\nword2/S\n", flags)
        self.assertEqual(allowed, ["ordinary/S"])
        self.assertEqual(excluded, ["blocked/S!", "forbidden/S*", "fragment/c"])
        self.assertEqual(other, 3)

    def test_unreviewed_formats_fail_closed(self):
        for affix in ("FLAG long\n", "SET ISO8859-1\n", "NOSUGGEST AB\n", "SFX S 0 s/T .\n"):
            with self.subTest(affix=affix), self.assertRaises(ValueError):
                generator.excluded_flags(affix)
        for dictionary in ("2\nonlyone\n", "1\nescaped\\/slash\n", "1\nword po:noun\n"):
            with self.subTest(dictionary=dictionary), self.assertRaises(ValueError):
                generator.partition_entries(dictionary, set())

    def test_bundled_list_and_typical_membership(self):
        generator.verify(generator.DATA)
        words = set((generator.DATA / "en.txt").read_text().splitlines())
        self.assertTrue({"document", "automatically", "address", "documents", "addressing"} <= words)
        self.assertFalse({"documnet", "omarchythm"} & words)

    def test_corrupt_sources_output_and_ordering_are_rejected(self):
        for corruption in ("source", "output", "ordering", "generator"):
            with self.subTest(corruption=corruption), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                for path in (*generator.SOURCES, "en.txt", "manifest.json"):
                    target = directory / path
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(generator.DATA / path, target)
                manifest = json.loads((directory / "manifest.json").read_text())
                if corruption == "source":
                    (directory / "source/index.dic").write_text("1\nchanged\n")
                elif corruption == "output":
                    (directory / "en.txt").write_text("changed\n")
                elif corruption == "generator":
                    manifest["generation"]["script_sha256"] = "0" * 64
                else:
                    data = b"zebra\napple\n"
                    (directory / "en.txt").write_bytes(data)
                    manifest["output"] = {"path": "en.txt", "bytes": len(data), "words": 2,
                                          "sha256": generator.sha256(data)}
                (directory / "manifest.json").write_text(json.dumps(manifest))
                with self.assertRaises(ValueError):
                    generator.verify(directory)

    @unittest.skipUnless(shutil.which("unmunch") and shutil.which("hunspell"), "Hunspell build tools unavailable")
    def test_real_expansion_cannot_reintroduce_flagged_word_family(self):
        try:
            unmunch = generator.generator_tool("unmunch", "hunspell")
        except ValueError as error:
            self.skipTest(str(error))
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / "source").mkdir()
            (directory / "source/index.aff").write_text(
                "SET UTF-8\nNOSUGGEST !\nFORBIDDENWORD *\nONLYINCOMPOUND c\nSFX S Y 1\nSFX S 0 s .\n")
            # The unflagged duplicate demonstrates why pre-expansion filtering
            # alone is insufficient for a deny-marked surface form.
            (directory / "source/index.dic").write_text(
                "6\nordinary/S\nblocked/S!\nblocked/S\nforbidden/S*\nfragment/c\nCapital/S\n")
            with patch.object(generator, "source_metadata", return_value={}):
                data, manifest = generator.generate(directory, unmunch)
            self.assertEqual(data, b"ordinary\nordinarys\n")
            self.assertEqual(manifest["generation"]["overlapping_words_removed"], 2)


if __name__ == "__main__":
    unittest.main()
