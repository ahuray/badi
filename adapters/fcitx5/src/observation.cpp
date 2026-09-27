#include "observation.h"

#include "sanitizer.h"

#include <string>

namespace badi::fcitx5 {

using Json = nlohmann::json;

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

bool observerAgrees(const Json &observed, const ContextWindow &context) {
    return observed.is_object() && observed.value("before", Json()) == context.before &&
           observed.value("after", Json()) == observedAfter(context);
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

} // namespace badi::fcitx5
