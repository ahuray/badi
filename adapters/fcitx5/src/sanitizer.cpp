#include "sanitizer.h"

#include <algorithm>
#include <cctype>
#include <limits>
#include <utility>

namespace badi::fcitx5 {
namespace {

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

// Controls, the line and paragraph separators, and format characters.
bool forbiddenOutputScalar(std::uint32_t value) {
    return value <= 0x1fU || (value >= 0x7fU && value <= 0x9fU) || value == 0x2028U || value == 0x2029U ||
           formatCharacter(value);
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
        if (!utf8Continuation(value[index + offset])) return std::nullopt;
        scalar = (scalar << 6U) | (static_cast<unsigned char>(value[index + offset]) & 0x3fU);
    }
    if ((width == 3 && scalar < 0x800U) || (width == 4 && scalar < 0x10000U) ||
        (scalar >= 0xd800U && scalar <= 0xdfffU) || scalar > 0x10ffffU) {
        return std::nullopt;
    }
    return std::pair{scalar, width};
}

} // namespace

bool formatCharacter(std::uint32_t scalar) {
    return scalar == 0xAD || (scalar >= 0x600 && scalar <= 0x605) || scalar == 0x61C || scalar == 0x6DD ||
           scalar == 0x70F || scalar == 0x890 || scalar == 0x891 || scalar == 0x8E2 || scalar == 0x180E ||
           (scalar >= 0x200B && scalar <= 0x200F) || (scalar >= 0x202A && scalar <= 0x202E) ||
           (scalar >= 0x2060 && scalar <= 0x2064) || (scalar >= 0x2066 && scalar <= 0x206F) ||
           scalar == 0xFEFF || (scalar >= 0xFFF9 && scalar <= 0xFFFB) || scalar == 0x110BD ||
           scalar == 0x110CD || (scalar >= 0x13430 && scalar <= 0x1343F) ||
           (scalar >= 0x1BCA0 && scalar <= 0x1BCA3) || (scalar >= 0x1D173 && scalar <= 0x1D17A) ||
           scalar == 0xE0001 || (scalar >= 0xE0020 && scalar <= 0xE007F);
}

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
    // One trailing ASCII space separates a mid-sentence insertion from the
    // next word. A second space was already rejected above.
    if (!anyNonSpace || words > kMaxSuggestionWords) {
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

namespace {

// A Chromium-family browser names each --app window
// "<prefix><host>_<path>-<profile>" with the URL path's slashes as
// underscores; the path starts with one, so the name holds "__".
// Installed-PWA windows ("<prefix><extension id>-<profile>") and the
// browser's own builds ("brave-browser*") have none and stay unmapped.
bool webAppWindow(std::string_view program, std::string_view prefix) {
    if (!program.starts_with(prefix) || program.size() > 255 ||
        program.find("__", prefix.size()) == std::string_view::npos) return false;
    return std::all_of(program.begin(), program.end(), [](unsigned char byte) {
        return byte > ' ' && byte < 0x7f && byte != '/';
    });
}

} // namespace

bool braveWebAppWindow(std::string_view program) { return webAppWindow(program, "brave-"); }

// Omarchy opens web apps in Chromium unless another supported browser is the
// default. Google Chrome names its --app windows the same way; the observer
// admits only Chromium's own executable under this identity, so those stay
// denied.
bool chromiumWebAppWindow(std::string_view program) { return webAppWindow(program, "chrome-"); }

std::optional<std::string> programAppId(std::string_view program) {
    // A web-app window shows pages whose origin, not this window name,
    // selects policy, so it keeps the browser's identity.
    if (braveWebAppWindow(program)) return "brave-origin";
    if (chromiumWebAppWindow(program)) return "chromium";
    // VS Code 1.140 renamed its Wayland app id from "code"; its grants,
    // sessions and observer rule keep the original identity.
    auto app = canonicalAppId(program);
    if (app == "com.microsoft.vscode") return "code";
    // Every LibreOffice window reports its first window's id, usually
    // libreoffice-startcenter, so no module id can select a path on its own;
    // the observer's window rule picks the module.
    if (app && app->starts_with("libreoffice-")) return "libreoffice";
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
