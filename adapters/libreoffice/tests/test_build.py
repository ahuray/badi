"""The .oxt is reproducible, complete and declares only files it contains."""

import importlib.util
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ElementTree
import zipfile

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("extension_build", ROOT / "build.py")
build = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build)
MANIFEST = "{http://openoffice.org/2001/manifest}"


class BuildTest(unittest.TestCase):
    def test_archive_is_reproducible_and_self_consistent(self):
        with tempfile.TemporaryDirectory() as directory:
            first = build.build(Path(directory) / "first.oxt").read_bytes()
            second = build.build(Path(directory) / "second.oxt")
            self.assertEqual(first, second.read_bytes())
            with zipfile.ZipFile(second) as archive:
                names = set(archive.namelist())
                self.assertFalse([name for name in names if "__pycache__" in name or name.endswith(".pyc")])
                for name in names:
                    if name.endswith((".xml", ".xcu", ".components")):
                        ElementTree.fromstring(archive.read(name))
                    if name.endswith(".py"):
                        compile(archive.read(name), name, "exec")
                manifest = ElementTree.fromstring(archive.read("META-INF/manifest.xml"))
                declared = {entry.get(MANIFEST + "full-path") for entry in manifest}
                self.assertTrue(declared <= names, declared - names)
                self.assertEqual(declared, {"badi.components", "ProtocolHandler.xcu",
                                            "Accelerators.xcu", "Addons.xcu"})
                components = archive.read("badi.components").decode()
                self.assertIn('uri="badi_writer.py"', components)
                self.assertIn("pythonpath/badi_libreoffice/broker.py", names)


if __name__ == "__main__":
    unittest.main()
