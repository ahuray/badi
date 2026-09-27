#pragma once

#include "state.h"

#include <nlohmann/json.hpp>

#include <optional>
#include <string_view>

namespace badi::fcitx5 {

// What the accessibility observer's replies (badi.accessibility.v1) mean for
// Badi's authority over one field. Missing or malformed data fails closed.

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

// The field itself was declined (password, purpose, selection), as opposed
// to an absent or blind observer.
bool observerDeniedField(const nlohmann::json &error);

std::string_view snapshotFailureReason(const nlohmann::json &error);

} // namespace badi::fcitx5
