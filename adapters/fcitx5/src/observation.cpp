#include "observation.h"

#include "sanitizer.h"

#include <limits>
#include <string>
#include <utility>

namespace badi::fcitx5 {

using Json = nlohmann::json;

Json inspectRequest(std::string_view appId) {
    return Json{{"op", "inspect"}, {"app_id", appId}};
}

Json unnamedIbusInspectRequest(bool selection) {
    auto request = Json{{"op", "inspect"}, {"frontend", "ibus"}};
    if (selection) request["selection"] = true;
    return request;
}

Json snapshotRequest(const Json &focus, bool window) {
    auto request = Json{{"op", "snapshot"}, {"binding", focus.value("binding", Json())},
                        {"policy_target", focus.value("target", Json())}};
    if (window) request["window"] = true;
    return request;
}

std::string observedWindowText(const Json &observed) {
    const auto text = observed.value("window_text", Json());
    if (!text.is_string()) return {};
    const auto &value = text.get_ref<const std::string &>();
    const auto scalars = decodeUtf8(value);
    if (!scalars || scalars->empty() || scalars->size() > kMaxWindowTextScalars || !validContextText(value))
        return {};
    return value;
}

Json previewRequest(const Json &focus, std::string_view text, std::uint64_t ttlMs,
                    const std::optional<CaretRect> &caret, std::size_t strong) {
    auto request = snapshotRequest(focus);
    request["op"] = "preview";
    request["text"] = text;
    request["expected_caret"] = focus.value("caret", Json());
    request["expected_total_chars"] = focus.value("total_chars", Json());
    request["ttl_ms"] = ttlMs;
    if (strong != 0) request["strong"] = strong;
    if (caret) {
        request["caret_rect"] = Json{{"x", caret->x}, {"y", caret->y}, {"width", caret->width},
                                     {"height", caret->height}, {"scale", caret->scale}};
    }
    return request;
}

bool observerAnswered(const Json &reply) {
    return reply.value("ok", Json()) == true && reply.contains("focus");
}

Json revisionInspectRequest(std::string_view appId) {
    auto request = inspectRequest(appId);
    request["selection"] = true;
    return request;
}

Json selectionSnapshotRequest(const Json &focus) {
    auto request = snapshotRequest(focus);
    request["selection"] = true;
    return request;
}

std::optional<NativeEditTarget> revisionEditTarget(const Json &focus, std::string_view appId) {
    if (!focus.is_object() || focus.value("selection_count", Json()) != 1) return std::nullopt;
    // The same identity and target checks as a caret, on a copy that reads
    // as one; the selection itself is checked against Fcitx before use.
    auto caret = focus;
    caret["selection_count"] = 0;
    return inspectedEditTarget(caret, appId);
}

bool sameSelectedField(const Json &captured, const Json &observed) {
    return captured.is_object() && observed.is_object() &&
           observed.value("binding", Json()) == captured.value("binding", Json()) &&
           observed.value("target", Json()) == captured.value("target", Json()) &&
           observed.value("purpose", Json()) == "plain_text" && observed.value("selection_count", Json()) == 1;
}

bool observerAgreesSelection(const Json &observed, const SelectionWindow &window) {
    if (!observed.is_object()) return false;
    const auto before = observed.value("before", Json());
    const auto selected = observed.value("selected", Json());
    const auto after = observed.value("after", Json());
    if (!before.is_string() || !selected.is_string() || !after.is_string() ||
        observed.contains("scope") || selected != window.selected) {
        return false;
    }
    const auto &observedBefore = before.get_ref<const std::string &>();
    const auto &observedAfter = after.get_ref<const std::string &>();
    // Fcitx may hold less of the field than the observer, never other text;
    // Chromium ends a block with a paragraph end the field's text lacks.
    const bool afterAgrees = observedAfter.starts_with(window.after) || window.after == observedAfter + "\n" ||
                             window.after == observedAfter + "\n\n";
    return observedBefore.ends_with(window.before) && afterAgrees;
}

bool matchesObservedFocus(const Json &captured, const Json &observed, bool requireLength) {
    if (!captured.is_object() || !observed.is_object() ||
        observed.value("binding", Json()) != captured.value("binding", Json()) ||
        observed.value("target", Json()) != captured.value("target", Json()) ||
        !captured.value("caret", Json()).is_number_integer() ||
        !observed.value("caret", Json()).is_number_integer() ||
        observed["caret"] < 0 || observed["caret"] != captured["caret"] ||
        observed.value("purpose", Json()) != "plain_text" ||
        observed.value("selection_count", Json()) != 0) return false;
    if (!requireLength) return true;
    const auto total = observed.value("total_chars", Json());
    return total.is_number_integer() && total >= observed["caret"] &&
           (!captured.contains("total_chars") || captured["total_chars"] == total);
}

bool sameObservedField(const Json &captured, const Json &observed) {
    return captured.is_object() && observed.is_object() &&
           observed.value("binding", Json()) == captured.value("binding", Json()) &&
           observed.value("target", Json()) == captured.value("target", Json()) &&
           observed.value("purpose", Json()) == "plain_text" &&
           observed.value("selection_count", Json()) == 0;
}

bool sameObservedFieldIdentity(const Json &previous, const Json &current) {
    // Only for a denial surviving reacquisition. Display and dispatch still
    // require the complete, current binding including its observation epoch.
    if (!previous.is_object() || !current.is_object()) return false;
    auto oldBinding = previous.value("binding", Json());
    auto newBinding = current.value("binding", Json());
    auto oldTarget = previous.value("target", Json());
    auto newTarget = current.value("target", Json());
    if (!oldBinding.is_object() || !newBinding.is_object() ||
        !oldTarget.is_object() || !newTarget.is_object()) return false;
    oldBinding.erase("epoch");
    newBinding.erase("epoch");
    oldTarget.erase("target_id");
    newTarget.erase("target_id");
    return oldBinding == newBinding && oldTarget == newTarget;
}

bool continuedFieldSnapshot(const Json &previous, const Json &observed) {
    return sameObservedField(previous, observed) && matchesObservedFocus(observed, observed, true);
}

std::optional<NativeEditTarget> inspectedEditTarget(const Json &focus, std::string_view appId) {
    const std::string app(appId);
    if (!focus.is_object() || !focus.contains("binding") || !focus.contains("target") ||
        !focus["binding"].is_object() || !focus["target"].is_object() ||
        focus["binding"].value("app_id", Json()) != app ||
        focus.value("purpose", Json()) != "plain_text" ||
        focus.value("selection_count", Json()) != 0 ||
        !focus["target"].contains("target_id") || !focus["target"]["target_id"].is_string() ||
        !validOpaqueId(focus["target"]["target_id"].get<std::string>()) ||
        !matchesObservedFocus(focus, focus)) return std::nullopt;
    const auto &target = focus["target"];
    const auto kind = target.value("kind", Json());
    if (kind == "browser" && target.value("app_id", Json()) == app)
        return NativeEditTarget::BrowserOrigin;
    if (kind == "desktop_application" && target.value("app_id", Json()) == app)
        return NativeEditTarget::DesktopApplication;
    return std::nullopt;
}

std::optional<std::pair<std::string, NativeEditTarget>> unnamedIbusEditTarget(const Json &focus,
                                                                               bool selection) {
    if (!focus.is_object() || !focus.contains("binding") || !focus["binding"].is_object()) return std::nullopt;
    const auto app = focus["binding"].value("app_id", Json());
    if (!app.is_string()) return std::nullopt;
    const auto &appId = app.get_ref<const std::string &>();
    if (!validLinuxAppId(appId) || !unnamedIbusObservedApp(appId)) return std::nullopt;
    const auto editTarget = selection ? revisionEditTarget(focus, appId) : inspectedEditTarget(focus, appId);
    if (!editTarget || !nativeEditingAvailable(appId, *editTarget, NativeEditPath::Observed)) return std::nullopt;
    return std::make_pair(appId, *editTarget);
}

// Exactly Fcitx's text before the caret; or, when the observer could serialize
// only the caret's own block (a list, quote or table precedes it), that block's
// text ending Fcitx's text at a line start.
bool beforeAgrees(const Json &observed, const std::string &before) {
    const auto text = observed.value("before", Json());
    const auto scope = observed.value("scope", Json());
    if (!text.is_string() || !(scope.is_null() || scope == "block")) return false;
    const auto &block = text.get_ref<const std::string &>();
    // An empty block agrees with almost any line end. That is not corroboration.
    if (scope == "block" && block.empty()) return false;
    if (block == before) return true;
    return scope == "block" && before.size() > block.size() && before.ends_with(block) &&
           before[before.size() - block.size() - 1] == '\n';
}

void restoreObservedTrailingSpaces(ContextWindow &context, const Json &observed) {
    if (!observed.is_object() || observed.value("after", Json()) != "" || !context.after.empty() ||
        !context.paragraphEnd.empty()) return;
    const auto before = observed.value("before", Json());
    if (!before.is_string()) return;
    const auto &text = before.get_ref<const std::string &>();
    if (text.size() <= context.before.size() || !text.starts_with(context.before)) return;
    const auto spaces = text.size() - context.before.size();
    if (text.find_first_not_of(' ', context.before.size()) != std::string::npos) return;
    context.before = text;
    context.anchor += spaces;
    context.head += spaces;
}

// Exactly Fcitx's text after the caret; or, for a block-scoped snapshot, the
// rest of the caret's block and its block end beginning Fcitx's text, which
// goes on with the blocks below.
bool afterAgrees(const Json &observed, const std::string &after) {
    const auto text = observed.value("after", Json());
    if (text == after) return true;
    if (!text.is_string() || observed.value("scope", Json()) != "block") return false;
    const auto &block = text.get_ref<const std::string &>();
    // Like an empty block before the caret, an empty one after it proves nothing.
    return !block.empty() && after.size() > block.size() && after.starts_with(block);
}

bool observerAgrees(const Json &observed, const ContextWindow &context) {
    if (!observed.is_object() || !beforeAgrees(observed, context.before)) return false;
    const auto after = observed.value("after", Json());
    if (afterAgrees(observed, observedAfter(context))) return true;
    // Both name the end of the field when Fcitx has nothing after the caret and
    // the observer only its last block's end: an EditContext editor (CodeMirror)
    // sends its own text, and an editor last on its page gets no block end.
    return context.after.empty() && context.paragraphEnd.empty() && (after == "\n" || after == "\n\n");
}

bool observerCorroborates(const Json &captured, const Json &observed, const ContextWindow &context) {
    return matchesObservedFocus(captured, observed, true) && observerAgrees(observed, context);
}

bool zenPostCommitApp(std::string_view appId) {
    return appId == "zen" || appId == "zen-twilight";
}

void ZenPendingCommit::clear() {
    active_ = false;
    appId_.clear();
    field_ = nullptr;
    preCommit_ = ContextWindow{};
    steps_.clear();
    committedScalars_ = 0;
    expectedBefore_.clear();
}

bool ZenPendingCommit::sameField(const Json &focus) const {
    if (!active_ || !sameObservedFieldIdentity(field_, focus) ||
        focus.value("purpose", Json()) != "plain_text" ||
        focus.value("selection_count", Json()) != 0) {
        return false;
    }
    const auto baseEpoch = field_["binding"].value("epoch", Json());
    const auto focusEpoch = focus["binding"].value("epoch", Json());
    return baseEpoch.is_number_integer() && focusEpoch.is_number_integer() && focusEpoch >= baseEpoch;
}

bool ZenPendingCommit::matchesFcitx(const ContextWindow &rawFcitx) const {
    return active_ && rawFcitx.identityKnown &&
           rawFcitx.before == preCommit_.before &&
           rawFcitx.after == preCommit_.after &&
           rawFcitx.paragraphEnd == preCommit_.paragraphEnd &&
           rawFcitx.anchor == preCommit_.anchor &&
           rawFcitx.head == preCommit_.head &&
           rawFcitx.language == preCommit_.language;
}

void ZenPendingCommit::record(std::string_view appId, const Json &observedField,
                              const ContextWindow &rawFcitx, std::string_view committed) {
    if (!zenPostCommitApp(appId) || committed.empty() || !rawFcitx.identityKnown ||
        rawFcitx.anchor != rawFcitx.head ||
        !matchesObservedFocus(observedField, observedField, true)) {
        clear();
        return;
    }
    const auto addedScalars = decodeUtf8(committed);
    if (!addedScalars || addedScalars->empty() || !validContextText(committed)) {
        clear();
        return;
    }
    Json baselineField;
    ContextWindow baselineContext;
    std::vector<Step> nextSteps;
    std::size_t nextScalars = 0;
    if (active_ && appId_ == appId && matchesFcitx(rawFcitx) && sameField(observedField)) {
        auto corroborated = rawFcitx;
        if (!applyToCorroborated(appId, corroborated, observedField)) {
            clear();
            return;
        }
        baselineField = field_;
        baselineField["binding"]["epoch"] = observedField["binding"]["epoch"];
        baselineContext = preCommit_;
        nextSteps = steps_;
        nextSteps.push_back(Step{std::string(committed), addedScalars->size()});
        nextScalars = committedScalars_ + addedScalars->size();
    } else {
        if (!observerAgrees(observedField, rawFcitx)) {
            clear();
            return;
        }
        baselineField = observedField;
        baselineContext = rawFcitx;
        nextSteps.push_back(Step{std::string(committed), addedScalars->size()});
        nextScalars = addedScalars->size();
    }
    if (nextScalars > kMaxBeforeScalars) {
        clear();
        return;
    }
    const auto preScalars = decodeUtf8(baselineContext.before);
    if (!preScalars) {
        clear();
        return;
    }
    std::string combined = baselineContext.before;
    for (const auto &step : nextSteps) combined.append(step.text);
    const auto windowed = scalarWindow(combined, preScalars->size() + nextScalars, kMaxBeforeScalars, 0);
    if (!windowed || windowed->before.empty()) {
        clear();
        return;
    }
    active_ = true;
    appId_ = std::string(appId);
    field_ = std::move(baselineField);
    preCommit_ = std::move(baselineContext);
    steps_ = std::move(nextSteps);
    committedScalars_ = nextScalars;
    expectedBefore_ = std::string(windowed->before);
}

ZenFcitxUpdate ZenPendingCommit::updateFcitx(std::string_view appId,
                                             const ContextWindow &rawFcitx) {
    if (!active_ || !zenPostCommitApp(appId) || appId_ != appId) {
        clear();
        return ZenFcitxUpdate::Cleared;
    }
    if (matchesFcitx(rawFcitx)) return ZenFcitxUpdate::Unchanged;
    if (!rawFcitx.identityKnown || rawFcitx.anchor != rawFcitx.head ||
        preCommit_.anchor != preCommit_.head ||
        !rawFcitx.after.empty() || !rawFcitx.paragraphEnd.empty() ||
        !preCommit_.after.empty() || !preCommit_.paragraphEnd.empty() ||
        rawFcitx.language != preCommit_.language ||
        rawFcitx.multiline != preCommit_.multiline) {
        clear();
        return ZenFcitxUpdate::Cleared;
    }
    const auto preScalars = decodeUtf8(preCommit_.before);
    const auto currentScalars = decodeUtf8(rawFcitx.before);
    if (!preScalars || !currentScalars || preCommit_.head < preScalars->size()) {
        clear();
        return ZenFcitxUpdate::Cleared;
    }
    std::string prefixText;
    std::size_t prefixScalars = 0;
    for (std::size_t k = 1; k <= steps_.size(); ++k) {
        prefixText.append(steps_[k - 1].text);
        prefixScalars += steps_[k - 1].scalars;
        if (preCommit_.head > std::numeric_limits<std::size_t>::max() - prefixScalars) break;
        const auto expectedHead = preCommit_.head + prefixScalars;
        if (rawFcitx.head != expectedHead) continue;
        if (expectedHead <= kMaxBeforeScalars
                ? currentScalars->size() != expectedHead
                : currentScalars->size() != kMaxBeforeScalars) {
            break;
        }
        const std::string combined = preCommit_.before + prefixText;
        const auto windowed = scalarWindow(combined, preScalars->size() + prefixScalars,
                                           kMaxBeforeScalars, 0);
        if (!windowed || rawFcitx.before != windowed->before) break;
        if (k == steps_.size()) {
            clear();
            return ZenFcitxUpdate::Cleared;
        }
        if (!field_.is_object() || !field_.value("caret", Json()).is_number_integer() ||
            !field_.value("total_chars", Json()).is_number_integer()) {
            clear();
            return ZenFcitxUpdate::Cleared;
        }
        const auto baseCaret = field_["caret"].get<std::int64_t>();
        const auto baseTotal = field_["total_chars"].get<std::int64_t>();
        const auto delta = static_cast<std::int64_t>(prefixScalars);
        if (baseCaret < 0 || baseTotal < baseCaret ||
            baseCaret > std::numeric_limits<std::int64_t>::max() - delta ||
            baseTotal > std::numeric_limits<std::int64_t>::max() - delta) {
            clear();
            return ZenFcitxUpdate::Cleared;
        }
        steps_.erase(steps_.begin(), steps_.begin() + static_cast<std::ptrdiff_t>(k));
        committedScalars_ -= prefixScalars;
        field_["caret"] = baseCaret + delta;
        field_["total_chars"] = baseTotal + delta;
        if (field_.contains("before")) field_["before"] = rawFcitx.before;
        preCommit_.before = rawFcitx.before;
        preCommit_.anchor = rawFcitx.anchor;
        preCommit_.head = rawFcitx.head;
        std::string remainingCombined = preCommit_.before;
        for (const auto &step : steps_) remainingCombined.append(step.text);
        const auto nextWindow = scalarWindow(remainingCombined,
                                             currentScalars->size() + committedScalars_,
                                             kMaxBeforeScalars, 0);
        if (!nextWindow || nextWindow->before.empty()) {
            clear();
            return ZenFcitxUpdate::Cleared;
        }
        expectedBefore_ = std::string(nextWindow->before);
        return ZenFcitxUpdate::Rebased;
    }
    clear();
    return ZenFcitxUpdate::Cleared;
}

bool ZenPendingCommit::applyToCorroborated(std::string_view appId, ContextWindow &context,
                                           const Json &observed) const {
    if (!active_ || !zenPostCommitApp(appId) || appId_ != appId || !matchesFcitx(context) ||
        !sameField(observed) || !observed.value("scope", Json()).is_null() ||
        !observed.value("caret", Json()).is_number_integer() ||
        !observed.value("total_chars", Json()).is_number_integer()) {
        return false;
    }
    const auto baseCaret = field_["caret"].get<std::int64_t>();
    const auto baseTotal = field_["total_chars"].get<std::int64_t>();
    const auto caret = observed["caret"].get<std::int64_t>();
    const auto total = observed["total_chars"].get<std::int64_t>();
    const auto delta = static_cast<std::int64_t>(committedScalars_);
    if (baseCaret < 0 || baseTotal < baseCaret ||
        baseCaret > std::numeric_limits<std::int64_t>::max() - delta ||
        baseTotal > std::numeric_limits<std::int64_t>::max() - delta ||
        caret != baseCaret + delta || total != baseTotal + delta) {
        return false;
    }
    auto expected = context;
    expected.before = expectedBefore_;
    expected.anchor += committedScalars_;
    expected.head += committedScalars_;
    if (!observerAgrees(observed, expected)) return false;
    context = std::move(expected);
    return true;
}

bool ZenPendingCommit::corroborate(std::string_view appId, ContextWindow &context,
                                   const Json &observed) {
    if (applyToCorroborated(appId, context, observed)) {
        field_["binding"]["epoch"] = observed["binding"]["epoch"];
        return true;
    }
    clear();
    return false;
}

std::optional<std::string_view> automaticQuietReason(const Json &observed) {
    if (!observed.is_object()) return std::nullopt;
    const auto quiet = observed.value("quiet", Json());
    if (quiet.is_null()) return std::nullopt;
    if (quiet == "file_dialog") return "file_dialog";
    if (quiet == "narrow_field") return "narrow_field";
    // A quiet fact this build does not name still quiets automatic work.
    return "quiet_field";
}

std::optional<std::string_view> observedRequestBlocked(const Json &observed, const ContextWindow &context,
                                                       bool foreignIme, bool automatic) {
    if (!context.identityKnown) return "field_identity_unknown";
    if (context.before.empty()) return "empty_prefix";
    if (foreignIme) return "foreign_ime_active";
    if (automatic) {
        if (const auto quiet = automaticQuietReason(observed)) return quiet;
    }
    if (!observerAgrees(observed, context)) return "observer_context_mismatch";
    return std::nullopt;
}

bool observerDeniedField(const Json &error) {
    return error == "sensitive_field" || error == "unsupported_field" || error == "ineligible_field" ||
           error == "selection_present" || error == "invalid_caret";
}

std::string_view snapshotFailureReason(const Json &error) {
    if (error == "stale_binding") return "observer_stale_binding";
    if (error == "operation_timeout") return "observer_timeout";
    return "observer_snapshot_denied";
}

bool invalidates(const Json &event, std::string_view appId) {
    const auto app = event.is_object() ? event.value("app_id", Json()) : Json();
    if (!app.is_string()) return true;
    const auto &named = app.get_ref<const std::string &>();
    return appId.empty() || named.empty() || named == appId;
}

bool InspectionSchedule::reinspectAfterInvalidation(bool fieldChanged) {
    if (inputSerial_ != inspectedSerial_) return true;
    // The first change after an early inspection can be that input's own;
    // it inspects again after the typing pause instead of waiting for input.
    if (std::exchange(inspectedEarly_, false)) return true;
    if (fieldChanged || idleInvalidations_ == kMaxIdleReinspections) return false;
    ++idleInvalidations_;
    return true;
}

} // namespace badi::fcitx5
