import importlib.util
import io
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
        compositor = patch.object(launcher, "wait_for_compositor", return_value=None)
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
        result, reason = self.select()
        self.assertIsNone(reason)
        self.assertEqual(result, {**self.environment, "FCITX_ADDON_DIRS": str(self.root / "addons") + ":" + self.environment["FCITX_ADDON_DIRS"]})
        self.assertEqual(self.environment["FCITX_ADDON_DIRS"], "/existing/custom:/usr/lib/fcitx5")

    def test_each_updated_runtime_file_and_module_falls_back(self):
        for filename in [*launcher.RUNTIME_FILES, str(self.root / "addons/libwaylandim.so")]:
            with self.subTest(filename=filename):
                result, reason = self.select(changed=filename)
                self.assertIsNotNone(reason)
                self.assertEqual(result, self.environment)

    def test_unknown_receipt_version_and_missing_receipt_fall_back(self):
        for version in ("5.1.21", "5.1.23"):
            self.receipt["source"]["upstream_version"] = version
            self.write_receipt()
            self.assertEqual(self.select(), (self.environment, "build receipt differs"))
        (self.root / "build-receipt.json").unlink()
        self.assertEqual(self.select(), (self.environment, "build receipt or module unreadable"))

    def test_loader_overrides_and_alternate_arguments_keep_system_behavior(self):
        for name in launcher.LOADER_OVERRIDES:
            environment = {**self.environment, name: "user-defined"}
            self.assertEqual(self.select(environment=environment), (environment, "loader override set"))
        self.assertEqual(self.select(arguments=["--enable", "unrelated"]), (self.environment, "service arguments differ"))

    def test_mismatch_removes_only_inherited_private_selection(self):
        environment = {**self.environment, "FCITX_ADDON_DIRS": f"/custom:{self.root}/addons::/usr/lib/fcitx5"}
        result, reason = self.select(environment=environment, changed=launcher.PROGRAM)
        self.assertEqual(reason, "Fcitx runtime changed since the build")
        self.assertEqual(result, {**environment, "FCITX_ADDON_DIRS": "/custom::/usr/lib/fcitx5"})

    def test_an_existing_wayland_module_override_keeps_its_own_behavior(self):
        custom = self.root / "user-addons"
        custom.mkdir()
        (custom / "libwayland.so").write_bytes(b"user module")
        environment = {**self.environment, "FCITX_ADDON_DIRS": str(custom) + ":/usr/lib/fcitx5"}
        self.assertEqual(self.select(environment=environment),
                         (environment, "another wayland addon overrides the system one"))

    def test_local_mismatch_never_waits_for_the_compositor(self):
        launcher.wait_for_compositor.side_effect = AssertionError("compositor queried")
        self.assertEqual(self.select(arguments=["--enable", "unrelated"]), (self.environment, "service arguments differ"))

    def test_compositor_fallback_reason_is_reported(self):
        launcher.wait_for_compositor.side_effect = launcher.CompositorStarting("Hyprland instance not listed")
        self.assertEqual(self.select(), (self.environment, "Hyprland instance not listed"))

    def test_fallback_exec_preserves_original_arguments_and_environment(self):
        arguments = ["--enable", "user-addon", "--disable", "notificationitem"]
        with patch.object(launcher.sys, "argv", ["launch.py", *arguments]), patch.object(launcher.os, "environ", self.environment), patch.object(launcher, "select_environment", return_value=(self.environment, "service arguments differ")), patch.object(launcher.os, "execve") as execute, patch.object(launcher.sys, "stderr", new_callable=io.StringIO) as stderr:
            launcher.main()
        execute.assert_called_once_with(launcher.PROGRAM, [launcher.PROGRAM, *arguments], self.environment)
        self.assertEqual(stderr.getvalue(), "Badi Fcitx compatibility override inactive (service arguments differ); "
                                            "launching the system frontend.\n")


class CompositorTests(unittest.TestCase):
    ENVIRONMENT = {"HYPRLAND_INSTANCE_SIGNATURE": "private-test", "WAYLAND_DISPLAY": "wayland-1"}
    VERSION = {"version": launcher.COMPOSITOR_VERSION, "commit": launcher.COMPOSITOR_COMMIT, "dirty": False}
    INSTANCES = [{"instance": "private-test", "wl_socket": "wayland-1"}]

    def replies(self, *values):
        return patch.object(launcher.subprocess, "check_output",
                            side_effect=[value if isinstance(value, BaseException) else json.dumps(value).encode()
                                         for value in values])

    def test_only_exact_active_compositor_and_display_are_admitted(self):
        for changed, reason in [(self.VERSION, None), ({**self.VERSION, "version": "0.57.0"}, "Hyprland build differs"),
                                ({**self.VERSION, "dirty": True}, "Hyprland build differs"),
                                ({**self.VERSION, "commit": "different"}, "Hyprland build differs")]:
            with self.subTest(version=changed), self.replies(self.INSTANCES, changed):
                if reason is None:
                    launcher.check_compositor(self.ENVIRONMENT)
                else:
                    with self.assertRaisesRegex(launcher.Fallback, reason):
                        launcher.check_compositor(self.ENVIRONMENT)
        with self.replies(self.INSTANCES, self.VERSION), self.assertRaisesRegex(launcher.Fallback, "another display"):
            launcher.check_compositor({**self.ENVIRONMENT, "WAYLAND_DISPLAY": "another-display"})
        with self.assertRaisesRegex(launcher.Fallback, "no Hyprland session"):
            launcher.check_compositor({})

    def test_startup_failures_are_retryable_and_mismatches_are_not(self):
        starting = [launcher.subprocess.CalledProcessError(1, "hyprctl"), launcher.subprocess.TimeoutExpired("hyprctl", .5)]
        for failure in starting:
            with self.subTest(failure=type(failure).__name__), self.replies(failure):
                with self.assertRaises(launcher.CompositorStarting):
                    launcher.check_compositor(self.ENVIRONMENT)
        with patch.object(launcher.subprocess, "check_output", return_value=b"no socket found"), \
                self.assertRaises(launcher.CompositorStarting):
            launcher.check_compositor(self.ENVIRONMENT)
        with self.replies([]), self.assertRaisesRegex(launcher.CompositorStarting, "not listed"):
            launcher.check_compositor(self.ENVIRONMENT)
        with patch.object(launcher.subprocess, "check_output", side_effect=FileNotFoundError), \
                self.assertRaises(launcher.Fallback) as missing:
            launcher.check_compositor(self.ENVIRONMENT)
        self.assertNotIsInstance(missing.exception, launcher.CompositorStarting)
        with patch.object(launcher.subprocess, "check_output", return_value=b" " * 8193), \
                self.assertRaisesRegex(launcher.Fallback, "too large"):
            launcher.check_compositor(self.ENVIRONMENT)

    def test_a_starting_compositor_is_awaited_with_bounded_backoff(self):
        now = [0.0]
        sleeps = []

        def sleep(seconds):
            sleeps.append(seconds)
            now[0] += seconds

        not_ready = launcher.subprocess.CalledProcessError(1, "hyprctl")
        with self.replies(not_ready, not_ready, [], self.INSTANCES, self.VERSION) as calls:
            launcher.wait_for_compositor(self.ENVIRONMENT, clock=lambda: now[0], sleep=sleep)
        self.assertEqual(calls.call_count, 5)
        self.assertEqual(sleeps, [.05, .1, .2])

    def test_waiting_stops_at_the_deadline_with_the_last_reason(self):
        now = [0.0]
        sleeps = []

        def sleep(seconds):
            sleeps.append(seconds)
            now[0] += seconds

        with patch.object(launcher.subprocess, "check_output", side_effect=launcher.subprocess.CalledProcessError(1, "hyprctl")), \
                self.assertRaisesRegex(launcher.CompositorStarting, "hyprctl instances failed"):
            launcher.wait_for_compositor(self.ENVIRONMENT, clock=lambda: now[0], sleep=sleep)
        self.assertAlmostEqual(sum(sleeps), launcher.COMPOSITOR_WAIT_SECONDS)
        self.assertLessEqual(max(sleeps), .5)

    def test_a_definite_mismatch_is_not_retried(self):
        with self.replies(self.INSTANCES, {**self.VERSION, "dirty": True}) as calls, \
                self.assertRaisesRegex(launcher.Fallback, "build differs"):
            launcher.wait_for_compositor(self.ENVIRONMENT, clock=lambda: 0.0,
                                         sleep=lambda _seconds: self.fail("a definite mismatch was retried"))
        self.assertEqual(calls.call_count, 2)


if __name__ == "__main__":
    unittest.main()
