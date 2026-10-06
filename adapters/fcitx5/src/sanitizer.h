#pragma once

#include <cstddef>
#include <cstdint>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace badi::fcitx5 {

constexpr std::size_t kMaxBeforeScalars = 512;
constexpr std::size_t kMaxAfterScalars = 128;
constexpr std::size_t kMaxSuggestionScalars = 64;
constexpr std::size_t kMaxSuggestionWords = 8;
constexpr std::size_t kMaxContextSourceBytes = 65'536;

std::optional<std::vector<std::uint32_t>> decodeUtf8(std::string_view value);
std::optional<std::string> sanitizeSuggestion(std::string_view value);

struct ScalarWindow {
    std::string_view before;
    std::string_view after;
};
// Up to `beforeCount` Unicode scalar values before scalar offset `caret` and up
// to `afterCount` after it, decoding `value` once. Nullopt when any of `value`
// is invalid UTF-8 or the caret lies beyond its end.
std::optional<ScalarWindow> scalarWindow(std::string_view value, std::size_t caret,
                                         std::size_t beforeCount, std::size_t afterCount);
bool validLinuxAppId(std::string_view value);
// Folds an ASCII app identifier (^[A-Za-z][A-Za-z0-9_-]*(\.[A-Za-z][A-Za-z0-9_-]*)*$,
// at most 128 bytes) to the lowercase form validLinuxAppId accepts.
std::optional<std::string> canonicalAppId(std::string_view program);
// canonicalAppId, with an app's renamed Wayland id mapped to its existing identity.
std::optional<std::string> programAppId(std::string_view program);
bool validLanguageTag(std::string_view value);
bool validContextText(std::string_view value);
bool validOpaqueId(std::string_view value);
bool validSessionId(std::string_view value);

} // namespace badi::fcitx5
