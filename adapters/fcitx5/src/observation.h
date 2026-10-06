#pragma once

#include "state.h"

#include <nlohmann/json.hpp>

#include <cstdint>
#include <optional>
#include <string_view>

namespace badi::fcitx5 {

// What the accessibility observer's replies (badi.accessibility.v1) mean for
// Badi's authority over one field. Missing or malformed data fails closed.

nlohmann::json inspectRequest(std::string_view appId);
// Re-reads the exact field an earlier reply bound (`focus`).
nlohmann::json snapshotRequest(const nlohmann::json &focus);
// The caret rectangle an app gave its input method, relative to its window
// in physical pixels (Qt's Fcitx module declares RelativeRect).
struct CaretRect {
    int x = 0;
    int y = 0;
    int width = 0;
    int height = 0;
    double scale = 1;
};

// Asks the observer to draw `text` at the caret it last verified for `focus`.
// `caret` is the observer's fallback where the field reports no glyph extents.
nlohmann::json previewRequest(const nlohmann::json &focus, std::string_view text, std::uint64_t ttlMs,
                              const std::optional<CaretRect> &caret = std::nullopt);

bool observerAnswered(const nlohmann::json &reply);

// Same binding, target and caret on a plain-text field without a selection. A
// cropped toolkit buffer can be identical at different absolute positions, so
// snapshots and previews (requireLength) must also agree on the field length.
bool matchesObservedFocus(const nlohmann::json &captured, const nlohmann::json &observed,
                          bool requireLength = false);

// Nullopt when the inspected field's metadata is malformed or names another
// app; prior authority is then retired rather than falling back.
std::optional<NativeEditTarget> inspectedEditTarget(const nlohmann::json &focus, std::string_view appId);

// The observer's text around the caret equals Fcitx's live context.
bool observerAgrees(const nlohmann::json &observed, const ContextWindow &context);

// A snapshot of the same field, caret and length whose text equals Fcitx's.
bool observerCorroborates(const nlohmann::json &captured, const nlohmann::json &observed,
                          const ContextWindow &context);

// Why an observed snapshot cannot become a request for this Fcitx context;
// nullopt when both agree on a nonempty prefix at the end of the text.
std::optional<std::string_view> observedRequestBlocked(const nlohmann::json &observed,
                                                       const ContextWindow &context, bool foreignIme);

// The field itself was declined (password, purpose, selection), as opposed
// to an absent or blind observer.
bool observerDeniedField(const nlohmann::json &error);

std::string_view snapshotFailureReason(const nlohmann::json &error);

// An invalidation that names no app concerns every app.
bool invalidates(const nlohmann::json &event, std::string_view appId);

// When the observer inspects the focused field again. Fcitx input (keys or
// changed surrounding text) inspects once typing pauses. An invalidation
// without input since the last inspection backs off (240, 480, 960 ms) and
// then waits for input, so a busy page cannot drive an inspection loop; a
// changed field always waits for input.
class InspectionSchedule {
public:
    static constexpr std::uint64_t kTypingPauseUs = 120'000;
    // After a key that typed the suggestion's next characters, or a word
    // acceptance, the broker can answer at once (ADR 0004). This short pause
    // lets the field's own accessibility events arrive first.
    static constexpr std::uint64_t kTypeThroughPauseUs = 30'000;
    static constexpr unsigned int kMaxIdleReinspections = 3;
    // The previous keystroke's snapshot or preview reply can still be pending
    // when typing pauses, or a previous field's after a focus switch. Its reply
    // takes milliseconds, so retry soon, within the observer's 500 ms deadline.
    static constexpr std::uint64_t kBusyRetryUs = 20'000;
    static constexpr unsigned int kMaxBusyRetries = 25;

    void input() {
        ++inputSerial_;
        idleInvalidations_ = 0;
    }
    void inspecting() { inspectedSerial_ = inputSerial_; }
    [[nodiscard]] bool reinspectAfterInvalidation(bool fieldChanged);
    [[nodiscard]] std::uint64_t delayUs(bool immediate, bool typedThrough = false, bool busyRetry = false) const {
        if (immediate) return 1;
        if (busyRetry) return kBusyRetryUs;
        return typedThrough ? kTypeThroughPauseUs : kTypingPauseUs << idleInvalidations_;
    }
    void resetBusyRetries() { busyRetries_ = 0; }
    [[nodiscard]] bool retryWhileBusy() { return busyRetries_++ < kMaxBusyRetries; }

private:
    std::uint64_t inputSerial_ = 0;
    std::uint64_t inspectedSerial_ = 0;
    unsigned int idleInvalidations_ = 0;
    unsigned int busyRetries_ = 0;
};

} // namespace badi::fcitx5
