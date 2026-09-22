import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("compat_launch", ROOT / "launch.py")
launcher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(launcher)


class LaunchTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        compositor = patch.object(launcher, "compatible_compositor", return_value=True)
        compositor.start()
        self.addCleanup(compositor.stop)
        self.receipt = {
            "schema": "badi.fcitx-wayland-compat-build.v1",
            "source": {"upstream_version": launcher.VERSION, "upstream_commit": launcher.COMMIT},
            "installed_build_versions": {name: launcher.VERSION for name in ("Fcitx5Core", "Fcitx5Config", "Fcitx5Utils")},
            "runtime_files_sha256": {name: "runtime-sha" for name in launcher.RUNTIME_FILES},
            "artifact_sha256": "module-sha",
        }
        self.write_receipt()
        self.environment = {"FCITX_ADDON_DIRS": "/existing/custom:/usr/lib/fcitx5", "KEEP_SETTING": "unchanged"}

    def write_receipt(self):
        (self.root / "build-receipt.json").write_text(json.dumps(self.receipt))

    def select(self, *, environment=None, arguments=None, changed=None):
        def digest(path):
            if str(path) == changed:
                return "updated-file"
            return "module-sha" if path.name == "libwaylandim.so" else "runtime-sha"
        with patch.object(launcher, "digest", side_effect=digest):
            return launcher.select_environment(
                arguments if arguments is not None else ["--disable", "notificationitem"],
                environment if environment is not None else self.environment, self.root)

    def test_matching_runtime_only_prepends_its_private_module(self):
        result, enabled = self.select()
        self.assertTrue(enabled)
        self.assertEqual(result, {**self.environment, "FCITX_ADDON_DIRS": str(self.root / "addons") + ":" + self.environment["FCITX_ADDON_DIRS"]})
        self.assertEqual(self.environment["FCITX_ADDON_DIRS"], "/existing/custom:/usr/lib/fcitx5")

    def test_each_updated_runtime_file_and_module_falls_back(self):
        for filename in [*launcher.RUNTIME_FILES, str(self.root / "addons/libwaylandim.so")]:
            with self.subTest(filename=filename):
                result, enabled = self.select(changed=filename)
                self.assertFalse(enabled)
                self.assertEqual(result, self.environment)

    def test_unknown_receipt_version_and_missing_receipt_fall_back(self):
        self.receipt["source"]["upstream_version"] = "5.1.22"
        self.write_receipt()
        self.assertEqual(self.select(), (self.environment, False))
        (self.root / "build-receipt.json").unlink()
        self.assertEqual(self.select(), (self.environment, False))

    def test_loader_overrides_and_alternate_arguments_keep_system_behavior(self):
        for name in launcher.LOADER_OVERRIDES:
            environment = {**self.environment, name: "user-defined"}
            self.assertEqual(self.select(environment=environment), (environment, False))
        self.assertEqual(self.select(arguments=["--enable", "unrelated"]), (self.environment, False))

    def test_mismatch_removes_only_inherited_private_selection(self):
        environment = {**self.environment, "FCITX_ADDON_DIRS": f"/custom:{self.root}/addons::/usr/lib/fcitx5"}
        result, enabled = self.select(environment=environment, changed=launcher.PROGRAM)
        self.assertFalse(enabled)
        self.assertEqual(result, {**environment, "FCITX_ADDON_DIRS": "/custom::/usr/lib/fcitx5"})

    def test_an_existing_wayland_module_override_keeps_its_own_behavior(self):
        custom = self.root / "user-addons"
        custom.mkdir()
        (custom / "libwayland.so").write_bytes(b"user module")
        environment = {**self.environment, "FCITX_ADDON_DIRS": str(custom) + ":/usr/lib/fcitx5"}
        self.assertEqual(self.select(environment=environment), (environment, False))

    def test_fallback_exec_preserves_original_arguments_and_environment(self):
        arguments = ["--enable", "user-addon", "--disable", "notificationitem"]
        with patch.object(launcher.sys, "argv", ["launch.py", *arguments]), patch.object(launcher.os, "environ", self.environment), patch.object(launcher, "select_environment", return_value=(self.environment, False)), patch.object(launcher.os, "execve") as execute:
            launcher.main()
        execute.assert_called_once_with(launcher.PROGRAM, [launcher.PROGRAM, *arguments], self.environment)


class CompositorTests(unittest.TestCase):
    def test_only_exact_active_compositor_and_display_are_admitted(self):
        environment = {"HYPRLAND_INSTANCE_SIGNATURE": "private-test", "WAYLAND_DISPLAY": "wayland-1"}
        version = {"version": "0.56.2", "commit": launcher.COMPOSITOR_COMMIT, "dirty": False}
        instances = [{"instance": "private-test", "wl_socket": "wayland-1"}]
        for changed, admitted in [(version, True), ({**version, "version": "0.57.0"}, False), ({**version, "dirty": True}, False), ({**version, "commit": "different"}, False)]:
            with self.subTest(version=changed), patch.object(launcher.subprocess, "check_output", side_effect=[json.dumps(instances).encode(), json.dumps(changed).encode()]):
                self.assertEqual(launcher.compatible_compositor(environment), admitted)
        with patch.object(launcher.subprocess, "check_output", side_effect=[json.dumps(instances).encode(), json.dumps(version).encode()]):
            self.assertFalse(launcher.compatible_compositor({**environment, "WAYLAND_DISPLAY": "another-display"}))
        self.assertFalse(launcher.compatible_compositor({}))


if __name__ == "__main__":
    unittest.main()
