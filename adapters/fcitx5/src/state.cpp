#include "state.h"

#include "sanitizer.h"

#include <algorithm>
#include <array>
#include <iomanip>
#include <limits>
#include <sstream>

namespace badi::fcitx5 {
namespace {

constexpr std::uint64_t kMaxSafeCounter = (std::uint64_t{1} << 53U) - 1U;
// Chromium's text-input serialization of a <p> block end (see state.h).
constexpr std::string_view kParagraphEnd = "\n\n";

std::uint64_t mix(std::string_view value, std::uint64_t seed) {
    auto hash = seed;
    for (const auto byte : value) {
        hash ^= static_cast<unsigned char>(byte);
        hash *= 0x100000001b3ULL;
    }
    return hash;
}

// IME-parity apps are exactly the accessibility observer's web-app rules
// (adapters/accessibility/daemon.py APPS); tests/observer-identities.py keeps
// the lists equal, because an id without a rule could never be observed.
constexpr std::array<std::string_view, 4> kImeParityBrowsers{
    "chromium", "chromium-browser", "brave-origin", "zen"};
constexpr std::array<std::string_view, 4> kImeParityDesktopApps{"chatgpt", "code", "cursor", "discord"};

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
    constexpr std::array<std::string_view, 23> prefixes{
        "chrome-", "crx_", "chromium-", "google-chrome", "brave-", "microsoft-edge", "msedge-",
        "vivaldi-", "opera-", "yandex-browser", "electron", "discord-", "com.google.chrome",
        "org.chromium.", "com.brave.", "com.microsoft.edge", "com.vivaldi.", "com.opera.",
        "io.github.ungoogled_software.", "com.visualstudio.code", "com.vscodium.", "com.discordapp.",
        "dev.vencord."};
    return std::find(exact.begin(), exact.end(), appId) != exact.end() ||
           std::any_of(prefixes.begin(), prefixes.end(),
                       [appId](std::string_view prefix) { return appId.starts_with(prefix); });
}

// Gecko-family browser identities other than the observed "zen": Firefox and
// its channels, Flatpak ids and PWAsForFirefox web-app windows (FFPWA-<id>),
// other Zen builds (Twilight, Flatpak), and Firefox forks. Their commitString
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

NativeAppClass classifyNativeApp(std::string_view appId) {
    // Browser/Electron commitString behaves like a typed keystroke: page script
    // can retarget it during beforeinput and undo may coalesce it with typing.
    // These apps therefore accept only through an observed field, append-only.
    // Zen is the one Gecko identity with an observer rule; its urlbar shares
    // the page's input context without a Url purpose, so the observer, not
    // allowsNativeContext(), keeps browser UI out.
    // Obsidian's editor plugin owns its fields, so a second integration would
    // double-suggest.
    constexpr std::array<std::string_view, 2> unavailable{"obsidian", "md.obsidian.obsidian"};
    const auto listed = [appId](const auto &ids) {
        return std::find(ids.begin(), ids.end(), appId) != ids.end();
    };
    if (!validLinuxAppId(appId) || listed(unavailable) || unobservedGeckoFamily(appId))
        return NativeAppClass::Unavailable;
    if (listed(kImeParityBrowsers)) return NativeAppClass::ImeParityBrowser;
    if (listed(kImeParityDesktopApps)) return NativeAppClass::ImeParityDesktop;
    if (unobservedChromiumFamily(appId)) return NativeAppClass::Unavailable;
    return NativeAppClass::NativeExact;
}

bool imeParityApp(std::string_view appId) {
    const auto kind = classifyNativeApp(appId);
    return kind == NativeAppClass::ImeParityBrowser || kind == NativeAppClass::ImeParityDesktop;
}

bool nativeObservationAvailable(std::string_view appId) {
    return classifyNativeApp(appId) != NativeAppClass::Unavailable;
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
    case NativeAppClass::Unavailable:
        return false;
    }
    return false;
}

std::string_view focusReason(std::string_view appId) {
    if (appId.empty()) return "unidentified_app";
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

LocalAction decideLocalAction(bool invokeChord, bool acceptChord,
                              bool escapeKey, bool hasLiveOwnedCandidate,
                              const PanelObservation &panel) {
    if (hasForeignImeUi(panel)) return LocalAction::PassThrough;
    if (invokeChord) return LocalAction::Invoke;
    if (!hasLiveOwnedCandidate || !hasOwnedCandidate(panel)) {
        return LocalAction::PassThrough;
    }
    if (acceptChord) return LocalAction::Accept;
    if (escapeKey) return LocalAction::Dismiss;
    return LocalAction::PassThrough;
}

LocalAction decideTabAction(bool eligibleContext, bool hasLiveOwnedCandidate,
                            const PanelObservation &panel, NativeEditPath path) {
    if (hasForeignImeUi(panel)) return LocalAction::PassThrough;
    if (hasLiveOwnedCandidate && hasOwnedCandidate(panel)) return LocalAction::Accept;
    // Observed fields suggest on their own, so Tab without a suggestion keeps
    // its meaning (next field, indent); requesting there takes the chord.
    const bool tabRequests = eligibleContext && path == NativeEditPath::Manual;
    return tabRequests ? LocalAction::Invoke : LocalAction::PassThrough;
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
    if (context.paragraphEndAfter || context.after != kParagraphEnd) return;
    context.after.clear();
    context.paragraphEndAfter = true;
}

std::string observedAfter(const ContextWindow &context) {
    return context.paragraphEndAfter ? std::string(kParagraphEnd) : context.after;
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
    return true;
}

void SessionState::focusOut() {
    focused_ = false;
    lastContext_.reset();
    clearSuggestion();
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

void SessionState::retireObservation() {
    editPath_ = NativeEditPath::Manual;
    invalidateContext();
}

void SessionState::invalidateContext() {
    if (!focused_) return;
    coordinates_.revision = coordinates_.revision >= kMaxSafeCounter
                                ? 1
                                : coordinates_.revision + 1;
    coordinates_.fingerprint.clear();
    lastContext_.reset();
    clearSuggestion();
}

std::optional<ContextUpdate>
SessionState::updateContext(ContextWindow context) {
    if (!focused_ || !editingAvailable()) return std::nullopt;
    // IME-parity context is always corroborated by the observer; the unknown
    // identity manual contract is limited to native exact apps.
    // The paragraph-end normalization is the observed IME-parity rule.
    if ((imeParityApp(appId_) && !context.identityKnown) ||
        (context.paragraphEndAfter && (!imeParityApp(appId_) || !context.identityKnown ||
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
    const auto clean = sanitizeSuggestion(suggestion.text);
    if (!focused_ || !clean || suggestion.expiresAtMs <= nowMs ||
        suggestion.expiresAtMs > kMaxSafeCounter ||
        !validOpaqueId(suggestion.requestId) ||
        !validOpaqueId(suggestion.suggestionId) ||
        suggestion.coordinates != coordinates_) {
        return false;
    }
    suggestion.text = *clean;
    visible_ = std::move(suggestion);
    pendingAcceptance_.reset();
    return true;
}

std::optional<AcceptRequest>
SessionState::requestAcceptance(std::uint64_t nowMs,
                               const PanelObservation &panel) {
    if (!editingAvailable() || !hasOwnedCandidate(panel)) {
        clearSuggestion();
        return std::nullopt;
    }
    if (!focused_ || hasForeignImeUi(panel) ||
        pendingAcceptance_ || !visible_ || visible_->expiresAtMs <= nowMs ||
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
        .expectedText = visible_->text,
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
    if (!focused_ || hasForeignImeUi(panel) || !visible_ ||
        visible_->expiresAtMs <= nowMs ||
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
        prepare.acceptance != "all") {
        return std::nullopt;
    }
    CommitDispatch dispatch{
        .coordinates = prepare.coordinates,
        .controlId = prepare.controlId,
        .suggestionId = prepare.suggestionId,
        .text = prepare.text,
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
}

std::string SessionState::nextFingerprint(const ContextWindow &context) const {
    const auto material = fingerprintSalt_ + "\x1f" + coordinates_.sessionId +
                          "\x1f" + appId_ + "\x1f" + targetId_ + "\x1f" +
                          context.before + "\x1f" + observedAfter(context) + "\x1f" +
                          context.language + "\x1f" +
                          std::to_string(context.anchor) + ":" +
                          std::to_string(context.head) + ":" +
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
