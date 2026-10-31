// Explicit revision of one selection (broker ADR 0012): capture, text
// safety, the panel's word diff, the read-back check, the revision state and
// its wire request.

#include "observation.h"
#include "revision.h"
#include "state.h"
#include "transport.h"

#include <nlohmann/json.hpp>

#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>

namespace {

using namespace badi::fcitx5;
using Json = nlohmann::json;
constexpr auto kSession = "550e8400-e29b-41d4-a716-446655440000";
constexpr auto kSalt = "0123456789abcdef0123456789abcdef";
constexpr PanelObservation kOwnedPanel{.candidates = true, .candidatesOwnedByBadi = true};

void check(bool condition, const char *message) {
    if (!condition) throw std::runtime_error(message);
}

void capturesOneLineSelectionWithItsSurroundings() {
    const std::string text = "Yesterday i has went to the shop with Anna.";
    const auto window = captureSelectionWindow(text, 32, 10, true, "en");
    check(window.has_value(), "a one-line selection is captured");
    check(window->before == "Yesterday " && window->selected == "i has went to the shop" &&
              window->after == " with Anna.",
          "the selection and the text around it");
    check(window->anchor == 10 && window->head == 32, "Fcitx's anchor and cursor are kept as reported");
    // Selected backwards: the same text, the anchor after the cursor.
    const auto backwards = captureSelectionWindow(text, 10, 32, true, "en");
    check(backwards && backwards->selected == window->selected && backwards->anchor == 32,
          "a backward selection reads the same");
    // Scalar offsets, not bytes.
    const auto umlaut = captureSelectionWindow("Für Anna: Ich habe gegangen.", 27, 10, false, "de");
    check(umlaut && umlaut->selected == "Ich habe gegangen" && umlaut->before == "Für Anna: ",
          "offsets count scalars");
    check(!captureSelectionWindow(text, 5, 5, true, "en"), "a caret is no selection");
    check(!captureSelectionWindow("first line\nsecond line", 15, 3, true, "en"), "one line only");
    check(!captureSelectionWindow(std::string(700, 'a'), 650, 0, false, "en"), "at most 600 scalars");
    check(!captureSelectionWindow("a     b", 5, 1, false, "en"), "spaces alone are nothing to revise");
    check(!captureSelectionWindow(text, 99, 10, true, "en"), "a cursor beyond the text");
    check(!captureSelectionWindow(text, 32, 10, true, "not a tag!"), "a valid language tag");
}

void revisionTextIsOneVisibleLine() {
    check(safeRevisionText("Tu viens demain\xe2\x80\xaf?", 600), "French narrow no-break space");
    check(safeRevisionText("café\xc2\xa0noir", 600), "no-break space");
    for (const auto *unsafe : {"two\nlines", "tab\there", "bidi \xe2\x80\xaeoverride", "zero\xe2\x80\x8bwidth", "soft\xc2\xadhyphen",
                               "line\xe2\x80\xa8separator", "ideographic\xe3\x80\x80space", ""}) {
        check(!safeRevisionText(unsafe, 600), unsafe);
    }
    check(!safeRevisionText("\xff\xfe", 600), "invalid UTF-8");
    check(!safeRevisionText(std::string(601, 'a'), 600), "the bound counts scalars");
    check(safeRevisionText(std::string(600, 'a'), 600), "up to the bound");
}

void theDiffMarksRemovedThenAddedWords() {
    const auto diff = revisionDiff("i has went to the shop", "i went to the shop");
    check(diff == std::vector<DiffPart>{{DiffKind::Same, "i"}, {DiffKind::Removed, "has"},
                                        {DiffKind::Same, "went to the shop"}},
          "a removed word between kept words");
    const auto replaced = revisionDiff("their going home", "they're going home.");
    check(replaced == std::vector<DiffPart>{{DiffKind::Removed, "their"}, {DiffKind::Added, "they're"},
                                            {DiffKind::Same, "going"}, {DiffKind::Removed, "home"},
                                            {DiffKind::Added, "home."}},
          "a replaced word is shown removed, then added");
    check(revisionDiff("same", "same") == std::vector<DiffPart>{{DiffKind::Same, "same"}}, "nothing changed");
    check(revisionDiff("", "new words") == std::vector<DiffPart>{{DiffKind::Added, "new words"}}, "all added");
}

void theDiffWrapsAtWordsAndSaysWhenItIsCut() {
    const auto diff = revisionDiff("alpha beta gamma delta epsilon", "alpha beta gamma delta epsilon zeta");
    const auto lines = wrapDiff(diff, 12, 5);
    check(lines.complete && lines.lines.size() == 3, "words wrap within the width");
    check(lines.lines[0] == std::vector<DiffPart>{{DiffKind::Same, "alpha beta"}}, "the first line");
    check(lines.lines[2] == std::vector<DiffPart>{{DiffKind::Same, "epsilon"}, {DiffKind::Added, "zeta"}},
          "kinds stay apart on a line");
    const auto cut = wrapDiff(diff, 12, 2);
    check(!cut.complete && cut.lines.size() == 2, "more lines than allowed are cut and say so");
    const auto long_word = wrapDiff({{DiffKind::Same, "supercalifragilistic"}}, 8, 3);
    check(long_word.complete && long_word.lines.size() == 1, "a word longer than the width stays whole");
}

void aRevisionLandsOnlyWhereTheSelectionWas() {
    SelectionWindow window{.before = "Yesterday ", .selected = "i has went to the shop", .after = " with Anna.",
                           .language = "en"};
    check(revisionLanded("Yesterday i went to the shop", " with Anna.", window, "i went to the shop"),
          "the field reads as the revision left it");
    check(!revisionLanded("Yesterday i has went to the shop", " with Anna.", window, "i went to the shop"),
          "the selection is still there");
    check(!revisionLanded("Yesterday i went to the shop", "", window, "i went to the shop"),
          "the text after it is gone");
    check(revisionLanded("Yesterday i went to the shop", " with Anna.\n\n", window, "i went to the shop"),
          "a paragraph end Chromium adds after it");
    check(!revisionLanded("Yesterday i went to the shop", " with Bob.", window, "i went to the shop"),
          "the text after it changed");
    check(!revisionLanded("", "", window, "i went to the shop"), "an empty window confirms nothing");
    // A window that slid: only its tail is known, and it must match.
    const std::string longBefore(500, 'x');
    SelectionWindow slid{.before = longBefore, .selected = "teh end", .after = "", .language = "en"};
    check(revisionLanded(std::string(400, 'x') + "the end", "", slid, "the end"), "a shorter window's tail");
}

void noticesNameWhatTheUserCanDo() {
    check(revisionClearNotice("no_change") == std::optional<std::string_view>("Badi found nothing to change"),
          "nothing to change");
    check(revisionClearNotice("unsupported_language").has_value(), "a language Badi does not revise");
    check(!revisionClearNotice("dismissed") && !revisionClearNotice("superseded") && !revisionClearNotice("expired"),
          "the user's own retirement says nothing");
    check(revisionHelp("grammar", "i has went to the shop") ==
              "Grammar · Tab apply · Esc dismiss · Ctrl+Shift+Space refine",
          "grammar help");
    check(revisionHelp("refine", "i has went to the shop") ==
              "Refined · Tab apply · Esc dismiss · Ctrl+Shift+Space grammar",
          "refine help for a long selection");
    check(revisionHelp("refine", "beautiful") ==
              "Refined · Tab apply · Esc dismiss · Ctrl+Shift+Space synonyms",
          "refine help for a short selection");
    check(revisionHelp("synonyms", "beautiful") ==
              "Synonym · Tab apply · Esc dismiss · Ctrl+Shift+Space another",
          "synonym help");
}

SessionState focused(std::string_view app = "org.gnome.texteditor") {
    SessionState state;
    check(state.focusIn(kSession, "input-context-1", std::string(app), kSalt), "focus");
    return state;
}

SelectionWindow selection() {
    return *captureSelectionWindow("Yesterday i has went to the shop with Anna.", 32, 10, true, "en");
}

Revision revisionFor(const RevisionUpdate &update, std::uint64_t expiresAt = 60'000) {
    return Revision{
        .coordinates = update.coordinates,
        .requestId = update.requestId,
        .revisionId = "r:1",
        .mode = update.mode,
        .original = update.selection.selected,
        .text = "i went to the shop",
        .expiresAtMs = expiresAt,
    };
}

void aRevisionIsShownAndAppliedOnceOverItsSelection() {
    auto state = focused();
    const auto update = state.requestRevision(selection(), "grammar");
    check(update.has_value(), "a revision request");
    check(update->coordinates.revision == 1 && update->coordinates.fingerprint.size() == 32,
          "a new field revision with its own fingerprint");
    check(!state.lastContext(), "the caret context is superseded");
    auto wrong = revisionFor(*update);
    wrong.original = "something else";
    check(!state.showRevision(wrong, 10), "a revision of another selection is not shown");
    check(state.showRevision(revisionFor(*update), 10), "the revision is shown");
    const auto accept = state.requestRevisionAcceptance(20, kOwnedPanel);
    check(accept && accept->suggestionId == "r:1" && accept->acceptance == "all", "Tab accepts it whole");
    check(!state.requestRevisionAcceptance(21, kOwnedPanel), "one acceptance at a time");
    CommitPrepare grant{
        .coordinates = update->coordinates,
        .controlId = accept->controlId,
        .suggestionId = "r:1",
        .text = "i went to the shop",
        .acceptance = "all",
        .replaceSelection = SelectionRange{10, 32},
    };
    auto other = grant;
    other.replaceSelection = SelectionRange{10, 31};
    check(!state.authorizeRevisionCommit(other, 30, kOwnedPanel), "another selection is not authorized");
    const auto dispatch = state.authorizeRevisionCommit(grant, 30, kOwnedPanel);
    check(dispatch && dispatch->text == "i went to the shop" && dispatch->replaceSelection == grant.replaceSelection,
          "the grant over exactly the selection");
    check(!state.visibleRevision() && !state.revisionRequest(), "applied once");
    check(!state.authorizeRevisionCommit(grant, 31, kOwnedPanel), "never twice");
}

void anyFieldChangeOrExpiryRetiresTheRevision() {
    auto state = focused();
    const auto update = state.requestRevision(selection(), "grammar");
    check(state.showRevision(revisionFor(*update), 10), "shown");
    state.invalidateContext();
    check(!state.visibleRevision() && !state.revisionRequest(), "a field change retires it");

    const auto again = state.requestRevision(selection(), "refine");
    check(state.showRevision(revisionFor(*again, 50), 10), "shown again");
    check(!state.requestRevisionAcceptance(60, kOwnedPanel) && !state.visibleRevision(), "an expired one is gone");

    const auto third = state.requestRevision(selection(), "grammar");
    check(state.showRevision(revisionFor(*third), 10), "shown a third time");
    const auto dismiss = state.requestRevisionDismissal(20, kOwnedPanel);
    check(dismiss && dismiss->suggestionId == "r:1" && !state.visibleRevision(), "Escape dismisses it");

    const auto fourth = state.requestRevision(selection(), "grammar");
    check(state.clearRevisionIf(fourth->coordinates, std::string("r:unknown-yet")),
          "a broker clear ends a request whose revision is not shown yet");

    check(!state.requestRevision(selection(), "rewrite"), "only grammar, refine and synonyms");
    auto chromium = focused("chromium");
    check(!chromium.requestRevision(selection(), "grammar"), "an IME-parity app needs the observer's field");
}

void theRequestCarriesExactlyTheSelection() {
    auto state = focused();
    const auto update = state.requestRevision(selection(), "grammar");
    const auto body = serializeRevisionEnvelope(*update, 7);
    check(body.has_value(), "serialized");
    const auto message = Json::parse(*body);
    check(message["type"] == "revision.request" && message["id"] == update->requestId, "the request");
    const auto &payload = message["payload"];
    check(payload["selected"] == "i has went to the shop" && payload["before"] == "Yesterday " &&
              payload["after"] == " with Anna." && payload["mode"] == "grammar",
          "the selection and its surroundings");
    check(payload["selection"] == Json{{"anchor", 10}, {"head", 32}, {"unit", "unicode_scalar_values"}},
          "the selection's offsets");
    check(payload["field"]["identity_known"] == false && payload["field"]["purpose"] == "unknown",
          "an unobserved native field is the manual contract");
    check(payload["activation"] == "manual", "always an explicit request");
    auto broken = *update;
    broken.selection.head = 31;
    check(!serializeRevisionEnvelope(broken, 7), "offsets must span the selection");
}

void aSelectionIsTheSameOnlyAsFarAsItsUseNeeds() {
    const auto requested = selection();
    auto moved = requested;
    check(sameSelection(requested, requested, true) && sameSelection(requested, requested, false), "itself");
    moved.after = " with Anna.\n\n";
    check(sameSelection(moved, requested, false) && !sameSelection(moved, requested, true),
          "text after it changes nothing while it waits or shows, but blocks sending and applying");
    const auto differs = [&requested](auto change) {
        auto changed = requested;
        change(changed);
        return !sameSelection(changed, requested, false);
    };
    check(differs([](SelectionWindow &window) { window.selected = "i has gone to the shop"; }) &&
              differs([](SelectionWindow &window) { window.before = "Today "; }) &&
              differs([](SelectionWindow &window) { window.anchor = 11; }) &&
              differs([](SelectionWindow &window) { window.head = 31; }),
          "its text, the text before it and its offsets always count");
}

void synonymsTakeAWordOrShortPhraseAndCycle() {
    check(synonymSelection("beautiful") && synonymSelection("  big  ") && synonymSelection("a lot of"),
          "a word or short phrase takes a synonym");
    check(!synonymSelection("one two three four") && !synonymSelection("word.") &&
              !synonymSelection("2024") && !synonymSelection("") && !synonymSelection("a@b.example"),
          "a clause, a marked word, a number and an address do not");
    check(nextRevisionMode("grammar", "beautiful") == "refine", "grammar is refined next");
    check(nextRevisionMode("refine", "beautiful") == "synonyms", "a short refined selection takes a synonym");
    check(nextRevisionMode("refine", "i has went to the shop") == "grammar",
          "a long refined selection is checked again");
    check(nextRevisionMode("synonyms", "beautiful") == "synonyms", "a synonym is followed by another");
    auto state = focused();
    const auto window = captureSelectionWindow("The view from the hill is beautiful today", 35, 26, true, "en");
    check(window.has_value(), "the synonym selection is captured");
    const auto update = state.requestRevision(*window, "synonyms");
    check(update && update->mode == "synonyms", "a synonyms request");
    const auto body = serializeRevisionEnvelope(*update, 7);
    check(body && Json::parse(*body)["payload"]["mode"] == "synonyms" &&
              Json::parse(*body)["payload"]["selected"] == "beautiful",
          "the synonyms request carries exactly the word");
}

Json selectedFocus(std::string_view app = "org.gnome.texteditor", int selections = 1) {
    return Json{{"binding", {{"bus", ":1.7"}, {"path", "/field/1"}, {"app_id", app}, {"epoch", 3}}},
                {"target", {{"kind", "desktop_application"}, {"app_id", app}, {"target_id", "field-1"}}},
                {"purpose", "plain_text"},
                {"selection_count", selections},
                {"caret", 32},
                {"total_chars", 43}};
}

void theObserverBindsExactlyTheSelectedField() {
    check(revisionEditTarget(selectedFocus(), "org.gnome.texteditor") == NativeEditTarget::DesktopApplication,
          "a field of this app holding one selection binds like a caret would");
    auto browser = selectedFocus("chromium");
    browser["target"] = Json{{"kind", "browser"}, {"app_id", "chromium"}, {"origin", "https://example.test"},
                             {"target_id", "field-1"}};
    check(revisionEditTarget(browser, "chromium") == NativeEditTarget::BrowserOrigin, "a browser field by origin");
    check(!revisionEditTarget(selectedFocus("org.gnome.texteditor", 0), "org.gnome.texteditor") &&
              !revisionEditTarget(selectedFocus("org.gnome.texteditor", 2), "org.gnome.texteditor"),
          "only exactly one selection");
    check(!revisionEditTarget(selectedFocus("other.app"), "org.gnome.texteditor"), "another app's field");
    auto password = selectedFocus();
    password["purpose"] = "password";
    auto noCaret = selectedFocus();
    noCaret.erase("caret");
    auto unnamed = selectedFocus();
    unnamed["target"].erase("target_id");
    check(!revisionEditTarget(password, "org.gnome.texteditor") && !revisionEditTarget(noCaret, "org.gnome.texteditor") &&
              !revisionEditTarget(unnamed, "org.gnome.texteditor") && !revisionEditTarget(Json(), "org.gnome.texteditor"),
          "a password, a field without a caret or a target id, and no focus never bind");

    const auto captured = selectedFocus();
    check(sameSelectedField(captured, selectedFocus()), "the same binding and target, still one selection");
    auto otherBinding = selectedFocus();
    otherBinding["binding"]["epoch"] = 4;
    auto otherTarget = selectedFocus();
    otherTarget["target"]["target_id"] = "field-2";
    check(!sameSelectedField(captured, otherBinding) && !sameSelectedField(captured, otherTarget) &&
              !sameSelectedField(captured, selectedFocus("org.gnome.texteditor", 0)) &&
              !sameSelectedField(captured, password) && !sameSelectedField(Json(), captured),
          "another observation epoch or target, a collapsed selection or a password is not the field");

    const auto window = selection();
    const auto observed = [](std::string before, std::string selected, std::string after) {
        return Json{{"before", std::move(before)}, {"selected", std::move(selected)}, {"after", std::move(after)}};
    };
    check(observerAgreesSelection(observed("Yesterday ", "i has went to the shop", " with Anna."), window),
          "the same selection and surroundings agree");
    check(observerAgreesSelection(observed("Before that. Yesterday ", "i has went to the shop", " with Anna. Then"),
                                  window),
          "Fcitx may hold less of the field than the observer");
    auto paragraphEnd = window;
    paragraphEnd.after = " with Anna.\n\n";
    check(observerAgreesSelection(observed("Yesterday ", "i has went to the shop", " with Anna."), paragraphEnd),
          "Chromium's paragraph end after Fcitx's text agrees");
    check(!observerAgreesSelection(observed("Yesterday ", "i has gone to the shop", " with Anna."), window) &&
              !observerAgreesSelection(observed("Today ", "i has went to the shop", " with Anna."), window) &&
              !observerAgreesSelection(observed("Yesterday ", "i has went to the shop", " with Ann"), window),
          "another selection, other text before it or text cut from after it disagree");
    auto scoped = observed("Yesterday ", "i has went to the shop", " with Anna.");
    scoped["scope"] = "block";
    auto missing = observed("Yesterday ", "i has went to the shop", " with Anna.");
    missing.erase("after");
    check(!observerAgreesSelection(scoped, window) && !observerAgreesSelection(missing, window) &&
              !observerAgreesSelection(Json(), window),
          "a block-scoped, incomplete or absent snapshot never agrees");
}

} // namespace

int main() {
    try {
        capturesOneLineSelectionWithItsSurroundings();
        revisionTextIsOneVisibleLine();
        theDiffMarksRemovedThenAddedWords();
        theDiffWrapsAtWordsAndSaysWhenItIsCut();
        aRevisionLandsOnlyWhereTheSelectionWas();
        noticesNameWhatTheUserCanDo();
        aRevisionIsShownAndAppliedOnceOverItsSelection();
        anyFieldChangeOrExpiryRetiresTheRevision();
        theRequestCarriesExactlyTheSelection();
        aSelectionIsTheSameOnlyAsFarAsItsUseNeeds();
        synonymsTakeAWordOrShortPhraseAndCycle();
        theObserverBindsExactlyTheSelectedField();
    } catch (const std::exception &error) {
        std::cerr << "not ok - " << error.what() << '\n';
        return 1;
    }
    std::cout << "ok - selection revision core\n";
    return 0;
}
