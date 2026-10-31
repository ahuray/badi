#include "state.h"

#include "sanitizer.h"

#include <algorithm>
#include <array>
#include <cctype>
#include <iomanip>
#include <limits>
#include <sstream>
#include <utility>

namespace badi::fcitx5 {
namespace {

constexpr std::uint64_t kMaxSafeCounter = (std::uint64_t{1} << 53U) - 1U;

std::uint64_t mix(std::string_view value, std::uint64_t seed) {
    auto hash = seed;
    for (const auto byte : value) {
        hash ^= static_cast<unsigned char>(byte);
        hash *= 0x100000001b3ULL;
    }
    return hash;
}

// IME-parity apps are exactly the accessibility observer's web-app and
// observed-only rules (adapters/accessibility/daemon.py APPS);
// tests/observer-identities.py keeps the lists equal, because an id without a
// rule could never be observed.
constexpr std::array<std::string_view, 5> kImeParityBrowsers{
    "chromium", "chromium-browser", "brave-origin", "zen", "zen-twilight"};
constexpr std::array<std::string_view, 6> kImeParityDesktopApps{"chatgpt", "code", "com.anthropic.claude", "cursor",
                                                                 "discord", "grok-bot"};
// Native apps on the same observed, append-only contract: one Fcitx program
// id serves modules or fields the observer must tell apart (LibreOffice Calc
// cells must keep Tab), so no manual path may exist.
constexpr std::array<std::string_view, 1> kImeParityNativeApps{"libreoffice"};
// Terminals whose typed line and commitString were tried live.
constexpr std::array<std::string_view, 2> kTerminalApps{"com.mitchellh.ghostty", "foot"};

// Chromium-family identities without an observer rule: other Chromium
// browsers, channels and Flatpak ids, installed web-app windows (Wayland
// chrome-<id>-<profile>, X11 crx_<id>), shared Electron runtimes and other
// builds of the IME-parity apps. Their commitString is equally retargetable,
// and nothing can corroborate their fields, so none may use the manual path.
bool unobservedChromiumFamily(std::string_view appId) {
    constexpr std::array<std::string_view, 20> exact{
        "chrome", "brave", "opera", "vivaldi", "msedge", "helium", "helium-browser", "thorium",
        "thorium-browser", "cromite", "ungoogled-chromium", "code-oss", "code-insiders", "codium",
        "vscodium", "discordcanary", "discordptb", "vesktop", "legcord", "webcord"};
    constexpr std::array<std::string_view, 24> prefixes{
        "chrome-", "crx_", "chromium-", "google-chrome", "brave-", "microsoft-edge", "msedge-",
        "vivaldi-", "opera-", "yandex-browser", "electron", "discord-", "com.google.chrome",
        "org.chromium.", "com.brave.", "com.microsoft.edge", "com.vivaldi.", "com.opera.",
        "io.github.ungoogled_software.", "com.visualstudio.code", "com.microsoft.vscode", "com.vscodium.",
        "com.discordapp.", "dev.vencord."};
    return std::find(exact.begin(), exact.end(), appId) != exact.end() ||
           std::any_of(prefixes.begin(), prefixes.end(),
                       [appId](std::string_view prefix) { return appId.starts_with(prefix); });
}

// Gecko-family browser identities other than the observed Zen builds: Firefox
// and its channels, Flatpak ids and PWAsForFirefox web-app windows (FFPWA-<id>),
// other Zen builds (alpha, beta, Flatpak), and Firefox forks. Their commitString
// is as retargetable as Zen's, their urlbar need not carry a Url purpose, and
// no observer rule corroborates their fields, so none may use the manual path.
// Gecko mail clients (Thunderbird) are not browsers and keep the native contract.
bool unobservedGeckoFamily(std::string_view appId) {
    constexpr std::array<std::string_view, 6> exact{
        "iceweasel", "icecat", "palemoon", "seamonkey", "basilisk", "torbrowser"};
    constexpr std::array<std::string_view, 17> prefixes{
        "firefox", "org.mozilla.firefox", "ffpwa-", "zen-", "app.zen_browser.",
        "io.github.zen_browser.", "librewolf", "io.gitlab.librewolf", "floorp", "one.ablaze.",
        "waterfox", "net.waterfox.", "mullvadbrowser", "mullvad-browser", "net.mullvad.",
        "tor-browser", "org.torproject."};
    return std::find(exact.begin(), exact.end(), appId) != exact.end() ||
           std::any_of(prefixes.begin(), prefixes.end(),
                       [appId](std::string_view prefix) { return appId.starts_with(prefix); });
}

} // namespace

bool hasForeignImeUi(const PanelObservation &panel) {
    return panel.preedit || panel.clientPreedit || panel.foreignAuxiliary ||
           (panel.candidates && !panel.candidatesOwnedByBadi);
}

bool hasOwnedCandidate(const PanelObservation &panel) {
    return panel.candidates && panel.candidatesOwnedByBadi &&
           !hasForeignImeUi(panel);
}

bool allowsNativeContext(::fcitx::CapabilityFlags capabilities) {
    constexpr std::array denied{
        ::fcitx::CapabilityFlag::PasswordOrSensitive,
        ::fcitx::CapabilityFlag::Disable,
        ::fcitx::CapabilityFlag::Email,
        ::fcitx::CapabilityFlag::Digit,
        ::fcitx::CapabilityFlag::Url,
        ::fcitx::CapabilityFlag::Dialable,
        ::fcitx::CapabilityFlag::Number,
        ::fcitx::CapabilityFlag::Terminal,
        ::fcitx::CapabilityFlag::Date,
        ::fcitx::CapabilityFlag::Time,
    };
    return !!(capabilities & ::fcitx::CapabilityFlag::SurroundingText) &&
           std::none_of(denied.begin(), denied.end(),
                        [capabilities](const auto flag) {
                            return !!(capabilities & flag);
                        });
}

bool SurroundingEcho::capabilityChanged(::fcitx::CapabilityFlags before, ::fcitx::CapabilityFlags after) {
    // Fcitx reports the old flags without preedit bits it disabled and the
    // new ones as the client sent them; neither preedit bit gates a field.
    const auto changed = (before ^ after)
                             .unset(::fcitx::CapabilityFlag::Preedit)
                             .unset(::fcitx::CapabilityFlag::FormattedPreedit);
    if (changed == ::fcitx::CapabilityFlag::SurroundingText) {
        const bool granted = after.test(::fcitx::CapabilityFlag::SurroundingText);
        if (!granted && textUnderGrant_) {
            textUnderGrant_ = false;
            withdrawn_ = true;
            return true;
        }
        if (granted && withdrawn_) {
            withdrawn_ = false;
            return true;
        }
    }
    reset();
    return false;
}

::fcitx::CapabilityFlags observedFieldCapabilities(::fcitx::CapabilityFlags current,
                                                   const SurroundingEcho &echo) {
    return echo.withdrawn() ? current | ::fcitx::CapabilityFlag::SurroundingText : current;
}

NativeAppClass classifyNativeApp(std::string_view appId) {
    // Browser/Electron commitString behaves like a typed keystroke: page script
    // can retarget it during beforeinput and undo may coalesce it with typing.
    // These apps therefore accept only through an observed field, append-only.
    // Zen and Zen Twilight are the Gecko identities with an observer rule,
    // checked before the Gecko family their "zen-" prefix belongs to; the
    // urlbar shares the page's input context without a Url purpose, so the
    // observer, not allowsNativeContext(), keeps browser UI out.
    // Obsidian's editor plugin owns its fields, so a second integration would
    // double-suggest.
    constexpr std::array<std::string_view, 2> unavailable{"obsidian", "md.obsidian.obsidian"};
    const auto listed = [appId](const auto &ids) {
        return std::find(ids.begin(), ids.end(), appId) != ids.end();
    };
    if (!validLinuxAppId(appId) || listed(unavailable)) return NativeAppClass::Unavailable;
    if (listed(kTerminalApps)) return NativeAppClass::Terminal;
    if (listed(kImeParityBrowsers)) return NativeAppClass::ImeParityBrowser;
    if (unobservedGeckoFamily(appId)) return NativeAppClass::Unavailable;
    if (listed(kImeParityDesktopApps) || listed(kImeParityNativeApps)) return NativeAppClass::ImeParityDesktop;
    if (unobservedChromiumFamily(appId)) return NativeAppClass::Unavailable;
    return NativeAppClass::NativeExact;
}

bool imeParityApp(std::string_view appId) {
    const auto kind = classifyNativeApp(appId);
    return kind == NativeAppClass::ImeParityBrowser || kind == NativeAppClass::ImeParityDesktop;
}

bool correctsAfterChecks(std::string_view appId) {
    return imeParityApp(appId) && appId != "libreoffice";
}

bool correctionLanded(std::string_view before, std::string_view expected) {
    constexpr std::size_t kReadBackBytes = 64;
    auto tail = expected.size() > kReadBackBytes ? expected.substr(expected.size() - kReadBackBytes) : expected;
    while (!tail.empty() && utf8Continuation(tail.front())) tail.remove_prefix(1);
    return !tail.empty() && before.ends_with(tail);
}

std::optional<unsigned> nativeReplacementChars(bool imeParity, std::string_view before,
                                                std::string_view suffix) {
    if (imeParity || suffix.empty() || !before.ends_with(suffix)) return std::nullopt;
    unsigned count = 0;
    for (const char byte : suffix) {
        if (!utf8Continuation(byte)) ++count;
    }
    if (count == 0) return std::nullopt;
    return count;
}

bool nativeObservationAvailable(std::string_view appId) {
    const auto kind = classifyNativeApp(appId);
    return kind != NativeAppClass::Unavailable && kind != NativeAppClass::Terminal;
}

bool nativeEditingAvailable(std::string_view appId, NativeEditTarget target,
                            NativeEditPath path) {
    switch (classifyNativeApp(appId)) {
    case NativeAppClass::NativeExact:
        return target == NativeEditTarget::DesktopApplication;
    case NativeAppClass::ImeParityBrowser:
        return path == NativeEditPath::Observed && target == NativeEditTarget::BrowserOrigin;
    case NativeAppClass::ImeParityDesktop:
        return path == NativeEditPath::Observed && target == NativeEditTarget::DesktopApplication;
    case NativeAppClass::Terminal:
        return path == NativeEditPath::TypedLine && target == NativeEditTarget::DesktopApplication;
    case NativeAppClass::Unavailable:
        return false;
    }
    return false;
}

std::string_view focusReason(std::string_view appId) {
    if (appId.empty()) return "unidentified_app";
    if (classifyNativeApp(appId) == NativeAppClass::Terminal) return "terminal_typed_line";
    if (!nativeObservationAvailable(appId)) return "editor_transaction_unavailable";
    return imeParityApp(appId) ? "awaiting_observed_field" : "checking_app_policy";
}

std::string_view editingUnavailableReason(std::string_view appId, bool fieldObserved) {
    if (!imeParityApp(appId)) return "editor_transaction_unavailable";
    return fieldObserved ? "ime_parity_target_mismatch" : "ime_parity_unobserved";
}

std::optional<std::string_view> clearNoticeText(std::string_view reason) {
    if (reason == "no_suggestion") return "Badi has no continuation — try a longer phrase";
    if (reason == "provider_timeout" || reason == "provider_error")
        return "Badi could not finish — press Tab to retry";
    return std::nullopt;
}

bool matchesCapturedContext(
    const std::optional<ContextUpdate> &captured,
    const std::optional<ContextWindow> &current) {
    return captured && current && captured->context == *current;
}

bool claimInvokeChord(bool editingAvailable, bool policyAllowed, bool foreignIme) {
    return editingAvailable && policyAllowed && !foreignIme;
}

LocalAction decideLocalAction(bool invokeChord, bool acceptChord, bool hasLiveOwnedCandidate,
                              const PanelObservation &panel) {
    if (hasForeignImeUi(panel)) return LocalAction::PassThrough;
    if (invokeChord) return LocalAction::Invoke;
    if (acceptChord && hasLiveOwnedCandidate && hasOwnedCandidate(panel)) return LocalAction::AcceptAll;
    return LocalAction::PassThrough;
}

TabAction decideTabAction(bool eligibleContext, bool hasLiveOwnedCandidate,
                          const PanelObservation &panel, NativeEditPath path) {
    if (hasForeignImeUi(panel)) return TabAction::PassThrough;
    if (hasLiveOwnedCandidate && hasOwnedCandidate(panel)) return TabAction::AcceptWord;
    // Observed fields suggest on their own, so Tab without a suggestion keeps
    // its meaning (next field, indent); requesting there takes the chord.
    const bool tabRequests = eligibleContext && path == NativeEditPath::Manual;
    return tabRequests ? TabAction::Invoke : TabAction::PassThrough;
}

bool manualTabRequests(NativeEditPath path, bool editingAvailable, bool policyAllowed,
                       const std::optional<ContextWindow> &context) {
    return path == NativeEditPath::Manual && editingAvailable && policyAllowed && tabEligibleContext(context);
}

PreKeyAction decidePreKey(const PreKey &key, bool editingAvailable, bool noticeShown,
                          bool suggestionVisible, const PanelObservation &panel) {
    // Type-through only shortens the next inspection; the key always reaches
    // the app. Its own edit retires the observed field until the observer
    // binds it again, so a fast next key finds no edit path yet.
    const bool typingThrough = key.typesSuggestion && !key.repeat && !hasForeignImeUi(panel);
    // The literal-Tab chord is a real Tab wherever Badi would take Tab
    // (a suggestion shows, a Tab waits for its remainder, the manual path
    // requests); elsewhere it stays the app's key (a browser's previous tab).
    if (key.literalTab && !hasForeignImeUi(panel) && (suggestionVisible || key.tabTaken)) {
        return PreKeyAction::LiteralTab;
    }
    if (!editingAvailable || key.repeat) {
        if (key.modifier) return PreKeyAction::PassThrough;
        return typingThrough ? PreKeyAction::CancelTypingThrough : PreKeyAction::Cancel;
    }
    if (key.tab) return PreKeyAction::Tab;
    if (key.escape && noticeShown && !suggestionVisible && !hasForeignImeUi(panel)) {
        return PreKeyAction::CloseNotice;
    }
    // Badi's own live candidate: Escape dismisses it and Ctrl+Right takes it.
    const bool ownedUi = suggestionVisible && panel.candidatesOwnedByBadi && hasOwnedCandidate(panel);
    if (key.escape && ownedUi) return PreKeyAction::Dismiss;
    if (key.all && ownedUi) return PreKeyAction::AcceptAll;
    if (key.modifier || key.chord) return PreKeyAction::PassThrough;
    if (key.escape) return PreKeyAction::CancelDeclining;
    // The candidate may already be hidden by an earlier typed-through key.
    return typingThrough ? PreKeyAction::CancelTypingThrough : PreKeyAction::Cancel;
}

bool typesSuggestionStart(std::string_view typed, std::string_view suggestion) {
    return !typed.empty() && typed.size() < suggestion.size() && suggestion.starts_with(typed);
}

bool printableKey(std::string_view typed) {
    return !typed.empty() && static_cast<unsigned char>(typed[0]) >= 0x20 && typed != "\x7f";
}

std::optional<std::string> tracedKeyText(std::string_view typed, bool fieldReadable) {
    if (!fieldReadable || !printableKey(typed)) return std::nullopt;
    return std::string(typed);
}

bool tabMayHoldForRest(bool restPending, bool restAutomatic, bool foreignIme,
                       bool suggestionVisible) {
    return restPending && restAutomatic && !foreignIme && !suggestionVisible;
}

bool heldTabMatchesRest(std::string_view rest, const Suggestion &suggestion) {
    return !rest.empty() && !suggestion.replaceBefore && !suggestion.acceptWord.empty() &&
           suggestion.text.starts_with(suggestion.acceptWord) &&
           rest.starts_with(suggestion.acceptWord);
}

bool isLiteralTabChord(const ::fcitx::Key &key) {
    const ::fcitx::KeyStates modifiers{::fcitx::KeyState::Ctrl, ::fcitx::KeyState::Alt, ::fcitx::KeyState::Shift,
                                       ::fcitx::KeyState::Super};
    return (key.sym() == FcitxKey_Tab || key.sym() == FcitxKey_ISO_Left_Tab) &&
        (key.states() & modifiers) == literalTabChord().states();
}

bool lastModifierReleased(const ::fcitx::Key &release) {
    if (!release.isModifier()) return false;
    const ::fcitx::KeyStates held{::fcitx::KeyState::Ctrl, ::fcitx::KeyState::Alt, ::fcitx::KeyState::Shift,
                                  ::fcitx::KeyState::Super};
    const auto remaining = release.states() & held & ~::fcitx::Key::keySymToStates(release.sym());
    return !remaining.testAny(held);
}

CandidateParts candidateParts(const Suggestion &suggestion) {
    const auto first = suggestion.text.find_first_not_of(' ');
    if (first == std::string::npos) return {};
    const std::string_view text = std::string_view(suggestion.text).substr(first);
    const auto word = std::string_view(suggestion.acceptWord).substr(
        std::min(first, suggestion.acceptWord.size()));
    if (suggestion.replaceBefore) {
        const auto trimmed = [](std::string_view value) {
            const auto first = value.find_first_not_of(' ');
            if (first == std::string_view::npos) return std::string_view{};
            return value.substr(first, value.find_last_not_of(' ') - first + 1);
        };
        const auto typo = trimmed(*suggestion.replaceBefore);
        return {typo.empty() ? std::string() : std::string(typo) + " \u2192 ", std::string(trimmed(text)), {}};
    }
    if (word.empty() || !text.starts_with(word)) return {{}, std::string(text), {}};
    return {{}, std::string(word), std::string(text.substr(word.size()))};
}

std::string previewText(const Suggestion &suggestion) {
    if (!suggestion.replaceBefore) return suggestion.text;
    return "\u2192 " + candidateParts(suggestion).strong;
}

std::size_t previewStrong(const Suggestion &suggestion) {
    if (suggestion.replaceBefore || suggestion.acceptWord.empty() ||
        suggestion.acceptWord.size() >= suggestion.text.size() ||
        !std::string_view(suggestion.text).starts_with(suggestion.acceptWord)) return 0;
    const auto scalars = decodeUtf8(suggestion.acceptWord);
    return scalars ? scalars->size() : 0;
}

bool typeThroughContextReady(std::string_view base, std::string_view typed, std::string_view before) {
    constexpr std::size_t kSlideBytes = 128;
    if (typed.empty() || typed.size() > before.size()) return false;
    if (!before.ends_with(typed)) return false;
    const auto stem = before.substr(0, before.size() - typed.size());
    if (stem == base) return true;
    if (base.size() < kSlideBytes || stem.size() < kSlideBytes) return false;
    return stem.ends_with(base.substr(base.size() - kSlideBytes));
}

InvokeRoute routeInvoke(std::string_view appId, bool editingAvailable, bool fieldObserved) {
    if (editingAvailable) return fieldObserved ? InvokeRoute::ObservedField : InvokeRoute::Manual;
    return imeParityApp(appId) && !fieldObserved ? InvokeRoute::InspectField : InvokeRoute::Unavailable;
}

std::optional<std::string_view> manualRequestNotice(const RequestFacts &facts) {
    if (!facts.connected) return notice::kReconnecting;
    if (!facts.policyKnown) return notice::kCheckingPermission;
    if (!facts.policyAllowed) {
        return facts.paused ? notice::kPaused : facts.snoozed ? notice::kSnoozed : notice::kDisabled;
    }
    if (!facts.fresh) return notice::kNeedsFreshContext;
    return std::nullopt;
}

std::optional<std::string_view> inspectionNotice(const RequestFacts &facts) {
    if (!facts.fresh) return notice::kNeedsFreshContext;
    if (!facts.fieldAllowed) return notice::kFieldUnreadable;
    return std::nullopt;
}

std::string_view tabDecisionReason(const RequestFacts &facts,
                                   const std::optional<ContextWindow> &context) {
    if (!facts.policyKnown) return "checking_app_policy";
    if (!facts.policyAllowed) return "app_disabled";
    if (facts.foreignIme) return "foreign_ime";
    if (!facts.fieldAllowed) return "field_denied";
    if (!facts.fresh) return "no_fresh_context";
    if (!context) return "context_unavailable";
    if (!context->after.empty()) return "caret_not_at_end";
    if (!supportedWritingLanguage(context->language)) return "language_unsupported";
    return tabEligibleContext(context) ? "eligible" : "empty_context";
}

bool tabEligibleContext(const std::optional<ContextWindow> &context) {
    return context && context->after.empty() &&
           supportedWritingLanguage(context->language) &&
           context->before.find_first_not_of(" \t\r\n") != std::string::npos;
}

bool supportedWritingLanguage(std::string_view language) {
    const auto primary = language.substr(0, language.find('-'));
    return validLanguageTag(language) && (primary == "en" || primary == "de" || primary == "fa");
}

std::optional<ContextWindow> captureContextWindow(std::string_view text,
                                                  std::size_t cursor,
                                                  std::size_t anchor,
                                                  bool multiline,
                                                  std::string language) {
    // Do not inspect or copy text that policy cannot serialize.
    if (cursor != anchor || text.size() > kMaxContextSourceBytes || !validLanguageTag(language)) {
        return std::nullopt;
    }
    const auto window = scalarWindow(text, cursor, kMaxBeforeScalars, kMaxAfterScalars);
    if (!window || !validContextText(window->before) || !validContextText(window->after)) return std::nullopt;
    ContextWindow result;
    result.before = std::string(window->before);
    result.after = std::string(window->after);
    result.anchor = anchor;
    result.head = cursor;
    result.language = std::move(language);
    result.multiline = multiline;
    return result;
}

void normalizeObservedParagraphEnd(ContextWindow &context) {
    if (!context.paragraphEnd.empty() || (context.after != "\n\n" && context.after != "\n")) return;
    context.paragraphEnd = std::exchange(context.after, {});
}

std::string observedAfter(const ContextWindow &context) {
    return context.paragraphEnd.empty() ? context.after : context.paragraphEnd;
}

LineEdit lineEditFor(const ::fcitx::Key &key, std::string_view typed) {
    const auto states = key.states();
    const bool ctrl = !!(states & ::fcitx::KeyState::Ctrl);
    const bool alt = !!(states & ::fcitx::KeyState::Alt);
    const auto sym = key.sym();
    // Fcitx may report a Ctrl letter in either case.
    const auto letter = [sym](char lower) {
        return sym == static_cast<::fcitx::KeySym>(lower) || sym == static_cast<::fcitx::KeySym>(lower - 32);
    };
    if (ctrl && !alt) {
        if (letter('c') || letter('u')) return LineEdit::Clear;
        if (letter('w')) return LineEdit::EraseWord;
        if (letter('d')) return LineEdit::DeleteForward;
        if (letter('k')) return LineEdit::KillAfter;
        if (letter('b')) return LineEdit::Left;
        if (letter('f')) return LineEdit::Right;
        if (letter('a')) return LineEdit::Home;
        if (letter('e')) return LineEdit::End;
    }
    switch (sym) {
    case FcitxKey_Return: case FcitxKey_KP_Enter:
        return LineEdit::Clear;
    case FcitxKey_BackSpace:
        return ctrl || alt ? LineEdit::EraseWord : LineEdit::Erase;
    case FcitxKey_Delete: case FcitxKey_KP_Delete:
        return ctrl || alt ? LineEdit::Lose : LineEdit::DeleteForward;
    case FcitxKey_Left: case FcitxKey_KP_Left:
        return ctrl || alt ? LineEdit::Lose : LineEdit::Left;
    case FcitxKey_Right: case FcitxKey_KP_Right:
        return ctrl || alt ? LineEdit::Lose : LineEdit::Right;
    case FcitxKey_Home: case FcitxKey_KP_Home:
        return LineEdit::Home;
    case FcitxKey_End: case FcitxKey_KP_End:
        return LineEdit::End;
    case FcitxKey_Up: case FcitxKey_Down: case FcitxKey_KP_Up: case FcitxKey_KP_Down:
    case FcitxKey_Page_Up: case FcitxKey_Page_Down:
        return LineEdit::Lose;
    default:
        break;
    }
    // Readline's word motion and deletion.
    if (alt && !ctrl && (letter('b') || letter('f') || letter('d'))) return LineEdit::Lose;
    if (printableKey(typed)) return LineEdit::Type;
    return LineEdit::Restart;
}

void TypedLine::apply(LineEdit edit, std::string_view typed) {
    if (edit == LineEdit::Clear) return clear();
    if (edit == LineEdit::Restart) return clear();
    if (edit == LineEdit::Lose) return lose();
    if (!tracking_) {
        // End puts the caret back at the end of whatever the line holds.
        if (edit == LineEdit::End) clear();
        return;
    }
    switch (edit) {
    case LineEdit::Type: return type(typed);
    case LineEdit::Erase: return erase();
    case LineEdit::EraseWord: return eraseWord();
    case LineEdit::DeleteForward:
        // Ctrl+D on an empty line ends the input instead.
        if (text_.empty()) return clear();
        if (caret_ < text_.size()) {
            auto end = caret_ + 1;
            while (end < text_.size() && utf8Continuation(text_[end])) ++end;
            text_.erase(caret_, end - caret_);
        }
        return;
    case LineEdit::KillAfter: text_.erase(caret_); return;
    case LineEdit::Left: if (!left()) lose(); return;
    case LineEdit::Right: if (!right()) lose(); return;
    case LineEdit::Home: caret_ = 0; return;
    case LineEdit::End: caret_ = text_.size(); return;
    case LineEdit::Clear: case LineEdit::Restart: case LineEdit::Lose: return;
    }
}

void TypedLine::type(std::string_view text) {
    if (!tracking_) return;
    text_.insert(caret_, text);
    caret_ += text.size();
    // Keep whole scalars of the last context's worth of bytes.
    constexpr std::size_t limit = kMaxBeforeScalars * 4;
    if (text_.size() > limit) {
        auto start = text_.size() - limit;
        while (start < text_.size() && utf8Continuation(text_[start])) ++start;
        text_.erase(0, start);
        caret_ = caret_ > start ? caret_ - start : 0;
    }
}

void TypedLine::erase(std::size_t count) {
    if (!tracking_) return;
    for (; count > 0 && caret_ > 0; --count) {
        auto start = caret_ - 1;
        while (start > 0 && utf8Continuation(text_[start])) --start;
        text_.erase(start, caret_ - start);
        caret_ = start;
    }
}

void TypedLine::eraseWord() {
    const auto head = before();
    auto end = head.find_last_not_of(' ');
    std::size_t start = 0;
    if (end != std::string_view::npos) {
        const auto space = head.find_last_of(' ', end);
        start = space == std::string_view::npos ? 0 : space + 1;
    }
    text_.erase(start, caret_ - start);
    caret_ = start;
}

bool TypedLine::left() {
    if (caret_ == 0) return false;
    do --caret_; while (caret_ > 0 && utf8Continuation(text_[caret_]));
    return true;
}

bool TypedLine::right() {
    if (caret_ >= text_.size()) return false;
    do ++caret_; while (caret_ < text_.size() && utf8Continuation(text_[caret_]));
    return true;
}

void TypedLine::clear() {
    text_.clear();
    caret_ = 0;
    tracking_ = true;
}

void TypedLine::lose() {
    text_.clear();
    caret_ = 0;
    tracking_ = false;
}

std::size_t TypedLine::caretScalars() const {
    const auto head = before();
    return static_cast<std::size_t>(std::count_if(head.begin(), head.end(),
                                                  [](char byte) { return !utf8Continuation(byte); }));
}

bool TypedLine::prose() const {
    const auto head = before();
    const auto first = head.find_first_not_of(' ');
    if (first == std::string_view::npos) return false;
    const auto gap = head.find(' ', first);
    return gap != std::string_view::npos && head.find_first_not_of(' ', gap) != std::string_view::npos &&
           std::any_of(head.begin(), head.end(), [](unsigned char byte) {
               return std::isalpha(byte) || byte >= 0x80U;
           });
}

bool SessionState::focusIn(std::string sessionId, std::string targetId,
                           std::string appId, std::string fingerprintSalt,
                           NativeEditTarget target, NativeEditPath path) {
    if (!validOpaqueId(targetId) || !validLinuxAppId(appId) || !validSessionId(sessionId) ||
        fingerprintSalt.size() < 16 || !validOpaqueId(fingerprintSalt)) {
        focusOut();
        return false;
    }
    coordinates_.sessionId = std::move(sessionId);
    targetId_ = std::move(targetId);
    appId_ = std::move(appId);
    fingerprintSalt_ = std::move(fingerprintSalt);
    editTarget_ = target;
    editPath_ = path;
    coordinates_.focusEpoch =
        coordinates_.focusEpoch >= kMaxSafeCounter ? 1 : coordinates_.focusEpoch + 1;
    coordinates_.revision = 0;
    coordinates_.fingerprint.clear();
    focused_ = true;
    lastContext_.reset();
    clearSuggestion();
    clearRevision();
    return true;
}

void SessionState::focusOut() {
    focused_ = false;
    lastContext_.reset();
    clearSuggestion();
    clearRevision();
    coordinates_ = {};
    appId_.clear();
    targetId_.clear();
    fingerprintSalt_.clear();
    editTarget_ = NativeEditTarget::Unsupported;
    editPath_ = NativeEditPath::Manual;
}

void SessionState::denyEditing() {
    editTarget_ = NativeEditTarget::Unsupported;
    invalidateContext();
}

bool SessionState::retireObservation(std::string sessionId, std::string targetId, std::string fingerprintSalt) {
    const auto appId = appId_;
    return focusIn(std::move(sessionId), std::move(targetId), appId, std::move(fingerprintSalt),
                   NativeEditTarget::DesktopApplication, NativeEditPath::Manual);
}

std::string suggestRequestId(const Coordinates &coordinates) {
    return "fcitx.suggest." + std::to_string(coordinates.focusEpoch) + "." + std::to_string(coordinates.revision);
}

RefusedRequest SessionState::refuseRequest(std::string_view requestId) {
    if (!focused_ || requestId.empty()) return RefusedRequest::None;
    if (pendingAcceptance_ && pendingAcceptance_->controlId == requestId) {
        clearSuggestion();
        return RefusedRequest::Acceptance;
    }
    if ((revisionAcceptance_ && revisionAcceptance_->controlId == requestId) ||
        (revisionRequest_ && revisionRequest_->requestId == requestId)) {
        clearRevision();
        return RefusedRequest::Revision;
    }
    // The context stays: only a new edit or an explicit request asks again.
    if (lastContext_ && requestId == suggestRequestId(lastContext_->coordinates)) return RefusedRequest::Suggestion;
    return RefusedRequest::None;
}

void SessionState::invalidateContext() {
    if (!focused_) return;
    coordinates_.revision = coordinates_.revision >= kMaxSafeCounter
                                ? 1
                                : coordinates_.revision + 1;
    coordinates_.fingerprint.clear();
    lastContext_.reset();
    clearSuggestion();
    clearRevision();
}

std::optional<ContextUpdate>
SessionState::updateContext(ContextWindow context) {
    if (!focused_ || !editingAvailable()) return std::nullopt;
    // IME-parity context is always corroborated by the observer; the unknown
    // identity manual contract is limited to native exact apps.
    // The paragraph-end normalization is the observed IME-parity rule.
    if ((imeParityApp(appId_) && !context.identityKnown) ||
        (!context.paragraphEnd.empty() && (!imeParityApp(appId_) || !context.identityKnown ||
                                    editPath_ != NativeEditPath::Observed || !context.after.empty())) ||
        context.anchor != context.head ||
        !validLanguageTag(context.language) || !validContextText(context.before) || !validContextText(context.after)) {
        invalidateContext();
        return std::nullopt;
    }
    coordinates_.revision = coordinates_.revision >= kMaxSafeCounter
                                ? 1
                                : coordinates_.revision + 1;
    coordinates_.fingerprint = nextFingerprint(context);
    clearSuggestion();
    clearRevision();
    ContextUpdate update{
        .coordinates = coordinates_,
        .context = std::move(context),
        .appId = appId_,
        .targetId = targetId_,
    };
    lastContext_ = update;
    return update;
}

bool SessionState::showSuggestion(Suggestion suggestion, std::uint64_t nowMs) {
    if (!editingAvailable()) {
        clearSuggestion();
        return false;
    }
    // A streamed extension cannot change the exact text already accepted.
    if (pendingAcceptance_) return false;
    const auto clean = sanitizeSuggestion(suggestion.text);
    if (!focused_ || !clean || suggestion.expiresAtMs <= nowMs ||
        suggestion.expiresAtMs > kMaxSafeCounter ||
        !validOpaqueId(suggestion.requestId) ||
        !validOpaqueId(suggestion.suggestionId) ||
        suggestion.coordinates != coordinates_) {
        return false;
    }
    const auto word = sanitizeSuggestion(suggestion.acceptWord);
    suggestion.acceptWord = word && clean->starts_with(*word) ? *word : *clean;
    suggestion.text = *clean;
    if (suggestion.replaceBefore) {
        const auto saved = lastContext();
        if ((imeParityApp(appId_) && !correctsAfterChecks(appId_)) || !saved ||
            !saved->context.before.ends_with(*suggestion.replaceBefore)) {
            clearSuggestion();
            return false;
        }
    }
    alternativeList_.reset();
    visible_ = std::move(suggestion);
    pendingAcceptance_.reset();
    return true;
}

std::optional<std::string> alternativeListPick(const std::vector<std::string> &words, char digit) {
    if (digit < '1' || digit > '9') return std::nullopt;
    const auto index = static_cast<std::size_t>(digit - '1');
    if (index >= words.size()) return std::nullopt;
    return words[index];
}

bool SessionState::showAlternativeList(AlternativeList list, std::uint64_t nowMs) {
    if (!focused_ || !editingAvailable() || pendingAcceptance_ || list.words.empty() ||
        list.words.size() > 4 || list.expiresAtMs <= nowMs ||
        list.expiresAtMs > kMaxSafeCounter || !validOpaqueId(list.requestId) ||
        !validOpaqueId(list.suggestionId) || list.coordinates != coordinates_ ||
        !visible_ || visible_->suggestionId != list.suggestionId) {
        return false;
    }
    std::vector<std::string> words;
    for (const auto &word : list.words) {
        const auto clean = sanitizeSuggestion(word);
        if (!clean || clean->starts_with(' ') || clean->find(' ') != std::string::npos ||
            std::find(words.begin(), words.end(), *clean) != words.end()) {
            return false;
        }
        words.push_back(*clean);
    }
    list.words = std::move(words);
    alternativeList_ = std::move(list);
    // The broker retired the shown suggestion when it listed its words.
    visible_.reset();
    return true;
}

std::optional<AlternativePickRequest>
SessionState::requestAlternativePick(std::string word, std::uint64_t nowMs,
                                      const PanelObservation &panel) {
    if (!editingAvailable() || !hasOwnedCandidate(panel) || pendingAcceptance_ ||
        !alternativeList_ || alternativeList_->expiresAtMs <= nowMs ||
        alternativeList_->coordinates != coordinates_ ||
        std::find(alternativeList_->words.begin(), alternativeList_->words.end(), word) ==
            alternativeList_->words.end()) {
        if (alternativeList_ && alternativeList_->expiresAtMs <= nowMs) clearAlternativeList();
        return std::nullopt;
    }
    AlternativePickRequest request{
        .coordinates = alternativeList_->coordinates,
        .controlId = "fcitx.alternative." +
                     std::to_string(alternativeList_->coordinates.focusEpoch) + "." +
                     std::to_string(alternativeList_->coordinates.revision) + "." +
                     std::to_string(++alternativeSerial_),
        .suggestionId = alternativeList_->suggestionId,
        .word = std::move(word),
    };
    clearAlternativeList();
    return request;
}

void SessionState::clearAlternativeList() { alternativeList_.reset(); }

std::optional<AcceptRequest>
SessionState::requestAcceptance(std::uint64_t nowMs,
                               const PanelObservation &panel, bool word) {
    if (!editingAvailable() || !hasOwnedCandidate(panel)) {
        clearSuggestion();
        return std::nullopt;
    }
    if (!focused_ || pendingAcceptance_ || !visible_ || visible_->expiresAtMs <= nowMs ||
        visible_->coordinates != coordinates_) {
        if (visible_ && visible_->expiresAtMs <= nowMs) clearSuggestion();
        return std::nullopt;
    }
    AcceptRequest request{
        .coordinates = visible_->coordinates,
        .controlId = "fcitx.accept." +
                     std::to_string(visible_->coordinates.focusEpoch) + "." +
                     std::to_string(visible_->coordinates.revision),
        .suggestionId = visible_->suggestionId,
        .expectedText = word ? visible_->acceptWord : visible_->text,
        .acceptance = word ? "word" : "all",
    };
    pendingAcceptance_ = request;
    return request;
}

std::optional<DismissRequest>
SessionState::requestDismissal(std::uint64_t nowMs,
                               const PanelObservation &panel) {
    if (!hasOwnedCandidate(panel)) {
        clearSuggestion();
        return std::nullopt;
    }
    if (!focused_ || !visible_ || visible_->expiresAtMs <= nowMs ||
        visible_->coordinates != coordinates_) {
        if (visible_ && visible_->expiresAtMs <= nowMs) clearSuggestion();
        return std::nullopt;
    }
    DismissRequest request{
        .coordinates = visible_->coordinates,
        .controlId = "fcitx.dismiss." +
                     std::to_string(visible_->coordinates.focusEpoch) + "." +
                     std::to_string(visible_->coordinates.revision),
        .suggestionId = visible_->suggestionId,
    };
    clearSuggestion();
    return request;
}

std::optional<DismissRequest>
SessionState::requestAlternative(std::uint64_t nowMs, const PanelObservation &panel) {
    if (!editingAvailable() || !hasOwnedCandidate(panel) || pendingAcceptance_ || !visible_ ||
        visible_->replaceBefore || visible_->expiresAtMs <= nowMs || visible_->coordinates != coordinates_) {
        return std::nullopt;
    }
    return DismissRequest{
        .coordinates = visible_->coordinates,
        .controlId = "fcitx.alternative." + std::to_string(visible_->coordinates.focusEpoch) + "." +
                     std::to_string(visible_->coordinates.revision) + "." + std::to_string(++alternativeSerial_),
        .suggestionId = visible_->suggestionId,
    };
}

std::optional<CommitDispatch>
SessionState::authorizeCommit(const CommitPrepare &prepare,
                              std::uint64_t nowMs,
                              const PanelObservation &panel) {
    if (!editingAvailable() || !hasOwnedCandidate(panel)) {
        clearSuggestion();
        return std::nullopt;
    }
    if (!focused_ || !visible_ || !pendingAcceptance_ ||
        visible_->expiresAtMs <= nowMs ||
        prepare.coordinates != coordinates_ ||
        prepare.coordinates != pendingAcceptance_->coordinates ||
        prepare.controlId != pendingAcceptance_->controlId ||
        prepare.suggestionId != pendingAcceptance_->suggestionId ||
        prepare.text != pendingAcceptance_->expectedText ||
        prepare.acceptance != pendingAcceptance_->acceptance ||
        prepare.replaceBefore != visible_->replaceBefore) {
        return std::nullopt;
    }
    CommitDispatch dispatch{
        .coordinates = prepare.coordinates,
        .controlId = prepare.controlId,
        .suggestionId = prepare.suggestionId,
        .text = prepare.text,
        .replaceBefore = prepare.replaceBefore,
    };
    clearSuggestion();
    return dispatch;
}

bool SessionState::clearSuggestionIf(
    const Coordinates &coordinates,
    const std::optional<std::string> &suggestionId) {
    if (!visible_ || coordinates != visible_->coordinates ||
        (suggestionId && *suggestionId != visible_->suggestionId)) {
        return false;
    }
    clearSuggestion();
    return true;
}

void SessionState::clearSuggestion() {
    visible_.reset();
    pendingAcceptance_.reset();
    alternativeList_.reset();
}

std::optional<RevisionUpdate> SessionState::requestRevision(SelectionWindow selection, std::string mode) {
    if (!focused_ || !editingAvailable() ||
        (mode != "grammar" && mode != "refine" && mode != "synonyms") ||
        (imeParityApp(appId_) && !selection.identityKnown) || !validLanguageTag(selection.language) ||
        !safeRevisionText(selection.selected, kMaxSelectedScalars) || !validContextText(selection.before) ||
        !validContextText(selection.after) || selection.anchor == selection.head) {
        invalidateContext();
        return std::nullopt;
    }
    coordinates_.revision = coordinates_.revision >= kMaxSafeCounter ? 1 : coordinates_.revision + 1;
    coordinates_.fingerprint = fingerprintOf("revision\x1f" + selection.before + "\x1e" + selection.selected +
                                             "\x1e" + selection.after + "\x1f" + selection.language + "\x1f" +
                                             std::to_string(selection.anchor) + ":" +
                                             std::to_string(selection.head));
    lastContext_.reset();
    clearSuggestion();
    clearRevision();
    revisionRequest_ = RevisionUpdate{
        .coordinates = coordinates_,
        .requestId = "fcitx.revise." + std::to_string(coordinates_.focusEpoch) + "." +
                     std::to_string(coordinates_.revision),
        .mode = std::move(mode),
        .selection = std::move(selection),
        .appId = appId_,
        .targetId = targetId_,
    };
    return revisionRequest_;
}

bool SessionState::showRevision(Revision revision, std::uint64_t nowMs) {
    if (!focused_ || !editingAvailable() || !revisionRequest_ || revision_ ||
        revision.coordinates != revisionRequest_->coordinates || revision.coordinates != coordinates_ ||
        revision.requestId != revisionRequest_->requestId || revision.mode != revisionRequest_->mode ||
        revision.original != revisionRequest_->selection.selected || revision.text == revision.original ||
        !safeRevisionText(revision.text, kMaxRevisedScalars) || !validOpaqueId(revision.revisionId) ||
        revision.expiresAtMs <= nowMs || revision.expiresAtMs > kMaxSafeCounter) {
        return false;
    }
    revision_ = std::move(revision);
    return true;
}

std::optional<AcceptRequest> SessionState::requestRevisionAcceptance(std::uint64_t nowMs,
                                                                     const PanelObservation &panel) {
    if (!focused_ || !editingAvailable() || !revision_ || revisionAcceptance_ || !hasOwnedCandidate(panel) ||
        revision_->coordinates != coordinates_) {
        return std::nullopt;
    }
    if (revision_->expiresAtMs <= nowMs) {
        clearRevision();
        return std::nullopt;
    }
    revisionAcceptance_ = AcceptRequest{
        .coordinates = revision_->coordinates,
        .controlId = "fcitx.revise.accept." + std::to_string(revision_->coordinates.focusEpoch) + "." +
                     std::to_string(revision_->coordinates.revision),
        .suggestionId = revision_->revisionId,
        .expectedText = revision_->text,
        .acceptance = "all",
    };
    return revisionAcceptance_;
}

std::optional<DismissRequest> SessionState::requestRevisionDismissal(std::uint64_t nowMs,
                                                                     const PanelObservation &panel) {
    if (!focused_ || !revision_ || revisionAcceptance_ || revision_->coordinates != coordinates_ ||
        revision_->expiresAtMs <= nowMs || hasForeignImeUi(panel)) {
        clearRevision();
        return std::nullopt;
    }
    DismissRequest request{
        .coordinates = revision_->coordinates,
        .controlId = "fcitx.revise.dismiss." + std::to_string(revision_->coordinates.focusEpoch) + "." +
                     std::to_string(revision_->coordinates.revision),
        .suggestionId = revision_->revisionId,
    };
    clearRevision();
    return request;
}

std::optional<CommitDispatch> SessionState::authorizeRevisionCommit(const CommitPrepare &prepare,
                                                                    std::uint64_t nowMs,
                                                                    const PanelObservation &panel) {
    if (!focused_ || !editingAvailable() || !revision_ || !revisionRequest_ || !revisionAcceptance_ ||
        !hasOwnedCandidate(panel) || revision_->expiresAtMs <= nowMs || prepare.coordinates != coordinates_ ||
        prepare.coordinates != revisionAcceptance_->coordinates ||
        prepare.controlId != revisionAcceptance_->controlId ||
        prepare.suggestionId != revisionAcceptance_->suggestionId || prepare.text != revision_->text ||
        prepare.acceptance != "all" || prepare.replaceBefore ||
        prepare.replaceSelection != SelectionRange{revisionRequest_->selection.anchor,
                                                   revisionRequest_->selection.head}) {
        return std::nullopt;
    }
    CommitDispatch dispatch{
        .coordinates = prepare.coordinates,
        .controlId = prepare.controlId,
        .suggestionId = prepare.suggestionId,
        .text = prepare.text,
        .replaceSelection = prepare.replaceSelection,
    };
    clearRevision();
    return dispatch;
}

bool SessionState::clearRevisionIf(const Coordinates &coordinates,
                                   const std::optional<std::string> &revisionId) {
    // Before a revision is shown its id is unknown here; the coordinates
    // name the request.
    if (!revisionRequest_ || coordinates != revisionRequest_->coordinates ||
        (revisionId && revision_ && *revisionId != revision_->revisionId)) {
        return false;
    }
    clearRevision();
    return true;
}

void SessionState::clearRevision() {
    revisionRequest_.reset();
    revision_.reset();
    revisionAcceptance_.reset();
}

std::string SessionState::nextFingerprint(const ContextWindow &context) const {
    return fingerprintOf(context.before + "\x1f" + observedAfter(context) + "\x1f" + context.language + "\x1f" +
                         std::to_string(context.anchor) + ":" + std::to_string(context.head));
}

std::string SessionState::fingerprintOf(std::string_view field) const {
    const auto material = fingerprintSalt_ + "\x1f" + coordinates_.sessionId +
                          "\x1f" + appId_ + "\x1f" + targetId_ + "\x1f" + std::string(field) + ":" +
                          std::to_string(coordinates_.focusEpoch) + ":" +
                          std::to_string(coordinates_.revision);
    const std::array hashes{
        mix(material, mix(fingerprintSalt_, 0xcbf29ce484222325ULL)),
        mix(material, mix(fingerprintSalt_, 0x9e3779b97f4a7c15ULL)),
    };
    std::ostringstream output;
    output << std::hex << std::setfill('0');
    for (const auto hash : hashes) output << std::setw(16) << hash;
    return output.str();
}

} // namespace badi::fcitx5
