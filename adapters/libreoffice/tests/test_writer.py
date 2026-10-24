"""Fast checks for the Writer controller's attempt lifecycle.

The office is replaced by the few UNO names the extension imports and a
callback queue standing in for the main thread, so a superseded or failed
broker request can be exercised without LibreOffice.
"""

import importlib.util
from pathlib import Path
import queue
import sys
import threading
import types
import unittest
import unittest.mock

ROOT = Path(__file__).resolve().parents[3]
EXTENSION = ROOT / "adapters" / "libreoffice" / "extension"
sys.path.insert(0, str(EXTENSION / "pythonpath"))

from badi_libreoffice import broker  # noqa: E402


def uno_stubs():
    """The UNO modules badi_writer imports, as empty interfaces."""
    def interfaces(*names):
        return {name: type(name, (), {}) for name in names}

    class ImplementationHelper:
        def addImplementation(self, *_arguments):
            pass

    contents = {
        "uno": {"createUnoStruct": lambda _name: types.SimpleNamespace()},
        "unohelper": {"Base": type("Base", (), {}), "ImplementationHelper": ImplementationHelper},
        "com": {}, "com.sun": {}, "com.sun.star": {},
        "com.sun.star.awt": interfaces("XCallback", "XKeyHandler"),
        "com.sun.star.frame": interfaces("FeatureStateEvent", "XDispatch", "XDispatchProvider"),
        "com.sun.star.lang": interfaces("XInitialization", "XServiceInfo"),
        "com.sun.star.task": interfaces("XJob"),
        "com.sun.star.util": interfaces("XModifyListener"),
        "com.sun.star.view": interfaces("XSelectionChangeListener"),
    }
    modules = {}
    for name, attributes in contents.items():
        modules[name] = types.ModuleType(name)
        modules[name].__dict__.update(attributes)
    return modules


def load_writer():
    with unittest.mock.patch.dict(sys.modules, uno_stubs()):
        spec = importlib.util.spec_from_file_location("badi_writer_under_test", EXTENSION / "badi_writer.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
    return module


writer = load_writer()


class MainThread:
    """AsyncCallback: callbacks wait here until the test runs them."""

    def __init__(self):
        self.callbacks = queue.Queue()

    def addCallback(self, callback, _data):
        self.callbacks.put(callback)

    def run_next(self, timeout=2):
        self.callbacks.get(timeout=timeout).notify(None)


class FakeAttempt:
    """A broker attempt whose request waits until the test answers it."""

    made = []

    def __init__(self, on_revoked=None):
        self.on_revoked = on_revoked
        self.closed = threading.Event()
        self.answer = threading.Event()
        self.result = None
        FakeAttempt.made.append(self)

    def request(self, _before, _after, _language, _explicit):
        self.answer.wait(2)
        if self.closed.is_set():
            raise broker.BrokerError("closed")
        if isinstance(self.result, Exception):
            raise self.result
        return self.result

    def report(self, _status, _proposal):
        if self.closed.is_set():
            raise broker.BrokerError("closed")

    def finish(self):
        self.close()

    def close(self):
        self.closed.set()


class Binding:
    frame = object()
    controller = model = types.SimpleNamespace()
    runtime_uid = "document"
    before, tail, language = "We need an adress ", "", "en"

    def current(self):
        return True


class ControllerTest(unittest.TestCase):
    def setUp(self):
        FakeAttempt.made = []
        self.main = MainThread()
        context = types.SimpleNamespace(ServiceManager=types.SimpleNamespace(
            createInstanceWithContext=lambda _name, _context: self.main))
        self.controller = writer.Controller(context)
        for target, name, value in ((writer.broker, "Attempt", FakeAttempt),
                                    (writer.Binding, "capture", staticmethod(lambda _frame: (Binding(), None)))):
            patcher = unittest.mock.patch.object(target, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        self.addCleanup(lambda: [attempt.answer.set() for attempt in FakeAttempt.made])

    def state(self):
        return self.controller._phase, self.controller._outcome

    def test_a_request_replacing_an_automatic_check_closes_its_broker_session(self):
        self.controller._request(Binding.frame, Binding())
        automatic = FakeAttempt.made[0]
        self.assertEqual(self.state(), ("requesting", "ready"))
        self.controller.invoke(Binding.frame, "correct")
        self.assertTrue(automatic.closed.is_set(), "closed at once, not after its 3 s answer")
        explicit = FakeAttempt.made[1]
        automatic.answer.set()
        self.main.run_next()  # The superseded attempt's failure arrives and is ignored.
        self.assertEqual(self.state(), ("requesting", "stale"))
        self.assertFalse(explicit.closed.is_set())

    def test_an_unexpected_worker_failure_ends_the_request(self):
        self.controller._request(Binding.frame)
        attempt = FakeAttempt.made[0]
        attempt.result = RuntimeError("socket")
        attempt.answer.set()
        self.main.run_next()
        self.assertEqual(self.state(), ("idle", "protocol"))
        self.assertTrue(attempt.closed.is_set())

    def test_a_grant_arriving_after_the_attempt_ended_is_reported_quietly(self):
        attempt = FakeAttempt()
        attempt.close()  # A revoke or expiry finished it while the grant was in flight.
        self.controller._apply(self.controller._token + 1, attempt, object())
        self.assertEqual(self.state(), ("idle", "ready"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
