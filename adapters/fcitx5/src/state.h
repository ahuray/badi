#pragma once

#include <fcitx-utils/capabilityflags.h>

#include <cstdint>
#include <optional>
#include <string>
#include <string_view>

namespace badi::fcitx5 {

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
    bool sensitive = false;
    bool multiline = false;
    bool composing = false;

    bool operator==(const ContextWindow &) const = default;
};

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
};

enum class LocalAction { PassThrough, Invoke, Accept, Dismiss };

class SurroundingFreshness {
public:
    void focusIn() { fresh_ = false; }
    void focusOut() { fresh_ = false; }
    void capabilityChanged() { fresh_ = false; }
    void surroundingTextUpdated() { fresh_ = true; }
    [[nodiscard]] bool fresh() const { return fresh_; }

private:
    bool fresh_ = false;
};

bool hasForeignImeUi(const PanelObservation &panel);
bool allowsNativeContext(::fcitx::CapabilityFlags capabilities);
bool supportedAppId(std::string_view appId);
bool matchesCapturedContext(
    const std::optional<ContextUpdate> &captured,
    const std::optional<ContextWindow> &current);
LocalAction decideLocalAction(bool invokeChord, bool acceptChord,
                              bool escapeKey, bool hasLiveOwnedCandidate,
                              const PanelObservation &panel);
std::optional<ContextWindow> captureContextWindow(std::string_view text,
                                                  std::size_t cursor,
                                                  std::size_t anchor,
                                                  bool sensitive,
                                                  bool multiline,
                                                  bool composing,
                                                  std::string language);

class SessionState {
public:
    bool focusIn(std::string sessionId, std::string targetId,
                 std::string appId, std::string fingerprintSalt);
    void focusOut();
    void invalidateContext();
    std::optional<ContextUpdate> updateContext(ContextWindow context);

    bool showSuggestion(Suggestion suggestion, std::uint64_t nowMs);
    std::optional<AcceptRequest>
    requestAcceptance(std::uint64_t nowMs, const PanelObservation &panel);
    std::optional<DismissRequest>
    requestDismissal(std::uint64_t nowMs, const PanelObservation &panel);
    std::optional<CommitDispatch> authorizeCommit(const CommitPrepare &prepare,
                                                  std::uint64_t nowMs);
    bool clearSuggestionIf(const Coordinates &coordinates,
                           const std::optional<std::string> &suggestionId);
    void clearSuggestion();

    [[nodiscard]] bool focused() const { return focused_; }
    [[nodiscard]] bool sensitive() const { return sensitive_; }
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
    bool focused_ = false;
    bool sensitive_ = false;
    std::optional<ContextUpdate> lastContext_;
    std::optional<Suggestion> visible_;
    std::optional<AcceptRequest> pendingAcceptance_;
};

} // namespace badi::fcitx5
