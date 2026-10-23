#pragma once

#include "revision.h"

#include <fcitx-utils/capabilityflags.h>
#include <fcitx-utils/key.h>

#include <cstdint>
#include <optional>
#include <string>
#include <string_view>

namespace badi::fcitx5 {

enum class NativeEditTarget { DesktopApplication, BrowserOrigin, Unsupported };
// Manual is the unknown-identity explicit contract; Observed is a field bound
// by the accessibility observer's inspect result; TypedLine is a terminal's
// input context, whose only context is the line typed into it (ADR 0009).
enum class NativeEditPath { Manual, Observed, TypedLine };

// Every native decision starts from the class of the canonical app id.
enum class NativeAppClass {
    // User-granted native apps: the manual unknown-identity contract or an
    // observed desktop field.
    NativeExact,
    // Chromium-based apps and Zen (Gecko) without an editor-owned channel. Only
    // an observed field of the matching target kind may accept, append-only,
    // like typing.
    ImeParityBrowser,
    ImeParityDesktop,
    // Terminals: no surrounding text or accessible text, so the typed line is
    // the context and acceptance types the text (ADR 0009).
    Terminal,
    // Other Gecko browsers, Chromium-family ids without an observer rule, and
    // apps whose own Badi integration owns editing.
    Unavailable,
};

NativeAppClass classifyNativeApp(std::string_view appId);
bool imeParityApp(std::string_view appId);
// IME-parity apps whose typos Badi corrects in place once the observer has
// confirmed the exact field (ADR 0011): all but LibreOffice, whose Writer
// extension owns its corrections.
bool correctsAfterChecks(std::string_view appId);
// Whether the app's own text before the caret now ends as a correction left
// it, `expected` being that text: its last 64 bytes at most, since the
// surrounding window may slide.
bool correctionLanded(std::string_view before, std::string_view expected);
// UTF-8 scalar count of `suffix` when a native field's text before the caret
// ends with that exact suffix; none for an append-only (`imeParity`) app.
std::optional<unsigned> nativeReplacementChars(bool imeParity, std::string_view before,
                                                std::string_view suffix);
// Whether this app may be inspected by the observer at all.
bool nativeObservationAvailable(std::string_view appId);
bool nativeEditingAvailable(std::string_view appId, NativeEditTarget target,
                            NativeEditPath path);
// Content-free debug reasons. IME-parity apps never fall back to the
// unknown-identity manual path, so their reasons name the missing field.
std::string_view focusReason(std::string_view appId);
std::string_view editingUnavailableReason(std::string_view appId, bool fieldObserved);
// The notice for a broker clear without a suggestion, if the user should see one.
std::optional<std::string_view> clearNoticeText(std::string_view reason);

struct Coordinates {
    std::string sessionId;
    std::uint64_t focusEpoch = 0;
    std::uint64_t revision = 0;
    std::string fingerprint;

    bool operator==(const Coordinates &) const = default;
};

struct ContextWindow {
    std::string before;
    std::string after;
    std::uint64_t anchor = 0;
    std::uint64_t head = 0;
    std::string language;
    bool multiline = false;
    bool identityKnown = false;
    bool explicitRequest = true;
    // Fcitx reported exactly Chromium's block end after the caret ("\n\n" for a
    // <p>, "\n" for a <div> line), normalized away from `after` (see
    // normalizeObservedParagraphEnd); empty otherwise.
    std::string paragraphEnd = {};

    bool operator==(const ContextWindow &other) const {
        // Invocation mode is request metadata, not an edit to the field.
        return before == other.before && after == other.after && anchor == other.anchor &&
            head == other.head && language == other.language && multiline == other.multiline &&
            identityKnown == other.identityKnown && paragraphEnd == other.paragraphEnd;
    }
};

// Chromium's text-input surrounding text ends every <p> with "\n\n" and every
// <div> line with "\n" when anything is rendered after the editor, so the
// caret at the end of a rich editor's last block reports after == "\n\n" or
// "\n". For an observed IME-parity field that exact suffix counts as end of
// field: `after` becomes empty for eligibility and the broker, and
// `paragraphEnd` keeps the raw text. The caller normalizes only observed
// IME-parity contexts; each observer agreement check compares observedAfter(),
// so the observer must report the same end. Any other suffix is unchanged.
void normalizeObservedParagraphEnd(ContextWindow &context);
// The after-caret text exactly as Fcitx reported it.
std::string observedAfter(const ContextWindow &context);

struct ContextUpdate {
    Coordinates coordinates;
    ContextWindow context;
    std::string appId;
    std::string targetId;
    // The observed field's window text while the broker says context_words;
    // empty otherwise and on every unobserved path.
    std::string windowText = {};
};

struct Suggestion {
    Coordinates coordinates;
    std::string requestId;
    std::string suggestionId;
    std::string text;
    std::uint64_t expiresAtMs = 0;
    // The broker's next-word part of `text`; the whole text when absent.
    std::string acceptWord = {};
    // Exact typed suffix a native field must delete before committing `text`.
    std::optional<std::string> replaceBefore = std::nullopt;
};

struct AcceptRequest {
    Coordinates coordinates;
    std::string controlId;
    std::string suggestionId;
    std::string expectedText;
    // "all" or "word", as the broker's commit grant must repeat it.
    std::string acceptance = "all";
};

struct DismissRequest {
    Coordinates coordinates;
    std::string controlId;
    std::string suggestionId;
};

// A selection's scalar anchor and head, as a revision's grant names them.
struct SelectionRange {
    std::uint64_t anchor = 0;
    std::uint64_t head = 0;

    bool operator==(const SelectionRange &) const = default;
};

struct CommitPrepare {
    Coordinates coordinates;
    std::string controlId;
    std::string suggestionId;
    std::string text;
    std::string acceptance;
    std::optional<std::string> replaceBefore = std::nullopt;
    // A revision's grant: `text` replaces exactly this selection (ADR 0012).
    std::optional<SelectionRange> replaceSelection = std::nullopt;
};

struct CommitDispatch {
    Coordinates coordinates;
    std::string controlId;
    std::string suggestionId;
    std::string text;
    std::optional<std::string> replaceBefore = std::nullopt;
    std::optional<SelectionRange> replaceSelection = std::nullopt;
};

// An explicit revision request for one selection (ADR 0012). It is a field
// revision of its own, whose fingerprint covers the selection.
struct RevisionUpdate {
    Coordinates coordinates;
    std::string requestId;
    std::string mode; // "grammar" or "refine"
    SelectionWindow selection;
    std::string appId;
    std::string targetId;
};

// The broker's revision of that selection, until applied or retired.
struct Revision {
    Coordinates coordinates;
    std::string requestId;
    std::string revisionId;
    std::string mode;
    std::string original;
    std::string text;
    std::uint64_t expiresAtMs = 0;
};

struct RevisionClear {
    Coordinates coordinates;
    std::optional<std::string> revisionId;
    std::string reason;
};

struct PanelObservation {
    bool preedit = false;
    bool clientPreedit = false;
    bool candidates = false;
    bool candidatesOwnedByBadi = false;
    bool foreignAuxiliary = false;
};

// Tab accepts one word; Ctrl+Right and the accept chord accept all of it.
// Every surface uses this order (the broker accepts a spelling replacement
// whole either way, because its next word is the whole correction).
enum class LocalAction { PassThrough, Invoke, AcceptAll, AcceptWord, Dismiss };

// A key before the input method sees it.
struct PreKey {
    bool modifier = false;
    bool repeat = false;
    bool tab = false;    // plain Tab that no earlier handler took
    bool escape = false;
    bool chord = false;  // Badi's invoke or accept chord, handled after the input method
    bool all = false;    // Ctrl+Right that no earlier handler took
    // Ctrl+Shift+Tab that no earlier handler took: a real Tab for the app.
    bool literalTab = false;
    // A plain Tab would not reach the app even without a visible suggestion:
    // one is held for a returning remainder, or the manual path requests.
    bool tabTaken = false;
    // The key types the next characters of Badi's suggestion, visible or
    // hidden by an earlier typed-through key.
    bool typesSuggestion = false;
};

enum class PreKeyAction {
    PassThrough,
    Cancel,           // the key may edit the field: retire Badi's context and candidate
    CancelDeclining,  // Escape without Badi's candidate: also never re-suggest this context
    // Cancel, for a key that types the suggestion's next characters: the
    // broker may carry the remainder (ADR 0004), so inspect again sooner.
    CancelTypingThrough,
    Tab,              // decideTabAction() owns plain Tab
    CloseNotice,      // Escape closes Badi's notice and still reaches the application
    Dismiss,          // Escape dismisses Badi's candidate
    AcceptAll,        // Ctrl+Right accepts the whole candidate
    LiteralTab,       // Ctrl+Shift+Tab: retire the candidate and send the app a real Tab
};

// The action's name in the debug trace.
constexpr std::string_view preKeyActionName(PreKeyAction action) {
    switch (action) {
    case PreKeyAction::PassThrough: return "pass_through";
    case PreKeyAction::Cancel: return "cancel";
    case PreKeyAction::CancelDeclining: return "cancel_declining";
    case PreKeyAction::CancelTypingThrough: return "type_through";
    case PreKeyAction::Tab: return "tab";
    case PreKeyAction::CloseNotice: return "close_notice";
    case PreKeyAction::Dismiss: return "dismiss";
    case PreKeyAction::AcceptAll: return "accept_all";
    case PreKeyAction::LiteralTab: return "literal_tab";
    }
    return "unknown";
}

// What Badi shows in Fcitx's auxiliary text.
namespace notice {
inline constexpr std::string_view kUnavailableApp = "Badi cannot safely insert suggestions in this app yet";
inline constexpr std::string_view kFieldUnreadable = "Badi cannot read this text field — run badi debug status";
inline constexpr std::string_view kObserverUnavailable = "Badi cannot see this text field — check badi doctor";
inline constexpr std::string_view kNeedsFreshContext = "Badi needs fresh context — type, then invoke again";
inline constexpr std::string_view kReconnecting = "Badi is reconnecting — check badi doctor if this persists";
inline constexpr std::string_view kCheckingPermission = "Badi is checking this application's permission";
inline constexpr std::string_view kPaused = "Badi paused — resume from the tray";
inline constexpr std::string_view kDisabled = "Badi is disabled for this application";
inline constexpr std::string_view kSnoozed = "Badi is snoozed here — badi resume ends it";
inline constexpr std::string_view kModelNotConnected = "Badi model is not connected";
inline constexpr std::string_view kThinking = "Badi is thinking…";
inline constexpr std::string_view kLineUnknown = "Badi lost track of this line — press Enter or Ctrl+U";
inline constexpr std::string_view kRequestFailed = "Badi request failed — check the tray status";
inline constexpr std::string_view kCorrectionUnconfirmed = "Badi could not confirm that correction — check the text";
inline constexpr std::string_view kRevisionSelection = "Badi revises one line of up to 600 characters — select again";
inline constexpr std::string_view kRevisionUnconfirmed = "Badi could not confirm that revision — check the text";
} // namespace notice

// How an explicit request (the invoke chord, or Tab on the manual path) proceeds.
enum class InvokeRoute {
    InspectField,   // IME-parity without an observed field: ask the observer first
    Unavailable,    // no edit path: explain, read nothing
    ObservedField,  // re-inspect the observed field, which then requests
    Manual,         // the unknown-identity manual contract
};

InvokeRoute routeInvoke(std::string_view appId, bool editingAvailable, bool fieldObserved);

// Fcitx and broker state of one focused field, as a request or Tab sees it.
struct RequestFacts {
    bool connected = false;
    bool paused = false;
    bool policyKnown = false;
    bool policyAllowed = false;
    bool snoozed = false;       // the denial is the user's snooze of this app or site
    bool fresh = false;         // surrounding text arrived since focus or changed authority
    bool fieldAllowed = false;  // allowsNativeContext()
    bool foreignIme = false;
};

// The notice for a manual request that has to wait; nullopt when the field may be read.
std::optional<std::string_view> manualRequestNotice(const RequestFacts &facts);
// The notice for an IME-parity inspection that cannot start; nullopt when it may.
std::optional<std::string_view> inspectionNotice(const RequestFacts &facts);
// Content-free debug reason for a Tab press. `context` is Fcitx's, absent
// while a foreign input method owns the panel.
std::string_view tabDecisionReason(const RequestFacts &facts,
                                   const std::optional<ContextWindow> &context);

// The broker's view of one binding: a session opened at `coordinates` and the
// policy answer for its target. Focus, a new field, a new authority epoch and
// a closed connection retire both.
struct BrokerSession {
    Coordinates coordinates;
    bool opened = false;
    bool policyKnown = false;
    bool policyAllowed = false;
    std::string languageOverride;
    // The broker turned context on for this target (ADR 0010).
    bool contextWords = false;
    // The denial is a snooze of this app or site, not a setting.
    bool snoozed = false;

    void open(const Coordinates &session) {
        coordinates = session;
        coordinates.revision = 0;
        coordinates.fingerprint.clear();
        opened = true;
    }
    void answer(bool allowed, std::string language = {}, bool words = false, bool snoozedHere = false) {
        policyKnown = true;
        policyAllowed = allowed;
        languageOverride = allowed ? std::move(language) : std::string{};
        contextWords = allowed && words;
        snoozed = !allowed && snoozedHere;
    }
    void retire() {
        opened = false;
        policyKnown = false;
        policyAllowed = false;
        languageOverride.clear();
        contextWords = false;
        snoozed = false;
    }
};

class SurroundingFreshness {
public:
    void focusIn() { fresh_ = false; }
    void focusOut() { fresh_ = false; }
    void capabilityChanged() { fresh_ = false; }
    // Losing the broker transport retires broker grants, not Fcitx's current
    // surrounding text. Explicit requests may read it again after reconnect;
    // automatic requests wait for a new surrounding-text event.
    void transportLost() { automatic_ = false; }
    void surroundingTextUpdated() {
        fresh_ = true;
        automatic_ = true;
    }
    [[nodiscard]] bool fresh() const { return fresh_; }
    [[nodiscard]] bool freshForAutomatic() const { return fresh_ && automatic_; }

private:
    bool fresh_ = false;
    bool automatic_ = false;
};

// fcitx5-qt 5.1.16 (fcitx/fcitx5-qt@bac684ce07d9) withdraws SurroundingText
// after every input-method update that does not query it, and fcitx5-qt
// sends one itself (for the cursor rectangle) after every key, right after
// the update that sent the text. Fcitx keeps that text. This recognises exactly that echo: SurroundingText alone
// withdrawn after text arrived under it, and SurroundingText alone granted
// back while withdrawn. Any other change ends it. The addon honours it only
// for an observed field, whose observer must still agree with the text and
// caret before display and dispatch. fcitx5-qt applies Fcitx's delete
// request to its own copy of the text, which the echo clears, so a
// replacement is neither shown nor dispatched while withdrawn.
class SurroundingEcho {
public:
    // True when the change is the echo or its undo, so the field keeps its
    // freshness and observation.
    bool capabilityChanged(::fcitx::CapabilityFlags before, ::fcitx::CapabilityFlags after);
    void surroundingTextUpdated(::fcitx::CapabilityFlags current) {
        if (current.test(::fcitx::CapabilityFlag::SurroundingText)) textUnderGrant_ = true;
    }
    void reset() {
        textUnderGrant_ = false;
        withdrawn_ = false;
    }
    [[nodiscard]] bool withdrawn() const { return withdrawn_; }

private:
    bool textUnderGrant_ = false;
    bool withdrawn_ = false;
};

// The flags an observed field is judged by: Fcitx's, with SurroundingText
// while only the echo above withdrew it.
::fcitx::CapabilityFlags observedFieldCapabilities(::fcitx::CapabilityFlags current,
                                                   const SurroundingEcho &echo);

bool hasForeignImeUi(const PanelObservation &panel);
bool hasOwnedCandidate(const PanelObservation &panel);
bool allowsNativeContext(::fcitx::CapabilityFlags capabilities);
bool supportedWritingLanguage(std::string_view language);
bool matchesCapturedContext(
    const std::optional<ContextUpdate> &captured,
    const std::optional<ContextWindow> &current);
// Ctrl+Shift+Space requests only in a granted field Badi can edit. Otherwise
// the chord reaches the application; `badi debug` keeps the reason.
bool claimInvokeChord(bool editingAvailable, bool policyAllowed, bool foreignIme);

LocalAction decideLocalAction(bool invokeChord, bool acceptChord,
                              bool escapeKey, bool hasLiveOwnedCandidate,
                              const PanelObservation &panel);
LocalAction decideTabAction(bool eligibleContext, bool hasLiveOwnedCandidate,
                            const PanelObservation &panel, NativeEditPath path);
// Only Tab, Escape and Ctrl+Right on Badi's own UI are consumed before the
// input method.
PreKeyAction decidePreKey(const PreKey &key, bool editingAvailable, bool noticeShown,
                          bool suggestionVisible, const PanelObservation &panel);
bool tabEligibleContext(const std::optional<ContextWindow> &context);
// `typed` (a key's text) is a non-empty proper prefix of `suggestion`.
bool typesSuggestionStart(std::string_view typed, std::string_view suggestion);
// A key that types text, not a control key (Tab and Escape "type" one too).
bool printableKey(std::string_view typed);
// What a key adds to a `badi debug on --text` trace: a printable key's own
// text, only from a field Badi may edit for an allowed app (ADR 0008). A
// password or denied field, or an app or terminal the user has not allowed,
// adds nothing; a sensitive field's keys are not traced at all.
std::optional<std::string> tracedKeyText(std::string_view typed, bool fieldReadable);
// A Tab pressed while the type-through remainder of a word acceptance is
// still on its way back may be held for it, instead of reaching the app.
// Only a remainder that returns on its own (an observed field, a terminal's
// typed line) may hold one: on the manual path that same Tab is the request
// that brings the remainder, so holding it would wait for nothing. At most
// one Tab is held per acceptance: when the hold times out, or a broker
// clear answers the request for the remainder, the addon ends the
// remainder's automatic return (`restAutomatic` false) and the next Tab
// reaches the application.
bool tabMayHoldForRest(bool restPending, bool restAutomatic, bool foreignIme,
                       bool suggestionVisible);
// The displayed suggestion still begins with the word a held Tab meant:
// the broker's next word of the display is the pending remainder's start.
// A spelling replacement is never taken on a held Tab.
bool heldTabMatchesRest(std::string_view rest, const Suggestion &suggestion);

// Badi's literal-Tab chord. Omarchy keeps Alt+Tab for windows, and a lone
// Alt tap opens the menu bar in Zen, VS Code and Writer, so it is the
// Ctrl+Shift family of Badi's other chords.
inline const ::fcitx::Key &literalTabChord() {
    static const ::fcitx::Key key("Control+Shift+Tab");
    return key;
}

// A forwarded key carries the modifiers still held: Wayland's
// input-method-v2 frontend sends it through its virtual keyboard under the
// current modifier state, so a Tab sent at once would reach the app as
// Ctrl+Shift+Tab. A literal Tab waits for the release that leaves none of
// Ctrl, Alt, Shift and Super down.
bool lastModifierReleased(const ::fcitx::Key &release);
// The panel's two parts of a suggestion: the word Tab takes (`strong`) and
// the rest. The leading space is dropped, since the panel is its own box. A
// spelling replacement is taken whole: `teh → the`, the correction strong.
struct CandidateParts {
    std::string lead; // a correction's misspelled word and arrow
    std::string strong;
    std::string rest;
};
CandidateParts candidateParts(const Suggestion &suggestion);
// What the grey preview draws at the caret: a continuation itself, or a
// correction as `→ the`, since the typo it replaces is already in view.
// The Fcitx panel, its fallback, names the typo as well.
std::string previewText(const Suggestion &suggestion);
// The field text includes the characters typed through a suggestion whose
// context was `base`. A shorter window may keep only the last 128 bytes of
// `base`, matching the broker's type-through slide rule.
bool typeThroughContextReady(std::string_view base, std::string_view typed, std::string_view before);
// The bounded window around a collapsed caret. The caller has already denied
// sensitive, special-purpose and composing contexts.
std::optional<ContextWindow> captureContextWindow(std::string_view text,
                                                  std::size_t cursor,
                                                  std::size_t anchor,
                                                  bool multiline,
                                                  std::string language);

// The line typed into a terminal since its last boundary, held in memory
// only. Keys whose effect on the line is unknown (cursor movement, paste,
// completion) stop tracking until the next empty line.
// What a key the terminal received does to the line it edits.
enum class LineEdit {
    Type,          // printable text, inserted at the caret
    Erase,         // Backspace
    EraseWord,     // Ctrl+W, Ctrl/Alt+Backspace
    DeleteForward, // Delete, Ctrl+D
    KillAfter,     // Ctrl+K
    Left,          // Left, Ctrl+B
    Right,         // Right, Ctrl+F
    Home,          // Home, Ctrl+A
    End,           // End, Ctrl+E
    Clear,         // Enter, Ctrl+C, Ctrl+U: a known, empty line
    Restart,       // an invisible change that leaves the caret at the end
    Lose,          // the caret may be anywhere: stop following
};
// Readline's and most TUIs' line keys. A key outside this table changes
// the line invisibly (a paste, completion, a new prompt line), except word
// and vertical motion, which can put the caret anywhere.
LineEdit lineEditFor(const ::fcitx::Key &key, std::string_view typed);

class TypedLine {
public:
    // Applies one key; motion past either end of the line stops following,
    // since a multi-line prompt may move to another line there.
    void apply(LineEdit edit, std::string_view typed = {});
    void type(std::string_view text);
    // Backspace `count` scalars before the caret, as a correction does.
    void erase(std::size_t count = 1);
    void clear();
    void restart();
    void lose();
    [[nodiscard]] bool tracking() const { return tracking_; }
    [[nodiscard]] const std::string &text() const { return text_; }
    [[nodiscard]] std::string_view before() const { return std::string_view(text_).substr(0, caret_); }
    // Scalars before the caret.
    [[nodiscard]] std::size_t caretScalars() const;
    // At least two words with letters before the caret: a single token (a
    // command, a secret) never becomes a request on its own.
    [[nodiscard]] bool prose() const;

private:
    bool left();
    bool right();
    void eraseWord();
    std::string text_;
    std::size_t caret_ = 0; // byte offset, on a scalar boundary
    bool tracking_ = true;
};

class SessionState {
public:
    bool focusIn(std::string sessionId, std::string targetId,
                 std::string appId, std::string fingerprintSalt,
                 NativeEditTarget target = NativeEditTarget::DesktopApplication,
                 NativeEditPath path = NativeEditPath::Manual);
    void focusOut();
    void invalidateContext();
    void denyEditing();
    // The observer lost this field: context and candidates end, and only the
    // native exact class may continue through the manual path.
    void retireObservation();
    std::optional<ContextUpdate> updateContext(ContextWindow context);

    bool showSuggestion(Suggestion suggestion, std::uint64_t nowMs);
    // `word` accepts only the suggestion's next word.
    std::optional<AcceptRequest>
    requestAcceptance(std::uint64_t nowMs, const PanelObservation &panel, bool word = false);
    std::optional<DismissRequest>
    requestDismissal(std::uint64_t nowMs, const PanelObservation &panel);
    std::optional<CommitDispatch> authorizeCommit(const CommitPrepare &prepare,
                                                  std::uint64_t nowMs,
                                                  const PanelObservation &panel);
    bool clearSuggestionIf(const Coordinates &coordinates,
                           const std::optional<std::string> &suggestionId);
    void clearSuggestion();

    // Explicit revision of one selection (ADR 0012). A request supersedes
    // the caret's context and any suggestion; every later field change,
    // focus change or invalidation retires it with the revision shown.
    std::optional<RevisionUpdate> requestRevision(SelectionWindow selection, std::string mode);
    bool showRevision(Revision revision, std::uint64_t nowMs);
    std::optional<AcceptRequest> requestRevisionAcceptance(std::uint64_t nowMs, const PanelObservation &panel);
    std::optional<DismissRequest> requestRevisionDismissal(std::uint64_t nowMs, const PanelObservation &panel);
    std::optional<CommitDispatch> authorizeRevisionCommit(const CommitPrepare &prepare, std::uint64_t nowMs,
                                                          const PanelObservation &panel);
    // Retires the request at `coordinates` and its revision, unless a shown
    // revision has another id than `revisionId`.
    bool clearRevisionIf(const Coordinates &coordinates, const std::optional<std::string> &revisionId);
    void clearRevision();
    [[nodiscard]] const std::optional<RevisionUpdate> &revisionRequest() const { return revisionRequest_; }
    [[nodiscard]] const std::optional<Revision> &visibleRevision() const { return revision_; }
    [[nodiscard]] bool revisionAcceptancePending() const { return revisionAcceptance_.has_value(); }

    [[nodiscard]] bool focused() const { return focused_; }
    [[nodiscard]] bool editingAvailable() const {
        return nativeEditingAvailable(appId_, editTarget_, editPath_);
    }
    [[nodiscard]] NativeEditPath editPath() const { return editPath_; }
    [[nodiscard]] bool suggestionVisible() const { return visible_.has_value(); }
    // A Tab or Ctrl+→ already asked to accept this suggestion.
    [[nodiscard]] bool acceptancePending() const { return pendingAcceptance_.has_value(); }
    [[nodiscard]] std::string_view visibleText() const {
        return visible_ ? std::string_view(visible_->text) : std::string_view();
    }
    [[nodiscard]] const Coordinates &coordinates() const { return coordinates_; }
    [[nodiscard]] const std::string &appId() const { return appId_; }
    [[nodiscard]] const std::string &targetId() const { return targetId_; }
    [[nodiscard]] const std::optional<ContextUpdate> &lastContext() const {
        return lastContext_;
    }

private:
    std::string nextFingerprint(const ContextWindow &context) const;
    std::string fingerprintOf(std::string_view material) const;

    Coordinates coordinates_;
    std::string appId_;
    std::string targetId_;
    std::string fingerprintSalt_;
    NativeEditTarget editTarget_ = NativeEditTarget::Unsupported;
    NativeEditPath editPath_ = NativeEditPath::Manual;
    bool focused_ = false;
    std::optional<ContextUpdate> lastContext_;
    std::optional<Suggestion> visible_;
    std::optional<AcceptRequest> pendingAcceptance_;
    std::optional<RevisionUpdate> revisionRequest_;
    std::optional<Revision> revision_;
    std::optional<AcceptRequest> revisionAcceptance_;
};

} // namespace badi::fcitx5
