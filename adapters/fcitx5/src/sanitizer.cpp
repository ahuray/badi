#include "sanitizer.h"

#include <algorithm>
#include <cctype>
#include <limits>
#include <utility>

namespace badi::fcitx5 {
namespace {

bool continuation(unsigned char byte) { return (byte & 0xc0U) == 0x80U; }

bool arabicLetter(std::uint32_t value) {
    return (value >= 0x0620U && value <= 0x063fU) ||
           (value >= 0x0641U && value <= 0x064aU) ||
           (value >= 0x066eU && value <= 0x066fU) ||
           (value >= 0x0671U && value <= 0x06d3U) || value == 0x06d5U ||
           (value >= 0x06e5U && value <= 0x06e6U) ||
           (value >= 0x06eeU && value <= 0x06efU) ||
           (value >= 0x06faU && value <= 0x06fcU) || value == 0x06ffU;
}

bool safeJoiner(const std::vector<std::uint32_t> &scalars, std::size_t index) {
    return index > 0 && index + 1 < scalars.size() &&
           arabicLetter(scalars[index - 1]) && arabicLetter(scalars[index + 1]);
}

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

// The scalar value starting at byte `index` and its width in bytes, or
// nullopt for an invalid, overlong, surrogate or truncated sequence.
std::optional<std::pair<std::uint32_t, std::size_t>> scalarAt(std::string_view value,
                                                              std::size_t index) {
    const auto first = static_cast<unsigned char>(value[index]);
    std::uint32_t scalar = 0;
    std::size_t width = 0;
    if (first <= 0x7fU) {
        return std::pair<std::uint32_t, std::size_t>{first, 1};
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
    if ((width == 3 && scalar < 0x800U) || (width == 4 && scalar < 0x10000U) ||
        (scalar >= 0xd800U && scalar <= 0xdfffU) || scalar > 0x10ffffU) {
        return std::nullopt;
    }
    return std::pair{scalar, width};
}

} // namespace

std::optional<std::vector<std::uint32_t>> decodeUtf8(std::string_view value) {
    std::vector<std::uint32_t> result;
    result.reserve(value.size());
    for (std::size_t index = 0; index < value.size();) {
        const auto scalar = scalarAt(value, index);
        if (!scalar) return std::nullopt;
        result.push_back(scalar->first);
        index += scalar->second;
    }
    return result;
}

std::optional<ScalarWindow> scalarWindow(std::string_view value, std::size_t caret,
                                         std::size_t beforeCount, std::size_t afterCount) {
    const auto first = caret > beforeCount ? caret - beforeCount : 0;
    const auto last = caret + std::min(afterCount, std::numeric_limits<std::size_t>::max() - caret);
    std::size_t firstByte = value.size();
    std::size_t caretByte = value.size();
    std::size_t lastByte = value.size();
    std::size_t scalars = 0;
    for (std::size_t index = 0; index < value.size(); ++scalars) {
        if (scalars == first) firstByte = index;
        if (scalars == caret) caretByte = index;
        if (scalars == last) lastByte = index;
        const auto scalar = scalarAt(value, index);
        if (!scalar) return std::nullopt;
        index += scalar->second;
    }
    if (caret > scalars) return std::nullopt;
    return ScalarWindow{
        .before = value.substr(firstByte, caretByte - firstByte),
        .after = value.substr(caretByte, lastByte - caretByte),
    };
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
    for (std::size_t index = 0; index < scalars->size(); ++index) {
        const auto scalar = (*scalars)[index];
        if ((forbiddenOutputScalar(scalar) && !(scalar == 0x200cU && safeJoiner(*scalars, index))) ||
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

bool validContextText(std::string_view value) {
    const auto scalars = decodeUtf8(value);
    if (!scalars) return false;
    for (std::size_t index = 0; index < scalars->size(); ++index) {
        const auto scalar = (*scalars)[index];
        if (scalar == '\n' || scalar == '\t') continue;
        if (forbiddenOutputScalar(scalar) && !(scalar == 0x200cU && safeJoiner(*scalars, index))) return false;
    }
    return true;
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

std::optional<std::string> canonicalAppId(std::string_view program) {
    // Toolkits report reverse-DNS or window-class names with capitals, e.g.
    // Qt's "Telegram". Only an ASCII identifier shape is folded; titles and
    // other display strings never become a policy identity.
    if (program.empty() || program.size() > 128) return std::nullopt;
    std::string result;
    result.reserve(program.size());
    bool segmentStart = true;
    for (const auto character : program) {
        const auto byte = static_cast<unsigned char>(character);
        const bool letter = (byte >= 'a' && byte <= 'z') || (byte >= 'A' && byte <= 'Z');
        if (byte == '.') {
            if (segmentStart) return std::nullopt;
            segmentStart = true;
        } else if (segmentStart ? !letter
                                : !(letter || (byte >= '0' && byte <= '9') || byte == '_' || byte == '-')) {
            return std::nullopt;
        } else {
            segmentStart = false;
        }
        result.push_back(static_cast<char>(byte >= 'A' && byte <= 'Z' ? byte - 'A' + 'a' : byte));
    }
    if (segmentStart) return std::nullopt;
    return result;
}

std::optional<std::string> programAppId(std::string_view program) {
    // VS Code 1.140 renamed its Wayland app id from "code"; its grants,
    // sessions and observer rule keep the original identity.
    auto app = canonicalAppId(program);
    if (app == "com.microsoft.vscode") return "code";
    return app;
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
