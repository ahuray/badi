import importlib.util
import io
import json
import pathlib
import tarfile
import tempfile
import sys
import unittest
from unittest.mock import patch

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
spec = importlib.util.spec_from_file_location("compat_build", ROOT / "build.py")
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


class BuildSafetyTests(unittest.TestCase):
    def test_every_linked_fcitx_component_must_match_exact_version(self):
        pinned = builder.VERSION
        for other in ("5.1.21", "5.1.23"):
            for replies in ([other], [pinned, other], [pinned, pinned, other]):
                with self.subTest(replies=replies), patch.object(builder.subprocess, "check_output", side_effect=replies):
                    with self.assertRaisesRegex(RuntimeError, f"exactly {pinned}"):
                        builder.installed_versions()

    def test_same_version_for_all_components_is_admitted(self):
        with patch.object(builder.subprocess, "check_output", return_value=builder.VERSION + "\n"):
            self.assertEqual(set(builder.installed_versions()), {"Fcitx5Core", "Fcitx5Config", "Fcitx5Utils"})

    def test_pinned_patch_changes_only_the_rebuilt_frontend(self):
        # The build links the system core, so a core API change (as in the
        # full upstream change) would fail to load. Keep the backport local.
        manifest = json.loads((ROOT / "manifest.json").read_text())
        self.assertEqual((manifest["upstream_version"], manifest["upstream_commit"], manifest["patch"]),
                         (builder.VERSION, builder.SOURCE_COMMIT, builder.PATCH))
        patch_file = ROOT / builder.PATCH
        self.assertEqual(builder.digest(patch_file), manifest["patch_sha256"])
        targets = {line.split(None, 1)[1] for line in patch_file.read_text().splitlines()
                   if line.startswith(("--- ", "+++ "))}
        self.assertEqual(targets, {"a/src/frontend/waylandim/waylandimserverv2.cpp",
                                   "b/src/frontend/waylandim/waylandimserverv2.cpp"})

    def test_archive_hash_and_size_are_both_required(self):
        with tempfile.TemporaryDirectory() as directory:
            file = pathlib.Path(directory) / "source.tar.gz"
            file.write_bytes(b"changed source")
            with self.assertRaisesRegex(RuntimeError, "SHA256"):
                builder.verify_file(file, "0" * 64, file.stat().st_size)
            with self.assertRaisesRegex(RuntimeError, "byte count"):
                builder.verify_file(file, builder.digest(file), 1)

    def test_archive_cannot_escape_or_link_outside_its_source_root(self):
        prefix = f"fcitx5-{builder.SOURCE_COMMIT}"
        for name, symlink in [("../escape", False), (prefix + "/../escape", False), ("other-root/file", False), (prefix + "/link", True)]:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                archive = root / "source.tar.gz"
                with tarfile.open(archive, "w:gz") as target:
                    item = tarfile.TarInfo(name)
                    if symlink:
                        item.type = tarfile.SYMTYPE
                        item.linkname = "/tmp"
                        target.addfile(item)
                    else:
                        item.size = 1
                        target.addfile(item, io.BytesIO(b"x"))
                with self.assertRaises(RuntimeError):
                    builder.extract_source(archive, root / "out")
                self.assertFalse((root / "out").exists())

    def test_source_internal_icon_symlink_is_preserved(self):
        prefix = f"fcitx5-{builder.SOURCE_COMMIT}"
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            archive = root / "source.tar.gz"
            with tarfile.open(archive, "w:gz") as target:
                item = tarfile.TarInfo(prefix + "/icons/fcitx5.png")
                item.size = 1
                target.addfile(item, io.BytesIO(b"x"))
                link = tarfile.TarInfo(prefix + "/icons/fcitx.png")
                link.type = tarfile.SYMTYPE
                link.linkname = "fcitx5.png"
                target.addfile(link)
            builder.extract_source(archive, root / "out")
            self.assertEqual((root / "out/fcitx5/icons/fcitx.png").read_bytes(), b"x")


if __name__ == "__main__":
    unittest.main()
