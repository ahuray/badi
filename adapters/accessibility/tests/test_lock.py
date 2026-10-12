"""Fresh lock replies and discovery invalidation, without a running desktop."""
import json
import os
from pathlib import Path
import struct
import sys
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from contract import Denied
from desktop import LOCK_FLAGS, LockEndpoint, SocketLockQuery


class SocketLockTests(unittest.TestCase):
    def query(self, raw, *, changed=False, peer=None):
        connection = Mock()
        connection.getsockopt.return_value = struct.pack("3i", *(peer or (123, os.getuid(), 0)))
        pending = bytearray(raw)
        def receive(count):
            # Split every reply across reads, as a real stream is allowed to.
            count = min(count, 3)
            result = bytes(pending[:count])
            del pending[:count]
            return result
        connection.recv.side_effect = receive
        cache = Mock(stamp="old")
        cache.current_stamp.return_value = "new" if changed else "old"
        with patch("desktop.socket.socket", return_value=connection):
            try:
                with SocketLockQuery(lambda limit: limit, cache, ("/private/ipc.sock", 123)):
                    pass
            finally:
                connection.close.assert_called_once()

    @staticmethod
    def response(state):
        text = json.dumps(state).encode("utf-16-be")
        return b"\x05\x00" + struct.pack(">I", len(text)) + text

    def test_only_a_fresh_complete_explicit_unlock_from_the_same_process_passes(self):
        unlocked = {flag: False for flag in LOCK_FLAGS}
        raw = self.response(unlocked)
        self.query(raw)
        for flag in LOCK_FLAGS:
            with self.subTest(flag=flag), self.assertRaisesRegex(Denied, "desktop_locked"):
                self.query(self.response({**unlocked, flag: True}))
        for invalid in (b"", b"\x02\x00", raw[:-1], b"\x05\x00\xff\xff\xff\xff", self.response({})):
            with self.subTest(raw=invalid[:6]), self.assertRaises(Denied):
                self.query(invalid)
        with self.assertRaises(Denied):
            self.query(raw, changed=True)
        with self.assertRaises(Denied):
            self.query(raw, peer=(999, os.getuid(), 0))

    def test_discovery_is_cached_only_until_the_instance_or_executable_changes(self):
        cache = LockEndpoint()
        stamp = ("/run/user/1000", "/usr/share/omarchy", "/usr/bin/qs", (1, 2, 3), (4, 5, 6))
        entry = {"id": "example", "pid": 123, "config_path": "/usr/share/omarchy/shell/shell.qml"}
        replies = [Mock(stdout=b"Quickshell 0.3.1 (test)"), Mock(stdout=json.dumps([entry]).encode())]
        with patch.object(cache, "current_stamp", return_value=stamp) as current, \
             patch("desktop.subprocess.run", side_effect=replies * 2) as run:
            self.assertEqual(cache.resolve(lambda limit: limit)[1], 123)
            self.assertEqual(cache.resolve(lambda limit: limit)[1], 123)
            self.assertEqual(run.call_count, 2)
            current.return_value = (*stamp[:-1], (4, 5, 7))
            self.assertEqual(cache.resolve(lambda limit: limit)[1], 123)
            self.assertEqual(run.call_count, 4)
