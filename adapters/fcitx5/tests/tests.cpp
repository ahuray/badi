#include "observation.h"
#include "sanitizer.h"
#include "state.h"
#include "transport.h"

#include <fcitx-utils/key.h>
#include <nlohmann/json.hpp>

#include <algorithm>
#include <array>
#include <cstdint>
#include <iostream>
#include <fstream>
#include <stdexcept>
#include <string>
#include <tuple>
#include <vector>

namespace {

using namespace badi::fcitx5;
constexpr auto kSession = "550e8400-e29b-41d4-a716-446655440000";
constexpr auto kSalt = "0123456789abcdef0123456789abcdef";
constexpr PanelObservation kOwnedPanel{.candidates = true,
                                       .candidatesOwnedByBadi = true};

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
        .multiline = true,
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
    check(supportedWritingLanguage("en-US") && supportedWritingLanguage("de-DE") &&
          supportedWritingLanguage("fa"), "tested writing language tags must reach manual invocation");
    check(!supportedWritingLanguage("end") && !supportedWritingLanguage("fr") &&
          !supportedWritingLanguage("de_"), "unknown language tags must not be treated as supported");
    check(validLinuxAppId("omawrite"), "Omawrite must be supported");
    check(validLinuxAppId("com.github.xournalpp.xournalpp"),
          "canonical Xournal++ id must be supported");
    check(validLinuxAppId("org.gnome.texteditor"), "native identity must not require a compiled allowlist");
    check(!validLinuxAppId("A window title"), "native identity must be canonical");

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
    check(sanitizeSuggestion("final ") == "final ",
          "one trailing space is the mid-sentence separator");
    check(!sanitizeSuggestion("final  "), "two trailing spaces must fail");
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
    check(nativeReplacementChars(false, "This is teh ", "teh ") == 4,
          "a native suffix is deleted in UTF-8 scalars");
    check(!nativeReplacementChars(true, "This is teh ", "teh "),
          "IME-parity must not delete");
    check(!nativeReplacementChars(false, "This is teh ", "other "),
          "a stale suffix must not delete");
    check(!nativeReplacementChars(false, "This is teh ", ""),
          "an empty suffix must not delete");

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

void coalescedFramesRespectIndividualLimits() {
    const std::string body(kMaxFrameBytes, 'x');
    const auto large = encodeFrame(body);
    const auto small = encodeFrame("{}");
    check(large.has_value() && small.has_value(), "valid frames encode");
    auto joined = *large;
    joined.insert(joined.end(), small->begin(), small->end());
    for (const std::size_t chunkSize : {std::size_t{1}, std::size_t{8192}, joined.size()}) {
        FrameDecoder decoder;
        std::vector<std::string> decoded;
        for (std::size_t offset = 0; offset < joined.size(); offset += chunkSize) {
            check(decoder.feed(std::span(joined).subspan(
                      offset, std::min(chunkSize, joined.size() - offset))),
                  "coalescing must not turn valid frames into an oversized frame");
            auto frames = decoder.takeFrames();
            decoded.insert(decoded.end(), frames.begin(), frames.end());
        }
        check(decoded == std::vector<std::string>{body, "{}"},
              "all chunk boundaries must decode the same exact frames");
    }
}

void sessionControlResultIsExact() {
    constexpr auto valid = R"({"v":2,"id":"fcitx.accept.1.1","type":"control.result","mono_ms":7,"payload":{"action":"accept_all","accepted":true,"reason":"accepted","paused":false}})";
    check(strictSessionControlResult(valid),
          "requested accept-all result must be recognized");
    constexpr auto dismiss = R"({"v":2,"id":"fcitx.dismiss.1.1","type":"control.result","mono_ms":7,"payload":{"action":"dismiss","accepted":true,"reason":"accepted","paused":false}})";
    check(strictSessionControlResult(dismiss),
          "requested dismissal result must be recognized");
    constexpr auto word = R"({"v":2,"id":"fcitx.accept.1.1","type":"control.result","mono_ms":7,"payload":{"action":"accept_word","accepted":true,"reason":"accepted","paused":false}})";
    check(strictSessionControlResult(word),
          "requested accept-word result must be recognized");
    constexpr auto wrongAction = R"({"v":2,"id":"fcitx.accept.1.1","type":"control.result","mono_ms":7,"payload":{"action":"request","accepted":true,"reason":"accepted","paused":false}})";
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
              dispatched && !dispatched->suggestionId && dispatched->reason == "provider_error" &&
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
              dispatched && dispatched->suggestionId == "suggestion-1" && dispatched->reason == "expired",
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
    check(!captureContextWindow("selected", 8, 0, false, "en"),
          "noncollapsed selection must serialize nothing");
    check(!captureContextWindow(std::string(kMaxContextSourceBytes + 1, 'x'), 0,
                                0, false, "en"),
          "oversized toolkit context must fail before proportional allocation");
    check(!captureContextWindow("thank you", 10, 10, false, "en"),
          "a caret beyond the text serializes nothing");
    check(!captureContextWindow(std::string("thank you") + std::string(200, 'x') + "\xff", 9, 9, false, "en"),
          "invalid UTF-8 anywhere in the toolkit text, even outside the window, serializes nothing");
}

void qtSurroundingEchoIsNarrow() {
    using ::fcitx::CapabilityFlag;
    using ::fcitx::CapabilityFlags;
    const auto base = CapabilityFlags(CapabilityFlag::Preedit) | CapabilityFlag::Multiline;
    const auto granted = base | CapabilityFlag::SurroundingText;
    const auto withdrawnEcho = [&] {
        SurroundingEcho echo;
        echo.capabilityChanged(base, granted);
        echo.surroundingTextUpdated(granted);
        echo.capabilityChanged(granted, base);
        return echo;
    };

    // One key in a Qt Quick field under fcitx5-qt 5.1.16: grant, text, withdrawal.
    SurroundingEcho echo;
    check(!echo.capabilityChanged(base, granted), "a first grant is an ordinary capability change");
    echo.surroundingTextUpdated(granted);
    check(echo.capabilityChanged(granted, base) && echo.withdrawn(),
          "SurroundingText alone withdrawn right after text arrived under it is the echo");
    check(allowsNativeContext(observedFieldCapabilities(base, echo)),
          "an observed field keeps the surrounding text the echo withdrew");
    check(!allowsNativeContext(base), "Fcitx's own flags still fail closed");
    check(echo.capabilityChanged(base, granted) && !echo.withdrawn(),
          "SurroundingText granted back while withdrawn undoes the echo");
    echo.surroundingTextUpdated(granted);
    check(echo.capabilityChanged(granted, base) && echo.withdrawn(), "every key repeats the echo");
    echo.reset();
    check(!echo.withdrawn() && !allowsNativeContext(observedFieldCapabilities(base, echo)),
          "focus ends the echo");

    SurroundingEcho bare;
    bare.capabilityChanged(base, granted);
    check(!bare.capabilityChanged(granted, base) && !bare.withdrawn(),
          "a withdrawal with no text since the grant is an ordinary change");
    SurroundingEcho late;
    late.surroundingTextUpdated(base);
    late.capabilityChanged(base, granted);
    check(!late.capabilityChanged(granted, base) && !late.withdrawn(),
          "text that arrived without SurroundingText arms nothing");
    SurroundingEcho twice = withdrawnEcho();
    check(!twice.capabilityChanged(granted, base) && !twice.withdrawn(),
          "a withdrawal while already withdrawn is not the echo");

    for (const auto other : {CapabilityFlag::Password, CapabilityFlag::Sensitive, CapabilityFlag::Disable,
                             CapabilityFlag::Email, CapabilityFlag::Url, CapabilityFlag::Number,
                             CapabilityFlag::Multiline}) {
        auto changed = withdrawnEcho();
        check(!changed.capabilityChanged(base, base ^ other) && !changed.withdrawn(),
              "any other capability change ends the echo");
        SurroundingEcho together;
        together.capabilityChanged(base, granted);
        together.surroundingTextUpdated(granted);
        check(!together.capabilityChanged(granted, base ^ other) && !together.withdrawn(),
              "a withdrawal together with another flag is not the echo");
    }
    check(!allowsNativeContext(observedFieldCapabilities(base | CapabilityFlag::Password, withdrawnEcho())),
          "the echo restores only SurroundingText; denied purposes stay denied");

    // Fcitx reports the old flags without preedit bits the user disabled.
    SurroundingEcho preedit;
    preedit.capabilityChanged(base.unset(CapabilityFlag::Preedit), granted);
    preedit.surroundingTextUpdated(granted);
    check(preedit.capabilityChanged(granted.unset(CapabilityFlag::Preedit), base) && preedit.withdrawn(),
          "disabled preedit bits do not hide the echo");
}

void contextWindowIsBoundedInScalarValues() {
    std::string text;
    for (int index = 0; index < 600; ++index) text += "\u0622";
    for (int index = 0; index < 200; ++index) text += "\U0001F98A";
    const auto context = captureContextWindow(text, 600, 600, true, "fa");
    check(context && context->before.size() == 512 * 2 && context->after.size() == 128 * 4 &&
              context->before.substr(0, 2) == "\u0622" && context->after.substr(0, 4) == "\U0001F98A" &&
              context->head == 600 && context->anchor == 600,
          "the window holds 512 scalar values before and 128 after the caret");
    const auto start = captureContextWindow("thank you", 0, 0, false, "en");
    check(start && start->before.empty() && start->after == "thank you", "a caret at the start has no prefix");
    const auto end = captureContextWindow("thank you", 9, 9, false, "en");
    check(end && end->before == "thank you" && end->after.empty(), "a caret at the end has no suffix");
    const auto window = scalarWindow("abcdef", 3, 2, 2);
    check(window && window->before == "bc" && window->after == "de", "scalarWindow clamps both sides");
}

void transportReconnectKeepsFcitxTextButNoBrokerGrant() {
    // Mirrors the addon: Fcitx freshness gates capture, while SessionState
    // carries every broker-bound revision, candidate and commit grant.
    const auto capture = [](const SurroundingFreshness &freshness) {
        return freshness.fresh()
                   ? captureContextWindow("thank you", 9, 9, true, "en")
                   : std::nullopt;
    };
    AuthorityContinuity authority;
    const AuthoritySnapshot connected{
        .authorityEpoch = 4, .settingsRevision = 2, .paused = false, .initial = true};
    check(!authority.observe(connected), "the first connection has no earlier authority to retire");

    SurroundingFreshness freshness;
    auto state = focusedState();
    freshness.surroundingTextUpdated();
    const auto before = state.updateContext(*capture(freshness));
    check(before && state.showSuggestion(suggestionFor(*before), 100),
          "a candidate is visible before the idle disconnect");
    const auto accept = state.requestAcceptance(100, kOwnedPanel);
    check(accept.has_value(), "an acceptance is pending before the idle disconnect");
    const CommitPrepare prepare{accept->coordinates, accept->controlId,
                                accept->suggestionId, accept->expectedText, "all"};

    state.invalidateContext();
    freshness.transportLost();
    check(!state.suggestionVisible() && !state.lastContext(),
          "transport loss retires the candidate and captured context");
    check(!state.authorizeCommit(prepare, 101, kOwnedPanel),
          "a commit grant cannot survive its connection");
    check(!state.showSuggestion(suggestionFor(*before), 101),
          "a suggestion from the closed connection cannot redisplay");
    check(freshness.fresh() && !freshness.freshForAutomatic(),
          "unchanged Fcitx text stays readable only for an explicit request");

    check(!authority.observe(connected), "an unchanged reconnect keeps Fcitx freshness");
    const auto context = capture(freshness);
    check(decideTabAction(tabEligibleContext(context), state.suggestionVisible(), {},
                          NativeEditPath::Manual) == LocalAction::Invoke,
          "Tab is claimed after a transport-only reconnect without new input");
    const auto republished = state.updateContext(*context);
    check(republished && republished->coordinates.sessionId == before->coordinates.sessionId &&
              republished->coordinates.focusEpoch == before->coordinates.focusEpoch &&
              republished->coordinates.revision > before->coordinates.revision &&
              republished->coordinates.fingerprint != before->coordinates.fingerprint,
          "the reopened session receives context at a new revision and fingerprint");
    check(!state.showSuggestion(suggestionFor(*before), 102) &&
              !state.authorizeCommit(prepare, 102, kOwnedPanel),
          "republished context cannot revive the retired suggestion or grant");
    check(state.showSuggestion(suggestionFor(*republished), 102),
          "a suggestion for the republished revision may display");
    const auto current = state.requestAcceptance(102, kOwnedPanel);
    check(current && current->controlId != accept->controlId,
          "acceptance after reconnect uses a new control identity");
    const CommitPrepare currentPrepare{current->coordinates, current->controlId,
                                       current->suggestionId, current->expectedText, "all"};
    check(state.authorizeCommit(currentPrepare, 103, kOwnedPanel).has_value() &&
              !state.authorizeCommit(currentPrepare, 103, kOwnedPanel),
          "the new grant dispatches exactly once");
    freshness.surroundingTextUpdated();
    check(freshness.freshForAutomatic(), "new surrounding text resumes automatic requests");

    for (const auto changed : {
             AuthoritySnapshot{.authorityEpoch = 0, .settingsRevision = 2, .paused = false, .initial = true},
             AuthoritySnapshot{.authorityEpoch = 0, .settingsRevision = 3, .paused = false, .initial = true},
             AuthoritySnapshot{.authorityEpoch = 0, .settingsRevision = 3, .paused = true, .initial = true},
             AuthoritySnapshot{.authorityEpoch = 1, .settingsRevision = 3, .paused = true, .initial = false},
         }) {
        freshness.transportLost();
        check(authority.observe(changed),
              "an epoch, settings or pause change must retire Fcitx freshness");
        freshness.capabilityChanged();
        check(decideTabAction(tabEligibleContext(capture(freshness)), false, {},
                              NativeEditPath::Manual) == LocalAction::PassThrough,
              "changed authority requires new surrounding text before Tab reads");
        freshness.surroundingTextUpdated();
    }
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
    auto selected = update;
    selected.context.anchor = 0;
    check(!serializeContextEnvelope(selected, 42),
          "a noncollapsed selection must not serialize");
}

void sessionWireSeparatesPolicyFromExplicitRequest() {
    auto state = focusedState();
    const auto target = desktopApplicationTarget(state.appId(), state.targetId());
    check(target.has_value(), "a canonical app id and context id name a desktop target");
    const auto body = serializeSessionOpenEnvelope(state.coordinates(), *target, 41);
    check(body.has_value(), "valid desktop session should serialize");
    const auto value = nlohmann::json::parse(*body);
    check(value["v"] == 2 && value["type"] == "session.open" &&
              value["revision"] == 0,
          "desktop session wire must be canonical v2 revision zero");
    check(value["id"] == std::string("fcitx.open.") + kSession,
          "every session open is named by its session");
    check(value["payload"]["activation"] == "always",
          "session must match installed always policy");
    check(value["payload"]["target"]["kind"] == "desktop_application" &&
              value["payload"]["target"]["app_id"] == "omawrite" &&
              value["payload"]["target"]["target_id"] == "input-context-1" &&
              !value["payload"]["target"].contains("origin"),
          "session target must retain exact Linux identity");
    check(!desktopApplicationTarget("Omawrite", state.targetId()),
          "non-canonical Linux identity must not serialize");
    check(!desktopApplicationTarget("omawrite", "not an id"), "an invalid context id names no target");

    const nlohmann::json browser{{"kind", "browser"}, {"origin", "https://allowed.example.test"},
                                 {"target_id", "observed-target"}};
    const auto observed = serializeSessionOpenEnvelope(state.coordinates(), browser, 41);
    check(observed && nlohmann::json::parse(*observed)["payload"]["target"] == browser,
          "an observed field's inspect target opens verbatim");
    check(!serializeSessionOpenEnvelope(state.coordinates(), "browser", 41) &&
              !serializeSessionOpenEnvelope(state.coordinates(),
                                            {{"kind", "browser"}, {"origin", std::string(4096, 'x')}}, 41),
          "a target must be a bounded object");
    auto unopened = state.coordinates();
    unopened.sessionId = "session-1";
    check(!serializeSessionOpenEnvelope(unopened, *target, 41), "a session needs its UUID");
}

void staleAndDuplicateCommitsCannotDispatch() {
    auto state = focusedState();
    const auto update = explicitContext(state);
    check(state.showSuggestion(suggestionFor(update), 100),
          "current suggestion should display");
    const auto accept = state.requestAcceptance(100, kOwnedPanel);
    check(accept.has_value(), "current candidate should request acceptance");
    check(accept->controlId.size() < 64, "control id must stay bounded");
    const CommitPrepare prepare{
        .coordinates = accept->coordinates,
        .controlId = accept->controlId,
        .suggestionId = accept->suggestionId,
        .text = accept->expectedText,
        .acceptance = "all",
    };
    const auto dispatch = state.authorizeCommit(prepare, 100, kOwnedPanel);
    check(dispatch && dispatch->text == " for your time",
          "exact broker authorization should dispatch once");
    check(!state.authorizeCommit(prepare, 100, kOwnedPanel),
          "duplicate prepare must not dispatch");

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

void missingCandidateCannotAuthorizeAcceptance() {
    auto state = focusedState();
    const auto update = explicitContext(state);
    check(state.showSuggestion(suggestionFor(update), 100), "show candidate");
    check(!state.requestAcceptance(100, {}),
          "a removed candidate must never request acceptance");
    check(!state.requestDismissal(100, {}),
          "a removed candidate must never request dismissal");

    for (const auto panel : {PanelObservation{},
                            PanelObservation{.candidates = true},
                            PanelObservation{.candidatesOwnedByBadi = true},
                            PanelObservation{.preedit = true,
                                             .candidates = true,
                                             .candidatesOwnedByBadi = true}}) {
        check(state.showSuggestion(suggestionFor(update), 100), "restore candidate");
        const auto accept = state.requestAcceptance(100, kOwnedPanel);
        check(accept.has_value(), "visible owned candidate may request acceptance");
        const CommitPrepare prepare{accept->coordinates, accept->controlId,
                                    accept->suggestionId, accept->expectedText, "all"};
        check(!state.authorizeCommit(prepare, 100, panel),
              "ownership loss during authorization must revoke the commit");
        check(!state.authorizeCommit(prepare, 100, kOwnedPanel),
              "restoring a panel must not revive a revoked commit");
    }
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
    check(claimInvokeChord(true, true, false),
          "a granted field Badi can edit still takes Ctrl+Shift+Space");
    check(!claimInvokeChord(false, true, false) && !claimInvokeChord(true, false, false) &&
              !claimInvokeChord(false, false, false) && !claimInvokeChord(true, true, true),
          "without a grant, without an edit path, or under a foreign IME the chord reaches the app");
    check(decideLocalAction(false, true, false, true, ownedCandidates) ==
              LocalAction::AcceptAll,
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
    constexpr auto manual = NativeEditPath::Manual;
    constexpr auto observed = NativeEditPath::Observed;
    check(decideTabAction(true, false, {}, manual) == LocalAction::Invoke,
          "manual Tab requests at an eligible end-of-text caret");
    check(decideTabAction(true, true, ownedCandidates, manual) == LocalAction::AcceptWord,
          "Tab accepts one word of the owned live candidate before generic IME navigation");
    check(decideTabAction(false, false, {}, manual) == LocalAction::PassThrough,
          "Tab preserves navigation and indentation without eligible context");
    check(decideTabAction(true, true, foreignCandidates, manual) == LocalAction::PassThrough,
          "Tab leaves foreign candidate navigation intact");
    check(decideTabAction(true, false, {.foreignAuxiliary = true}, manual) == LocalAction::PassThrough,
          "Tab yields to foreign auxiliary UI");
    check(decideTabAction(true, false, {}, observed) == LocalAction::PassThrough,
          "observed Tab without a suggestion stays the application's Tab (next field, indent)");
    check(decideTabAction(true, true, ownedCandidates, observed) == LocalAction::AcceptWord,
          "observed Tab accepts one word of a visible suggestion");

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

void preInputKeysConsumeOnlyBadiUi() {
    const PanelObservation owned{.candidates = true, .candidatesOwnedByBadi = true};
    const PanelObservation foreign{.preedit = true};
    const PreKey letter{};
    const PreKey modifier{.modifier = true};
    const PreKey tab{.tab = true};
    const PreKey escape{.escape = true};
    const PreKey chord{.chord = true};
    const auto decide = [](const PreKey &key, bool notice, bool visible, const PanelObservation &panel,
                           bool editing = true) { return decidePreKey(key, editing, notice, visible, panel); };

    check(decide(letter, false, false, {}, false) == PreKeyAction::Cancel &&
              decide(tab, false, false, {}, false) == PreKeyAction::Cancel &&
              decide(escape, true, false, {}, false) == PreKeyAction::Cancel &&
              decide(modifier, false, false, {}, false) == PreKeyAction::PassThrough,
          "without an edit path every key passes on and only cancels Badi's state");
    check(decide({.repeat = true, .tab = true}, false, true, owned) == PreKeyAction::Cancel &&
              decide({.repeat = true, .escape = true}, false, true, owned) == PreKeyAction::Cancel &&
              decide({.modifier = true, .repeat = true}, false, false, {}) == PreKeyAction::PassThrough,
          "auto-repeat never accepts, dismisses or declines");
    check(decide(tab, false, true, owned) == PreKeyAction::Tab && decide(tab, false, false, foreign) == PreKeyAction::Tab,
          "plain Tab is decided by decideTabAction");
    check(decide(escape, true, false, {}) == PreKeyAction::CloseNotice,
          "Escape closes Badi's own notice and still reaches the application");
    check(decide(escape, true, false, foreign) == PreKeyAction::CancelDeclining,
          "a foreign input method keeps Escape even over Badi's notice");
    check(decide(escape, true, true, owned) == PreKeyAction::Dismiss &&
              decide(escape, false, true, owned) == PreKeyAction::Dismiss,
          "Escape dismisses Badi's live candidate");
    check(decide(escape, false, true, {}) == PreKeyAction::CancelDeclining &&
              decide(escape, false, false, {}) == PreKeyAction::CancelDeclining,
          "Escape without Badi's candidate declines the current context");
    check(decide(letter, false, true, owned) == PreKeyAction::Cancel &&
              decide(modifier, false, true, owned) == PreKeyAction::PassThrough &&
              decide(chord, false, true, owned) == PreKeyAction::PassThrough,
          "typing cancels, while modifiers and Badi's chords wait for the input method");

    const PreKey all{.all = true};
    const PreKey typing{.typesSuggestion = true};
    check(decide(all, false, true, owned) == PreKeyAction::AcceptAll,
          "Ctrl+Right accepts the whole live candidate");
    check(decide(all, false, false, {}) == PreKeyAction::Cancel &&
              decide(all, false, true, foreign) == PreKeyAction::Cancel &&
              decide(all, false, true, owned, false) == PreKeyAction::Cancel &&
              decide({.repeat = true, .all = true}, false, true, owned) == PreKeyAction::Cancel,
          "without Badi's own live candidate Ctrl+Right stays the application's");
    check(decide(typing, false, true, owned) == PreKeyAction::CancelTypingThrough,
          "typing the suggestion's next characters cancels and inspects sooner");
    check(decide(typing, false, false, {}) == PreKeyAction::CancelTypingThrough,
          "type-through continues while an earlier typed-through key hid the remainder");
    check(decide(typing, false, false, {}, false) == PreKeyAction::CancelTypingThrough,
          "a fast next key types through while its field waits to be observed again");
    check(decide(typing, false, true, foreign) == PreKeyAction::Cancel &&
              decide({.repeat = true, .typesSuggestion = true}, false, true, owned) == PreKeyAction::Cancel &&
              decide({.repeat = true, .typesSuggestion = true}, false, false, {}, false) == PreKeyAction::Cancel,
          "foreign input-method UI or auto-repeat ends type-through");
    check(typesSuggestionStart(" ", " for your time") && typesSuggestionStart(" f", " for your time") &&
              !typesSuggestionStart("", " for your time") && !typesSuggestionStart("f", " for your time") &&
              !typesSuggestionStart(" for your time", " for your time") &&
              !typesSuggestionStart(" ", ""),
          "only a non-empty proper prefix of the suggestion types through it");
    check(typeThroughContextReady("Thank you", " ", "Thank you ") &&
              !typeThroughContextReady("Thank you", " ", "Thank you") &&
              !typeThroughContextReady("Thank you", " ", "Thank youx") &&
              typeThroughContextReady("Thank you", " f", "Thank you f"),
          "type-through waits until the typed characters are in the field");
    const std::string base(200, 'a');
    const auto tail = base.substr(base.size() - 128);
    check(typeThroughContextReady(base, " ", std::string("zz") + tail + " ") &&
              !typeThroughContextReady(base, " ", tail),
          "a slid field window still has to contain the typed characters");
}

void heldTabWaitsForTheCarriedRest() {
    check(tabMayHoldForRest(true, true, false, false),
          "a rest that returns on its own may hold one Tab");
    check(!tabMayHoldForRest(false, true, false, false),
          "no pending rest holds nothing: Tab stays the application's");
    check(!tabMayHoldForRest(true, false, false, false),
          "a manual rest arrives only by that Tab's own request, so Tab must request");
    check(!tabMayHoldForRest(true, true, true, false),
          "foreign input-method UI keeps Tab");
    check(!tabMayHoldForRest(true, true, false, true),
          "a visible suggestion takes Tab as a word acceptance instead");
    // At most one hold per word acceptance: the timeout (and a broker clear
    // answering the carried request) ends the remainder's automatic return,
    // which is exactly the input the addon clears in both places.
    bool restAutomatic = true;
    check(tabMayHoldForRest(true, restAutomatic, false, false),
          "the first Tab in the gap may be held for the returning remainder");
    restAutomatic = false;
    check(!tabMayHoldForRest(true, restAutomatic, false, false),
          "after the hold ends without the remainder, the next Tab reaches the application");

    Suggestion carried;
    carried.text = " for your time";
    carried.acceptWord = " for";
    check(heldTabMatchesRest(" for your time", carried),
          "the redisplayed rest still starts with the held Tab's word");
    Suggestion grown;
    grown.text = " for your kind time";
    grown.acceptWord = " for";
    check(heldTabMatchesRest(" for your time", grown),
          "only the next word must still match; what follows may differ");
    Suggestion other;
    other.text = " with your time";
    other.acceptWord = " with";
    check(!heldTabMatchesRest(" for your time", other),
          "a different first word is not the word the held Tab meant");
    Suggestion replacement;
    replacement.text = "the ";
    replacement.acceptWord = "the ";
    replacement.replaceBefore = "teh ";
    check(!heldTabMatchesRest("the rest", replacement),
          "a spelling replacement is never taken on a held Tab");
    check(!heldTabMatchesRest("", carried) && !heldTabMatchesRest(" for", Suggestion{}),
          "an empty rest or word matches nothing");
}

void wordAcceptanceGrantsOnlyTheNextWord() {
    auto state = focusedState();
    const auto update = explicitContext(state);
    auto suggestion = suggestionFor(update);
    suggestion.acceptWord = " for";
    check(state.showSuggestion(suggestion, 100), "a suggestion with a next word displays");
    const auto word = state.requestAcceptance(100, kOwnedPanel, true);
    check(word && word->expectedText == " for" && word->acceptance == "word",
          "a word acceptance expects only the next word");
    const CommitPrepare whole{word->coordinates, word->controlId, word->suggestionId, " for your time", "all"};
    check(!state.authorizeCommit(whole, 101, kOwnedPanel),
          "a whole-text grant cannot answer a word acceptance");
    const CommitPrepare mislabeled{word->coordinates, word->controlId, word->suggestionId, " for", "all"};
    check(!state.authorizeCommit(mislabeled, 101, kOwnedPanel),
          "the grant must repeat the requested acceptance");
    const CommitPrepare granted{word->coordinates, word->controlId, word->suggestionId, " for", "word"};
    const auto dispatch = state.authorizeCommit(granted, 101, kOwnedPanel);
    check(dispatch && dispatch->text == " for" && !state.suggestionVisible(),
          "the word grant dispatches once and retires the candidate");

    auto other = focusedState();
    const auto next = explicitContext(other);
    auto unrelated = suggestionFor(next);
    unrelated.acceptWord = " you";
    check(other.showSuggestion(unrelated, 100), "a suggestion displays");
    const auto fallback = other.requestAcceptance(100, kOwnedPanel, true);
    check(fallback && fallback->expectedText == " for your time",
          "a next word that is not the text's start falls back to the whole text");

    auto pending = focusedState();
    const auto shown = explicitContext(pending);
    check(pending.showSuggestion(suggestionFor(shown), 100), "a suggestion displays");
    const auto first = pending.requestAcceptance(100, kOwnedPanel);
    check(first && pending.acceptancePending() && pending.suggestionVisible(),
          "the first acceptance stays pending");
    check(!pending.requestAcceptance(101, kOwnedPanel) && !pending.requestAcceptance(102, kOwnedPanel, true),
          "a second Tab or Ctrl+Right does not queue another acceptance");
    check(pending.suggestionVisible() && pending.acceptancePending(),
          "the in-flight acceptance is not cleared by the second key");
    auto extension = suggestionFor(shown);
    extension.suggestionId = "extended";
    extension.text += " today";
    check(!pending.showSuggestion(extension, 102) && pending.acceptancePending() &&
              pending.visibleText() == first->expectedText,
          "a streamed extension cannot replace the text already accepted");
    const CommitPrepare stillFirst{first->coordinates, first->controlId, first->suggestionId,
                                   first->expectedText, "all"};
    check(pending.authorizeCommit(stillFirst, 103, kOwnedPanel).has_value(),
          "the first grant still dispatches");
}

void invokeRoutesByEditPathAndExplainsWaits() {
    check(routeInvoke("brave-origin", false, false) == InvokeRoute::InspectField &&
              routeInvoke("code", false, false) == InvokeRoute::InspectField,
          "an unobserved IME-parity app inspects its field on request");
    check(routeInvoke("brave-origin", false, true) == InvokeRoute::Unavailable &&
              routeInvoke("firefox", false, false) == InvokeRoute::Unavailable &&
              routeInvoke("omawrite", false, false) == InvokeRoute::Unavailable,
          "a mismatched observed field or unavailable app only explains");
    check(routeInvoke("code", true, true) == InvokeRoute::ObservedField &&
              routeInvoke("omawrite", true, true) == InvokeRoute::ObservedField,
          "an observed field is re-inspected, never read unobserved");
    check(routeInvoke("omawrite", true, false) == InvokeRoute::Manual,
          "only a native exact app without an observed field takes the manual path");

    const RequestFacts ready{.connected = true, .policyKnown = true, .policyAllowed = true, .fresh = true,
                             .fieldAllowed = true};
    check(!manualRequestNotice(ready), "a connected, permitted, fresh field is read at once");
    auto facts = ready;
    facts.connected = false;
    facts.policyKnown = false;
    check(manualRequestNotice(facts) == notice::kReconnecting, "reconnecting is explained first");
    facts.connected = true;
    check(manualRequestNotice(facts) == notice::kCheckingPermission, "an unanswered policy is explained");
    facts.policyKnown = true;
    facts.policyAllowed = false;
    facts.fresh = false;
    check(manualRequestNotice(facts) == notice::kDisabled, "a denied app is explained before stale text");
    facts.paused = true;
    check(manualRequestNotice(facts) == notice::kPaused, "a pause is named as a pause");
    facts = ready;
    facts.fresh = false;
    check(manualRequestNotice(facts) == notice::kNeedsFreshContext, "stale surrounding text asks for typing");

    check(!inspectionNotice(ready) && !inspectionNotice({.fresh = true, .fieldAllowed = true}),
          "an IME-parity inspection needs no broker policy to start");
    check(inspectionNotice({.fieldAllowed = true}) == notice::kNeedsFreshContext &&
              inspectionNotice({.fresh = true}) == notice::kFieldUnreadable,
          "a stale or denied field explains why nothing is inspected");
    check(notice::kUnavailableApp == "Badi cannot safely insert suggestions in this app yet" &&
              notice::kObserverUnavailable == "Badi cannot see this text field — check badi doctor" &&
              notice::kFieldUnreadable == "Badi cannot read this text field — run badi debug status",
          "notices keep the wording the desktop lane and runbook name");
}

void tabReasonNamesTheFirstBlocker() {
    const RequestFacts eligible{.policyKnown = true, .policyAllowed = true, .fresh = true, .fieldAllowed = true};
    const auto end = captureContextWindow("thank you", 9, 9, true, "en");
    check(tabDecisionReason(eligible, end) == "eligible", "an end-of-text prose caret is eligible");
    const auto reason = [&](auto change, const std::optional<ContextWindow> &context) {
        auto facts = eligible;
        change(facts);
        return tabDecisionReason(facts, context);
    };
    check(reason([](RequestFacts &facts) { facts.policyKnown = false; facts.policyAllowed = false; }, end) ==
              "checking_app_policy" &&
              reason([](RequestFacts &facts) { facts.policyAllowed = false; facts.foreignIme = true; }, end) ==
              "app_disabled" &&
              reason([](RequestFacts &facts) { facts.foreignIme = true; facts.fieldAllowed = false; }, end) ==
              "foreign_ime" &&
              reason([](RequestFacts &facts) { facts.fieldAllowed = false; facts.fresh = false; }, end) ==
              "field_denied" &&
              reason([](RequestFacts &facts) { facts.fresh = false; }, std::nullopt) == "no_fresh_context",
          "policy, foreign IME, field purpose and freshness are named in that order");
    check(tabDecisionReason(eligible, std::nullopt) == "context_unavailable" &&
              tabDecisionReason(eligible, captureContextWindow("thank you all", 9, 9, true, "en")) ==
                  "caret_not_at_end" &&
              tabDecisionReason(eligible, captureContextWindow("thank you", 9, 9, true, "fr")) ==
                  "language_unsupported" &&
              tabDecisionReason(eligible, captureContextWindow(" \n", 2, 2, true, "en")) == "empty_context",
          "the context names why Tab does not request");
}

void brokerSessionRetiresSessionAndPolicyTogether() {
    auto state = focusedState();
    const auto update = explicitContext(state);
    BrokerSession broker;
    check(!broker.opened && !broker.policyKnown && !broker.policyAllowed, "a binding starts without a session");
    broker.answer(true, "de");
    broker.open(update.coordinates);
    check(broker.opened && broker.policyKnown && broker.policyAllowed &&
              broker.coordinates.sessionId == kSession && broker.coordinates.revision == 0 &&
              broker.coordinates.fingerprint.empty() && broker.languageOverride == "de",
          "a session opens at revision zero of its focus epoch");
    broker.retire();
    check(!broker.opened && !broker.policyKnown && !broker.policyAllowed &&
              broker.coordinates.sessionId == kSession && broker.languageOverride.empty(),
          "retiring forgets the session and its policy, keeping the coordinates a close names");
    broker.answer(false);
    check(broker.policyKnown && !broker.policyAllowed, "a denial is a known answer");
    broker.answer(true, {}, true);
    check(broker.contextWords, "context on follows an allowed answer");
    broker.answer(false, {}, true);
    check(!broker.contextWords, "a denial never keeps context on");
    broker.answer(true, {}, true);
    broker.retire();
    check(!broker.contextWords, "retiring forgets context on");
}

void inspectionBacksOffWithoutInput() {
    InspectionSchedule schedule;
    check(schedule.delayUs(true) == 1 && schedule.delayUs(false) == 120'000,
          "an explicit request inspects at once, typing after a pause");
    check(schedule.delayUs(false, false, false, true) == 1,
          "a keystroke in a known field inspects at once");
    check(schedule.delayUs(false, true) == 30'000 && schedule.delayUs(true, true) == 1,
          "typing through a suggestion inspects after a short pause");
    check(schedule.delayUs(false, false, true) == 20'000 && schedule.delayUs(false, true, true) == 20'000,
          "a busy observer is retried soon, not after another typing pause");
    schedule.input();
    schedule.inspecting();
    for (const std::uint64_t delay : {240'000, 480'000, 960'000}) {
        check(schedule.reinspectAfterInvalidation(false) && schedule.delayUs(false) == delay,
              "idle invalidations back off");
    }
    check(!schedule.reinspectAfterInvalidation(false), "after three idle retries an invalidation waits for input");
    check(schedule.delayUs(true) == 1, "an explicit request is never delayed by the backoff");
    schedule.input();
    check(schedule.delayUs(false) == 120'000 && schedule.reinspectAfterInvalidation(true),
          "input resets the backoff, and a changed field inspects after input");
    schedule.inspecting();
    check(!schedule.reinspectAfterInvalidation(true), "a changed field without input waits for input");

    for (unsigned int attempt = 0; attempt < InspectionSchedule::kMaxBusyRetries; ++attempt) {
        check(schedule.retryWhileBusy(), "a busy observer is retried");
    }
    check(!schedule.retryWhileBusy(), "busy retries are bounded");
    schedule.resetBusyRetries();
    check(schedule.retryWhileBusy(), "a new request renews the busy retries");
}

void shiftedLetterChordUsesFcitxNormalization() {
    const auto reported =
        ::fcitx::Key(FcitxKey_Y, ::fcitx::KeyState::Ctrl_Shift).normalize();
    check(reported.check(::fcitx::Key("Control+Shift+Y")),
          "normalized Ctrl+Shift+Y must match its canonical Fcitx form");
    check(!reported.check(::fcitx::Key("Control+Y")),
          "acceptance must retain the explicit Shift modifier");
}

void nativePolicyMustBeCoherent() {
    const nlohmann::json allowed{
        {"v", 2}, {"type", "policy.status"}, {"id", "policy-1"}, {"mono_ms", 0},
        {"payload", {{"authority_epoch", 1}, {"settings_revision", 1}, {"paused", false},
            {"activation", "always"}, {"context_allowed", true}, {"display_allowed", true},
            {"suggestions_allowed", true}, {"learning_allowed", false}, {"reason", "matched_rule"}}}};
    check(strictPolicyStatus(allowed.dump()), "a coherent app grant must pass");
    for (const auto &language : {"en", "de", "fa", "fr", "de-DE"}) {
        auto preference = allowed;
        preference["payload"]["language_override"] = language;
        const auto tag = std::string_view(language);
        check(strictPolicyStatus(preference.dump()) == (tag == "en" || tag == "de" || tag == "fa"),
              "policy overrides use only canonical promoted language tags");
    }
    for (const auto key : {"context_allowed", "display_allowed", "paused", "learning_allowed"}) {
        auto invalid = allowed;
        invalid["payload"][key] = !invalid["payload"][key].get<bool>();
        check(!strictPolicyStatus(invalid.dump()), "contradictory native permission must fail");
    }
    auto invalid = allowed;
    invalid["payload"]["reason"] = "arbitrary";
    check(!strictPolicyStatus(invalid.dump()), "unknown policy reason must fail");
    invalid = allowed;
    invalid["payload"]["extra"] = true;
    check(!strictPolicyStatus(invalid.dump()), "additional policy fields must fail");
    auto denied = allowed;
    denied["payload"]["activation"] = "never";
    for (const auto key : {"context_allowed", "display_allowed", "suggestions_allowed"})
        denied["payload"][key] = false;
    check(strictPolicyStatus(denied.dump()), "a coherent denial is a valid response");
    auto words = allowed;
    words["payload"]["context_words"] = true;
    check(strictPolicyStatus(words.dump()), "context on is a fact about an allowed target");
    words["payload"]["context_words"] = "yes";
    check(!strictPolicyStatus(words.dump()), "context_words is a boolean");
    denied["payload"]["context_words"] = true;
    check(!strictPolicyStatus(denied.dump()), "a denied target never has context on");
    denied["payload"]["context_words"] = false;
    check(strictPolicyStatus(denied.dump()), "context off may be stated on a denial");
}


void observedFieldsAreAppendOnly() {
    auto state = focusedState();
    auto context = explicitContext(state, "check adress ").context;
    context.anchor = context.head = context.before.size();
    context.identityKnown = true;
    context.explicitRequest = false;
    const auto update = state.updateContext(context);
    check(update.has_value(), "observed context accepted");
    const auto serialized = serializeContextEnvelope(*update, 0);
    check(serialized.has_value(), "observed automatic context serializes");
    const auto payload = nlohmann::json::parse(*serialized)["payload"];
    check(payload["field"]["identity_known"] == true && payload["field"]["purpose"] == "normal" &&
          payload["explicit"] == false && payload["activation"] == "always", "automatic authority must be explicit on the wire");
    auto unknown = *update;
    unknown.context.identityKnown = false;
    check(!serializeContextEnvelope(unknown, 0), "unknown-widget automatic context must not serialize");
    const auto append = suggestionFor(*update);
    check(state.showSuggestion(append, 0), "ordinary observed append suggestions remain eligible");
    const auto accept = state.requestAcceptance(1, kOwnedPanel);
    check(accept.has_value(), "observed append acceptance");
    CommitPrepare prepare{accept->coordinates, accept->controlId, accept->suggestionId, "address", "all"};
    check(!state.authorizeCommit(prepare, 2, kOwnedPanel), "a different text cannot use an append acceptance");
    prepare.text = accept->expectedText;
    const auto dispatch = state.authorizeCommit(prepare, 3, kOwnedPanel);
    check(dispatch && dispatch->text == append.text, "native dispatch is the accepted append");
    check(!state.authorizeCommit(prepare, 4, kOwnedPanel), "append grants remain one shot");
}

constexpr std::array kImeParityBrowsers{"chromium", "chromium-browser", "brave-origin", "zen"};
constexpr std::array kImeParityDesktop{"chatgpt", "code", "cursor", "discord"};
// Only Zen's live-verified program() is IME-parity; other Gecko ids, including
// other Zen builds and Flatpak ids, have no observer rule.
constexpr std::array kUnavailableApps{"firefox", "firefox-esr", "org.mozilla.firefox", "zen-browser",
                                      "zen-alpha", "zen-beta", "app.zen_browser.zen", "librewolf",
                                      "floorp", "obsidian", "md.obsidian.obsidian"};
// Gecko-family browser ids beyond the explicit list: channels, Flatpak ids,
// PWAsForFirefox windows, other Zen builds and forks never reach the manual path.
constexpr std::array kUnobservedGeckoFamily{
    "firefox-beta", "firefox-nightly", "firefox-developer-edition", "firefoxdeveloperedition",
    "org.mozilla.firefox_beta", "org.mozilla.firefox.nightly", "ffpwa-01hvy3f0gdt6dcg5v6ttrhk4bt",
    "zen-twilight", "zen-browser-bin", "io.github.zen_browser.zen", "app.zen_browser.zen-twilight",
    "librewolf-bin", "io.gitlab.librewolf-community", "one.ablaze.floorp", "waterfox-g",
    "net.waterfox.waterfox", "mullvadbrowser", "mullvad-browser", "net.mullvad.mullvadbrowser",
    "tor-browser", "torbrowser", "org.torproject.torbrowser-launcher", "icecat", "palemoon",
    "seamonkey", "basilisk", "iceweasel"};
// Chromium-family ids with no observer rule never reach the manual path.
constexpr std::array kUnobservedChromiumFamily{
    "chrome", "google-chrome", "brave", "brave-browser", "chrome-app.hey.com__-default", "chrome-nngceckbapebfimnlniiiahkandclblb-default",
    "crx_nngceckbapebfimnlniiiahkandclblb", "brave-nngceckbapebfimnlniiiahkandclblb-default",
    "msedge-_nngceckbapebfimnlniiiahkandclblb-default", "com.google.chrome", "com.google.chromedev",
    "org.chromium.chromium", "com.brave.browser", "com.microsoft.edge", "com.vivaldi.vivaldi",
    "com.opera.opera", "io.github.ungoogled_software.ungoogled_chromium", "google-chrome-stable",
    "google-chrome-beta", "google-chrome-unstable", "chromium-freeworld", "brave-browser-beta",
    "brave-browser-nightly", "microsoft-edge", "microsoft-edge-stable", "microsoft-edge-beta",
    "vivaldi", "vivaldi-stable", "vivaldi-snapshot", "opera", "opera-beta", "helium", "thorium-browser",
    "yandex-browser", "electron", "electron42", "code-oss", "code-insiders", "codium", "vscodium",
    "com.visualstudio.code", "com.microsoft.vscodeinsiders", "com.vscodium.codium", "discord-canary",
    "discord-ptb", "discordcanary",
    "vesktop", "com.discordapp.discord", "dev.vencord.vesktop"};

void nativeAppClassesAreExplicit() {
    for (const auto app : kImeParityBrowsers) {
        check(classifyNativeApp(app) == NativeAppClass::ImeParityBrowser && imeParityApp(app) &&
                  nativeObservationAvailable(app),
              "Chromium-family browsers and Zen use IME parity");
    }
    for (const auto app : kImeParityDesktop) {
        check(classifyNativeApp(app) == NativeAppClass::ImeParityDesktop && imeParityApp(app) &&
                  nativeObservationAvailable(app),
              "Chromium-based desktop apps use IME parity");
    }
    for (const auto app : kUnavailableApps) {
        check(classifyNativeApp(app) == NativeAppClass::Unavailable && !imeParityApp(app) &&
                  !nativeObservationAvailable(app),
              "Gecko browsers and Obsidian stay unavailable to the native module");
    }
    for (const auto app : kUnobservedChromiumFamily) {
        check(classifyNativeApp(app) == NativeAppClass::Unavailable && !imeParityApp(app) &&
                  !nativeObservationAvailable(app),
              "Chromium-family ids without an observer rule are unavailable, never native exact");
    }
    for (const auto app : kUnobservedGeckoFamily) {
        check(classifyNativeApp(app) == NativeAppClass::Unavailable && !imeParityApp(app) &&
                  !nativeObservationAvailable(app),
              "Gecko-family browser ids other than zen are unavailable, never native exact");
    }
    // Family prefixes must not capture unrelated native identities. Gecko mail
    // clients are not browsers and keep the native exact contract.
    for (const auto app : {"omawrite", "com.github.xournalpp.xournalpp", "telegram",
                           "org.telegram.desktop", "org.gnome.texteditor", "electrum", "chromaprint",
                           "codeblocks", "org.kde.kate", "com.google.earthpro", "operator", "zenity",
                           "firefly", "waterfall", "torrential", "thunderbird", "org.mozilla.thunderbird"}) {
        check(classifyNativeApp(app) == NativeAppClass::NativeExact && nativeObservationAvailable(app),
              "user-granted native apps keep the exact native contract");
    }
    for (const auto app : {"", "Telegram", "A window title", "code ", "chromium\n"}) {
        check(classifyNativeApp(app) == NativeAppClass::Unavailable && !imeParityApp(app),
              "non-canonical or invalid identities are never classified as editable");
    }

    using Target = NativeEditTarget;
    using Path = NativeEditPath;
    for (const auto target : {Target::DesktopApplication, Target::BrowserOrigin, Target::Unsupported}) {
        for (const auto path : {Path::Manual, Path::Observed}) {
            check(nativeEditingAvailable("omawrite", target, path) == (target == Target::DesktopApplication),
                  "native exact apps edit desktop targets on either path");
            for (const auto app : {"brave-origin", "zen"}) {
                check(nativeEditingAvailable(app, target, path) ==
                          (path == Path::Observed && target == Target::BrowserOrigin),
                      "IME-parity browsers edit only observed browser-origin fields");
            }
            check(nativeEditingAvailable("code", target, path) ==
                      (path == Path::Observed && target == Target::DesktopApplication),
                  "IME-parity desktop apps edit only observed desktop fields");
            for (const auto app : kUnavailableApps) {
                check(!nativeEditingAvailable(app, target, path), "unavailable apps never edit");
            }
            for (const auto app : kUnobservedChromiumFamily) {
                check(!nativeEditingAvailable(app, target, path),
                      "unobserved Chromium-family apps never edit, even with a linux_app grant");
            }
            for (const auto app : kUnobservedGeckoFamily) {
                check(!nativeEditingAvailable(app, target, path),
                      "unobserved Gecko-family browsers never edit, even with a linux_app grant");
            }
        }
    }
}

void chromeAndBraveAliasesWithoutObserverRuleAreUnavailable() {
    for (const auto program : {"chrome", "Google-chrome", "brave", "Brave-browser"}) {
        const auto app = canonicalAppId(program);
        check(app && classifyNativeApp(*app) == NativeAppClass::Unavailable && !imeParityApp(*app) &&
                  !nativeObservationAvailable(*app),
              "Chrome and Brave aliases without an observer rule are unavailable, never native exact");
        for (const auto target : {NativeEditTarget::DesktopApplication, NativeEditTarget::BrowserOrigin}) {
            for (const auto path : {NativeEditPath::Manual, NativeEditPath::Observed}) {
                SessionState state;
                check(state.focusIn(kSession, "input-context-1", *app, kSalt, target, path) &&
                          !state.editingAvailable(),
                      "a granted Chrome or Brave alias cannot edit on any path");
            }
        }
    }
}

void reasonsAndNoticesNameTheBlockedPath() {
    check(focusReason("") == "unidentified_app" && focusReason("firefox") == "editor_transaction_unavailable" &&
              focusReason("chrome") == "editor_transaction_unavailable" &&
              focusReason("brave-origin") == "awaiting_observed_field" &&
              focusReason("code") == "awaiting_observed_field" && focusReason("omawrite") == "checking_app_policy",
          "focus reasons follow the app class");
    check(editingUnavailableReason("zen", false) == "ime_parity_unobserved" &&
              editingUnavailableReason("zen", true) == "ime_parity_target_mismatch" &&
              editingUnavailableReason("omawrite", false) == "editor_transaction_unavailable" &&
              editingUnavailableReason("obsidian", true) == "editor_transaction_unavailable",
          "IME-parity reasons name the missing or mismatched observed field");
    check(clearNoticeText("no_suggestion") == "Badi has no continuation — try a longer phrase" &&
              clearNoticeText("provider_timeout") == "Badi could not finish — press Tab to retry" &&
              clearNoticeText("provider_error") == clearNoticeText("provider_timeout") &&
              !clearNoticeText("expired") && !clearNoticeText("dismissed") && !clearNoticeText("stale"),
          "only abstentions and provider failures explain themselves to the user");
}

void canonicalAppIdsFoldAsciiCase() {
    check(canonicalAppId("Telegram") == "telegram", "Qt window-class identity folds to lowercase");
    check(canonicalAppId("org.Telegram.Desktop") == "org.telegram.desktop", "reverse DNS folds per segment");
    check(canonicalAppId("com.github.xournalpp.xournalpp") == "com.github.xournalpp.xournalpp" &&
              canonicalAppId("brave-origin") == "brave-origin" && canonicalAppId("zen_Browser-2") == "zen_browser-2",
          "canonical identities are unchanged apart from case");
    const std::string longest = "a" + std::string(127, 'B');
    check(canonicalAppId(longest) == "a" + std::string(127, 'b'), "128-byte identities are accepted");
    for (const auto &invalid : {std::string(), std::string("A window title"), std::string("1password"),
                                      std::string("-app"), std::string("_app"), std::string("app..id"),
                                      std::string(".app"), std::string("app."), std::string("app.1id"),
                                      std::string("app._id"), std::string("app.-id"), std::string("caf\xC3\xA9"),
                                      std::string("app/id"), std::string("app\0id", 6), "a" + std::string(128, 'b')}) {
        check(!canonicalAppId(invalid), "non-identifier programs are rejected, not repaired");
    }
    for (const auto program : {"Telegram", "Org.Gnome.TextEditor", "Code", "Brave-Origin", "FIREFOX"}) {
        const auto canonical = canonicalAppId(program);
        check(canonical && validLinuxAppId(*canonical),
              "every folded identity satisfies the broker's lowercase validator");
    }
    check(programAppId("com.microsoft.VSCode") == "code" && programAppId("Code") == "code" &&
              programAppId("Telegram") == "telegram" && !programAppId("A window title"),
          "VS Code's renamed Wayland id keeps its identity; other programs only fold");
    for (const auto program : {"brave-app.hey.com__-Default", "brave-launchpad.37signals.com__-Default",
                               "brave-app.zoom.us__wc_home-Default", "brave-x.com__-Profile_1",
                               "brave-127.0.0.1__0123abc_fixture-Default"}) {
        check(braveWebAppWindow(program) && programAppId(program) == "brave-origin",
              "Brave --app windows keep the browser identity, even with digit-led host labels");
    }
    for (const auto program : {"brave-nngceckbapebfimnlniiiahkandclblb-Default", "brave-browser",
                               "brave-x.com__-Profile 1", "brave-x.com__/-Default", "chrome-app.hey.com__-Default",
                               "brave__", "brave-app.hey.com_-Default"}) {
        check(!braveWebAppWindow(program) && programAppId(program) != "brave-origin",
              "PWAs, other builds, other browsers and malformed names are not Brave web-app windows");
    }
    check(!braveWebAppWindow("brave-" + std::string(240, 'a') + "__-Default"),
          "an overlong web-app window name is rejected");
    for (const auto program : {"chrome-app.hey.com__-Default", "chrome-app.basecamp.com__-Default",
                               "chrome-web.whatsapp.com__-Profile_1", "chrome-127.0.0.1__0123abc_fixture-Default"}) {
        check(chromiumWebAppWindow(program) && programAppId(program) == "chromium",
              "Omarchy's Chromium --app windows keep the browser identity");
    }
    for (const auto program : {"chrome-nngceckbapebfimnlniiiahkandclblb-Default", "chrome", "google-chrome",
                               "chrome-x.com__-Profile 1", "chrome-x.com__/-Default", "brave-app.hey.com__-Default",
                               "chromium-app.hey.com__-Default", "chrome-app.hey.com_-Default"}) {
        check(!chromiumWebAppWindow(program) && programAppId(program) != "chromium",
              "installed PWAs, other names and malformed names are not Chromium web-app windows");
    }
    check(!chromiumWebAppWindow("chrome-" + std::string(240, 'a') + "__-Default"),
          "an overlong Chromium web-app window name is rejected");
    check(classifyNativeApp("grok-bot") == NativeAppClass::ImeParityDesktop && imeParityApp("grok-bot"),
          "Grok Bot accepts only through its observed field");
    check(programAppId("com.anthropic.Claude") == "com.anthropic.claude" &&
              classifyNativeApp("com.anthropic.claude") == NativeAppClass::ImeParityDesktop &&
              imeParityApp("com.anthropic.claude") &&
              !nativeEditingAvailable("com.anthropic.claude", NativeEditTarget::DesktopApplication,
                                      NativeEditPath::Manual),
          "the Claude desktop app accepts only through its observed field");
    for (const auto program : {"libreoffice-startcenter", "libreoffice-writer", "libreoffice-calc", "LibreOffice-Impress"}) {
        check(programAppId(program) == "libreoffice", "every LibreOffice window keeps one identity");
    }
    check(classifyNativeApp("libreoffice") == NativeAppClass::ImeParityDesktop &&
              nativeEditingAvailable("libreoffice", NativeEditTarget::DesktopApplication, NativeEditPath::Observed) &&
              !nativeEditingAvailable("libreoffice", NativeEditTarget::DesktopApplication, NativeEditPath::Manual),
          "LibreOffice has no manual path, so Calc cells keep their Tab");
    check(classifyNativeApp(*canonicalAppId("com.microsoft.VSCode")) == NativeAppClass::Unavailable &&
              classifyNativeApp(*canonicalAppId("com.microsoft.VSCodeInsiders")) == NativeAppClass::Unavailable,
          "unmapped VS Code reverse-DNS ids never reach the native exact path");
    check(classifyNativeApp(*canonicalAppId("Code")) == NativeAppClass::ImeParityDesktop &&
              classifyNativeApp(*canonicalAppId("Brave-Origin")) == NativeAppClass::ImeParityBrowser &&
              classifyNativeApp(*canonicalAppId("FIREFOX")) == NativeAppClass::Unavailable &&
              classifyNativeApp(*canonicalAppId("Zen")) == NativeAppClass::ImeParityBrowser &&
              classifyNativeApp(*canonicalAppId("App.Zen_browser.Zen")) == NativeAppClass::Unavailable &&
              classifyNativeApp(*canonicalAppId("Telegram")) == NativeAppClass::NativeExact,
          "classification uses the folded identity");
    // Folding admits these mixed-case window classes; none may become a
    // native exact app with the unknown-identity manual path.
    for (const auto program : {"chrome-app.hey.com__-Default", "chrome-nngceckbapebfimnlniiiahkandclblb-Default",
                               "com.google.Chrome", "org.chromium.Chromium", "com.brave.Browser",
                               "Microsoft-edge", "Vivaldi-stable", "Code-OSS", "VSCodium", "Electron",
                               "FFPWA-01HVY3F0GDT6DCG5V6TTRHK4BT", "Zen-Twilight", "org.mozilla.Firefox",
                               "Firefox-Nightly", "LibreWolf"}) {
        const auto canonical = canonicalAppId(program);
        check(canonical && classifyNativeApp(*canonical) == NativeAppClass::Unavailable,
              "folded Chromium- and Gecko-family window classes stay unavailable");
    }
}

void imeParityRequiresObservedAppendOnlyField() {
    auto allowedState = focusedState();
    const auto manual = explicitContext(allowedState).context;
    auto observed = manual;
    observed.identityKnown = true;
    for (const auto &[app, target, wrongTarget] : {
             std::tuple{"brave-origin", NativeEditTarget::BrowserOrigin, NativeEditTarget::DesktopApplication},
             std::tuple{"zen", NativeEditTarget::BrowserOrigin, NativeEditTarget::DesktopApplication},
             std::tuple{"code", NativeEditTarget::DesktopApplication, NativeEditTarget::BrowserOrigin}}) {
        auto unobserved = focusedState(app);
        check(!unobserved.editingAvailable() && unobserved.editPath() == NativeEditPath::Manual,
              "an IME-parity focus without an observed field cannot edit");
        check(!unobserved.updateContext(manual) && !unobserved.updateContext(observed),
              "the unknown-identity manual path is refused for IME-parity apps");
        SessionState mismatched;
        check(mismatched.focusIn(kSession, "observed-target", app, kSalt, wrongTarget, NativeEditPath::Observed) &&
                  !mismatched.editingAvailable() && !mismatched.updateContext(observed),
              "an observed field of the other target kind cannot edit");

        SessionState state;
        check(state.focusIn(kSession, "observed-target", app, kSalt, target, NativeEditPath::Observed) &&
                  state.editingAvailable(),
              "an observed field of the app's target kind may edit");
        check(!state.updateContext(manual) && !state.lastContext(),
              "even an observed session refuses unknown-identity context");
        auto explicitObserved = state.updateContext(observed);
        check(explicitObserved.has_value(), "explicit observed context is accepted");
        const auto wire = nlohmann::json::parse(*serializeContextEnvelope(*explicitObserved, 0))["payload"];
        check(wire["field"]["identity_known"] == true && wire["field"]["purpose"] == "normal" &&
                  wire["explicit"] == true && wire["activation"] == "manual",
              "explicit IME-parity requests remain observed requests on the wire");
        auto automatic = observed;
        automatic.explicitRequest = false;
        const auto update = state.updateContext(automatic);
        check(update.has_value(), "automatic observed context is accepted");

        const auto append = suggestionFor(*update);
        check(state.showSuggestion(append, 0), "append suggestions display on the observed field");
        const auto accept = state.requestAcceptance(1, kOwnedPanel);
        check(accept.has_value(), "IME-parity acceptance");
        CommitPrepare prepare{accept->coordinates, accept->controlId, accept->suggestionId,
                              accept->expectedText, "some"};
        check(!state.authorizeCommit(prepare, 2, kOwnedPanel),
              "only an acceptance of the whole append can dispatch");
        const auto refreshed = state.updateContext(automatic);
        check(refreshed && state.showSuggestion(suggestionFor(*refreshed), 3),
              "a fresh context recovers after the refused dispatch");
        const auto fresh = state.requestAcceptance(4, kOwnedPanel);
        check(fresh.has_value(), "fresh append acceptance");
        prepare = {fresh->coordinates, fresh->controlId, fresh->suggestionId, fresh->expectedText, "all"};
        const auto dispatch = state.authorizeCommit(prepare, 5, kOwnedPanel);
        check(dispatch && dispatch->text == append.text &&
                  !state.authorizeCommit(prepare, 5, kOwnedPanel),
              "an IME-parity append dispatches exactly once");

        const auto again = state.updateContext(automatic);
        check(again && state.showSuggestion(suggestionFor(*again), 6), "candidate before observer loss");
        const auto pending = state.requestAcceptance(6, kOwnedPanel);
        check(pending.has_value(), "acceptance pending before observer loss");
        const CommitPrepare lost{pending->coordinates, pending->controlId, pending->suggestionId,
                                 pending->expectedText, "all"};
        state.retireObservation();
        check(!state.editingAvailable() && !state.suggestionVisible() && !state.lastContext() &&
                  !state.authorizeCommit(lost, 7, kOwnedPanel),
              "observer loss retires IME-parity context, candidate and commit grant");
        check(!state.updateContext(manual) && !state.updateContext(observed),
              "observer loss never falls back to the manual path");
    }

    SessionState native;
    check(native.focusIn(kSession, "observed-target", "omawrite", kSalt, NativeEditTarget::DesktopApplication,
                         NativeEditPath::Observed) && native.editingAvailable(),
          "native exact apps keep observed desktop fields");
    native.retireObservation();
    check(native.editingAvailable() && native.updateContext(manual).has_value(),
          "native exact apps keep their manual contract after observer loss");
    for (const auto app : kUnavailableApps) {
        for (const auto target : {NativeEditTarget::DesktopApplication, NativeEditTarget::BrowserOrigin}) {
            SessionState state;
            check(state.focusIn(kSession, "observed-target", app, kSalt, target, NativeEditPath::Observed) &&
                      !state.editingAvailable() && !state.updateContext(observed),
                  "unavailable apps cannot edit even through an observed field");
        }
    }
}

void unobservedParityAndUnavailableAppsCannotEdit() {
    auto allowedState = focusedState();
    const auto allowed = explicitContext(allowedState);
    std::vector<std::string_view> blocked(kImeParityBrowsers.begin(), kImeParityBrowsers.end());
    blocked.insert(blocked.end(), kImeParityDesktop.begin(), kImeParityDesktop.end());
    blocked.insert(blocked.end(), kUnavailableApps.begin(), kUnavailableApps.end());
    for (const auto app : blocked) {
        auto state = focusedState(app);
        check(!state.editingAvailable(), "IME-parity and unavailable apps cannot edit without an observed field");
        check(!state.updateContext(allowed.context), "blocked app must not publish manual context");
        auto context = allowed.context;
        context.identityKnown = true;
        context.explicitRequest = false;
        check(!state.updateContext(context), "a claimed identity cannot bypass the missing observed field");
        auto suggestion = suggestionFor(allowed);
        suggestion.coordinates = state.coordinates();
        check(!state.showSuggestion(suggestion, 0) && !state.requestAcceptance(1, kOwnedPanel),
              "an unobserved app cannot display or accept injected suggestions");
    }
    for (const auto kind : {NativeEditTarget::BrowserOrigin, NativeEditTarget::Unsupported}) {
        SessionState state;
        check(state.focusIn(kSession, "input-context-1", "unrecognized-browser-alias", kSalt, kind),
              "unsupported target retains focus for explicit diagnostic notice");
        check(!state.editingAvailable() && !state.updateContext(allowed.context),
              "browser or unknown target kind cannot fall back to native app permission");
    }
    auto state = focusedState();
    const auto update = explicitContext(state);
    check(state.showSuggestion(suggestionFor(update), 0), "native manual candidate still displays");
    const auto accept = state.requestAcceptance(1, kOwnedPanel);
    check(accept.has_value(), "native manual acceptance remains available");
    const CommitPrepare prepare{accept->coordinates, accept->controlId, accept->suggestionId,
                                accept->expectedText, "all"};
    state.denyEditing();
    check(!state.lastContext() && !state.suggestionVisible() &&
              !state.authorizeCommit(prepare, 2, kOwnedPanel),
          "invalid target metadata retires prior candidate and dispatch authority");
    check(!state.updateContext(allowed.context), "invalid target remains denied until fresh field binding");
}

nlohmann::json observedField(std::string_view kind = "desktop_application") {
    nlohmann::json target{{"kind", kind}, {"target_id", "observed-target"}};
    target["app_id"] = "code";
    return {{"binding", {{"app_id", "code"}, {"field", 7}}}, {"target", target}, {"caret", 9},
            {"purpose", "plain_text"}, {"selection_count", 0}, {"total_chars", 9}};
}

void observerRepliesFailClosed() {
    const auto field = observedField();
    check(observerAnswered({{"ok", true}, {"focus", field}}) && !observerAnswered({{"ok", true}}) &&
              !observerAnswered({{"ok", false}, {"focus", field}}) &&
              !observerAnswered({{"ok", "true"}, {"focus", field}}),
          "only ok:true with a focus record is an answer");

    check(matchesObservedFocus(field, field) && matchesObservedFocus(field, field, true),
          "an unchanged field matches with and without its length");
    for (const auto &[key, value] : {std::pair{"caret", nlohmann::json(10)}, {"caret", -1}, {"caret", "9"},
                                    {"purpose", "password"}, {"selection_count", 1},
                                    {"binding", {{"app_id", "code"}, {"field", 8}}},
                                    {"target", observedField("browser")["target"]}}) {
        auto changed = field;
        changed[key] = value;
        check(!matchesObservedFocus(field, changed), "a moved caret, other field, purpose or selection never matches");
    }
    auto typed = field;
    typed["caret"] = 12;
    typed["total_chars"] = 12;
    check(!matchesObservedFocus(field, typed) && sameObservedField(field, typed) &&
              continuedFieldSnapshot(field, typed),
          "typing keeps the same observed field and its snapshot can be published");
    auto brokenCaret = typed;
    brokenCaret["caret"] = -1;
    check(!continuedFieldSnapshot(field, brokenCaret),
          "a snapshot with no caret does not continue the field");
    check(!sameObservedField(field, [&] {
              auto denied = field;
              denied["purpose"] = "password";
              return denied;
          }()),
          "a sensitive purpose is not the same field");
    auto rebound = field;
    rebound["binding"]["epoch"] = 99;
    rebound["target"]["target_id"] = "fresh-observer-target";
    check(sameObservedFieldIdentity(field, rebound) && !sameObservedField(field, rebound),
          "reacquisition may preserve quieting but cannot preserve an editing binding");
    rebound["binding"]["path"] = "/another/field";
    check(!sameObservedFieldIdentity(field, rebound), "quieting cannot cross field identity");
    auto cropped = field;
    cropped["total_chars"] = 12;
    check(matchesObservedFocus(field, cropped) && !matchesObservedFocus(field, cropped, true),
          "the same caret in a longer field matches only when length is not required");
    auto shorter = field;
    shorter["total_chars"] = 8;
    auto unknownLength = field;
    unknownLength.erase("total_chars");
    check(!matchesObservedFocus(unknownLength, shorter, true) && !matchesObservedFocus(field, unknownLength, true) &&
              matchesObservedFocus(unknownLength, field, true),
          "a snapshot needs a length at or beyond the caret that agrees with any captured length");

    check(inspectedEditTarget(field, "code") == NativeEditTarget::DesktopApplication &&
              inspectedEditTarget(observedField("browser"), "code") == NativeEditTarget::BrowserOrigin,
          "an inspected plain-text field names its edit target");
    auto otherApp = field;
    otherApp["target"]["app_id"] = "cursor";
    auto otherBinding = field;
    otherBinding["binding"]["app_id"] = "cursor";
    auto unknownKind = observedField("terminal");
    auto missingId = field;
    missingId["target"].erase("target_id");
    auto invalidId = field;
    invalidId["target"]["target_id"] = "not an id";
    auto password = field;
    password["purpose"] = "password";
    auto noCaret = field;
    noCaret.erase("caret");
    for (const auto &focus : {otherApp, otherBinding, unknownKind, missingId, invalidId, password, noCaret,
                              nlohmann::json("focus"), nlohmann::json::object()}) {
        check(!inspectedEditTarget(focus, "code"), "malformed or foreign field metadata retires authority");
    }

    {
        // A terminal's only context is the line typed into it (ADR 0009).
        // Ghostty and foot, Omarchy's default terminal, were tried live.
        for (const auto terminal : {"com.mitchellh.ghostty", "foot"}) {
            check(classifyNativeApp(terminal) == NativeAppClass::Terminal &&
                      nativeEditingAvailable(terminal, NativeEditTarget::DesktopApplication,
                                             NativeEditPath::TypedLine) &&
                      !nativeEditingAvailable(terminal, NativeEditTarget::DesktopApplication,
                                              NativeEditPath::Manual) &&
                      !nativeObservationAvailable(terminal),
                  "terminals edit only through their typed line and are never observed");
        }
        check(classifyNativeApp("footclient") != NativeAppClass::Terminal &&
                  classifyNativeApp("kitty") != NativeAppClass::Terminal,
              "a terminal is listed only after its own live trial");
        TypedLine line;
        line.type("git");
        check(!line.prose(), "a single token is never a request");
        line.type(" commit the läst");
        line.apply(LineEdit::Erase);
        line.apply(LineEdit::EraseWord);
        check(line.text() == "git commit the " && line.prose(), "typing, Backspace and Ctrl+W edit the line");
        line.apply(LineEdit::Lose);
        line.type("more");
        check(!line.tracking() && line.text().empty(), "after word or vertical motion nothing is tracked");
        line.apply(LineEdit::Clear);
        check(line.tracking() && line.text().empty(), "a new line tracks again");
        line.type("see the image");
        line.apply(LineEdit::Restart);
        line.type("is a sample of");
        check(line.tracking() && line.text() == "is a sample of" && line.prose(),
              "after a paste only what is typed next is followed");

        // The caret moves within a line Badi watched being typed.
        TypedLine middle;
        middle.type("send it to me");
        for (int moved = 0; moved < 6; ++moved) middle.apply(LineEdit::Left);
        check(middle.before() == "send it" && middle.caretScalars() == 7, "Left moves the caret one scalar");
        middle.apply(LineEdit::Type, " back");
        check(middle.text() == "send it back to me" && middle.before() == "send it back",
              "typing inserts at the caret");
        middle.apply(LineEdit::DeleteForward);
        middle.apply(LineEdit::End);
        middle.erase(3);
        check(middle.text() == "send it backto" && middle.caretScalars() == 14,
              "Delete, End and a correction's Backspaces edit where the line editor does");
        middle.apply(LineEdit::Home);
        middle.apply(LineEdit::Left);
        check(!middle.tracking(), "motion past the start may change lines, so the line is lost");
        middle.apply(LineEdit::End);
        check(middle.tracking() && middle.text().empty(), "End resumes from the end of the line");
        check(lineEditFor(::fcitx::Key(FcitxKey_Left), "") == LineEdit::Left &&
                  lineEditFor(::fcitx::Key("Control+Left"), "") == LineEdit::Lose &&
                  lineEditFor(::fcitx::Key("Control+a"), "") == LineEdit::Home &&
                  lineEditFor(::fcitx::Key("Control+U"), "") == LineEdit::Clear &&
                  lineEditFor(::fcitx::Key("Alt+b"), "") == LineEdit::Lose &&
                  lineEditFor(::fcitx::Key(FcitxKey_Up), "") == LineEdit::Lose &&
                  lineEditFor(::fcitx::Key("Control+Shift+V"), "") == LineEdit::Restart &&
                  lineEditFor(::fcitx::Key(FcitxKey_a), "a") == LineEdit::Type,
              "line keys are classified as Readline and most TUIs apply them");
    }
    {
        Suggestion shown;
        shown.text = " is a sample";
        shown.acceptWord = " is";
        const auto parts = candidateParts(shown);
        check(parts.lead.empty() && parts.strong == "is" && parts.rest == " a sample",
              "the panel shows the word Tab takes apart from the rest, without the leading space");
        shown.replaceBefore = "teh ";
        shown.text = "the ";
        shown.acceptWord = "the ";
        const auto correction = candidateParts(shown);
        check(correction.lead == "teh \u2192 " && correction.strong == "the" && correction.rest.empty(),
              "a spelling replacement is shown as the typo and its whole correction");
    }
    const auto context = captureContextWindow("thank you", 9, 9, true, "en");
    check(context && observerAgrees({{"before", "thank you"}, {"after", ""}}, *context) &&
              !observerAgrees({{"before", "thank you!"}, {"after", ""}}, *context) &&
              !observerAgrees({{"before", "thank you"}}, *context) && !observerAgrees("thank you", *context),
          "the observer must report exactly Fcitx's text around the caret");
    check(observerAgrees({{"before", "thank you"}, {"after", "\n"}}, *context) &&
              observerAgrees({{"before", "thank you"}, {"after", "\n\n"}}, *context) &&
              !observerAgrees({{"before", "thank you"}, {"after", "\n\n\n"}}, *context) &&
              !observerAgrees({{"before", "thank you"}, {"after", " \n"}}, *context) &&
              !observerAgrees({{"before", "thank you"}, {"after", "\nmore"}}, *context),
          "an observed block end and an empty Fcitx after both name the end of the field, and nothing else does");
    {
        // Writer's sentence leaves out the spaces just typed at a paragraph end.
        auto writer = *context;
        restoreObservedTrailingSpaces(writer, {{"before", "thank you "}, {"after", ""}});
        check(writer.before == "thank you " && writer.head == 10 && writer.anchor == 10 &&
                  observerAgrees({{"before", "thank you "}, {"after", ""}}, writer),
              "only spaces the observer verified at the caret are restored");
        for (const auto &observed : {nlohmann::json{{"before", "thank you x"}, {"after", ""}},
                                     nlohmann::json{{"before", "thank you "}, {"after", "more"}},
                                     nlohmann::json{{"before", "thank you\t"}, {"after", ""}},
                                     nlohmann::json{{"before", "Thank you "}, {"after", ""}}}) {
            auto unchanged = *context;
            restoreObservedTrailingSpaces(unchanged, observed);
            check(unchanged == *context, "any other difference stays a mismatch");
        }
    }
    const auto quoted = captureContextWindow("Quoted list\n\nThanks for the", 27, 27, true, "en");
    check(quoted && observerAgrees({{"before", "Thanks for the"}, {"after", ""}, {"scope", "block"}}, *quoted) &&
              !observerAgrees({{"before", ""}, {"after", ""}, {"scope", "block"}}, *captureContextWindow("Above\n", 6, 6, true, "en")) &&
              !observerAgrees({{"before", "Thanks for the"}, {"after", ""}}, *quoted) &&
              !observerAgrees({{"before", "for the"}, {"after", ""}, {"scope", "block"}}, *quoted) &&
              !observerAgrees({{"before", "Thanks for the"}, {"after", ""}, {"scope", "field"}}, *quoted) &&
              !observerAgrees({{"before", "Thanks for th"}, {"after", ""}, {"scope", "block"}}, *quoted),
          "a block-scoped snapshot must be Fcitx's text from a line start to the caret; whole-field scope stays exact");
    const auto midText = captureContextWindow("thank you all", 9, 9, true, "en");
    check(midText && !observerAgrees({{"before", "thank you"}, {"after", "\n"}}, *midText),
          "a block end never stands for text Fcitx still has after the caret");
    auto paragraph = captureContextWindow("thank you\n\n", 9, 9, true, "en");
    normalizeObservedParagraphEnd(*paragraph);
    check(observerAgrees({{"before", "thank you"}, {"after", "\n\n"}}, *paragraph) &&
              !observerAgrees({{"before", "thank you"}, {"after", ""}}, *paragraph),
          "agreement compares the raw paragraph end, not the normalized context");

    for (const auto error : {"sensitive_field", "unsupported_field", "ineligible_field", "selection_present",
                             "invalid_caret"}) {
        check(observerDeniedField(error), "the observer declined the field itself");
    }
    check(!observerDeniedField("observer_unavailable") && !observerDeniedField(nullptr),
          "an absent observer is not a denied field");
    check(snapshotFailureReason("stale_binding") == "observer_stale_binding" &&
              snapshotFailureReason("operation_timeout") == "observer_timeout" &&
              snapshotFailureReason(nullptr) == "observer_snapshot_denied",
          "snapshot failures keep their content-free reasons");
}

void observerRequestsNameTheBoundField() {
    const auto field = observedField();
    check(inspectRequest("code") == nlohmann::json{{"op", "inspect"}, {"app_id", "code"}},
          "inspection names only the app");
    check(snapshotRequest(field) == nlohmann::json{{"op", "snapshot"}, {"binding", field["binding"]},
                                                   {"policy_target", field["target"]}},
          "a snapshot re-reads the exact binding and target");
    check(previewRequest(field, " for your time", 900) ==
              nlohmann::json{{"op", "preview"}, {"binding", field["binding"]}, {"policy_target", field["target"]},
                             {"text", " for your time"}, {"expected_caret", 9}, {"expected_total_chars", 9},
                             {"ttl_ms", 900}},
          "a preview carries the caret and length the observer must re-verify");
    check(previewRequest(field, " for", 900, CaretRect{.x = 1496, .y = 1636, .width = 2, .height = 36, .scale = 2})
              ["caret_rect"] == nlohmann::json{{"x", 1496}, {"y", 1636}, {"width", 2}, {"height", 36}, {"scale", 2.0}},
          "a preview may carry the app's window-relative caret rectangle");

    const auto context = captureContextWindow("thank you", 9, 9, true, "en");
    auto observed = field;
    observed["before"] = "thank you";
    observed["after"] = "";
    check(observerCorroborates(field, observed, *context), "the same field and text corroborate");
    auto moved = observed;
    moved["total_chars"] = 12;
    auto edited = observed;
    edited["before"] = "thank you!";
    check(!observerCorroborates(field, moved, *context) && !observerCorroborates(field, edited, *context),
          "another length or text never corroborates");

    auto observedContext = *context;
    observedContext.identityKnown = true;
    check(!observedRequestBlocked(observed, observedContext, false, true), "an agreeing end-of-text snapshot requests");
    check(observedRequestBlocked(observed, *context, false, true) == "field_identity_unknown" &&
              observedRequestBlocked(observed, observedContext, true, true) == "foreign_ime_active" &&
              observedRequestBlocked(edited, observedContext, false, true) == "observer_context_mismatch",
          "identity, foreign composition and disagreement block the request");
    auto empty = *captureContextWindow("thank you", 0, 0, true, "en");
    empty.identityKnown = true;
    auto middle = *captureContextWindow("thank you", 5, 5, true, "en");
    middle.identityKnown = true;
    auto middleObserved = observed;
    middleObserved["before"] = "thank";
    middleObserved["after"] = " you";
    middleObserved["caret"] = 5;
    check(observedRequestBlocked(observed, empty, false, true) == "empty_prefix" &&
              !observedRequestBlocked(middleObserved, middle, false, true),
          "a request needs a prefix; an agreeing caret mid-text requests");

    auto dialog = observed;
    dialog["quiet"] = "file_dialog";
    auto narrow = observed;
    narrow["quiet"] = "narrow_field";
    auto oddQuiet = observed;
    oddQuiet["quiet"] = 7;
    check(observedRequestBlocked(dialog, observedContext, false, true) == "file_dialog" &&
              observedRequestBlocked(narrow, observedContext, false, true) == "narrow_field" &&
              observedRequestBlocked(oddQuiet, observedContext, false, true) == "quiet_field",
          "dialog and narrow facts block automatic requests; an unknown quiet fact fails quiet");
    check(!observedRequestBlocked(dialog, observedContext, false, false) &&
              !observedRequestBlocked(narrow, observedContext, false, false),
          "an explicit request still works in a dialog or narrow field");
    check(automaticQuietReason(dialog) == "file_dialog" && automaticQuietReason(narrow) == "narrow_field" &&
              !automaticQuietReason(observed),
          "the inspected field's quiet fact can stop automatic work before a snapshot");

    check(invalidates({{"event", "invalidate"}}, "code") && invalidates({{"app_id", ""}}, "code") &&
              invalidates({{"app_id", "code"}}, "code") && invalidates({{"app_id", 7}}, "code") &&
              !invalidates({{"app_id", "cursor"}}, "code"),
          "an invalidation concerns every app unless it names another");
}

void windowTextTravelsOnlyWithAnObservedFieldAndWithinItsBound() {
    auto state = focusedState();
    auto context = explicitContext(state, "Hi Marin").context;
    context.anchor = context.head = context.before.size();
    context.identityKnown = true;
    context.explicitRequest = false;
    auto update = *state.updateContext(context);
    auto payload = nlohmann::json::parse(*serializeContextEnvelope(update, 0))["payload"];
    check(!payload.contains("window_text"), "no window text unless the addon has it");
    update.windowText = "Re: Venue deposit. Marinella Costa wrote: the deposit is due.";
    payload = nlohmann::json::parse(*serializeContextEnvelope(update, 0))["payload"];
    check(payload["window_text"] == update.windowText, "an observed field carries its window text");
    update.windowText = std::string(kMaxWindowTextScalars + 1, 'a');
    check(!serializeContextEnvelope(update, 0), "window text beyond its bound fails closed");
    update.windowText = "Marinella";
    update.context.identityKnown = false;
    check(!serializeContextEnvelope(update, 0), "an unobserved field never carries window text");

    const auto field = observedField();
    check(snapshotRequest(field, true)["window"] == true && !snapshotRequest(field).contains("window"),
          "only an asking snapshot names the window");
    auto observed = field;
    observed["window_text"] = "Okonkwo Adeyemi";
    check(observedWindowText(observed) == "Okonkwo Adeyemi", "a well-formed window text is kept");
    for (const auto &bad : {nlohmann::json(7), nlohmann::json(""), nlohmann::json(std::string(kMaxWindowTextScalars + 1, 'a'))}) {
        observed["window_text"] = bad;
        check(observedWindowText(observed).empty(), "a malformed or overlong window text is dropped");
    }
}

void observedParagraphEndIsEndOfField() {
    // Chromium sends after == "\n\n" for the caret at the end of a <p>;
    // Codex's ProseMirror composer is such a <p>.
    const auto capture = [](std::string_view text, std::size_t caret) {
        auto context = captureContextWindow(text, caret, caret, true, "en");
        check(context.has_value(), "the composer text is captured");
        context->identityKnown = true;
        context->explicitRequest = false;
        return *context;
    };
    auto raw = capture("thank you\n\n", 9);
    check(raw.after == "\n\n" && !tabEligibleContext(raw), "an unnormalized paragraph end is mid-text");
    auto normalized = raw;
    normalizeObservedParagraphEnd(normalized);
    check(normalized.after.empty() && normalized.paragraphEnd == "\n\n" && observedAfter(normalized) == "\n\n" &&
              normalized.before == raw.before && normalized.head == 9 && tabEligibleContext(normalized),
          "the paragraph end is end of field, and observer agreement still sees it");
    auto twice = normalized;
    normalizeObservedParagraphEnd(twice);
    check(twice == normalized, "normalization is idempotent");
    check(normalized != capture("thank you", 9) && observedAfter(capture("thank you", 9)).empty(),
          "a field that ends at the caret is a different field state");
    auto line = capture("thank you\n", 9);
    normalizeObservedParagraphEnd(line);
    check(line.after.empty() && line.paragraphEnd == "\n" && observedAfter(line) == "\n" && tabEligibleContext(line) &&
              line != normalized,
          "a <div> line's single break is end of field too, and a different field state from a <p> end");
    for (const auto text : {std::string_view("thank you\n\n\n"), std::string_view("thank you \n\n"),
                            std::string_view("thank you\n\nmore"), std::string_view("thank you\nmore"),
                            std::string_view("thank you all")}) {
        auto other = capture(text, 9);
        const auto copy = other;
        normalizeObservedParagraphEnd(other);
        check(other == copy && other.paragraphEnd.empty() && !tabEligibleContext(other),
              "every other after-caret text stays mid-text and fails closed");
    }
    check(!captureContextWindow("thank you\r\n\r\n", 9, 9, true, "en"),
          "a carriage return is never context, so it cannot become end of field");

    SessionState state;
    check(state.focusIn(kSession, "observed-target", "chatgpt", kSalt, NativeEditTarget::DesktopApplication,
                        NativeEditPath::Observed), "observed Codex composer");
    const auto update = state.updateContext(normalized);
    check(update && update->context.after.empty() && update->context.paragraphEnd == "\n\n",
          "the observed IME-parity context is accepted as end of field");
    const auto wire = nlohmann::json::parse(*serializeContextEnvelope(*update, 0))["payload"];
    check(wire["after"] == "" && wire["before"] == "thank you" && wire["selection"]["head"] == 9,
          "the broker receives after = \"\" at the unchanged caret");
    SessionState plain;
    check(plain.focusIn(kSession, "observed-target", "chatgpt", kSalt, NativeEditTarget::DesktopApplication,
                        NativeEditPath::Observed), "observed plain field");
    const auto ending = plain.updateContext(capture("thank you", 9));
    check(ending && ending->coordinates.fingerprint != update->coordinates.fingerprint,
          "the fingerprint binds the raw after-caret text");
    check(state.showSuggestion(suggestionFor(*update), 0), "append suggestion on the normalized context");
    const auto accept = state.requestAcceptance(1, kOwnedPanel);
    check(accept.has_value(), "acceptance of the normalized context");
    const CommitPrepare prepare{accept->coordinates, accept->controlId, accept->suggestionId,
                                accept->expectedText, "all"};
    const auto dispatch = state.authorizeCommit(prepare, 2, kOwnedPanel);
    check(dispatch && dispatch->text == suggestionFor(*update).text, "acceptance stays one append at the caret");

    auto unobserved = normalized;
    unobserved.identityKnown = false;
    check(!state.updateContext(unobserved), "the normalization requires an observed identity");
    auto extra = normalized;
    extra.after = "x";
    check(!state.updateContext(extra), "a normalized context cannot carry other after-caret text");
    auto native = focusedState("omawrite");
    check(!native.updateContext(normalized), "native exact apps keep their unnormalized contract");
    SessionState browser;
    check(browser.focusIn(kSession, "observed-target", "brave-origin", kSalt, NativeEditTarget::BrowserOrigin,
                          NativeEditPath::Observed) && browser.updateContext(normalized).has_value(),
          "IME-parity browsers share the observed rule");
}

} // namespace

int main(int argc, char **argv) {
    const std::vector<std::pair<const char *, void (*)()>> tests{
        {"native app classes", nativeAppClassesAreExplicit},
        {"Chrome and Brave aliases without an observer rule",
         chromeAndBraveAliasesWithoutObserverRuleAreUnavailable},
        {"canonical app ids", canonicalAppIdsFoldAsciiCase},
        {"debug reasons and clear notices", reasonsAndNoticesNameTheBlockedPath},
        {"IME parity requires an observed append-only field", imeParityRequiresObservedAppendOnlyField},
        {"unobserved IME parity and unavailable apps", unobservedParityAndUnavailableAppsCannotEdit},
        {"observer replies fail closed", observerRepliesFailClosed},
        {"observer requests name the bound field", observerRequestsNameTheBoundField},
        {"inspection backoff without input", inspectionBacksOffWithoutInput},
        {"pre-input keys consume only Badi UI", preInputKeysConsumeOnlyBadiUi},
        {"held Tab waits for the carried rest", heldTabWaitsForTheCarriedRest},
        {"word acceptance grants only the next word", wordAcceptanceGrantsOnlyTheNextWord},
        {"invoke routes and waiting notices", invokeRoutesByEditPathAndExplainsWaits},
        {"Tab decision reasons", tabReasonNamesTheFirstBlocker},
        {"broker session retirement", brokerSessionRetiresSessionAndPolicyTogether},
        {"observed paragraph end is end of field", observedParagraphEndIsEndOfField},
        {"observed fields are append-only", observedFieldsAreAppendOnly},
        {"state transitions and identity", stateTransitionsAndIdentity},
        {"native app policy coherence", nativePolicyMustBeCoherent},
        {"window text bounds", windowTextTravelsOnlyWithAnObservedFieldAndWithinItsBound},
        {"unchanged toolkit republish",
         unchangedToolkitRepublishPreservesAuthority},
        {"fingerprint binding", fingerprintBindsExactCaptureAndSalt},
        {"sanitizer and identifiers", sanitizerAndIdentifiers},
        {"framing", framingIsBoundedAndIncremental},
        {"coalesced maximum frame", coalescedFramesRespectIndividualLimits},
        {"exact session control result", sessionControlResultIsExact},
        {"optional suggestion clear field",
         suggestionClearMatchesOptionalWireField},
        {"sensitive zero context", sensitiveCompositionAndSelectionAreZeroContext},
        {"fcitx5-qt surrounding echo", qtSurroundingEchoIsNarrow},
        {"context window scalar bounds", contextWindowIsBoundedInScalarValues},
        {"transport reconnect keeps Fcitx text but no broker grant",
         transportReconnectKeepsFcitxTextButNoBrokerGrant},
        {"explicit manual context wire", contextWireIsExplicitManualV2},
        {"session policy and explicit request split",
         sessionWireSeparatesPolicyFromExplicitRequest},
        {"stale and duplicate commit", staleAndDuplicateCommitsCannotDispatch},
        {"missing candidate ownership", missingCandidateCannotAuthorizeAcceptance},
        {"foreign IME and manual keys", foreignImeAndManualKeysYieldCooperatively},
        {"shifted letter chord normalization",
         shiftedLetterChordUsesFcitxNormalization},
    };
    try {
        check(argc == 2, "orthographic joiner fixtures path is required");
        std::ifstream input(argv[1]);
        check(input.good(), "orthographic fixtures must exist");
        const auto corpus = nlohmann::json::parse(input);
        for (const auto &fixture : corpus.at("fixtures")) {
            const auto text = fixture.at("text").get<std::string>();
            const auto valid = fixture.at("valid").get<bool>();
            check(sanitizeSuggestion(text).has_value() == valid && validContextText(text) == valid,
                  fixture.at("name").get_ref<const std::string &>().c_str());
        }
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
