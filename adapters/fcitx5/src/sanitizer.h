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
std::optional<std::string> scalarSlice(std::string_view value,
                                       std::size_t first,
                                       std::size_t count);
bool validLinuxAppId(std::string_view value);
bool validLanguageTag(std::string_view value);
bool validOpaqueId(std::string_view value);
bool validSessionId(std::string_view value);

} // namespace badi::fcitx5
