#include "revision.h"

#include "sanitizer.h"

#include <algorithm>
#include <utility>

namespace badi::fcitx5 {

namespace {

bool continuation(char byte) { return (static_cast<unsigned char>(byte) & 0xC0U) == 0x80U; }

// The byte offset of each scalar of valid UTF-8 `text`, and of its end.
std::vector<std::size_t> scalarOffsets(std::string_view text) {
    std::vector<std::size_t> offsets;
    for (std::size_t index = 0; index < text.size(); ++index) {
        if (!continuation(text[index])) offsets.push_back(index);
    }
    offsets.push_back(text.size());
    return offsets;
}

// Unicode general category Cf, which can reorder or hide text.
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

// Unicode White_Space other than the spaces a revision may keep.
bool otherWhitespace(std::uint32_t scalar) {
    return (scalar >= 0x09 && scalar <= 0x0D) || scalar == 0x85 || scalar == 0x1680 ||
           (scalar >= 0x2000 && scalar <= 0x200A) || scalar == 0x2028 || scalar == 0x2029 || scalar == 0x205F ||
           scalar == 0x3000;
}

std::vector<std::string_view> words(std::string_view text) {
    std::vector<std::string_view> result;
    std::size_t start = std::string_view::npos;
    for (std::size_t index = 0; index <= text.size(); ++index) {
        const bool space = index == text.size() || text[index] == ' ';
        if (space && start != std::string_view::npos) {
            result.push_back(text.substr(start, index - start));
            start = std::string_view::npos;
        } else if (!space && start == std::string_view::npos) {
            start = index;
        }
    }
    return result;
}

std::size_t scalarCount(std::string_view text) {
    return static_cast<std::size_t>(std::count_if(text.begin(), text.end(), [](char byte) {
        return !continuation(byte);
    }));
}

} // namespace

std::optional<SelectionWindow> captureSelectionWindow(std::string_view text, std::size_t cursor,
                                                      std::size_t anchor, bool multiline,
                                                      std::string language) {
    if (cursor == anchor || text.size() > kMaxContextSourceBytes || !validLanguageTag(language) ||
        !decodeUtf8(text)) {
        return std::nullopt;
    }
    const auto offsets = scalarOffsets(text);
    const auto start = std::min(cursor, anchor);
    const auto end = std::max(cursor, anchor);
    if (end >= offsets.size() || end - start > kMaxSelectedScalars) return std::nullopt;
    const auto before = scalarWindow(text, start, kMaxBeforeScalars, 0);
    const auto after = scalarWindow(text, end, 0, kMaxAfterScalars);
    if (!before || !after || !validContextText(before->before) || !validContextText(after->after)) {
        return std::nullopt;
    }
    const auto selected = text.substr(offsets[start], offsets[end] - offsets[start]);
    if (!safeRevisionText(selected, kMaxSelectedScalars) ||
        selected.find_first_not_of(" \xc2\xa0") == std::string_view::npos) {
        return std::nullopt;
    }
    return SelectionWindow{
        .before = std::string(before->before),
        .selected = std::string(selected),
        .after = std::string(after->after),
        .anchor = anchor,
        .head = cursor,
        .language = std::move(language),
        .multiline = multiline,
    };
}

bool safeRevisionText(std::string_view text, std::size_t maxScalars) {
    const auto scalars = decodeUtf8(text);
    if (!scalars || scalars->empty() || scalars->size() > maxScalars) return false;
    return std::none_of(scalars->begin(), scalars->end(), [](std::uint32_t scalar) {
        return scalar < 0x20 || (scalar >= 0x7F && scalar <= 0x9F) || formatCharacter(scalar) ||
               otherWhitespace(scalar);
    });
}

std::vector<DiffPart> revisionDiff(std::string_view original, std::string_view revised) {
    const auto from = words(original);
    const auto to = words(revised);
    // Longest common subsequence of words, kept from the end so the walk
    // below reads forward.
    std::vector<std::vector<std::uint16_t>> common(from.size() + 1, std::vector<std::uint16_t>(to.size() + 1));
    for (std::size_t i = from.size(); i-- > 0;) {
        for (std::size_t j = to.size(); j-- > 0;) {
            common[i][j] = from[i] == to[j] ? static_cast<std::uint16_t>(common[i + 1][j + 1] + 1)
                                             : std::max(common[i + 1][j], common[i][j + 1]);
        }
    }
    std::vector<DiffPart> parts;
    const auto add = [&parts](DiffKind kind, std::string_view word) {
        if (!parts.empty() && parts.back().kind == kind) {
            parts.back().text += ' ';
            parts.back().text += word;
        } else {
            parts.push_back(DiffPart{.kind = kind, .text = std::string(word)});
        }
    };
    std::size_t i = 0;
    std::size_t j = 0;
    while (i < from.size() || j < to.size()) {
        if (i < from.size() && j < to.size() && from[i] == to[j]) {
            add(DiffKind::Same, from[i]);
            ++i;
            ++j;
        } else if (i < from.size() && (j == to.size() || common[i + 1][j] >= common[i][j + 1])) {
            add(DiffKind::Removed, from[i++]);
        } else {
            add(DiffKind::Added, to[j++]);
        }
    }
    return parts;
}

DiffLines wrapDiff(const std::vector<DiffPart> &diff, std::size_t width, std::size_t lines) {
    DiffLines result;
    std::vector<DiffPart> line;
    std::size_t used = 0;
    const auto finish = [&] {
        if (line.empty()) return;
        result.lines.push_back(std::move(line));
        line.clear();
        used = 0;
    };
    for (const auto &part : diff) {
        for (const auto word : words(part.text)) {
            const auto size = scalarCount(word);
            if (used > 0 && used + 1 + size > width) finish();
            if (result.lines.size() == lines) {
                result.complete = false;
                return result;
            }
            if (!line.empty() && line.back().kind == part.kind) {
                line.back().text += ' ';
                line.back().text += word;
            } else {
                line.push_back(DiffPart{.kind = part.kind, .text = std::string(word)});
            }
            used += (used > 0 ? 1 : 0) + size;
        }
    }
    finish();
    return result;
}

bool revisionLanded(std::string_view before, std::string_view after, const SelectionWindow &window,
                    std::string_view revised) {
    const auto expected = window.before + std::string(revised);
    const auto length = std::min(before.size(), expected.size());
    // The window may hold less than the revision, but never less than a
    // word of it, so an empty or cut window confirms nothing.
    if (length < std::min<std::size_t>(revised.size(), 16)) return false;
    if (before != std::string_view(expected).substr(expected.size() - length)) return false;
    // Chromium may add a paragraph end after the text, or cut it shorter;
    // text that disappeared does not agree.
    return after.starts_with(window.after) || (!after.empty() && std::string_view(window.after).starts_with(after));
}

std::optional<std::string_view> revisionClearNotice(std::string_view reason) {
    if (reason == "no_change") return "Badi found nothing to change";
    if (reason == "invalid_output") return "Badi could not revise this — try a shorter selection";
    if (reason == "unsupported_language") return "Badi revises English, German, French, Spanish and Portuguese";
    if (reason == "revision_unavailable") return "Badi's revision model is not available — check badi doctor";
    if (reason == "provider_timeout") return "Badi took too long to revise this — try a shorter selection";
    if (reason == "provider_error") return "Badi revision failed — check badi doctor";
    if (reason == "paused") return "Badi paused — resume from the tray";
    if (reason == "policy_never") return "Badi is not allowed in this application";
    if (reason == "field_sensitive" || reason == "field_not_editable" || reason == "field_ambiguous" ||
        reason == "manual_required") {
        return "Badi cannot revise text in this field";
    }
    return std::nullopt;
}

std::string revisionHelp(std::string_view mode) {
    return std::string(mode == "refine" ? "Refined" : "Grammar") +
           " · Tab apply · Esc dismiss · Ctrl+Shift+Space " + (mode == "refine" ? "grammar" : "refine");
}

} // namespace badi::fcitx5
