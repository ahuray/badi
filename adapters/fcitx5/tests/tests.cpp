#include "sanitizer.h"
#include "state.h"
#include "transport.h"

#include <fcitx-utils/key.h>
#include <nlohmann/json.hpp>

#include <algorithm>
#include <array>
#include <cstdint>
#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>

namespace {

using namespace badi::fcitx5;
constexpr auto kSession = "550e8400-e29b-41d4-a716-446655440000";
constexpr auto kSalt = "0123456789abcdef0123456789abcdef";

void check(bool condition, const char *message) {
    if (!condition) throw std::runtime_error(message);
}

SessionState focusedState(std::string_view appId = "omawrite",
                          std::string_view salt = kSalt) {
    SessionState state;
    check(state.focusIn(kSession, "input-context-1", std::string(appId),
                        std::string(salt)),
          "focus should be accepted");
    return state;
}

ContextUpdate explicitContext(SessionState &state, std::string before = "thank you") {
    const auto update = state.updateContext(ContextWindow{
        .before = std::move(before),
        .after = "",
        .anchor = 9,
        .head = 9,
        .language = "en",
        .sensitive = false,
        .multiline = true,
        .composing = false,
    });
    check(update.has_value(), "explicit context should be accepted");
    return *update;
}

Suggestion suggestionFor(const ContextUpdate &update, std::uint64_t expiresAt = 500) {
    return Suggestion{
        .coordinates = update.coordinates,
        .requestId = "fcitx.suggest.1.1",
        .suggestionId = "suggestion-1",
        .text = " for your time",
        .expiresAtMs = expiresAt,
    };
}

void stateTransitionsAndIdentity() {
    check(supportedAppId("omawrite"), "Omawrite must be supported");
    check(supportedAppId("com.github.xournalpp.xournalpp"),
          "canonical Xournal++ id must be supported");
    check(!supportedAppId("xournalpp"), "noncanonical Xournal++ id must fail");

    auto state = focusedState();
    check(state.focused() && state.coordinates().focusEpoch == 1 &&
              state.coordinates().revision == 0,
          "focus must establish epoch one");
    const auto update = explicitContext(state);
    check(update.coordinates.revision == 1 &&
              update.coordinates.fingerprint.size() == 32,
          "context must advance a revision with bounded fingerprint");
    state.invalidateContext();
    check(state.coordinates().revision == 2 && !state.lastContext(),
          "ambient updates must only invalidate local state");
    state.focusOut();
    check(!state.focused() && state.appId().empty() && state.targetId().empty() &&
              state.coordinates().sessionId.empty(),
          "focus out must erase prior identifiers");
}

void unchangedToolkitRepublishPreservesAuthority() {
    auto state = focusedState();
    const auto update = explicitContext(state);
    check(matchesCapturedContext(state.lastContext(), update.context),
          "unchanged surrounding text must preserve the captured revision");

    auto changed = update.context;
    changed.before.push_back('!');
    changed.anchor += 1;
    changed.head += 1;
    check(!matchesCapturedContext(state.lastContext(), changed),
          "changed surrounding text must revoke captured authority");
    check(!matchesCapturedContext(state.lastContext(), std::nullopt),
          "unavailable native context must revoke captured authority");

    state.invalidateContext();
    check(!matchesCapturedContext(state.lastContext(), update.context),
          "missing captured authority must never be recreated by comparison");
}

void fingerprintBindsExactCaptureAndSalt() {
    auto first = focusedState();
    auto second = focusedState("omawrite", "fedcba9876543210fedcba9876543210");
    auto third = focusedState();
    const auto firstUpdate = explicitContext(first, "alpha");
    const auto secondUpdate = explicitContext(second, "alpha");
    const auto thirdUpdate = explicitContext(third, "bravo");
    check(firstUpdate.coordinates.fingerprint != secondUpdate.coordinates.fingerprint,
          "ephemeral salt must bind fingerprint");
    check(firstUpdate.coordinates.fingerprint != thirdUpdate.coordinates.fingerprint,
          "captured text must bind fingerprint");
}

void sanitizerAndIdentifiers() {
    check(sanitizeSuggestion(" for your time") == " for your time",
          "ordinary suggestion should pass");
    check(!sanitizeSuggestion(""), "empty suggestion must fail");
    check(!sanitizeSuggestion("   "), "space-only suggestion must fail");
    check(!sanitizeSuggestion(" two  spaces"), "double spaces must fail");
    check(!sanitizeSuggestion(" bad\nline"), "controls must fail");
    check(!sanitizeSuggestion(" bad\xE2\x80\x8Bmark"), "zero-width scalar must fail");
    constexpr std::array nonAsciiWhitespace{
        "\xC2\x85",     "\xC2\xA0",     "\xE1\x9A\x80", "\xE2\x80\x80",
        "\xE2\x80\x81", "\xE2\x80\x82", "\xE2\x80\x83", "\xE2\x80\x84",
        "\xE2\x80\x85", "\xE2\x80\x86", "\xE2\x80\x87", "\xE2\x80\x88",
        "\xE2\x80\x89", "\xE2\x80\x8A", "\xE2\x80\xA8", "\xE2\x80\xA9",
        "\xE2\x80\xAF", "\xE2\x81\x9F", "\xE3\x80\x80",
    };
    for (const auto whitespace : nonAsciiWhitespace) {
        check(!sanitizeSuggestion(std::string(" bad") + whitespace + "space"),
              "non-ASCII Unicode whitespace must fail");
    }
    check(!sanitizeSuggestion(std::string(" bad\xff", 5)), "invalid UTF-8 must fail");
    check(!sanitizeSuggestion(std::string(65, 'x')), "scalar limit must hold");
    check(!sanitizeSuggestion(" one two three four five six seven eight nine"),
          "word limit must hold");

    check(validLinuxAppId("omawrite"), "single segment app id should pass");
    check(validLinuxAppId("com.github.xournalpp.xournalpp"),
          "reverse DNS app id should pass");
    check(validLinuxAppId("omawrite.editor_") &&
              validLinuxAppId("omawrite.editor-"),
          "schema-valid trailing app-id punctuation should pass");
    check(!validLinuxAppId("foo._bar"), "segment must start lowercase");
    check(!validLinuxAppId("foo.-bar"), "segment must start lowercase");
    check(!validLinuxAppId("Foo.bar"), "uppercase app id must fail");
    check(validLanguageTag("en") && validLanguageTag("en-US"),
          "valid language tags should pass");
    check(!validLanguageTag("e") && !validLanguageTag("en_Us"),
          "invalid language tags should fail");
    check(validSessionId(kSession), "RFC UUID should pass");
    check(!validSessionId("session-1"), "opaque non-UUID session must fail");
}

void framingIsBoundedAndIncremental() {
    check(strictBoundedJsonObject(R"({"outer":{"value":1}})"),
          "strict JSON object should pass");
    check(!strictBoundedJsonObject(R"({"value":1,"value":2})"),
          "duplicate JSON keys must fail");
    check(!strictBoundedJsonObject(R"({/*comment*/"value":1})"),
          "JSON comments must fail");

    const auto frame = encodeFrame("{}");
    check(frame && frame->size() == 6, "two-byte JSON frame should encode");
    check((*frame)[0] == 2 && (*frame)[1] == 0 && (*frame)[2] == 0 &&
              (*frame)[3] == 0,
          "frame length must be little endian");
    FrameDecoder decoder;
    check(decoder.feed(std::span(frame->data(), 3)), "partial header should buffer");
    check(decoder.takeFrames().empty(), "partial header cannot emit");
    check(decoder.feed(std::span(frame->data() + 3, frame->size() - 3)),
          "remaining frame should decode");
    check(decoder.takeFrames() == std::vector<std::string>{"{}"},
          "decoded body must be exact");

    FrameDecoder empty;
    const std::array<std::uint8_t, 4> zero{0, 0, 0, 0};
    check(!empty.feed(zero) && empty.failed(), "zero frame must fail");
    FrameDecoder oversized;
    const auto tooLarge = static_cast<std::uint32_t>(kMaxFrameBytes + 1);
    const std::array<std::uint8_t, 4> header{
        static_cast<std::uint8_t>(tooLarge & 0xffU),
        static_cast<std::uint8_t>((tooLarge >> 8U) & 0xffU),
        static_cast<std::uint8_t>((tooLarge >> 16U) & 0xffU),
        static_cast<std::uint8_t>((tooLarge >> 24U) & 0xffU),
    };
    check(!oversized.feed(header) && oversized.failed(),
          "oversized frame must fail before body allocation");
    check(!encodeFrame(std::string(kMaxFrameBytes + 1, 'x')),
          "oversized outbound body must fail");
    check(strictBoundedJsonObject("{\"v\":2}"),
          "strict JSON object should parse");
    check(!strictBoundedJsonObject("{/*comment*/\"v\":2}"),
          "JSON comments must be rejected");
    check(!strictBoundedJsonObject("[]") &&
              !strictBoundedJsonObject("{\"v\":2} trailing"),
          "non-object and trailing JSON must be rejected");
}

void sessionControlResultIsExact() {
    constexpr auto valid = R"({"v":2,"id":"fcitx.accept.1.1","type":"control.result","mono_ms":7,"payload":{"action":"accept_all","accepted":true,"reason":"accepted","paused":false}})";
    check(strictSessionControlResult(valid),
          "requested accept-all result must be recognized");
    constexpr auto dismiss = R"({"v":2,"id":"fcitx.dismiss.1.1","type":"control.result","mono_ms":7,"payload":{"action":"dismiss","accepted":true,"reason":"accepted","paused":false}})";
    check(strictSessionControlResult(dismiss),
          "requested dismissal result must be recognized");
    constexpr auto wrongAction = R"({"v":2,"id":"fcitx.accept.1.1","type":"control.result","mono_ms":7,"payload":{"action":"accept_word","accepted":true,"reason":"accepted","paused":false}})";
    check(!strictSessionControlResult(wrongAction),
          "unrequested control actions must fail closed");
    constexpr auto extraKey = R"({"v":2,"id":"fcitx.accept.1.1","type":"control.result","mono_ms":7,"payload":{"action":"accept_all","accepted":true,"reason":"accepted","paused":false,"extra":false}})";
    check(!strictSessionControlResult(extraKey),
          "control results with extra keys must fail closed");
}

void suggestionClearMatchesOptionalWireField() {
    constexpr auto withoutSuggestion = R"({"v":2,"id":"fcitx.suggest.1.1","type":"suggestion.clear","session_id":"550e8400-e29b-41d4-a716-446655440000","focus_epoch":1,"revision":1,"mono_ms":7,"payload":{"fingerprint":"0123456789abcdef0123456789abcdef","reason":"provider_error"}})";
    check(strictSuggestionClear(withoutSuggestion),
          "clear without optional suggestion id must match broker wire");
    std::optional<ClearNotice> dispatched;
    check(dispatchSuggestionClear(
              nlohmann::json::parse(withoutSuggestion),
              [&dispatched](const ClearNotice &notice) { dispatched = notice; }) &&
              dispatched && !dispatched->suggestionId &&
              dispatched->coordinates.sessionId == kSession &&
              dispatched->coordinates.focusEpoch == 1 &&
              dispatched->coordinates.revision == 1,
          "clear without suggestion id must reach the callback safely");
    constexpr auto withSuggestion = R"({"v":2,"id":"fcitx.suggest.1.1","type":"suggestion.clear","session_id":"550e8400-e29b-41d4-a716-446655440000","focus_epoch":1,"revision":1,"mono_ms":7,"payload":{"fingerprint":"0123456789abcdef0123456789abcdef","suggestion_id":"suggestion-1","reason":"expired"}})";
    check(strictSuggestionClear(withSuggestion),
          "clear with a suggestion id must remain valid");
    dispatched.reset();
    check(dispatchSuggestionClear(
              nlohmann::json::parse(withSuggestion),
              [&dispatched](const ClearNotice &notice) { dispatched = notice; }) &&
              dispatched && dispatched->suggestionId == "suggestion-1",
          "clear with suggestion id must retain it through callback dispatch");
    constexpr auto nullSuggestion = R"({"v":2,"id":"fcitx.suggest.1.1","type":"suggestion.clear","session_id":"550e8400-e29b-41d4-a716-446655440000","focus_epoch":1,"revision":1,"mono_ms":7,"payload":{"fingerprint":"0123456789abcdef0123456789abcdef","suggestion_id":null,"reason":"expired"}})";
    check(!strictSuggestionClear(nullSuggestion),
          "present suggestion id must remain an opaque string");
}

void sensitiveCompositionAndSelectionAreZeroContext() {
    SurroundingFreshness freshness;
    check(!freshness.fresh(), "surrounding text must begin stale");
    freshness.surroundingTextUpdated();
    check(freshness.fresh(), "a post-focus surrounding update may arm capture");
    freshness.focusIn();
    check(!freshness.fresh(), "same-UUID focus-in must retire cached text");
    freshness.surroundingTextUpdated();
    freshness.capabilityChanged();
    check(!freshness.fresh(), "capability changes must retire cached text");
    freshness.surroundingTextUpdated();
    freshness.focusOut();
    check(!freshness.fresh(), "focus-out must retire cached text");

    check(!allowsNativeContext(::fcitx::CapabilityFlags()),
          "missing surrounding-text capability must fail closed");
    check(allowsNativeContext(::fcitx::CapabilityFlag::SurroundingText),
          "an ordinary surrounding-text context should remain eligible");
    for (const auto denied : {
             ::fcitx::CapabilityFlag::Password,
             ::fcitx::CapabilityFlag::Sensitive,
             ::fcitx::CapabilityFlag::Disable,
             ::fcitx::CapabilityFlag::Email,
             ::fcitx::CapabilityFlag::Digit,
             ::fcitx::CapabilityFlag::Url,
             ::fcitx::CapabilityFlag::Dialable,
             ::fcitx::CapabilityFlag::Number,
             ::fcitx::CapabilityFlag::Terminal,
             ::fcitx::CapabilityFlag::Date,
             ::fcitx::CapabilityFlag::Time,
         }) {
        auto capabilities =
            ::fcitx::CapabilityFlags(::fcitx::CapabilityFlag::SurroundingText);
        capabilities |= denied;
        check(!allowsNativeContext(capabilities),
              "sensitive and special-purpose capabilities must fail closed");
    }
    check(!captureContextWindow(std::string_view("\xff", 1), 999, 999, true,
                                false, false, "en"),
          "sensitive path must reject before inspecting bytes");
    check(!captureContextWindow("secret", 6, 6, false, false, true, "en"),
          "composing path must serialize nothing");
    check(!captureContextWindow("selected", 8, 0, false, false, false, "en"),
          "noncollapsed selection must serialize nothing");
    check(!captureContextWindow(std::string(kMaxContextSourceBytes + 1, 'x'), 0,
                                0, false, false, false, "en"),
          "oversized toolkit context must fail before proportional allocation");

    auto state = focusedState();
    auto sensitive = ContextWindow{};
    sensitive.before = "secret";
    sensitive.language = "en";
    sensitive.sensitive = true;
    check(!state.updateContext(std::move(sensitive)),
          "constructed sensitive context must fail closed");
    check(!state.lastContext(), "sensitive context must not be retained");
}

void contextWireIsExplicitManualV2() {
    auto state = focusedState();
    const auto update = explicitContext(state);
    const auto body = serializeContextEnvelope(update, 42);
    check(body.has_value(), "valid context envelope should serialize");
    const auto value = nlohmann::json::parse(*body);
    check(value["v"] == 2 && value["type"] == "context.changed",
          "context wire must be v2");
    check(value["payload"]["activation"] == "manual" &&
              value["payload"]["explicit"] == true,
          "context wire must be explicit manual only");
    check(value["payload"]["language"] == "en",
          "validated input-method language must be serialized");
    check(value["payload"]["selection"]["unit"] ==
              "unicode_scalar_values",
          "desktop selection must use Unicode scalar values");
    check(value["payload"]["field"]["purpose"] == "unknown" &&
              value["payload"]["field"]["identity_known"] == false,
          "Fcitx must not invent semantic widget identity or purpose");
    check(!value["payload"].contains("origin"),
          "desktop context must not gain browser origin");

    auto invalidLanguage = update;
    invalidLanguage.context.language = "";
    check(!serializeContextEnvelope(invalidLanguage, 42),
          "missing language must fail before serialization");
    auto sensitive = update;
    sensitive.context.sensitive = true;
    check(!serializeContextEnvelope(sensitive, 42),
          "sensitive context must not serialize");
}

void sessionWireSeparatesPolicyFromExplicitRequest() {
    auto state = focusedState();
    const auto body = serializeSessionOpenEnvelope(
        state.coordinates(), state.appId(), state.targetId(), 41);
    check(body.has_value(), "valid desktop session should serialize");
    const auto value = nlohmann::json::parse(*body);
    check(value["v"] == 2 && value["type"] == "session.open" &&
              value["revision"] == 0,
          "desktop session wire must be canonical v2 revision zero");
    check(value["payload"]["activation"] == "always",
          "session must match installed always policy");
    check(value["payload"]["target"]["kind"] == "desktop_application" &&
              value["payload"]["target"]["app_id"] == "omawrite" &&
              !value["payload"]["target"].contains("origin"),
          "session target must retain exact Linux identity");
    check(!serializeSessionOpenEnvelope(state.coordinates(), "Omawrite",
                                        state.targetId(), 41),
          "non-canonical Linux identity must not serialize");
}

void staleAndDuplicateCommitsCannotDispatch() {
    auto state = focusedState();
    const auto update = explicitContext(state);
    check(state.showSuggestion(suggestionFor(update), 100),
          "current suggestion should display");
    const auto accept = state.requestAcceptance(100, {});
    check(accept.has_value(), "current candidate should request acceptance");
    check(accept->controlId.size() < 64, "control id must stay bounded");
    const CommitPrepare prepare{
        .coordinates = accept->coordinates,
        .controlId = accept->controlId,
        .suggestionId = accept->suggestionId,
        .text = accept->expectedText,
        .acceptance = "all",
    };
    const auto dispatch = state.authorizeCommit(prepare, 100);
    check(dispatch && dispatch->text == " for your time",
          "exact broker authorization should dispatch once");
    check(!state.authorizeCommit(prepare, 100), "duplicate prepare must not dispatch");

    auto stale = focusedState();
    const auto staleUpdate = explicitContext(stale);
    check(stale.showSuggestion(suggestionFor(staleUpdate), 100),
          "stale test suggestion should display initially");
    stale.invalidateContext();
    check(!stale.requestAcceptance(100, {}),
          "ambient revision change must fence acceptance");
    check(!stale.showSuggestion(suggestionFor(staleUpdate), 100),
          "old revision must not redisplay");
    auto unsafeExpiry = suggestionFor(staleUpdate, (std::uint64_t{1} << 53U));
    check(!stale.showSuggestion(std::move(unsafeExpiry), 100),
          "non-JS-safe expiry must fail");
}

void foreignImeAndManualKeysYieldCooperatively() {
    const PanelObservation foreignPreedit{.preedit = true};
    const PanelObservation foreignCandidates{.candidates = true};
    const PanelObservation ownedCandidates{.candidates = true,
                                            .candidatesOwnedByBadi = true};
    check(hasForeignImeUi(foreignPreedit) && hasForeignImeUi(foreignCandidates),
          "foreign UI must be detected");
    check(!hasForeignImeUi(ownedCandidates), "owned panel must not self-yield");
    check(decideLocalAction(true, false, false, false, {}) == LocalAction::Invoke,
          "sole invoke chord should request explicitly");
    check(decideLocalAction(false, true, false, true, ownedCandidates) ==
              LocalAction::Accept,
          "accept chord should act only on owned live candidate");
    check(decideLocalAction(false, false, true, true, ownedCandidates) ==
              LocalAction::Dismiss,
          "escape should dismiss owned live candidate");
    check(decideLocalAction(false, false, true, true, {}) ==
              LocalAction::PassThrough,
          "stale local state must not consume escape without a current panel");
    check(decideLocalAction(false, false, true, true, foreignCandidates) ==
              LocalAction::PassThrough,
          "a foreign candidate panel must retain escape ownership");
    check(decideLocalAction(false, false, false, true, ownedCandidates) ==
              LocalAction::PassThrough,
          "all unrelated keys must pass through");
    check(decideLocalAction(true, false, false, false, foreignPreedit) ==
              LocalAction::PassThrough,
          "foreign IME must win even over invoke chord");

    auto state = focusedState();
    const auto update = explicitContext(state);
    check(state.showSuggestion(suggestionFor(update), 100),
          "foreign acceptance setup should display");
    check(!state.requestAcceptance(100, foreignCandidates),
          "foreign candidate panel must block Badi acceptance");

    auto dismissing = focusedState();
    const auto dismissUpdate = explicitContext(dismissing);
    check(dismissing.showSuggestion(suggestionFor(dismissUpdate), 100),
          "dismissal setup should display");
    const auto dismissal = dismissing.requestDismissal(100, ownedCandidates);
    check(dismissal && dismissal->suggestionId == "suggestion-1" &&
              !dismissing.suggestionVisible(),
          "dismissal must be revision-bound and clear local authority");
}

void shiftedLetterChordUsesFcitxNormalization() {
    const auto reported =
        ::fcitx::Key(FcitxKey_Y, ::fcitx::KeyState::Ctrl_Shift).normalize();
    check(reported.check(::fcitx::Key("Control+Shift+Y")),
          "normalized Ctrl+Shift+Y must match its canonical Fcitx form");
    check(!reported.check(::fcitx::Key("Control+Y")),
          "acceptance must retain the explicit Shift modifier");
}

} // namespace

int main() {
    const std::vector<std::pair<const char *, void (*)()>> tests{
        {"state transitions and identity", stateTransitionsAndIdentity},
        {"unchanged toolkit republish",
         unchangedToolkitRepublishPreservesAuthority},
        {"fingerprint binding", fingerprintBindsExactCaptureAndSalt},
        {"sanitizer and identifiers", sanitizerAndIdentifiers},
        {"framing", framingIsBoundedAndIncremental},
        {"exact session control result", sessionControlResultIsExact},
        {"optional suggestion clear field",
         suggestionClearMatchesOptionalWireField},
        {"sensitive zero context", sensitiveCompositionAndSelectionAreZeroContext},
        {"explicit manual context wire", contextWireIsExplicitManualV2},
        {"session policy and explicit request split",
         sessionWireSeparatesPolicyFromExplicitRequest},
        {"stale and duplicate commit", staleAndDuplicateCommitsCannotDispatch},
        {"foreign IME and manual keys", foreignImeAndManualKeysYieldCooperatively},
        {"shifted letter chord normalization",
         shiftedLetterChordUsesFcitxNormalization},
    };
    try {
        for (const auto &[name, test] : tests) {
            test();
            std::cout << "ok - " << name << '\n';
        }
    } catch (const std::exception &error) {
        std::cerr << "not ok - " << error.what() << '\n';
        return 1;
    }
    return 0;
}
