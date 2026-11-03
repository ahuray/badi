#pragma once

// Explicit revision of one selection (broker ADR 0012): what Fcitx captures,
// what the panel shows, and how the field must read once applied. Pure
// functions; the addon owns timing, the observer and the broker link.

#include <cstddef>
#include <cstdint>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace badi::fcitx5 {

inline constexpr std::size_t kMaxSelectedScalars = 600;
inline constexpr std::size_t kMaxRevisedScalars = 1'100;
inline constexpr std::uint64_t kMaxRevisionTtlMs = 60'000;
// The most words a selection may have for a synonym (broker ADR 0013).
inline constexpr std::size_t kMaxSynonymWords = 3;
// The panel shows a revision in lines of at most this many characters.
inline constexpr std::size_t kRevisionLineScalars = 64;
inline constexpr std::size_t kRevisionLines = 20;

// One selection on one line and the text around it, as Fcitx reports it.
struct SelectionWindow {
    std::string before;   // up to 512 scalars before the selection
    std::string selected; // 1–600 scalars, one line
    std::string after;    // up to 128 scalars after it
    // Fcitx's anchor and cursor, as reported: the selection spans exactly
    // `selected`.
    std::uint64_t anchor = 0;
    std::uint64_t head = 0;
    std::string language;
    bool multiline = false;
    bool identityKnown = false;

    bool operator==(const SelectionWindow &) const = default;
};

// The selection in `text` between Fcitx's `cursor` and `anchor` (scalar
// offsets), when it is one line of 1–600 scalars that a revision may read.
std::optional<SelectionWindow> captureSelectionWindow(std::string_view text, std::size_t cursor,
                                                      std::size_t anchor, bool multiline,
                                                      std::string language);

// The same selection: its text, the text before it, and its anchor and head;
// with `withAfter`, the text after it too. Sending and applying a revision
// need the field exactly as the broker saw it. While the revision waits or
// shows, Fcitx may report the text after the selection differently (Chromium
// cuts its window short or adds a paragraph end) without the selection
// moving, so that alone retires nothing; applying compares it again.
bool sameSelection(const SelectionWindow &current, const SelectionWindow &requested, bool withAfter);

// Text a revision may read or write: 1–`maxScalars` scalars of valid UTF-8,
// one line, no control or format (Cf) character, and no whitespace other
// than U+0020 and the no-break spaces U+00A0 and U+202F. The broker's
// `safe_revision_text` and the wire schema's revisionText say the same.
bool safeRevisionText(std::string_view text, std::size_t maxScalars);

// A word the revision kept, removed or added.
enum class DiffKind { Same, Removed, Added };
struct DiffPart {
    DiffKind kind = DiffKind::Same;
    std::string text;

    bool operator==(const DiffPart &) const = default;
};

// The words of `original` and `revised` in order, each marked by whether
// the revision kept, removed or added it (their longest common run kept).
// A removed word comes before the word added in its place.
std::vector<DiffPart> revisionDiff(std::string_view original, std::string_view revised);

// The diff in panel lines of at most `width` scalars each, words never
// split; `complete` is false when it needed more than `lines` lines.
struct DiffLines {
    std::vector<std::vector<DiffPart>> lines;
    bool complete = true;
};
DiffLines wrapDiff(const std::vector<DiffPart> &diff, std::size_t width, std::size_t lines);

// Whether the field reads as applying the revision left it: Fcitx's text
// before the collapsed caret ends as the selection's `before` followed by
// `revised` (as far as its window reaches), and the text after it is the
// selection's `after`.
bool revisionLanded(std::string_view before, std::string_view after, const SelectionWindow &window,
                    std::string_view revised);

// What the panel says for a broker revision.clear reason; nullopt says
// nothing (the user retired it, or an edit did).
std::optional<std::string_view> revisionClearNotice(std::string_view reason);

// Whether `selected` is a word or short phrase a synonym may replace:
// one to `kMaxSynonymWords` words with a letter each, and no digit,
// address or sentence mark. The broker's `synonym_selection` decides the
// same; this only chooses the chord's next mode and its help line.
bool synonymSelection(std::string_view selected);

// The mode the chord asks for while a revision of `shownMode` for
// `selected` shows: grammar, then refine, then a synonym for a short
// selection, and another synonym after that.
std::string nextRevisionMode(std::string_view shownMode, std::string_view selected);

// The panel's line under a revision: its mode and keys.
std::string revisionHelp(std::string_view mode, std::string_view selected);

} // namespace badi::fcitx5
