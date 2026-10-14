"""Badi spelling replacement for LibreOffice Writer.

`vnd.badi.libreoffice:correct` (Alt+F7, Tools ▸ Add-Ons) asks the local broker
to correct the word at or before the caret, and a document job checks each
finished word on its own; either shows the proposal in an infobar without
editing. Invoking it again, or the infobar's Replace button, accepts:
the extension revalidates the exact document, view, caret and revision, takes
the broker's one-shot grant, revalidates again and replaces exactly the bound
word inside one undo context, then reads the paragraph back before reporting
`applied`. Escape, any edit, a caret move, the close button, expiry or a broker
authority change dismisses. All document access runs on the office main
thread; only broker I/O runs on a worker thread. Nothing logs or keeps text
after the attempt ends.
"""

import os
import threading
import time

import uno
import unohelper
from com.sun.star.awt import XCallback, XKeyHandler
from com.sun.star.frame import FeatureStateEvent, XDispatch, XDispatchProvider
from com.sun.star.lang import XInitialization, XServiceInfo
from com.sun.star.task import XJob
from com.sun.star.util import XModifyListener
from com.sun.star.view import XSelectionChangeListener

from badi_libreoffice import broker
from badi_libreoffice import text as badi_text

IMPLEMENTATION = "io.github.ahuray.badi.WriterCorrection"
SERVICES = ("com.sun.star.frame.ProtocolHandler",)
PROTOCOL = "vnd.badi.libreoffice:"
# correct: request, or accept a visible proposal; accept: the infobar button;
# status: content-free state for tests and diagnostics, never in the UI.
COMMANDS = ("correct", "accept", "status")
INFOBAR = "io.github.ahuray.badi.correction"
UNDO_TITLE = "Badi correction"
ESCAPE = 1281  # com.sun.star.awt.Key.ESCAPE
INFO = 1  # com.sun.star.frame.InfobarType.INFO
NOTICE_SECONDS = 4.0
POLL_SECONDS = 0.25
# A finished word is checked once typing pauses this long.
AUTOMATIC_PAUSE_SECONDS = 0.5


def _grant_delay():
    """Test seam: hold a taken grant briefly so an edit can land before the
    mutation's revalidation. Bounded below the broker's 1.5 s commit lease;
    it delays only and cannot skip a check."""
    value = os.environ.get("BADI_LIBREOFFICE_GRANT_DELAY_MS", "")
    return int(value) / 1000 if value.isascii() and value.isdigit() and int(value) <= 1000 else 0

MESSAGES = {
    "no_correction": "Badi has no correction for the word before the caret.",
    "blocked": ("Badi spelling is off for LibreOffice Writer or Badi is paused. "
                "Allow it with: badi app libreoffice-writer on"),
    "offline": "The local Badi model is not running. Run: badi doctor",
    "unsupported_field": ("Badi corrects the word before the caret at the end of a body-text "
                          "paragraph, with no selection or change tracking."),
    "failed": "Badi could not verify the correction. Check the text; Ctrl+Z undoes any change.",
    "timeout": "The local Badi model did not answer in time.",
    "rejected": "Badi could not check this word. Run: badi doctor",
    "protocol": "Badi could not check this word. Run: badi doctor",
}


def _property(target, name):
    try:
        info = target.getPropertySetInfo()
        return target.getPropertyValue(name) if info.hasPropertyByName(name) else None
    except Exception:
        return None


def _supports(target, service):
    try:
        return bool(target.supportsService(service))
    except Exception:
        return False


class Binding:
    """The exact Writer state a proposal was requested for."""

    def __init__(self, frame, controller, model, before, tail, language):
        self.frame = frame
        self.controller = controller
        self.model = model
        self.runtime_uid = _property(model, "RuntimeUID")
        # The paragraph up to the corrected word's end, and the rest of it.
        self.before = before
        self.tail = tail
        self.language = language

    @classmethod
    def capture(cls, frame):
        """The collapsed caret in a body-text paragraph of an editable plain
        Writer document, or (None, reason)."""
        try:
            controller = frame.getController()
            model = controller.getModel() if controller is not None else None
            if (model is None or not _supports(model, "com.sun.star.text.TextDocument")
                    or _supports(model, "com.sun.star.text.WebDocument")
                    or _supports(model, "com.sun.star.text.GlobalDocument")
                    or model.isReadonly() or _property(model, "RecordChanges")):
                return None, "unsupported_field"
            selection = controller.getSelection()
            if (selection is None or not _supports(selection, "com.sun.star.text.TextRanges")
                    or selection.getCount() != 1):
                return None, "unsupported_field"
            view_cursor = controller.getViewCursor()
            body = model.getText()
            if (not view_cursor.isCollapsed() or view_cursor.getText() != body
                    or _property(view_cursor, "TextTable") is not None
                    or _property(view_cursor, "TextFrame") is not None
                    or _property(view_cursor, "TextField") is not None):
                return None, "unsupported_field"
            section = _property(view_cursor, "TextSection")
            if section is not None and _property(section, "IsProtected"):
                return None, "unsupported_field"
            before, after = _paragraph(body, view_cursor)
            locale = _property(view_cursor, "CharLocale")
            language = badi_text.language_tag(getattr(locale, "Language", ""),
                                              getattr(locale, "Country", ""))
        except Exception:
            return None, "unsupported_field"
        span = badi_text.correction_span(before, after)
        # The broker sees the paragraph up to the word: a correction replaces
        # the end of that text, and the tail stays as it is.
        if language is None or span is None or badi_text.bounded_context(span[0], "") is None:
            return None, "unsupported_field"
        return cls(frame, controller, model, span[0], span[1], language), None

    def current(self):
        """The same frame, view, document, paragraph, word and language."""
        fresh, _reason = Binding.capture(self.frame)
        return (fresh is not None and fresh.controller == self.controller
                and fresh.model == self.model and fresh.runtime_uid == self.runtime_uid
                and fresh.before == self.before and fresh.tail == self.tail
                and fresh.language == self.language)


def _paragraph(body, view_cursor):
    anchor = view_cursor.getStart()
    before = body.createTextCursorByRange(anchor)
    before.gotoStartOfParagraph(True)
    after = body.createTextCursorByRange(anchor)
    after.gotoEndOfParagraph(True)
    return before.getString(), after.getString()


class _Callback(unohelper.Base, XCallback):
    def __init__(self, function):
        self._function = function

    def notify(self, _data):
        self._function()


class _Watch(unohelper.Base, XModifyListener, XSelectionChangeListener, XKeyHandler):
    """Dismisses on any edit, caret or selection change, Escape or disposal."""

    def __init__(self, controller, token):
        self._controller = controller
        self._token = token
        self.mutating = False

    def modified(self, _event):
        if not self.mutating:
            self._controller.post(lambda: self._controller.invalidate(self._token, "stale"))

    def selectionChanged(self, _event):
        if not self.mutating:
            self._controller.post(lambda: self._controller.revalidate(self._token))

    def keyPressed(self, event):
        if event.KeyCode == ESCAPE and event.Modifiers == 0 and self._controller.showing(self._token):
            self._controller.post(lambda: self._controller.invalidate(self._token, "dismissed"))
            return True
        return False

    def keyReleased(self, _event):
        return False

    def disposing(self, _event):
        self._controller.post(lambda: self._controller.invalidate(self._token, "stale"))


class _Typing(unohelper.Base, XModifyListener):
    """Notices edits in one document so a finished word can be checked."""

    def __init__(self, controller, model):
        self._controller = controller
        self._model = model

    def modified(self, _event):
        self._controller.post(lambda: self._controller.typed(self._model))

    def disposing(self, _event):
        self._controller.post(lambda: self._controller.forget(self._model))


class Controller:
    """One correction at a time for the whole office process. State changes
    happen only on the main thread, through `post`."""

    def __init__(self, context):
        self._context = context
        self._async = context.ServiceManager.createInstanceWithContext(
            "com.sun.star.awt.AsyncCallback", context)
        self._token = 0
        self._phase = "idle"
        self._outcome = "ready"
        self._binding = None
        self._attempt = None
        self._proposal = None
        self._watch = None
        self._timer = None
        self._notice = None
        self._listeners = []
        # Automatic checks: the documents watched, the pending pause, the
        # last word checked, and whether the current attempt is automatic.
        self._typing = []
        self._pause = None
        self._checked = None
        self._automatic = False

    # Threads and listeners --------------------------------------------------

    def post(self, function):
        self._async.addCallback(_Callback(function), None)

    def _later(self, seconds, function):
        timer = threading.Timer(seconds, lambda: self.post(function))
        timer.daemon = True
        timer.start()
        return timer

    def add_listener(self, listener, url, source):
        self._listeners.append((listener, url, source))
        self._notify_one(listener, url, source)

    def remove_listener(self, listener, url):
        self._listeners = [entry for entry in self._listeners
                           if not (entry[0] == listener and entry[1].Complete == url.Complete)]

    def _notify_one(self, listener, url, source):
        event = FeatureStateEvent()
        event.FeatureURL = url
        event.IsEnabled = True
        event.Requery = False
        event.Source = source
        if url.Path == "status":
            event.State = f"{self._phase}:{self._outcome}"
        try:
            listener.statusChanged(event)
        except Exception:
            self.remove_listener(listener, url)

    def _publish(self, phase, outcome=None):
        self._phase = phase
        if outcome is not None:
            self._outcome = outcome
        for listener, url, source in list(self._listeners):
            if url.Path == "status":
                self._notify_one(listener, url, source)

    def showing(self, token):
        return token == self._token and self._phase == "showing"

    # Commands ---------------------------------------------------------------

    def watch(self, frame):
        try:
            self.watch_model(frame.getController().getModel())
        except Exception:
            pass

    def watch_model(self, model):
        """Checks each finished word of this document on its own."""
        if (model is None or not _supports(model, "com.sun.star.text.TextDocument")
                or any(watched == model for watched in self._typing)):
            return
        try:
            model.addModifyListener(_Typing(self, model))
        except Exception:
            return
        self._typing.append(model)

    def forget(self, model):
        self._typing = [watched for watched in self._typing if watched != model]

    def typed(self, model):
        if self._pause is not None:
            self._pause.cancel()
        self._pause = self._later(AUTOMATIC_PAUSE_SECONDS, lambda: self._check(model))

    def _check(self, model):
        """After a typing pause: a word just finished with a space, checked once."""
        self._pause = None
        if self._phase != "idle":
            return
        try:
            frame = model.getCurrentController().getFrame()
        except Exception:
            return
        binding, _reason = Binding.capture(frame)
        if binding is None or binding.tail or not binding.before.endswith(" ") \
                or not binding.before[-2:-1].isalpha():
            return
        key = (binding.runtime_uid, binding.before)
        if key == self._checked:
            return
        self._checked = key
        self._request(frame, binding)

    def invoke(self, frame, command):
        if command == "status":
            return
        if command == "correct" and self._phase == "requesting" and self._automatic:
            # The user's own request replaces an automatic check in flight.
            self._finish("stale")
        if self._phase == "showing":
            # Only the visible, still-current proposal in this window can be
            # accepted. A Replace button left in another window never touches
            # it; a new request anywhere retires it first.
            same = self._binding.frame == frame
            if same and self._visible() and self._binding.current():
                self._accept()
                return
            if command == "accept" and not same:
                return
            self._finish("stale")
        if command == "correct" and self._phase == "idle":
            self._request(frame)

    def _request(self, frame, automatic=None):
        """An explicit request, or the automatic check of `automatic` (a Binding)."""
        binding, reason = (automatic, None) if automatic is not None else Binding.capture(frame)
        if binding is None:
            self._notice_on(frame, reason)
            self._publish("idle", reason)
            return
        self._automatic = automatic is not None
        explicit = not self._automatic
        self._token += 1
        token = self._token
        self._binding = binding
        self._watch = _Watch(self, token)
        self._listen(True)
        self._publish("requesting")

        def revoked(code):
            self.post(lambda: self.invalidate(token, code))

        def work():
            attempt = None
            try:
                attempt = broker.Attempt(on_revoked=revoked)
                proposal = attempt.request(binding.before, "", binding.language, explicit)
                self.post(lambda: self._show(token, attempt, proposal))
            except broker.BrokerError as error:
                code = error.code
                if attempt is not None:
                    attempt.close()
                self.post(lambda: self.invalidate(token, code))

        threading.Thread(target=work, name="badi-correction", daemon=True).start()

    def _show(self, token, attempt, proposal):
        if token != self._token or self._phase != "requesting":
            attempt.close()
            return
        if proposal is None:
            attempt.finish()
            self._finish("no_correction")
            return
        if not self._binding.current():
            attempt.finish()
            self._finish("stale")
            return
        self._attempt = attempt
        self._proposal = proposal
        word = proposal.replace_before.rstrip(" ")
        corrected = proposal.text.rstrip(" ")
        if not self._infobar(f"Badi: {word} → {corrected}",
                             "Alt+F7 again or Replace accepts it. Esc dismisses.", True):
            self._finish("unsupported_field")
            return
        self._timer = self._later(proposal.ttl_ms / 1000, lambda: self.invalidate(token, "expired"))
        self._publish("showing", "proposal")
        self._poll(token)

    def _poll(self, token):
        """Selection events do not cover every caret move (API moves, some
        toolkits), so a visible proposal also rechecks its binding."""
        if token == self._token and self._phase == "showing":
            if not self._visible() or not self._binding.current():
                self.invalidate(token, "stale")
                return
            self._later(POLL_SECONDS, lambda: self._poll(token))

    def _accept(self):
        token = self._token
        attempt = self._attempt
        self._publish("accepting")
        self._cancel_timer()

        def work():
            try:
                proposal = attempt.authorize()
                delay = _grant_delay()
                if delay:
                    time.sleep(delay)
                self.post(lambda: self._apply(token, attempt, proposal))
            except broker.BrokerError as error:
                code = error.code
                attempt.close()
                self.post(lambda: self.invalidate(token, code))

        threading.Thread(target=work, name="badi-acceptance", daemon=True).start()

    def _apply(self, token, attempt, proposal):
        if token != self._token or self._phase != "accepting":
            attempt.report("stale", proposal)
            return
        status = self._replace(proposal)
        try:
            attempt.report(status, proposal)
        except broker.BrokerError:
            pass
        self._attempt = None
        if status == "applied":
            frame = self._binding.frame
            self._finish("applied", keep_infobar=True)
            self._notice_on(frame, None, "Badi replaced the word. Ctrl+Z undoes it.")
        else:
            self._finish(status)

    def _replace(self, proposal):
        """Revalidates, replaces the bound word in one undo context and reads
        the paragraph back. Returns the commit status to report."""
        binding = self._binding
        if not self._visible() or not binding.current():
            return "stale"
        original = proposal.replace_before
        try:
            body = binding.model.getText()
            view_cursor = binding.controller.getViewCursor()
            cursor = body.createTextCursorByRange(view_cursor.getStart())
            cursor.gotoStartOfParagraph(False)
            if (not cursor.goRight(len(binding.before), False) or not cursor.goLeft(len(original), True)
                    or cursor.getString() != original):
                return "stale"
            undo = binding.model.getUndoManager()
        except Exception:
            return "stale"
        self._watch.mutating = True
        try:
            undo.enterUndoContext(UNDO_TITLE)
            try:
                cursor.setString(proposal.text)
            finally:
                undo.leaveUndoContext()
        except Exception:
            return "failed"
        finally:
            self._watch.mutating = False
        try:
            before, after = _paragraph(body, binding.controller.getViewCursor())
            expected = binding.before[:len(binding.before) - len(original)] + proposal.text + binding.tail
            verified = (before + after == expected
                        and binding.controller.getViewCursor().isCollapsed()
                        and undo.getCurrentUndoActionTitle() == UNDO_TITLE)
        except Exception:
            verified = False
        return "applied" if verified else "failed"

    # Invalidation -----------------------------------------------------------

    def invalidate(self, token, reason):
        if token != self._token or self._phase == "idle":
            return
        if self._phase == "accepting" and reason in ("stale", "dismissed"):
            # The grant request is in flight; _apply revalidates and reports.
            return
        if reason == "dismissed" and self._attempt is not None:
            self._attempt.dismiss()
        self._finish(reason)

    def revalidate(self, token):
        if token == self._token and self._phase in ("requesting", "showing") \
                and not self._binding.current():
            self.invalidate(token, "stale")

    def _visible(self):
        try:
            return bool(self._binding.controller.hasInfobar(INFOBAR))
        except Exception:
            return False

    def _finish(self, outcome, keep_infobar=False):
        frame = self._binding.frame if self._binding is not None else None
        self._token += 1
        self._cancel_timer()
        self._listen(False)
        if self._attempt is not None:
            self._attempt.finish()
        self._attempt = None
        self._proposal = None
        self._binding = None
        self._watch = None
        if not keep_infobar:
            self._remove_infobar(frame)
            # An automatic check that found nothing, or failed, stays silent.
            if outcome in MESSAGES and not self._automatic:
                self._notice_on(frame, outcome)
        self._automatic = False
        self._publish("idle", outcome)

    def _cancel_timer(self):
        if self._timer is not None:
            self._timer.cancel()
            self._timer = None

    def _listen(self, enabled):
        binding, watch = self._binding, self._watch
        if binding is None or watch is None:
            return
        operations = (
            (binding.model, "addModifyListener", "removeModifyListener"),
            (binding.controller, "addSelectionChangeListener", "removeSelectionChangeListener"),
            (binding.controller, "addKeyHandler", "removeKeyHandler"),
        )
        for target, add, remove in operations:
            try:
                getattr(target, add if enabled else remove)(watch)
            except Exception:
                pass

    # Infobar ----------------------------------------------------------------

    def _infobar(self, primary, secondary, actions, controller=None):
        controller = controller or self._binding.controller
        try:
            if controller.hasInfobar(INFOBAR):
                controller.removeInfobar(INFOBAR)
            buttons = ()
            if actions:
                replace = uno.createUnoStruct("com.sun.star.beans.StringPair")
                replace.First = "Replace"
                replace.Second = PROTOCOL + "accept"
                buttons = (replace,)
            controller.appendInfobar(INFOBAR, primary, secondary, INFO, buttons, True)
            return True
        except Exception:
            return False

    def _remove_infobar(self, frame):
        try:
            controller = frame.getController() if frame is not None else None
            if controller is not None and controller.hasInfobar(INFOBAR):
                controller.removeInfobar(INFOBAR)
        except Exception:
            pass

    def _notice_on(self, frame, reason, message=None):
        """A short, content-free notice for an explicit command."""
        message = message or MESSAGES.get(reason)
        if frame is None or message is None:
            return
        try:
            controller = frame.getController()
        except Exception:
            return
        if not self._infobar(message, "", False, controller):
            return
        if self._notice is not None:
            self._notice.cancel()
        token = self._token
        self._notice = self._later(NOTICE_SECONDS, lambda: self._clear_notice(token, frame))

    def _clear_notice(self, token, frame):
        if token == self._token and self._phase == "idle":
            self._remove_infobar(frame)


_controller = None
_controller_lock = threading.Lock()


def controller_for(context):
    global _controller
    with _controller_lock:
        if _controller is None:
            _controller = Controller(context)
        return _controller


class Dispatch(unohelper.Base, XDispatch):
    def __init__(self, context, frame, command):
        self._context = context
        self._frame = frame
        self._command = command

    def dispatch(self, _url, _arguments):
        controller = controller_for(self._context)
        frame, command = self._frame, self._command
        # Accelerators arrive on the main thread; other callers are moved there.
        controller.post(lambda: controller.invoke(frame, command))

    def addStatusListener(self, listener, url):
        controller_for(self._context).add_listener(listener, url, self)

    def removeStatusListener(self, listener, url):
        controller_for(self._context).remove_listener(listener, url)


class WriterCorrection(unohelper.Base, XInitialization, XDispatchProvider, XServiceInfo):
    def __init__(self, context, *arguments):
        self._context = context
        self._frame = None
        if arguments:
            self.initialize(arguments)

    def initialize(self, arguments):
        for argument in arguments:
            if hasattr(argument, "getController"):
                self._frame = argument
        if self._frame is not None:
            controller = controller_for(self._context)
            frame = self._frame
            controller.post(lambda: controller.watch(frame))

    def queryDispatch(self, url, _target, _flags):
        if url.Protocol == PROTOCOL and url.Path in COMMANDS and self._frame is not None:
            return Dispatch(self._context, self._frame, url.Path)
        return None

    def queryDispatches(self, requests):
        return tuple(self.queryDispatch(request.FeatureURL, request.FrameName, request.SearchFlags)
                     for request in requests)

    def getImplementationName(self):
        return IMPLEMENTATION

    def supportsService(self, name):
        return name in SERVICES

    def getSupportedServiceNames(self):
        return SERVICES


class WriterWatch(unohelper.Base, XJob):
    """Runs on each new or opened document so its finished words are checked
    without Alt+F7 first."""

    def __init__(self, context, *_arguments):
        self._context = context

    def execute(self, arguments):
        for argument in arguments:
            if argument.Name != "Environment":
                continue
            for value in argument.Value:
                if value.Name == "Model" and _supports(value.Value, "com.sun.star.text.TextDocument"):
                    model = value.Value
                    controller = controller_for(self._context)
                    controller.post(lambda: controller.watch_model(model))
        return ()


g_ImplementationHelper = unohelper.ImplementationHelper()
g_ImplementationHelper.addImplementation(WriterCorrection, IMPLEMENTATION, SERVICES)
g_ImplementationHelper.addImplementation(WriterWatch, "io.github.ahuray.badi.WriterWatch",
                                         ("com.sun.star.task.Job",))
