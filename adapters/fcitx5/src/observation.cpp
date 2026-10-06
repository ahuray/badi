#include "observation.h"

#include "sanitizer.h"

#include <string>

namespace badi::fcitx5 {

using Json = nlohmann::json;

Json inspectRequest(std::string_view appId) {
    return Json{{"op", "inspect"}, {"app_id", appId}};
}

Json snapshotRequest(const Json &focus) {
    return Json{{"op", "snapshot"}, {"binding", focus.value("binding", Json())},
                {"policy_target", focus.value("target", Json())}};
}

Json previewRequest(const Json &focus, std::string_view text, std::uint64_t ttlMs,
                    const std::optional<CaretRect> &caret) {
    auto request = snapshotRequest(focus);
    request["op"] = "preview";
    request["text"] = text;
    request["expected_caret"] = focus.value("caret", Json());
    request["expected_total_chars"] = focus.value("total_chars", Json());
    request["ttl_ms"] = ttlMs;
    if (caret) {
        request["caret_rect"] = Json{{"x", caret->x}, {"y", caret->y}, {"width", caret->width},
                                     {"height", caret->height}, {"scale", caret->scale}};
    }
    return request;
}

bool observerAnswered(const Json &reply) {
    return reply.value("ok", Json()) == true && reply.contains("focus");
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
    if (kind == "browser") return NativeEditTarget::BrowserOrigin;
    if (kind == "desktop_application" && target.value("app_id", Json()) == app)
        return NativeEditTarget::DesktopApplication;
    return std::nullopt;
}

// Exactly Fcitx's text before the caret; or, when the observer could serialize
// only the caret's own block (a list, quote or table precedes it), that block's
// text ending Fcitx's text at a line start.
bool beforeAgrees(const Json &observed, const std::string &before) {
    const auto text = observed.value("before", Json());
    const auto scope = observed.value("scope", Json());
    if (!text.is_string() || !(scope.is_null() || scope == "block")) return false;
    const auto &block = text.get_ref<const std::string &>();
    if (block == before) return true;
    return scope == "block" && before.size() > block.size() && before.ends_with(block) &&
           before[before.size() - block.size() - 1] == '\n';
}

bool observerAgrees(const Json &observed, const ContextWindow &context) {
    if (!observed.is_object() || !beforeAgrees(observed, context.before)) return false;
    const auto after = observed.value("after", Json());
    if (after == observedAfter(context)) return true;
    // Both name the end of the field when Fcitx has nothing after the caret and
    // the observer only its last block's end: an EditContext editor (CodeMirror)
    // sends its own text, and an editor last on its page gets no block end.
    return context.after.empty() && context.paragraphEnd.empty() && (after == "\n" || after == "\n\n");
}

bool observerCorroborates(const Json &captured, const Json &observed, const ContextWindow &context) {
    return matchesObservedFocus(captured, observed, true) && observerAgrees(observed, context);
}

std::optional<std::string_view> observedRequestBlocked(const Json &observed, const ContextWindow &context,
                                                       bool foreignIme) {
    if (!context.identityKnown) return "field_identity_unknown";
    if (context.before.empty()) return "empty_prefix";
    if (!context.after.empty()) return "caret_not_at_end";
    if (foreignIme) return "foreign_ime_active";
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
    return named.empty() || named == appId;
}

bool InspectionSchedule::reinspectAfterInvalidation(bool fieldChanged) {
    if (inputSerial_ != inspectedSerial_) return true;
    if (fieldChanged || idleInvalidations_ == kMaxIdleReinspections) return false;
    ++idleInvalidations_;
    return true;
}

} // namespace badi::fcitx5
