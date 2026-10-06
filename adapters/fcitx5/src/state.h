#pragma once

#include <fcitx-utils/capabilityflags.h>

#include <cstdint>
#include <optional>
#include <string>
#include <string_view>

namespace badi::fcitx5 {

enum class NativeEditTarget { DesktopApplication, BrowserOrigin, Unsupported };
// Manual is the unknown-identity explicit contract; Observed is a field bound
// by the accessibility observer's inspect result.
enum class NativeEditPath { Manual, Observed };

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
    // Other Gecko browsers, Chromium-family ids without an observer rule, and
    // apps whose own Badi integration owns editing.
    Unavailable,
};

NativeAppClass classifyNativeApp(std::string_view appId);
bool imeParityApp(std::string_view appId);
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
};

struct Suggestion {
    Coordinates coordinates;
    std::string requestId;
    std::string suggestionId;
    std::string text;
    std::uint64_t expiresAtMs = 0;
    // The broker's next-word part of `text`; the whole text when absent.
    std::string acceptWord = {};
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

struct CommitPrepare {
    Coordinates coordinates;
    std::string controlId;
    std::string suggestionId;
    std::string text;
    std::string acceptance;
};

struct CommitDispatch {
    Coordinates coordinates;
    std::string controlId;
    std::string suggestionId;
    std::string text;
};

struct PanelObservation {
    bool preedit = false;
    bool clientPreedit = false;
    bool candidates = false;
    bool candidatesOwnedByBadi = false;
    bool foreignAuxiliary = false;
};

enum class LocalAction { PassThrough, Invoke, Accept, Dismiss };

// A key before the input method sees it.
struct PreKey {
    bool modifier = false;
    bool repeat = false;
    bool tab = false;    // plain Tab that no earlier handler took
    bool escape = false;
    bool chord = false;  // Badi's invoke or accept chord, handled after the input method
    bool word = false;   // Ctrl+Right that no earlier handler took
    // The key types the next characters of Badi's visible suggestion.
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
    CloseNotice,      // Escape closes Badi's notice
    Dismiss,          // Escape dismisses Badi's candidate
    AcceptWord,       // Ctrl+Right accepts the candidate's next word
};

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
inline constexpr std::string_view kModelNotConnected = "Badi model is not connected";
inline constexpr std::string_view kThinking = "Badi is thinking…";
inline constexpr std::string_view kRequestFailed = "Badi request failed — check the tray status";
inline constexpr std::string_view kSuggestionKeys = "Badi · Tab to accept · Ctrl+→ next word · Escape to dismiss";
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

    void open(const Coordinates &session) {
        coordinates = session;
        coordinates.revision = 0;
        coordinates.fingerprint.clear();
        opened = true;
    }
    void answer(bool allowed) {
        policyKnown = true;
        policyAllowed = allowed;
    }
    void retire() {
        opened = false;
        policyKnown = false;
        policyAllowed = false;
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

bool hasForeignImeUi(const PanelObservation &panel);
bool hasOwnedCandidate(const PanelObservation &panel);
bool allowsNativeContext(::fcitx::CapabilityFlags capabilities);
bool supportedWritingLanguage(std::string_view language);
bool matchesCapturedContext(
    const std::optional<ContextUpdate> &captured,
    const std::optional<ContextWindow> &current);
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
// The bounded window around a collapsed caret. The caller has already denied
// sensitive, special-purpose and composing contexts.
std::optional<ContextWindow> captureContextWindow(std::string_view text,
                                                  std::size_t cursor,
                                                  std::size_t anchor,
                                                  bool multiline,
                                                  std::string language);

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

    [[nodiscard]] bool focused() const { return focused_; }
    [[nodiscard]] bool editingAvailable() const {
        return nativeEditingAvailable(appId_, editTarget_, editPath_);
    }
    [[nodiscard]] NativeEditPath editPath() const { return editPath_; }
    [[nodiscard]] bool suggestionVisible() const { return visible_.has_value(); }
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
};

} // namespace badi::fcitx5
