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
    // Existing contract for user-granted native apps: manual unknown identity
    // or an observed desktop field.
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
    // Fcitx reported exactly Chromium's paragraph end after the caret,
    // normalized away from `after` (see normalizeObservedParagraphEnd).
    bool paragraphEndAfter = false;

    bool operator==(const ContextWindow &other) const {
        // Invocation mode is request metadata, not an edit to the field.
        return before == other.before && after == other.after && anchor == other.anchor &&
            head == other.head && language == other.language && multiline == other.multiline &&
            identityKnown == other.identityKnown && paragraphEndAfter == other.paragraphEndAfter;
    }
};

// Chromium's text-input surrounding text ends every <p> with "\n\n" when
// anything is rendered after the editor, so the caret at the end of a
// ProseMirror composer's last paragraph reports after == "\n\n". For an
// observed IME-parity field that exact suffix counts as end of field: `after`
// becomes empty for eligibility and the broker, and `paragraphEndAfter` keeps
// the raw text. The caller normalizes only observed IME-parity contexts; each
// observer agreement check compares observedAfter(). Any other suffix, a
// single "\n" included, is unchanged.
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
};

struct AcceptRequest {
    Coordinates coordinates;
    std::string controlId;
    std::string suggestionId;
    std::string expectedText;
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
bool tabEligibleContext(const std::optional<ContextWindow> &context);
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
    std::optional<AcceptRequest>
    requestAcceptance(std::uint64_t nowMs, const PanelObservation &panel);
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
