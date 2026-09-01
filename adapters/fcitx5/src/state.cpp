#include "state.h"

#include "sanitizer.h"

#include <algorithm>
#include <array>
#include <iomanip>
#include <limits>
#include <sstream>

namespace badi::fcitx5 {
namespace {

constexpr std::uint64_t kMaxSafeCounter = (std::uint64_t{1} << 53U) - 1U;

bool sameAddress(const Coordinates &left, const Coordinates &right) {
    return left == right;
}

std::uint64_t mix(std::string_view value, std::uint64_t seed) {
    auto hash = seed;
    for (const auto byte : value) {
        hash ^= static_cast<unsigned char>(byte);
        hash *= 0x100000001b3ULL;
    }
    return hash;
}

} // namespace

bool hasForeignImeUi(const PanelObservation &panel) {
    return panel.preedit || panel.clientPreedit ||
           (panel.candidates && !panel.candidatesOwnedByBadi);
}

bool allowsNativeContext(::fcitx::CapabilityFlags capabilities) {
    constexpr std::array denied{
        ::fcitx::CapabilityFlag::PasswordOrSensitive,
        ::fcitx::CapabilityFlag::Disable,
        ::fcitx::CapabilityFlag::Email,
        ::fcitx::CapabilityFlag::Digit,
        ::fcitx::CapabilityFlag::Url,
        ::fcitx::CapabilityFlag::Dialable,
        ::fcitx::CapabilityFlag::Number,
        ::fcitx::CapabilityFlag::Terminal,
        ::fcitx::CapabilityFlag::Date,
        ::fcitx::CapabilityFlag::Time,
    };
    return !!(capabilities & ::fcitx::CapabilityFlag::SurroundingText) &&
           std::none_of(denied.begin(), denied.end(),
                        [capabilities](const auto flag) {
                            return !!(capabilities & flag);
                        });
}

bool supportedAppId(std::string_view appId) {
    return appId == "omawrite" || appId == "com.github.xournalpp.xournalpp";
}

bool matchesCapturedContext(
    const std::optional<ContextUpdate> &captured,
    const std::optional<ContextWindow> &current) {
    return captured && current && captured->context == *current;
}

LocalAction decideLocalAction(bool invokeChord, bool acceptChord,
                              bool escapeKey, bool hasLiveOwnedCandidate,
                              const PanelObservation &panel) {
    if (hasForeignImeUi(panel)) return LocalAction::PassThrough;
    if (invokeChord) return LocalAction::Invoke;
    if (!hasLiveOwnedCandidate || !panel.candidatesOwnedByBadi) {
        return LocalAction::PassThrough;
    }
    if (acceptChord) return LocalAction::Accept;
    if (escapeKey) return LocalAction::Dismiss;
    return LocalAction::PassThrough;
}

std::optional<ContextWindow> captureContextWindow(std::string_view text,
                                                  std::size_t cursor,
                                                  std::size_t anchor,
                                                  bool sensitive,
                                                  bool multiline,
                                                  bool composing,
                                                  std::string language) {
    // Do not inspect or copy text that policy cannot serialize.
    if (sensitive || composing || cursor != anchor ||
        text.size() > kMaxContextSourceBytes ||
        !validLanguageTag(language)) {
        return std::nullopt;
    }
    const auto scalars = decodeUtf8(text);
    if (!scalars || cursor > scalars->size() || anchor > scalars->size()) {
        return std::nullopt;
    }
    ContextWindow result;
    result.language = std::move(language);
    result.multiline = multiline;

    const auto selectionStart = std::min(cursor, anchor);
    const auto selectionEnd = std::max(cursor, anchor);
    const auto beforeStart = selectionStart > kMaxBeforeScalars
                                 ? selectionStart - kMaxBeforeScalars
                                 : 0;
    const auto afterCount = std::min(kMaxAfterScalars,
                                     scalars->size() - selectionEnd);
    auto before = scalarSlice(text, beforeStart, selectionStart - beforeStart);
    auto after = scalarSlice(text, selectionEnd, afterCount);
    if (!before || !after) return std::nullopt;
    result.before = std::move(*before);
    result.after = std::move(*after);
    result.anchor = anchor;
    result.head = cursor;
    return result;
}

bool SessionState::focusIn(std::string sessionId, std::string targetId,
                           std::string appId, std::string fingerprintSalt) {
    if (!validOpaqueId(targetId) || !validLinuxAppId(appId) ||
        !supportedAppId(appId) || !validSessionId(sessionId) ||
        fingerprintSalt.size() < 16 || !validOpaqueId(fingerprintSalt)) {
        focusOut();
        return false;
    }
    coordinates_.sessionId = std::move(sessionId);
    targetId_ = std::move(targetId);
    appId_ = std::move(appId);
    fingerprintSalt_ = std::move(fingerprintSalt);
    coordinates_.focusEpoch =
        coordinates_.focusEpoch >= kMaxSafeCounter ? 1 : coordinates_.focusEpoch + 1;
    coordinates_.revision = 0;
    coordinates_.fingerprint.clear();
    focused_ = true;
    sensitive_ = false;
    lastContext_.reset();
    clearSuggestion();
    return true;
}

void SessionState::focusOut() {
    focused_ = false;
    sensitive_ = false;
    lastContext_.reset();
    clearSuggestion();
    coordinates_ = {};
    appId_.clear();
    targetId_.clear();
    fingerprintSalt_.clear();
}

void SessionState::invalidateContext() {
    if (!focused_) return;
    coordinates_.revision = coordinates_.revision >= kMaxSafeCounter
                                ? 1
                                : coordinates_.revision + 1;
    coordinates_.fingerprint.clear();
    lastContext_.reset();
    clearSuggestion();
}

std::optional<ContextUpdate>
SessionState::updateContext(ContextWindow context) {
    if (!focused_) return std::nullopt;
    if (context.sensitive || context.composing || context.anchor != context.head ||
        !validLanguageTag(context.language)) {
        invalidateContext();
        return std::nullopt;
    }
    coordinates_.revision = coordinates_.revision >= kMaxSafeCounter
                                ? 1
                                : coordinates_.revision + 1;
    coordinates_.fingerprint = nextFingerprint(context);
    sensitive_ = context.sensitive;
    clearSuggestion();
    ContextUpdate update{
        .coordinates = coordinates_,
        .context = std::move(context),
        .appId = appId_,
        .targetId = targetId_,
    };
    lastContext_ = update;
    return update;
}

bool SessionState::showSuggestion(Suggestion suggestion, std::uint64_t nowMs) {
    const auto clean = sanitizeSuggestion(suggestion.text);
    if (!focused_ || sensitive_ || !clean || suggestion.expiresAtMs <= nowMs ||
        suggestion.expiresAtMs > kMaxSafeCounter ||
        !validOpaqueId(suggestion.requestId) ||
        !validOpaqueId(suggestion.suggestionId) ||
        !sameAddress(suggestion.coordinates, coordinates_)) {
        return false;
    }
    suggestion.text = *clean;
    visible_ = std::move(suggestion);
    pendingAcceptance_.reset();
    return true;
}

std::optional<AcceptRequest>
SessionState::requestAcceptance(std::uint64_t nowMs,
                                const PanelObservation &panel) {
    if (!focused_ || sensitive_ || hasForeignImeUi(panel) ||
        pendingAcceptance_ || !visible_ || visible_->expiresAtMs <= nowMs ||
        !sameAddress(visible_->coordinates, coordinates_)) {
        if (visible_ && visible_->expiresAtMs <= nowMs) clearSuggestion();
        return std::nullopt;
    }
    AcceptRequest request{
        .coordinates = visible_->coordinates,
        .controlId = "fcitx.accept." +
                     std::to_string(visible_->coordinates.focusEpoch) + "." +
                     std::to_string(visible_->coordinates.revision),
        .suggestionId = visible_->suggestionId,
        .expectedText = visible_->text,
    };
    pendingAcceptance_ = request;
    return request;
}

std::optional<DismissRequest>
SessionState::requestDismissal(std::uint64_t nowMs,
                               const PanelObservation &panel) {
    if (!focused_ || sensitive_ || hasForeignImeUi(panel) || !visible_ ||
        visible_->expiresAtMs <= nowMs ||
        !sameAddress(visible_->coordinates, coordinates_)) {
        if (visible_ && visible_->expiresAtMs <= nowMs) clearSuggestion();
        return std::nullopt;
    }
    DismissRequest request{
        .coordinates = visible_->coordinates,
        .controlId = "fcitx.dismiss." +
                     std::to_string(visible_->coordinates.focusEpoch) + "." +
                     std::to_string(visible_->coordinates.revision),
        .suggestionId = visible_->suggestionId,
    };
    clearSuggestion();
    return request;
}

std::optional<CommitDispatch>
SessionState::authorizeCommit(const CommitPrepare &prepare,
                              std::uint64_t nowMs) {
    if (!focused_ || !visible_ || !pendingAcceptance_ ||
        visible_->expiresAtMs <= nowMs ||
        !sameAddress(prepare.coordinates, coordinates_) ||
        !sameAddress(prepare.coordinates, pendingAcceptance_->coordinates) ||
        prepare.controlId != pendingAcceptance_->controlId ||
        prepare.suggestionId != pendingAcceptance_->suggestionId ||
        prepare.text != pendingAcceptance_->expectedText ||
        prepare.acceptance != "all") {
        return std::nullopt;
    }
    CommitDispatch dispatch{
        .coordinates = prepare.coordinates,
        .controlId = prepare.controlId,
        .suggestionId = prepare.suggestionId,
        .text = prepare.text,
    };
    clearSuggestion();
    return dispatch;
}

bool SessionState::clearSuggestionIf(
    const Coordinates &coordinates,
    const std::optional<std::string> &suggestionId) {
    if (!visible_ || !sameAddress(coordinates, visible_->coordinates) ||
        (suggestionId && *suggestionId != visible_->suggestionId)) {
        return false;
    }
    clearSuggestion();
    return true;
}

void SessionState::clearSuggestion() {
    visible_.reset();
    pendingAcceptance_.reset();
}

std::string SessionState::nextFingerprint(const ContextWindow &context) const {
    const auto material = fingerprintSalt_ + "\x1f" + coordinates_.sessionId +
                          "\x1f" + appId_ + "\x1f" + targetId_ + "\x1f" +
                          context.before + "\x1f" + context.after + "\x1f" +
                          context.language + "\x1f" +
                          std::to_string(context.anchor) + ":" +
                          std::to_string(context.head) + ":" +
                          std::to_string(coordinates_.focusEpoch) + ":" +
                          std::to_string(coordinates_.revision);
    const std::array hashes{
        mix(material, mix(fingerprintSalt_, 0xcbf29ce484222325ULL)),
        mix(material, mix(fingerprintSalt_, 0x9e3779b97f4a7c15ULL)),
    };
    std::ostringstream output;
    output << std::hex << std::setfill('0');
    for (const auto hash : hashes) output << std::setw(16) << hash;
    return output.str();
}

} // namespace badi::fcitx5
