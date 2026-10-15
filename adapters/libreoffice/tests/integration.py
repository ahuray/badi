#!/usr/bin/env python3
"""Real-broker, real-Writer lane for the LibreOffice extension.

Starts `target/debug/badi-broker --provider phrase` and a headless soffice,
both under a temporary runtime, configuration and LibreOffice profile; the
extension is installed only into that profile. Drives Writer over a private
UNO pipe with disposable text and checks request, display without editing,
acceptance in one undo step, stale and denied paths and authority changes.
Never touches the desktop broker or ~/.config/libreoffice.
"""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest

ROOT = Path(__file__).resolve().parents[3]
EXTENSION = ROOT / "adapters" / "libreoffice"
BROKER = ROOT / "target" / "debug" / "badi-broker"
BADICTL = ROOT / "target" / "debug" / "badictl"
SOFFICE = shutil.which("soffice")
UNOPKG = shutil.which("unopkg")
COMMAND = "vnd.badi.libreoffice:"
INFOBAR = "io.github.ahuray.badi.correction"
WAIT = 20.0

if SOFFICE is None or UNOPKG is None:
    print("SKIPPED: soffice/unopkg not found; the LibreOffice Writer extension was not exercised.")
    sys.exit(0)

import uno  # noqa: E402  (system LibreOffice Python bridge)
import unohelper  # noqa: E402
from com.sun.star.beans import PropertyValue  # noqa: E402
from com.sun.star.frame import XStatusListener  # noqa: E402

sys.path.insert(0, str(EXTENSION))
import build as extension_build  # noqa: E402

GRANT = {"context_read": "allow", "display": "allow", "suggest": "allow", "learn": "block",
         "retention": {"mode": "none"}}


def settings(*apps, revision=1, paused=False):
    subjects = [{"identity": {"kind": "linux_app", "adapter": adapter, "app_id": app_id},
                 "permissions": GRANT} for adapter, app_id in sorted(apps)]
    return {"schema": "badi.settings.v2", "revision": revision, "paused": paused, "subjects": subjects}


def property_value(name, value):
    entry = PropertyValue()
    entry.Name = name
    entry.Value = value
    return entry


class Status(unohelper.Base, XStatusListener):
    """Content-free extension states, `phase:outcome`, in arrival order."""

    def __init__(self):
        self.states = []
        self.condition = threading.Condition()

    def statusChanged(self, event):
        with self.condition:
            self.states.append(event.State)
            self.condition.notify_all()

    def disposing(self, _event):
        pass

    def mark(self):
        with self.condition:
            return len(self.states)

    def wait(self, expected, after, timeout=WAIT):
        deadline = time.monotonic() + timeout
        with self.condition:
            while True:
                for state in self.states[after:]:
                    if state == expected:
                        return
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise AssertionError(f"waited for {expected}; saw {self.states[after:]}")
                self.condition.wait(remaining)


class WriterExtensionTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.root = Path(tempfile.mkdtemp(prefix="badi-libreoffice-test-"))
        cls.environment = {**os.environ, "XDG_RUNTIME_DIR": str(cls.root / "runtime"),
                           "XDG_CONFIG_HOME": str(cls.root / "config"),
                           "XDG_DATA_HOME": str(cls.root / "data"), "HOME": str(cls.root / "home"),
                           "SAL_USE_VCLPLUGIN": "svp",
                           # Holds each taken grant 300 ms before the mutation's
                           # revalidation, so a test edit can land in between.
                           "BADI_LIBREOFFICE_GRANT_DELAY_MS": "300"}
        for name in ("runtime", "data", "home", "config/badi"):
            (cls.root / name).mkdir(parents=True, mode=0o700)
        cls.write_settings(settings(("libreoffice", "libreoffice-writer")))
        cls.broker = subprocess.Popen([str(BROKER), "--provider", "phrase"], env=cls.environment,
                                      stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        socket_path = cls.root / "runtime" / "badi" / "broker.sock"
        deadline = time.monotonic() + WAIT
        while not socket_path.exists():
            if cls.broker.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError("broker did not start: " + cls.broker.stderr.read().decode()[-2000:])
            time.sleep(0.05)
        oxt = extension_build.build(cls.root / "badi-libreoffice.oxt")
        profile = f"-env:UserInstallation=file://{cls.root / 'profile'}"
        subprocess.run([UNOPKG, "add", "--suppress-license", profile, str(oxt)], check=True,
                       env=cls.environment, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                       timeout=120)
        pipe = f"badi-lo-test-{os.getpid()}"
        cls.office = subprocess.Popen([SOFFICE, "--headless", "--invisible", "--norestore",
                                       "--nologo", "--nodefault", "--nolockcheck", profile,
                                       f"--accept=pipe,name={pipe};urp;"],
                                      env=cls.environment, stdout=subprocess.DEVNULL,
                                      stderr=subprocess.DEVNULL)
        local = uno.getComponentContext()
        resolver = local.ServiceManager.createInstanceWithContext(
            "com.sun.star.bridge.UnoUrlResolver", local)
        deadline = time.monotonic() + 60
        while True:
            try:
                cls.context = resolver.resolve(f"uno:pipe,name={pipe};urp;StarOffice.ComponentContext")
                break
            except Exception:
                if cls.office.poll() is not None or time.monotonic() > deadline:
                    raise
                time.sleep(0.1)
        manager = cls.context.ServiceManager
        cls.desktop = manager.createInstanceWithContext("com.sun.star.frame.Desktop", cls.context)
        cls.dispatcher = manager.createInstanceWithContext("com.sun.star.frame.DispatchHelper",
                                                           cls.context)
        cls.transformer = manager.createInstanceWithContext("com.sun.star.util.URLTransformer",
                                                            cls.context)
        cls.status = Status()
        cls.documents = []
        first = cls.new_document()
        cls.status_dispatch = cls.dispatch_for(first, "status")
        cls.status_dispatch.addStatusListener(cls.status, cls.url("status"))

    @classmethod
    def tearDownClass(cls):
        try:
            for document in cls.documents:
                try:
                    document.close(True)
                except Exception:
                    pass
            try:
                cls.desktop.terminate()
            except Exception:
                pass
            try:
                cls.office.wait(timeout=30)
            except subprocess.TimeoutExpired:
                cls.office.kill()
                cls.office.wait()
        finally:
            cls.broker.terminate()
            try:
                cls.broker.wait(timeout=10)
            except subprocess.TimeoutExpired:
                cls.broker.kill()
            shutil.rmtree(cls.root, ignore_errors=True)

    # Helpers ----------------------------------------------------------------

    @classmethod
    def write_settings(cls, document):
        path = cls.root / "config" / "badi" / "settings.json"
        path.write_text(json.dumps(document))
        path.chmod(0o600)

    @classmethod
    def badictl(cls, *arguments, stdin=None):
        result = subprocess.run([str(BADICTL), *arguments], env=cls.environment, input=stdin,
                                capture_output=True, text=True, timeout=10, check=True)
        return json.loads(result.stdout) if result.stdout.strip() else None

    @classmethod
    def replace_settings(cls, document):
        current = cls.badictl("settings", "show", "--json")
        document["revision"] = current["revision"] + 1
        cls.badictl("settings", "replace", "--if-revision", str(current["revision"]), "--json", "-",
                    stdin=json.dumps(document))

    @classmethod
    def metrics(cls):
        return cls.badictl("status")["metrics"]

    @classmethod
    def url(cls, command):
        parsed = uno.createUnoStruct("com.sun.star.util.URL")
        parsed.Complete = COMMAND + command
        _ok, parsed = cls.transformer.parseStrict(parsed)
        return parsed

    @classmethod
    def new_document(cls):
        document = cls.desktop.loadComponentFromURL("private:factory/swriter", "_blank", 0,
                                                    (property_value("Hidden", False),))
        cls.documents.append(document)
        return document

    @classmethod
    def dispatch_for(cls, document, command):
        frame = document.getCurrentController().getFrame()
        dispatch = frame.queryDispatch(cls.url(command), "", 0)
        if dispatch is None:
            raise AssertionError(f"{command} has no dispatch; the protocol handler is not installed")
        return dispatch

    def invoke(self, document, command="correct"):
        frame = document.getCurrentController().getFrame()
        self.dispatcher.executeDispatch(frame, COMMAND + command, "", 0, ())

    def type_text(self, document, value):
        frame = document.getCurrentController().getFrame()
        for character in value:
            self.dispatcher.executeDispatch(frame, ".uno:InsertText", "", 0,
                                            (property_value("Text", character),))

    def fresh(self, value):
        document = self.new_document()
        self.type_text(document, value)
        return document

    @staticmethod
    def body(document):
        return document.getText().getString()

    @staticmethod
    def infobar(document):
        return document.getCurrentController().hasInfobar(INFOBAR)

    def request(self, document, expected="showing:proposal"):
        mark = self.status.mark()
        self.invoke(document)
        self.status.wait(expected, mark)
        return mark

    def settle(self, seconds=0.3):
        time.sleep(seconds)

    # Tests ------------------------------------------------------------------

    def test_01_shortcut_and_menu_command_are_registered(self):
        manager = self.context.ServiceManager
        supplier = manager.createInstanceWithContext(
            "com.sun.star.ui.ModuleUIConfigurationManagerSupplier", self.context)
        shortcuts = supplier.getUIConfigurationManager(
            "com.sun.star.text.TextDocument").getShortCutManager()
        key = uno.createUnoStruct("com.sun.star.awt.KeyEvent")
        key.KeyCode = uno.getConstantByName("com.sun.star.awt.Key.F7")
        key.Modifiers = uno.getConstantByName("com.sun.star.awt.KeyModifier.MOD2")
        self.assertEqual(shortcuts.getCommandByKeyEvent(key), COMMAND + "correct")
        provider = manager.createInstanceWithContext(
            "com.sun.star.configuration.ConfigurationProvider", self.context)
        menu = provider.createInstanceWithArguments(
            "com.sun.star.configuration.ConfigurationAccess",
            (property_value("nodepath", "/org.openoffice.Office.Addons/AddonUI/AddonMenu"),))
        entry = menu.getByName("io.github.ahuray.badi.correct")
        self.assertEqual(entry.URL, COMMAND + "correct")
        self.assertEqual(entry.Context, "com.sun.star.text.TextDocument")

    def test_02_request_shows_without_editing_and_one_undo_restores_the_typo(self):
        before = self.metrics()["commits_applied"]
        document = self.fresh("We need an adress ")
        undo = document.getUndoManager()
        titles = undo.getAllUndoActionTitles()
        self.request(document)
        self.assertEqual(self.body(document), "We need an adress ")
        self.assertTrue(self.infobar(document))
        self.assertEqual(undo.getAllUndoActionTitles(), titles, "showing must not edit")
        mark = self.status.mark()
        self.invoke(document)
        self.status.wait("idle:applied", mark)
        self.assertEqual(self.body(document), "We need an address ")
        self.assertEqual(undo.getCurrentUndoActionTitle(), "Badi correction")
        self.assertEqual(undo.getAllUndoActionTitles()[1:], titles)
        controller = document.getCurrentController()
        caret = controller.getViewCursor()
        self.assertEqual(document.getText().compareRegionEnds(caret, document.getText().getEnd()), 0)
        undo.undo()
        self.assertEqual(self.body(document), "We need an adress ")
        undo.redo()
        self.assertEqual(self.body(document), "We need an address ")
        undo.undo()
        undo.undo()
        self.assertEqual(self.body(document), "We need an adress", "typing stays its own undo steps")
        self.assertEqual(self.metrics()["commits_applied"], before + 1)

    def test_03_word_without_its_space_and_the_infobar_button_accept(self):
        document = self.fresh("Send the exampel")
        self.request(document)
        mark = self.status.mark()
        self.invoke(document, "accept")
        self.status.wait("idle:applied", mark)
        self.assertEqual(self.body(document), "Send the example")

    def test_04_an_edit_after_display_makes_the_proposal_stale(self):
        document = self.fresh("Use this adress ")
        self.request(document)
        mark = self.status.mark()
        self.type_text(document, "x")
        self.status.wait("idle:stale", mark)
        self.assertFalse(self.infobar(document))
        mark = self.status.mark()
        self.invoke(document, "accept")
        self.settle()
        self.assertEqual(self.body(document), "Use this adress x")
        self.assertEqual(self.status.states[mark:], [])

    def test_04b_the_word_at_the_caret_is_corrected_and_the_rest_is_kept(self):
        document = self.fresh("An adress here")
        document.getCurrentController().getViewCursor().goLeft(7, False)
        self.request(document)
        mark = self.status.mark()
        self.invoke(document)
        self.status.wait("idle:applied", mark)
        self.assertEqual(self.body(document), "An address here")

    def test_04c_a_finished_word_is_checked_without_a_request(self):
        document = self.new_document()  # The OnNew job watches it.
        mark = self.status.mark()
        self.type_text(document, "Send it to my adress ")
        self.status.wait("showing:proposal", mark)
        self.assertEqual(self.body(document), "Send it to my adress ")
        mark = self.status.mark()
        self.invoke(document)
        self.status.wait("idle:applied", mark)
        self.assertEqual(self.body(document), "Send it to my address ")

    def test_05_a_caret_move_makes_the_proposal_stale(self):
        document = self.fresh("Find the adress ")
        self.request(document)
        mark = self.status.mark()
        document.getCurrentController().getViewCursor().goLeft(1, False)
        self.status.wait("idle:stale", mark)
        self.assertEqual(self.body(document), "Find the adress ")

    def test_06_selection_and_tables_are_refused_before_the_broker(self):
        calls = self.metrics()["provider_calls"]
        document = self.fresh("A selected adress ")
        document.getCurrentController().getViewCursor().goLeft(3, True)
        self.request(document, "idle:unsupported_field")
        document = self.new_document()
        table = document.createInstance("com.sun.star.text.TextTable")
        table.initialize(1, 1)
        document.getText().insertTextContent(document.getText().getEnd(), table, False)
        cell = table.getCellByName("A1")
        document.getCurrentController().select(cell.getText().getStart())
        self.type_text(document, "adress ")
        self.request(document, "idle:unsupported_field")
        # Change tracking would turn one replacement into redlines.
        document = self.fresh("A tracked adress ")
        document.RecordChanges = True
        self.request(document, "idle:unsupported_field")
        self.assertEqual(self.body(document), "A tracked adress ")
        self.assertEqual(self.metrics()["provider_calls"], calls)

    def test_07_another_document_retires_the_visible_proposal(self):
        first = self.fresh("Their adress ")
        self.request(first)
        second = self.fresh("Our adress ")
        mark = self.status.mark()
        self.invoke(second)
        self.status.wait("idle:stale", mark)
        self.status.wait("showing:proposal", mark)
        self.assertFalse(self.infobar(first))
        # A Replace button invoked from the other window changes nothing.
        mark = self.status.mark()
        self.invoke(first, "accept")
        self.settle()
        self.assertEqual(self.status.states[mark:], [])
        self.assertEqual(self.body(first), "Their adress ")
        self.assertTrue(self.infobar(second))
        mark = self.status.mark()
        self.invoke(second)
        self.status.wait("idle:applied", mark)
        self.assertEqual(self.body(second), "Our address ")

    def test_08_ambiguous_words_get_no_correction(self):
        document = self.fresh("This is teh ")
        self.request(document, "idle:no_correction")
        self.assertEqual(self.body(document), "This is teh ")

    def test_09_authority_change_while_showing_revokes_without_editing(self):
        document = self.fresh("The new adress ")
        self.request(document)
        mark = self.status.mark()
        self.badictl("pause", "on")
        try:
            self.status.wait("idle:revoked", mark)
            self.assertFalse(self.infobar(document))
            self.request(document, "idle:blocked")
        finally:
            self.badictl("pause", "off")
        self.assertEqual(self.body(document), "The new adress ")

    def test_10_only_the_extension_grant_authorizes_it(self):
        calls = self.metrics()["provider_calls"]
        # The Fcitx IME-parity grant for LibreOffice is a different channel.
        self.replace_settings(settings(("fcitx", "libreoffice")))
        try:
            document = self.fresh("No grant adress ")
            self.request(document, "idle:blocked")
            self.assertEqual(self.body(document), "No grant adress ")
            self.assertEqual(self.metrics()["provider_calls"], calls)
        finally:
            self.replace_settings(settings(("libreoffice", "libreoffice-writer")))

    def test_11_an_unread_proposal_expires(self):
        document = self.fresh("Late adress ")
        mark = self.request(document)
        self.status.wait("idle:expired", mark, timeout=20)
        self.assertFalse(self.infobar(document))
        mark = self.status.mark()
        self.invoke(document, "accept")
        self.settle()
        self.assertEqual(self.body(document), "Late adress ")


    def test_12_an_edit_after_the_grant_is_reported_stale_without_editing(self):
        failures = self.metrics()["commit_failures"]
        document = self.fresh("Race the adress ")
        self.request(document)
        mark = self.status.mark()
        self.invoke(document, "accept")
        self.status.wait("accepting:proposal", mark)
        self.type_text(document, "x")
        self.status.wait("idle:stale", mark)
        self.assertEqual(self.body(document), "Race the adress x")
        self.assertEqual(self.metrics()["commit_failures"], failures + 1)

    def test_13_escape_dismisses_a_visible_proposal(self):
        document = self.fresh("Escape the adress ")
        self.request(document)
        controller = document.getCurrentController()
        window = controller.getFrame().getComponentWindow()
        window.setFocus()
        toolkit = self.context.ServiceManager.createInstanceWithContext("com.sun.star.awt.Toolkit",
                                                                        self.context)
        key = uno.createUnoStruct("com.sun.star.awt.KeyEvent")
        key.KeyCode = uno.getConstantByName("com.sun.star.awt.Key.ESCAPE")
        key.Source = window
        mark = self.status.mark()
        toolkit.keyPress(key)
        toolkit.keyRelease(key)
        self.status.wait("idle:dismissed", mark)
        self.assertFalse(self.infobar(document))
        self.assertEqual(self.body(document), "Escape the adress ")


if __name__ == "__main__":
    unittest.main(verbosity=2)
