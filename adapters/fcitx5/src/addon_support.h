#pragma once

// Small helpers shared by the addon's source files.

#include <fcitx-utils/key.h>
#include <fcitx/inputcontext.h>
#include <fcitx/surroundingtext.h>

#include <nlohmann/json.hpp>

#include <sys/random.h>
#include <time.h>

#include <array>
#include <cstdint>
#include <iomanip>
#include <optional>
#include <sstream>
#include <string>

#include "state.h"

namespace badi::fcitx5 {

using Json = nlohmann::json;

inline constexpr std::uint64_t kNoticeLifetimeUs = 5'000'000;
// One Tab may wait this long for the type-through remainder of a word
// acceptance to be displayed again; then it is dropped, never replayed.
inline constexpr std::uint64_t kHeldTabTimeoutUs = 700'000;
// Fcitx's sd-event loop treats a timer accuracy of 0 as its 250 ms default
// slack, which delayed the 120 ms typing pause by up to 250 ms.
inline constexpr std::uint64_t kTimerAccuracyUs = 1'000;

inline std::uint64_t monotonicUs() {
    timespec now{};
    if (clock_gettime(CLOCK_MONOTONIC, &now) != 0) return 0;
    return static_cast<std::uint64_t>(now.tv_sec) * 1'000'000ULL +
           static_cast<std::uint64_t>(now.tv_nsec) / 1'000ULL;
}

inline std::string uuidString(const ::fcitx::ICUUID &uuid) {
    static constexpr std::array<std::size_t, 4> dashes{4, 6, 8, 10};
    std::ostringstream output;
    output << std::hex << std::setfill('0');
    for (std::size_t index = 0; index < uuid.size(); ++index) {
        if (std::find(dashes.begin(), dashes.end(), index) != dashes.end()) {
            output << '-';
        }
        output << std::setw(2) << static_cast<unsigned int>(uuid[index]);
    }
    return output.str();
}

inline std::optional<std::array<std::uint8_t, 16>> randomBytes() {
    std::array<std::uint8_t, 16> bytes{};
    const auto count = ::getrandom(bytes.data(), bytes.size(), 0);
    if (count != static_cast<ssize_t>(bytes.size())) return std::nullopt;
    return bytes;
}

inline std::optional<std::string> randomSalt() {
    const auto bytes = randomBytes();
    if (!bytes) return std::nullopt;
    std::ostringstream output;
    output << std::hex << std::setfill('0');
    for (const auto byte : *bytes) output << std::setw(2) << static_cast<unsigned int>(byte);
    return output.str();
}

inline std::optional<std::string> randomUuid() {
    auto bytes = randomBytes();
    if (!bytes) return std::nullopt;
    auto &uuid = *bytes;
    uuid[6] = static_cast<std::uint8_t>((uuid[6] & 0x0fU) | 0x40U);  // version 4
    uuid[8] = static_cast<std::uint8_t>((uuid[8] & 0x3fU) | 0x80U);  // RFC 4122 variant
    return uuidString(uuid);
}

// The one app identity used for policy, sessions, debug and the observer.
// Empty when program() is not an app identifier.
inline std::string canonicalProgram(const ::fcitx::InputContext &inputContext) {
    return programAppId(inputContext.program()).value_or("");
}

inline const ::fcitx::Key &invokeChord() {
    static const ::fcitx::Key key("Control+Shift+space");
    return key;
}

inline const ::fcitx::Key &acceptChord() {
    static const ::fcitx::Key key("Control+Shift+Y");
    return key;
}

inline const ::fcitx::Key &allKey() {
    static const ::fcitx::Key key("Control+Right");
    return key;
}

// The text a key types, if it types without a command modifier.
inline std::string typedText(const ::fcitx::Key &key) {
    if (key.states().testAny(::fcitx::KeyStates{::fcitx::KeyState::Ctrl, ::fcitx::KeyState::Alt,
                                                ::fcitx::KeyState::Super})) return {};
    return ::fcitx::Key::keySymToUTF8(key.sym());
}

// A key a terminal line editor uses to put the caret inside the line, or
// to edit beside it: the typed line no longer ends at the caret. Any other
// unknown key (paste, completion, a new prompt line) leaves it at the end.
inline bool movesCaretIntoLine(const ::fcitx::Key &key) {
    const auto states = key.states();
    const bool ctrl = !!(states & ::fcitx::KeyState::Ctrl);
    const bool alt = !!(states & ::fcitx::KeyState::Alt);
    switch (key.sym()) {
    case FcitxKey_Left: case FcitxKey_Right: case FcitxKey_Up: case FcitxKey_Down:
    case FcitxKey_Home: case FcitxKey_Page_Up: case FcitxKey_Page_Down:
    case FcitxKey_KP_Left: case FcitxKey_KP_Right: case FcitxKey_KP_Up: case FcitxKey_KP_Down:
    case FcitxKey_KP_Home: case FcitxKey_Delete: case FcitxKey_KP_Delete:
        return true;
    default:
        break;
    }
    const auto sym = ::fcitx::Key(key.sym()).normalize().sym();
    const auto is = [sym](::fcitx::KeySym lower, ::fcitx::KeySym upper) { return sym == lower || sym == upper; };
    // Readline and Emacs-style motion: Ctrl+A/B/F/D/T, Alt+B/F/D.
    return (ctrl && !alt && (is(FcitxKey_a, FcitxKey_A) || is(FcitxKey_b, FcitxKey_B) ||
                             is(FcitxKey_f, FcitxKey_F) || is(FcitxKey_d, FcitxKey_D) ||
                             is(FcitxKey_t, FcitxKey_T))) ||
           (alt && !ctrl && (is(FcitxKey_b, FcitxKey_B) || is(FcitxKey_f, FcitxKey_F) ||
                             is(FcitxKey_d, FcitxKey_D)));
}

inline std::size_t surroundingDigest(const ::fcitx::SurroundingText &surrounding) {
    return std::hash<std::string_view>{}(surrounding.text()) ^
           (static_cast<std::size_t>(surrounding.cursor()) * 0x9e3779b97f4a7c15ULL) ^ surrounding.anchor();
}


} // namespace badi::fcitx5
