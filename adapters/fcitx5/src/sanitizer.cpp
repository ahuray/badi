#include "sanitizer.h"

#include <algorithm>
#include <cctype>
#include <limits>

namespace badi::fcitx5 {
namespace {

bool continuation(unsigned char byte) { return (byte & 0xc0U) == 0x80U; }

bool forbiddenOutputScalar(std::uint32_t value) {
    return value <= 0x1fU || (value >= 0x7fU && value <= 0x9fU) ||
           value == 0x00adU || (value >= 0x0600U && value <= 0x0605U) ||
           value == 0x061cU || value == 0x06ddU || value == 0x070fU ||
           (value >= 0x0890U && value <= 0x0891U) || value == 0x08e2U ||
           value == 0x180eU || (value >= 0x200bU && value <= 0x200fU) ||
           (value >= 0x2028U && value <= 0x202eU) ||
           (value >= 0x2060U && value <= 0x2064U) ||
           (value >= 0x2066U && value <= 0x206fU) || value == 0xfeffU ||
           (value >= 0xfff9U && value <= 0xfffbU) || value == 0x110bdU ||
           value == 0x110cdU || (value >= 0x13430U && value <= 0x1343fU) ||
           (value >= 0x1bca0U && value <= 0x1bca3U) ||
           (value >= 0x1d173U && value <= 0x1d17aU) || value == 0xe0001U ||
           (value >= 0xe0020U && value <= 0xe007fU);
}

bool unicodeWhitespace(std::uint32_t value) {
    return value == 0x20U || value == 0x85U || value == 0xa0U ||
           value == 0x1680U || (value >= 0x2000U && value <= 0x200aU) ||
           (value >= 0x2028U && value <= 0x2029U) || value == 0x202fU ||
           value == 0x205fU || value == 0x3000U;
}

} // namespace

std::optional<std::vector<std::uint32_t>> decodeUtf8(std::string_view value) {
    std::vector<std::uint32_t> result;
    result.reserve(value.size());
    for (std::size_t index = 0; index < value.size();) {
        const auto first = static_cast<unsigned char>(value[index]);
        std::uint32_t scalar = 0;
        std::size_t width = 0;
        if (first <= 0x7fU) {
            scalar = first;
            width = 1;
        } else if (first >= 0xc2U && first <= 0xdfU) {
            scalar = first & 0x1fU;
            width = 2;
        } else if (first >= 0xe0U && first <= 0xefU) {
            scalar = first & 0x0fU;
            width = 3;
        } else if (first >= 0xf0U && first <= 0xf4U) {
            scalar = first & 0x07U;
            width = 4;
        } else {
            return std::nullopt;
        }
        if (index + width > value.size()) return std::nullopt;
        for (std::size_t offset = 1; offset < width; ++offset) {
            const auto byte = static_cast<unsigned char>(value[index + offset]);
            if (!continuation(byte)) return std::nullopt;
            scalar = (scalar << 6U) | (byte & 0x3fU);
        }
        if ((width == 3 && scalar < 0x800U) ||
            (width == 4 && scalar < 0x10000U) ||
            (scalar >= 0xd800U && scalar <= 0xdfffU) || scalar > 0x10ffffU) {
            return std::nullopt;
        }
        result.push_back(scalar);
        index += width;
    }
    return result;
}

std::optional<std::string> sanitizeSuggestion(std::string_view value) {
    const auto scalars = decodeUtf8(value);
    if (!scalars || scalars->empty() || scalars->size() > kMaxSuggestionScalars ||
        value.size() > 4 * kMaxSuggestionScalars) {
        return std::nullopt;
    }
    std::size_t words = 0;
    bool inWord = false;
    bool anyNonSpace = false;
    bool previousSpace = false;
    for (const auto scalar : *scalars) {
        if (forbiddenOutputScalar(scalar) ||
            (unicodeWhitespace(scalar) && scalar != 0x20U)) {
            return std::nullopt;
        }
        const bool space = scalar == 0x20U;
        if (space && previousSpace) return std::nullopt;
        if (space) {
            inWord = false;
        } else {
            anyNonSpace = true;
            if (!inWord) ++words;
            inWord = true;
        }
        previousSpace = space;
    }
    if (!anyNonSpace || previousSpace || words > kMaxSuggestionWords) {
        return std::nullopt;
    }
    return std::string(value);
}

std::optional<std::string> scalarSlice(std::string_view value,
                                       std::size_t first,
                                       std::size_t count) {
    const auto scalars = decodeUtf8(value);
    if (!scalars || first > scalars->size()) return std::nullopt;
    const auto last = std::min(scalars->size(), first + count);
    std::size_t scalarIndex = 0;
    std::size_t byteIndex = 0;
    std::size_t firstByte = value.size();
    std::size_t lastByte = value.size();
    while (byteIndex < value.size()) {
        if (scalarIndex == first) firstByte = byteIndex;
        if (scalarIndex == last) {
            lastByte = byteIndex;
            break;
        }
        const auto byte = static_cast<unsigned char>(value[byteIndex]);
        byteIndex += byte <= 0x7fU ? 1 : byte <= 0xdfU ? 2 : byte <= 0xefU ? 3 : 4;
        ++scalarIndex;
    }
    if (first == scalars->size()) firstByte = value.size();
    if (last == scalars->size()) lastByte = value.size();
    return std::string(value.substr(firstByte, lastByte - firstByte));
}

bool validLinuxAppId(std::string_view value) {
    if (value.empty() || value.size() > 128) {
        return false;
    }
    std::size_t segmentStart = 0;
    for (std::size_t index = 0; index <= value.size(); ++index) {
        if (index != value.size() && value[index] != '.') continue;
        const auto segment = value.substr(segmentStart, index - segmentStart);
        if (segment.empty() || segment.front() < 'a' || segment.front() > 'z' ||
            !std::all_of(segment.begin(), segment.end(), [](unsigned char byte) {
                return (byte >= 'a' && byte <= 'z') ||
                       (byte >= '0' && byte <= '9') || byte == '_' || byte == '-';
            })) {
            return false;
        }
        segmentStart = index + 1;
    }
    return true;
}

bool validLanguageTag(std::string_view value) {
    if (value.size() < 2 || value.size() > 35) return false;
    std::size_t subtagStart = 0;
    for (std::size_t index = 0; index <= value.size(); ++index) {
        if (index != value.size() && value[index] != '-') continue;
        const auto subtag = value.substr(subtagStart, index - subtagStart);
        if (subtag.empty() ||
            !std::all_of(subtag.begin(), subtag.end(), [](unsigned char byte) {
                return std::isalnum(byte);
            })) {
            return false;
        }
        subtagStart = index + 1;
    }
    return true;
}

bool validOpaqueId(std::string_view value) {
    return !value.empty() && value.size() <= 128 &&
           std::all_of(value.begin(), value.end(), [](unsigned char byte) {
               return std::isalnum(byte) || byte == '.' || byte == '_' ||
                      byte == ':' || byte == '-';
           });
}

bool validSessionId(std::string_view value) {
    if (value.size() != 36) return false;
    for (std::size_t index = 0; index < value.size(); ++index) {
        if (index == 8 || index == 13 || index == 18 || index == 23) {
            if (value[index] != '-') return false;
        } else if (!((value[index] >= '0' && value[index] <= '9') ||
                     (value[index] >= 'a' && value[index] <= 'f'))) {
            return false;
        }
    }
    return value[14] >= '1' && value[14] <= '8' &&
           (value[19] == '8' || value[19] == '9' || value[19] == 'a' ||
            value[19] == 'b');
}

} // namespace badi::fcitx5
