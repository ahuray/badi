"""The focused field over AT-SPI: exact selection, purpose metadata, caret
position and bounded text, including Chromium rich-editor flattening."""
from __future__ import annotations

import contextlib
from dataclasses import dataclass
import time
from typing import NamedTuple
from urllib.parse import urlsplit

from contract import Denied, canonical_origin
from desktop import Desktop
from geometry import calibrated_geometry, gecko_calibrated_geometry, visual_direction

MAX_APPLICATIONS = 64
MAX_MATCHES = 8
MAX_TREE_NODES = 192
MAX_CHILDREN = 128
MAX_DOCUMENT_DEPTH = 24
MAX_FRAME_DEPTH = 96
MAX_URI_BYTES = 4096
# A rich editor root exposes each block as one U+FFFC embedded object. Larger
# documents are unsupported rather than slow; each block costs a few calls.
EMBEDDED_OBJECT = "￼"
MAX_RICH_BLOCKS = 64
# Chromium's text-input-v3 surrounding text ends every <p> with two line
# breaks when anything is rendered after the editor, and only one after a
# paragraph whose text already ends in a line break.
PARAGRAPH_END = "\n\n"


class CaretText(NamedTuple):
    """The Text interface holding the glyph before the caret, in its own coordinates."""
    text: object
    caret: int
    length: int


@dataclass(frozen=True)
class RichLayout:
    """A rich editor root's paragraphs in Chromium's flattened coordinates."""
    blocks: list
    caret: int
    total_chars: int
    selections: int
    caret_text: CaretText


class FieldBackend:
    """The Observer's backend: the one focused field of the verified active window.

    Every AT-SPI and desktop call first checks the operation's `deadline`,
    which the daemon sets per request.
    """

    def __init__(self, atspi, apps):
        self.atspi = atspi
        self.deadline = 0
        self.desktop = Desktop(apps, self.remaining)

    def budget(self):
        if time.monotonic() >= self.deadline:
            raise Denied("operation_timeout")

    def remaining(self, limit):
        self.budget()
        return min(limit, max(.001, self.deadline - time.monotonic()))

    def metadata(self, app_id, calibrate=False, check_lock=False):
        """The focused field's identity, purpose, state and caret; never its text.

        With `check_lock`, the session lock query runs while the field is
        read, and its denial replaces the field's result.
        """
        self.budget()
        app = self.desktop.app(app_id)
        with self.desktop.lock_query() if check_lock else contextlib.nullcontext():
            return self.field_metadata(app_id, app, calibrate)

    def field_metadata(self, app_id, app, calibrate):
        window = self.desktop.window(app)
        node = self.focused(window["pid"], app.browser, app.gecko)
        node.clear_cache()
        flags = node.get_state_set()
        role = node.get_role_name()
        if app.roles is not None and role not in app.roles:
            raise Denied("unsupported_field")
        attributes = self.text_attributes(node, role)
        uri = self.page_address(node, app)
        # Gecko serializes block boundaries differently and is unverified;
        # its embedded objects stay in the plain text and never agree.
        flatten = app.web and not app.gecko
        caret, count, selections, rich = self.caret_position(node, flatten)
        local = rich.caret_text if rich else CaretText(node.get_text_iface(), caret, count)
        sentence_start = self.sentence_start(local) if app.sentence_bounded else 0
        geometry = None
        if (calibrate and 1 <= local.caret <= local.length and flags.contains(self.atspi.StateType.SHOWING) and
                window.get("xwayland") is False):
            geometry = self.geometry(node, local.text, local.caret, window, app.gecko, caret)
        return {"direction": self.direction(local), "bus": node.app.bus_name, "path": node.path,
                "process_id": window["pid"], "app_id": app_id, "uri": uri, "browser": app.browser, "web": app.web,
                "role": role, "tag": attributes.get("tag", ""), "input_type": attributes.get("text-input-type", ""),
                **self.states(flags), "caret": caret, "total_chars": count, "selection_count": selections,
                "geometry": geometry, "node": node, "flatten": flatten, "blocks": rich.blocks if rich else None,
                "sentence_start": sentence_start}

    def sentence_start(self, local):
        """Where the caret's sentence starts. LibreOffice Writer gives Fcitx only
        that sentence as surrounding text, so a snapshot must begin there to agree."""
        text, caret, _length = local
        if caret < 1:
            return 0
        self.budget()
        sentence = self.atspi.Text.get_string_at_offset(text, caret - 1, self.atspi.TextGranularity.SENTENCE)
        if sentence is None or not 0 <= sentence.start_offset <= caret - 1 < sentence.end_offset:
            raise Denied("invalid_caret")
        return sentence.start_offset

    def text_attributes(self, node, role):
        """A text field's attributes; a password fails before any caret, extent or text call."""
        if "Text" not in node.get_interfaces():
            raise Denied("unsupported_field")
        attributes = node.get_attributes()
        if role == "password text" or attributes.get("text-input-type") == "password":
            raise Denied("sensitive_field")
        return attributes

    def page_address(self, node, app):
        if not app.browser:
            return ""
        return self.gecko_document_url(node) if app.gecko else self.document_uri(node)

    def states(self, flags):
        state = self.atspi.StateType
        return {"focused": flags.contains(state.FOCUSED), "editable": flags.contains(state.EDITABLE),
                "showing": flags.contains(state.SHOWING), "visible": flags.contains(state.VISIBLE),
                "enabled": flags.contains(state.ENABLED)}

    def direction(self, local):
        """The caret run's direction attribute, else the glyph order before the caret, else ""."""
        text, caret, length = local
        direction = ""
        if length and caret >= 0:
            run = self.atspi.Text.get_attribute_run(text, min(max(caret - 1, 0), length - 1), True)
            direction = run[0].get("direction", "")
        if not direction and 2 <= caret <= length:
            screen = self.atspi.CoordType.SCREEN
            direction = visual_direction(text.get_character_extents(caret - 2, screen),
                                         text.get_character_extents(caret - 1, screen))
        return direction

    def caret_position(self, node, flatten):
        """(caret, length, selections, rich layout or None) in the field's flattened coordinates."""
        self.budget()
        text = node.get_text_iface()
        caret, count, selections = text.get_caret_offset(), text.get_character_count(), text.get_n_selections()
        rich = self.rich_layout(node, count, caret) if flatten else None
        if rich is not None:
            caret, count, selections = rich.caret, rich.total_chars, selections + rich.selections
        return caret, count, selections, rich

    def position(self, metadata):
        """`metadata` with the same field's caret, length and selections read again."""
        caret, count, selections, rich = self.caret_position(metadata["node"], metadata["flatten"])
        return {**metadata, "caret": caret, "total_chars": count, "selection_count": selections,
                "blocks": rich.blocks if rich else None}

    def text(self, metadata, start, end):
        """The field's text in [start, end) of its flattened coordinates."""
        self.budget()
        if not metadata.get("blocks"):
            return self.atspi.Text.get_text(metadata["node"].get_text_iface(), start, end)
        return rich_text(metadata["blocks"], start, end, self.read_block)

    def read_block(self, block, first, last):
        self.budget()
        return self.atspi.Text.get_text(block["node"].get_text_iface(), first, last)

    def focused(self, pid, browser=False, gecko=False):
        """The process's one focused editable field; browser UI never counts."""
        editable = self.atspi.StateType.EDITABLE
        fields = [node for node in self.focused_nodes(pid) if node.get_state_set().contains(editable)]
        if gecko:
            return self.gecko_page_field(fields, pid)
        if browser:
            fields = self.chromium_page_fields(fields)
        return only_field(fields, pid)

    def gecko_page_field(self, fields, pid):
        """Gecko reports exactly one focused editable node: the urlbar while
        it has the keyboard, otherwise the page field. The urlbar shares the
        page's Fcitx input context without a Url purpose, so this observer,
        not the addon, must deny it: the one field's nearest Document must be
        web content with an HTTP(S) DocURL.
        """
        field = only_field(fields, pid)
        self.gecko_document_url(field)
        return field

    def chromium_page_fields(self, fields):
        """Chromium reports FOCUSED on the page field, the omnibox and its
        popup at once, so browser UI is ignored and exactly one page field
        must remain. Picking the page field while the omnibox has the
        keyboard is safe: the omnibox's Url purpose denies it in the addon,
        and every display and dispatch requires this field's snapshot to
        equal Fcitx's live surrounding text and caret.
        """
        fields = [node for node in fields if self.page_content(node)]
        if len(fields) == 1:
            try:
                canonical_origin(self.document_uri(fields[0]))
            except Denied as error:
                if error.timed_out:
                    raise
                raise Denied("focus_unavailable") from None
        return fields

    def focused_nodes(self, pid):
        """Focused nodes of process `pid`, matched in its own applications."""
        root = self.atspi.get_desktop(0)
        nodes = []
        for index in range(min(root.get_child_count(), MAX_APPLICATIONS)):
            self.budget()
            application = root.get_child_at_index(index)
            if application.get_process_id() != pid:
                continue
            if "Collection" in application.get_interfaces():
                nodes.extend(self.collection_matches(application))
            else:
                nodes.extend(self.walk_focused(application))
        return nodes

    def collection_matches(self, application):
        """The application's own match of focused editable nodes; no document text is visited."""
        A = self.atspi
        states = A.StateSet.new([A.StateType.FOCUSED, A.StateType.EDITABLE])
        rule = A.MatchRule.new(states, A.CollectionMatchType.ALL, {}, A.CollectionMatchType.ALL,
                               [], A.CollectionMatchType.ALL, [], A.CollectionMatchType.ALL, False)
        return application.get_collection_iface().get_matches(rule, A.CollectionSortOrder.CANONICAL, MAX_MATCHES, True)

    def walk_focused(self, application):
        """Focused nodes of an application without a Collection interface, breadth first."""
        focused, queue, visited = [], [application], 0
        while queue:
            self.budget()
            node = queue.pop(0)
            visited += 1
            if visited > MAX_TREE_NODES:
                raise Denied("tree_limit")
            if node.get_state_set().contains(self.atspi.StateType.FOCUSED):
                focused.append(node)
            queue.extend(node.get_child_at_index(i) for i in range(min(node.get_child_count(), MAX_CHILDREN)))
        return focused

    def page_content(self, node):
        """False only for Chromium's own interface; True for any document field.

        The omnibox has no document ancestor, and its popup (like other
        browser panels) is a chrome://*.top-chrome WebUI document. Every other
        document is page content, including a non-web one, and so is a field
        whose document cannot be read: it can never be skipped as browser UI.
        """
        try:
            uri = self.nearest_document_uri(node)
        except Denied as error:
            if error.timed_out:
                raise
            return True
        return uri is not None and not browser_interface(uri)

    def document_uri(self, node):
        uri = self.nearest_document_uri(node)
        if uri is None:
            raise Denied("origin_unavailable")
        return uri

    def nearest_document_uri(self, node):
        """The nearest document's URI; None only when no ancestor is a document."""
        for ancestor in self.ancestors(node):
            if "Document" in ancestor.get_interfaces():
                uri = self.document_attribute(ancestor, "URI")
                if uri:
                    return uri
        return None

    def gecko_document_url(self, node):
        """The HTTP(S) URL of a Gecko field's nearest Document, which must be a web page.

        Gecko names the attribute DocURL (its URI is empty). Its browser window
        is itself a Document (role frame), so the urlbar and other browser UI
        resolve to a non-web document. Only the nearest Document counts: an
        empty or non-HTTP(S) URL (about:, moz-extension:, file:, a loading
        frame) is never replaced by an outer page's. Such fields are denied as
        unsupported, without reading their text.
        """
        for ancestor in self.ancestors(node):
            if "Document" in ancestor.get_interfaces():
                if ancestor.get_role_name() != "document web":
                    raise Denied("unsupported_field")
                url = self.document_attribute(ancestor, "DocURL")
                try:
                    canonical_origin(url)
                except Denied:
                    raise Denied("unsupported_field") from None
                return url
        raise Denied("origin_unavailable")

    def ancestors(self, node):
        """`node` and its ancestors, nearest first; origin_unavailable past MAX_DOCUMENT_DEPTH."""
        for _ in range(MAX_DOCUMENT_DEPTH):
            self.budget()
            if node is None:
                return
            yield node
            node = node.get_parent()
        raise Denied("origin_unavailable")

    def document_attribute(self, document, name):
        value = self.atspi.Document.get_document_attribute_value(document.get_document_iface(), name) or ""
        if len(value.encode()) > MAX_URI_BYTES:
            raise Denied("origin_unavailable")
        return value

    def geometry(self, node, text, caret, window, gecko=False, reported=None):
        """Calibrate this request's caret; None lets Fcitx show its own panel.

        `text` and `caret` locate the glyph before the caret: the field's own
        text, or a rich root's caret paragraph. `reported` is the caret in
        the field's (flattened) coordinates, which the reply must name.
        """
        try:
            extents = self.screen_extents(node, text, caret)
            if extents is None:
                return None
            monitors = self.desktop.monitors()
        except Denied as error:
            if error.timed_out:
                raise
            return None
        except Exception:
            return None
        rects = {name: {"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height}
                 for name, rect in extents.items()}
        calibrate = gecko_calibrated_geometry if gecko else calibrated_geometry
        return calibrate(rects["frame"], rects["document"], rects["field"], rects["glyph"],
                         window, monitors, caret if reported is None else reported)

    def screen_extents(self, node, text, caret):
        """SCREEN extents of the outermost frame, nearest web document, field and glyph before the caret."""
        frame, document = self.frame_and_document(node)
        if frame is None or document is None:
            return None
        extents = {}
        for name, item in (("frame", frame), ("document", document), ("field", node)):
            if "Component" not in item.get_interfaces():
                return None
            self.budget()
            extents[name] = item.get_component_iface().get_extents(self.atspi.CoordType.SCREEN)
        self.budget()
        extents["glyph"] = text.get_character_extents(caret - 1, self.atspi.CoordType.SCREEN)
        return extents

    def frame_and_document(self, node):
        """The outermost frame and the field's nearest web document ancestor."""
        ancestor, frame, document = node.get_parent(), None, None
        for _ in range(MAX_FRAME_DEPTH):
            self.budget()
            if ancestor is None:
                return frame, document
            role = ancestor.get_role_name()
            if role == "application":
                return frame, document
            if document is None and role == "document web":
                document = ancestor
            if role in ("frame", "window", "dialog"):
                frame = ancestor
            ancestor = ancestor.get_parent()
        return None, None

    def rich_layout(self, root, count, caret):
        """Flattened coordinates of a Chromium rich editor root, or None for a plain field.

        A ProseMirror composer (Codex desktop) is a focused `entry` whose text
        is one U+FFFC per paragraph; the paragraph holds the typed text,
        reports the caret, and the root's caret is the offset of that
        paragraph's embedded object. Only lengths, caret offsets and structure
        are read here, never prose. Anything the flattening cannot resolve
        fails closed: root text that is not solely embedded paragraphs, a
        paragraph with its own embedded objects (placeholder, image, mention,
        link), another role, a foreign parent, or an ambiguous caret.
        """
        if "Hypertext" not in root.get_interfaces():
            return None
        self.budget()
        hypertext = root.get_hypertext_iface()
        links = self.atspi.Hypertext.get_n_links(hypertext)
        if links == 0:
            return None
        if not 1 <= links <= MAX_RICH_BLOCKS or links != count:
            raise Denied("unsupported_field")
        blocks, carets, selections, start = [], [], 0, 0
        for index in range(links):
            paragraph = self.linked_paragraph(root, hypertext, index)
            text = paragraph.get_text_iface()
            length, offset = text.get_character_count(), text.get_caret_offset()
            selections += text.get_n_selections()
            blocks.append({"node": paragraph, "start": start, "length": length})
            carets.append(offset)
            start += length + len(PARAGRAPH_END)
        return flattened_layout(blocks, carets, selections, start, caret)

    def linked_paragraph(self, root, hypertext, index):
        """The plain paragraph that the root's embedded object `index` links to."""
        A = self.atspi
        self.budget()
        link = A.Hypertext.get_link(hypertext, index)
        if (link is None or A.Hyperlink.get_start_index(link) != index or
                A.Hyperlink.get_end_index(link) != index + 1 or A.Hyperlink.get_n_anchors(link) != 1):
            raise Denied("unsupported_field")
        paragraph = A.Hyperlink.get_object(link, 0)
        if not self.plain_paragraph_of(paragraph, root):
            raise Denied("unsupported_field")
        return paragraph

    def plain_paragraph_of(self, node, root):
        """A `paragraph` child of `root` whose own text embeds no further objects."""
        parent = node.get_parent() if node is not None else None
        if (node is None or parent is None or node.get_role_name() != "paragraph" or
                (parent.app.bus_name, parent.path) != (root.app.bus_name, root.path)):
            return False
        interfaces = node.get_interfaces()
        return ("Text" in interfaces and "Hypertext" in interfaces and
                self.atspi.Hypertext.get_n_links(node.get_hypertext_iface()) == 0)


def only_field(fields, pid):
    if len(fields) != 1 or fields[0].get_process_id() != pid:
        raise Denied("focus_unavailable")
    return fields[0]


def browser_interface(uri):
    """Chromium renders parts of its own interface (such as the omnibox popup) as top-chrome WebUI."""
    try:
        parsed = urlsplit(uri)
        return parsed.scheme == "chrome" and (parsed.hostname or "").endswith(".top-chrome")
    except ValueError:
        return False


def flattened_layout(blocks, carets, selections, total_chars, root_caret):
    """The layout when exactly the paragraph that the root's caret names holds the caret."""
    if not 0 <= root_caret < len(blocks) or not 0 <= carets[root_caret] <= blocks[root_caret]["length"] or any(
            offset != -1 for index, offset in enumerate(carets) if index != root_caret):
        raise Denied("invalid_caret")
    block, caret = blocks[root_caret], carets[root_caret]
    return RichLayout(blocks, block["start"] + caret, total_chars, selections,
                      CaretText(block["node"].get_text_iface(), caret, block["length"]))


def rich_text(blocks, start, end, read):
    """Text of [start, end) in a rich root's flattened coordinates.

    Each block is its paragraph text followed by PARAGRAPH_END, exactly as
    Chromium serializes a paragraph that does not end in a line break. One that
    does (an empty ProseMirror paragraph, a trailing hard break) gets only one
    more break from Chromium, which these lengths cannot express without
    reading prose at inspection, so it fails closed whenever its end is in the
    window. `read(block, first, last)` returns block-local text and must not
    contain embedded objects, whose text cannot be resolved.
    """
    parts = []
    for block in blocks:
        first, length = block["start"], block["length"]
        separator = first + length
        low, high = max(start, first), min(end, separator)
        value = _paragraph_text(read(block, low - first, high - first)) if low < high else ""
        parts.append(value)
        if start < separator + len(PARAGRAPH_END) and separator < end:
            last = value[-1:] if high == separator and value else _last_character(block, read)
            if not isinstance(last, str) or len(last) != 1 or last in ("\n", EMBEDDED_OBJECT):
                raise Denied("unsupported_field")
            parts.append(PARAGRAPH_END[max(start - separator, 0):end - separator])
    return "".join(parts)


def _paragraph_text(value):
    if not isinstance(value, str) or EMBEDDED_OBJECT in value:
        raise Denied("unsupported_field")
    return value


def _last_character(block, read):
    length = block["length"]
    return read(block, length - 1, length) if length else ""
