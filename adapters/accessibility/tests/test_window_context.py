"""Same-window context: the nearest heading and preceding paragraph for the Lab,
and the displayed text near the field for finishing words (ADR 0010)."""
import pathlib
import sys
import time
from types import SimpleNamespace
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from contract import Denied  # noqa: E402
from field import (EMBEDDED_OBJECT, MAX_WINDOW_NODES, VOCABULARY_CHARS, VOCABULARY_NODES,  # noqa: E402
                   WINDOW_HEADING_CHARS, WINDOW_PARAGRAPH_CHARS, FieldBackend, window_text)

PAGE = "https://mail.example/compose"
SHOWN = frozenset({"showing"})


class States:
    def __init__(self, names):
        self.names = names

    def contains(self, name):
        return name in self.names


class Text:
    def __init__(self, value, reads):
        self.value, self.reads = value, reads

    def get_character_count(self):
        return len(self.value)


class Node:
    def __init__(self, role, text=None, children=(), states=SHOWN, interfaces=None):
        self.role, self.states, self.parent = role, states, None
        self.children = list(children)
        for child in self.children:
            child.parent = self
        self.value = text
        self.reads = []
        self.interfaces = interfaces if interfaces is not None else (["Text"] if text is not None else [])

    def get_role_name(self):
        return self.role

    def get_state_set(self):
        return States(self.states)

    def get_parent(self):
        return self.parent

    def get_index_in_parent(self):
        return self.parent.children.index(self)

    def get_child_count(self):
        return len(self.children)

    def get_child_at_index(self, index):
        return self.children[index]

    def get_interfaces(self):
        return self.interfaces

    def get_text_iface(self):
        return Text(self.value, self.reads)


def read(text, start, end):
    text.reads.append((start, end))
    return text.value[start:end]


ATSPI = SimpleNamespace(StateType=SimpleNamespace(EDITABLE="editable", SHOWING="showing"),
                        Text=SimpleNamespace(get_text=read))


def backend(uri=PAGE):
    value = FieldBackend(ATSPI, {})
    value.budget = lambda: None
    value.deadline = time.monotonic() + 60
    value.nearest_document_uri = lambda _node: uri
    return value


def field():
    return Node("entry", "", states=frozenset({"showing", "editable", "focused"}))


def metadata(node, uri=PAGE, web=True, gecko=False):
    return {"node": node, "uri": uri, "web": web, "gecko": gecko}


class WindowContextTests(unittest.TestCase):
    def test_nearest_heading_and_paragraph_above_the_field(self):
        target = field()
        Node("document web", children=[
            Node("heading", "An older section"),
            Node("paragraph", "Text of an older section that must not be used."),
            Node("section", children=[
                Node("heading", "Re: Invoice 4821 for September"),
                Node("paragraph", "Hi Anna, could you resend the corrected invoice?"),
                Node("form", children=[Node("label", "Reply"), target]),
            ]),
        ])
        self.assertEqual(backend().window_context(metadata(target)), {
            "heading": "Re: Invoice 4821 for September",
            "paragraph": "Hi Anna, could you resend the corrected invoice?",
        })

    def test_other_fields_passwords_hidden_text_and_nested_documents_are_never_read(self):
        target = field()
        password = Node("password text", "hunter2")
        other_field = Node("entry", "draft of another field", states=frozenset({"showing", "editable"}))
        hidden = Node("paragraph", "hidden text", states=frozenset())
        frame_paragraph = Node("paragraph", "text inside another frame")
        Node("document web", children=[
            Node("heading", "Settings"),
            Node("paragraph", "Visible guidance."),
            Node("document web", children=[frame_paragraph]),
            password, other_field, hidden,
            target,
        ])
        self.assertEqual(backend().window_context(metadata(target)),
                         {"heading": "Settings", "paragraph": "Visible guidance."})
        for node in (password, other_field, hidden, frame_paragraph):
            self.assertEqual(node.reads, [], node.role)

    def test_unknown_scope_or_native_apps_give_nothing(self):
        target = field()
        Node("document web", children=[Node("heading", "Title"), target])
        self.assertIsNone(backend().window_context(metadata(target, web=False)))
        self.assertIsNone(backend(None).window_context(metadata(target, uri="")), "no document at all")
        # The binding names another document than the one the field is in now.
        self.assertIsNone(backend("https://other.example/").window_context(metadata(target)))
        # No document ancestor at all.
        orphan = field()
        Node("panel", children=[Node("heading", "Title"), orphan])
        self.assertIsNone(backend().window_context(metadata(orphan)))

    def test_a_field_with_nothing_above_it_gives_nothing(self):
        target = field()
        Node("document web", children=[target, Node("paragraph", "Text below the field")])
        self.assertIsNone(backend().window_context(metadata(target)))

    def test_parts_are_bounded_and_cut_at_words(self):
        target = field()
        long_heading = "Quarterly planning " * 20
        long_paragraph = ("word " * 400) + "the closing words of the paragraph"
        Node("document web", children=[Node("heading", long_heading), Node("paragraph", long_paragraph), target])
        result = backend().window_context(metadata(target))
        self.assertLessEqual(len(result["heading"]), WINDOW_HEADING_CHARS)
        self.assertTrue(result["heading"].startswith("Quarterly planning"))
        self.assertLessEqual(len(result["paragraph"]), WINDOW_PARAGRAPH_CHARS)
        self.assertTrue(result["paragraph"].endswith("the closing words of the paragraph"))
        self.assertFalse(result["paragraph"].startswith("ord"), "cut at a word boundary")

    def test_the_walk_stops_within_its_node_budget(self):
        target = field()
        filler = [Node("generic", children=[Node("generic")]) for _ in range(MAX_WINDOW_NODES)]
        heading = Node("heading", "Too far away")
        Node("document web", children=[heading, *filler, target])
        self.assertIsNone(backend().window_context(metadata(target)))
        self.assertEqual(heading.reads, [])

    def test_embedded_objects_and_spacing_are_normalized(self):
        self.assertEqual(window_text(f"Read the {EMBEDDED_OBJECT} guide\n\n  today", 100, tail=True),
                         "Read the guide today")


if __name__ == "__main__":
    unittest.main()


class VocabularyTextTests(unittest.TestCase):
    def test_displayed_text_near_the_field_nearest_first_never_another_field(self):
        target = field()
        Node("document web", children=[
            Node("heading", "Re: Venue deposit for the June offsite"),
            Node("paragraph", "Marinella Costa wrote: the deposit is due on Friday."),
            Node("section", children=[
                Node("entry", "Secret subject draft", states=frozenset({"showing", "editable"})),
                Node("password text", "hunter2hunter2"),
                Node("paragraph", "Hidden note", states=frozenset()),
                Node("document web", "Another frame"),
                Node("label", "To"),
                Node("link", "Okonkwo Adeyemi"),
                target,
                Node("paragraph", f"Quoted {EMBEDDED_OBJECT} reply below", children=[Node("link", "Priyanka")]),
            ]),
        ])
        text = backend().vocabulary_text(metadata(target))
        # Earlier siblings nearest first, then later ones, level by level.
        self.assertEqual(text, "Okonkwo Adeyemi To Quoted reply below Priyanka "
                               "Marinella Costa wrote: the deposit is due on Friday. "
                               "Re: Venue deposit for the June offsite")
        for hidden in ("Secret subject", "hunter2", "Hidden note", "Another frame"):
            self.assertNotIn(hidden, text)

    def test_only_a_confirmed_web_document(self):
        target = field()
        Node("document web", children=[Node("paragraph", "Some words"), target])
        self.assertIsNone(backend().vocabulary_text(metadata(target, web=False)))
        self.assertIsNone(backend(None).vocabulary_text(metadata(target, uri="")), "no document at all")
        self.assertIsNone(backend("https://other.example/").vocabulary_text(metadata(target)))
        orphan = field()
        Node("section", children=[Node("paragraph", "No document"), orphan])
        self.assertIsNone(backend().vocabulary_text(metadata(orphan)))

    def test_the_walk_stops_at_its_character_and_node_bounds(self):
        target = field()
        Node("document web", children=[*(Node("paragraph", "word " * 100) for _ in range(30)), target])
        self.assertLessEqual(len(backend().vocabulary_text(metadata(target))), VOCABULARY_CHARS)
        crowd = field()
        Node("document web", children=[*(Node("paragraph", "x") for _ in range(VOCABULARY_NODES + 50)), crowd])
        self.assertLessEqual(backend().vocabulary_text(metadata(crowd)).count("x"), VOCABULARY_NODES)
        late = field()
        Node("document web", children=[Node("paragraph", "Too late"), late])
        slow = backend()
        slow.deadline = time.monotonic()
        self.assertIsNone(slow.vocabulary_text(metadata(late)), "no headroom left: nothing is read")


class DocumentAnchorTests(unittest.TestCase):
    """Each engine names the field's document its own way (field_document)."""

    def page(self):
        target = field()
        Node("document web", children=[Node("paragraph", "Marinella Costa wrote about Okonkwo."), target])
        return target

    def test_gecko_uses_doc_url_because_its_uri_is_empty(self):
        target = self.page()
        gecko = backend("")
        gecko.gecko_document_url = lambda _node: PAGE
        self.assertIn("Marinella", gecko.vocabulary_text(metadata(target, gecko=True)))
        self.assertIsNotNone(gecko.window_context(metadata(target, gecko=True)))
        gecko.gecko_document_url = lambda _node: "https://other.example/"
        self.assertIsNone(gecko.vocabulary_text(metadata(target, gecko=True)), "another page's document")

        def browser_ui(_node):
            raise Denied("unsupported_field")
        gecko.gecko_document_url = browser_ui
        self.assertIsNone(gecko.vocabulary_text(metadata(target, gecko=True)), "browser UI is no web page")

    def test_a_chromium_based_app_anchors_on_its_own_document(self):
        target = self.page()
        app = backend("file:///opt/app/resources/index.html")
        self.assertIn("Okonkwo", app.vocabulary_text(metadata(target, uri="")),
                      "a desktop app's page has no browser origin to match")
        self.assertIsNone(backend(None).vocabulary_text(metadata(target, uri="")))
        self.assertIsNone(app.vocabulary_text(metadata(target, uri="", web=False)), "native fields get nothing")
