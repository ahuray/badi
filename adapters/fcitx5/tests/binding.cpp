// Unit tests for Fcitx InputBinding's resettable parts, distinct expiry
// timers (notices, suggestions, revisions, commit waits and post-commit
// read-backs), owner-tagged PanelState, and unified retireBinding() across
// InputChanged, PolicyDenied, AuthorityChanged, Disconnected,
// ObservedFieldChanged, EditingDenied and FocusReset.

#include "binding.h"

#include <fcitx/candidatelist.h>

#include <cstdint>
#include <functional>
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string>
#include <utility>

namespace {

using namespace badi::fcitx5;
using Json = nlohmann::json;

constexpr auto kSession = "550e8400-e29b-41d4-a716-446655440000";
constexpr auto kContext = "660e8400-e29b-41d4-a716-446655440001";
constexpr auto kSalt = "0123456789abcdef0123456789abcdef";

void check(bool condition, const char *message) {
    if (!condition) throw std::runtime_error(message);
}

std::unique_ptr<::fcitx::EventSourceTime> arm(::fcitx::EventLoop &loop, std::uint64_t delayUs,
                                              std::function<void()> action) {
    auto timer = loop.addTimeEvent(CLOCK_MONOTONIC, 0, kTimerAccuracyUs,
        [action = std::move(action)](::fcitx::EventSourceTime *, std::uint64_t) {
            action();
            return true;
        });
    timer->setNextInterval(delayUs);
    timer->setOneShot();
    return timer;
}

Json zenObservedField() {
    return Json{
        {"binding", {{"app_id", "zen"}, {"accessible_id", "field-1"}, {"window_id", "win-1"}, {"pid", 42}}},
        {"target", {{"kind", "browser"}, {"app_id", "zen"}, {"target_id", "visit-1"},
                    {"scheme", "https"}, {"host", "allowed.example.test"}, {"port", 443}}},
        {"purpose", "plain_text"},
        {"caret", 5},
        {"total_chars", 5},
        {"selection_count", 0},
        {"before", "Thank"},
        {"after", ""},
    };
}

SelectionWindow sampleSelection(bool identityKnown = true) {
    auto window = captureSelectionWindow("Yesterday i went home.", 16, 10, false, "en");
    check(window.has_value(), "sample selection captured");
    window->identityKnown = identityKnown;
    return *window;
}

ContextWindow sampleContext(bool identityKnown = true) {
    auto window = captureContextWindow("Thank", 5, 5, false, "en");
    check(window.has_value(), "sample context captured");
    window->identityKnown = identityKnown;
    return *window;
}

void populateActiveBinding(::fcitx::EventLoop &loop, InputBinding &binding,
                           std::string appId = "zen", NativeEditTarget target = NativeEditTarget::BrowserOrigin,
                           NativeEditPath path = NativeEditPath::Observed) {
    check(binding.state.focusIn(kSession, "visit-1", std::move(appId), kSalt, target, path),
          "binding focuses session");
    binding.broker.answer(true, "en", true);
    binding.broker.open(binding.state.coordinates());

    binding.surrounding.freshness.surroundingTextUpdated();
    binding.surrounding.digest = 12345;

    auto candidates = std::make_shared<::fcitx::CommonCandidateList>();
    binding.panel.owner = PanelOwner::Suggestion;
    binding.panel.candidates = candidates;
    binding.panel.auxiliary = "aux";
    binding.panel.overlayOwned = true;

    binding.notice.generation = 3;
    binding.notice.expiryTimer = arm(loop, 500'000, [] {});

    auto context = sampleContext(path == NativeEditPath::Observed);
    const auto update = binding.state.updateContext(context);
    check(update.has_value(), "context update admitted");
    const Suggestion suggestion{
        .coordinates = update->coordinates,
        .requestId = suggestRequestId(update->coordinates),
        .suggestionId = "sug-1",
        .text = " you for writing",
        .expiresAtMs = 50'000,
        .acceptWord = " you",
    };
    check(binding.state.showSuggestion(suggestion, 1'000), "suggestion shown in SessionState");

    binding.suggestion.pendingId = "sug-pending";
    binding.suggestion.requestExplicit = true;
    binding.suggestion.dismissedContext = context;
    binding.suggestion.showAfterUs = 99'000;
    binding.suggestion.held = suggestion;
    binding.suggestion.holdTimer = arm(loop, 500'000, [] {});
    binding.suggestion.expiryTimer = arm(loop, 500'000, [] {});
    binding.suggestion.commitTimer = arm(loop, 500'000, [] {});

    binding.typeThrough.active = true;
    binding.typeThrough.automatic = true;
    binding.typeThrough.rest = " for writing";
    binding.typeThrough.base = "Thank";
    binding.typeThrough.typed = " you";
    binding.typeThrough.waits = 2;
    binding.typeThrough.heldTab = true;
    binding.typeThrough.heldTabRest = " for writing";
    binding.typeThrough.heldTabTimer = arm(loop, 500'000, [] {});

    binding.literalTab.count = 2;
    binding.literalTab.onKeyRelease = true;
    binding.literalTab.code = 23;
    binding.literalTab.timer = arm(loop, 500'000, [] {});

    binding.terminal.line.type("git commit -m hello world");
    binding.terminal.timer = arm(loop, 500'000, [] {});

    const auto field = zenObservedField();
    binding.observation.focus = field;
    if (binding.state.appId() == "zen") {
        binding.observation.zenCommit.record("zen", field, context, " you");
        check(binding.observation.zenCommit.active(), "Zen post-commit expectation recorded");
    }
    binding.observation.visit = field;
    binding.observation.visitId = "visit-1";
    binding.observation.windowVisitId = "visit-1";
    binding.observation.windowText = "nearby heading";
    binding.observation.generation = 7;
    binding.observation.explicitRequest = true;
    binding.observation.waitingForForeignUi = true;
    binding.observation.pendingBrokerStart = true;
    binding.observation.timer = arm(loop, 500'000, [] {});

    const auto selection = sampleSelection();
    binding.revision.waiting = std::make_pair(selection, std::string("grammar"));
    binding.revision.cycle = std::make_pair(selection, std::string("refine"));
    binding.revision.retries = 4;
    binding.revision.retryStep = RevisionRetryStep::Inspect;
    binding.revision.retryTimer = arm(loop, 500'000, [] {});
    binding.revision.noticeTimer = arm(loop, 500'000, [] {});
    binding.revision.expiryTimer = arm(loop, 500'000, [] {});
    binding.revision.commitTimer = arm(loop, 500'000, [] {});

    binding.readBack.correction = "Thank you";
    binding.readBack.correctionTimer = arm(loop, 500'000, [] {});
    binding.readBack.revision = std::make_pair(selection, std::string("I went"));
    binding.readBack.revisionTimer = arm(loop, 500'000, [] {});
}

void partsResetIndependentlyAndPanelHonoursOwner() {
    ::fcitx::EventLoop loop;
    InputBinding binding;
    populateActiveBinding(loop, binding);

    // Clearing suggestion display keeps showAfterUs and dismissedContext for
    // the typing pause, and leaves Notice and Revision timers untouched.
    binding.suggestion.clearDisplay();
    check(!binding.suggestion.pendingId.has_value() && !binding.suggestion.held.has_value(),
          "clearDisplay clears pending and held suggestions");
    check(!binding.suggestion.holdTimer->isEnabled() &&
              !binding.suggestion.expiryTimer->isEnabled() &&
              !binding.suggestion.commitTimer->isEnabled(),
          "clearDisplay disables all suggestion timers");
    check(binding.suggestion.showAfterUs == 99'000 && binding.suggestion.dismissedContext.has_value(),
          "clearDisplay keeps typing-pause deadline and dismissedContext");
    check(binding.notice.expiryTimer->isEnabled() && binding.revision.expiryTimer->isEnabled(),
          "suggestion clearDisplay never disarms notice or revision timers");

    // TypeThroughState separates carry clearing from held-Tab consumption.
    binding.typeThrough.clearCarry();
    check(!binding.typeThrough.active && !binding.typeThrough.automatic && binding.typeThrough.rest.empty(),
          "clearCarry clears remainder fields");
    check(binding.typeThrough.heldTab && binding.typeThrough.heldTabRest == " for writing" &&
              binding.typeThrough.heldTabTimer->isEnabled(),
          "clearCarry leaves heldTab intact for displaySuggestion to consume");
    binding.typeThrough.dropHeldTab();
    check(!binding.typeThrough.heldTab && binding.typeThrough.heldTabRest.empty() &&
              !binding.typeThrough.heldTabTimer->isEnabled(),
          "dropHeldTab clears heldTab and disables its timer");

    // PanelState owner scoping prevents cross-feature clobbering.
    binding.panel.owner = PanelOwner::Revision;
    binding.panel.candidates = std::make_shared<::fcitx::CommonCandidateList>();
    binding.panel.auxiliary = "Tab apply";
    binding.panel.overlayOwned = false;
    check(!binding.panel.clear(nullptr, PanelOwner::Suggestion),
          "a stale suggestion clear leaves a Revision panel untouched");
    check(!binding.panel.clear(nullptr, PanelOwner::Notice),
          "a notice clear leaves a Revision panel untouched");
    check(binding.panel.owner == PanelOwner::Revision && binding.panel.candidates != nullptr &&
              binding.panel.auxiliary == "Tab apply",
          "Revision panel still owns its candidates and help line");
    (void)binding.panel.clear(nullptr, PanelOwner::Revision);
    check(binding.panel.owner == PanelOwner::None && binding.panel.candidates == nullptr &&
              binding.panel.auxiliary.empty(),
          "matching Revision owner clears the panel");
}

void expiryTimersFireIndependentlyWithoutClobberingOtherParts() {
    ::fcitx::EventLoop loop;
    InputBinding binding;
    bool noticeExpired = false;
    bool suggestionExpired = false;
    bool revisionExpired = false;
    bool callbackSelfDisabled = false;

    // 1. Notice armed first (5ms), then replaced by a Suggestion (25ms):
    // clearing Notice disarms its 5ms timer so the Suggestion survives past 5ms
    // and expires only at its own 25ms deadline.
    binding.panel.owner = PanelOwner::Notice;
    binding.panel.auxiliary = "Badi is thinking…";
    const auto noticeGen = ++binding.notice.generation;
    binding.notice.expiryTimer = arm(loop, 5'000, [&] {
        if (binding.notice.generation == noticeGen) {
            noticeExpired = true;
            (void)binding.panel.clear(nullptr, PanelOwner::Notice);
        }
    });

    binding.notice.clear();
    binding.panel.owner = PanelOwner::Suggestion;
    binding.panel.auxiliary.clear();
    binding.panel.candidates = std::make_shared<::fcitx::CommonCandidateList>();
    binding.suggestion.expiryTimer = arm(loop, 20'000, [&] {
        suggestionExpired = true;
        binding.suggestion.clearDisplay();
        (void)binding.panel.clear(nullptr, PanelOwner::Suggestion);
    });

    // 2. Simultaneously arm a Revision expiry timer (35ms) and verify
    // suggestion expiry at 20ms neither disarms Revision nor wipes a Revision
    // panel when Revision takes over.
    binding.revision.expiryTimer = arm(loop, 35'000, [&] {
        revisionExpired = true;
        // Calling clearActive() inside its own callback must safely disable in
        // place without destroying the running closure.
        binding.revision.clearActive();
        callbackSelfDisabled = true;
        loop.exit();
    });

    auto guard = arm(loop, 500'000, [&] { loop.exit(); });
    loop.exec();

    check(!noticeExpired, "cleared notice timer never fires over a suggestion or revision");
    check(suggestionExpired, "suggestion expiry timer fires at its own lease");
    check(revisionExpired && callbackSelfDisabled,
          "revision expiry timer fires independently and safely disables itself in callback");
    check(!binding.revision.expiryTimer->isEnabled(), "revision timer disabled after firing");
}

void authorityChangeAndDisconnectRetireAllGrantsDeterministically() {
    ::fcitx::EventLoop loop;

    // AuthorityChanged retires broker, freshness, suggestion, typeThrough/heldTab,
    // revision, notice, zenCommit and windowText, while keeping observed focus/visit
    // for immediate policy re-query and keeping terminal typedLine.
    {
        InputBinding binding;
        populateActiveBinding(loop, binding);
        const auto res = retireBinding(binding, RetireReason::AuthorityChanged, kContext);
        check(res.ghostHidden && res.overlayHidden, "AuthorityChanged reports hidden ghost and overlay");
        check(!binding.broker.opened && !binding.broker.policyKnown && !binding.broker.policyAllowed,
              "AuthorityChanged retires BrokerSession");
        check(!binding.surrounding.freshness.fresh() && !binding.surrounding.freshness.freshForAutomatic(),
              "AuthorityChanged invalidates surrounding freshness");
        check(!binding.state.lastContext().has_value() && !binding.state.suggestionVisible(),
              "AuthorityChanged invalidates SessionState context and suggestion");
        check(!binding.panel.ownsSurface() && binding.panel.owner == PanelOwner::None,
              "AuthorityChanged clears PanelState");
        check(!binding.notice.expiryTimer->isEnabled(), "AuthorityChanged disables notice timer");
        check(!binding.suggestion.requestExplicit && binding.suggestion.showAfterUs == 0 &&
                  !binding.suggestion.dismissedContext.has_value() &&
                  !binding.suggestion.holdTimer->isEnabled() &&
                  !binding.suggestion.expiryTimer->isEnabled() &&
                  !binding.suggestion.commitTimer->isEnabled(),
              "AuthorityChanged resets SuggestionState completely");
        check(!binding.typeThrough.active && !binding.typeThrough.automatic &&
                  binding.typeThrough.rest.empty() && !binding.typeThrough.heldTab &&
                  !binding.typeThrough.heldTabTimer->isEnabled(),
              "AuthorityChanged resets TypeThroughState so Tab is never held while paused");
        check(!binding.revision.waiting.has_value() && !binding.revision.cycle.has_value() &&
                  !binding.revision.retryTimer->isEnabled() &&
                  !binding.revision.noticeTimer->isEnabled() &&
                  !binding.revision.expiryTimer->isEnabled() &&
                  !binding.revision.commitTimer->isEnabled(),
              "AuthorityChanged resets RevisionState and all four revision timers");
        check(!binding.observation.zenCommit.active() && binding.observation.windowText.empty() &&
                  !binding.observation.explicitRequest,
              "AuthorityChanged clears zenCommit, cached windowText and explicitRequest");
        check(binding.observed() && binding.observation.visitId == "visit-1",
              "AuthorityChanged keeps observed focus and visitId for immediate policy re-query");
        check(!binding.terminal.timer->isEnabled() && !binding.terminal.line.text().empty(),
              "AuthorityChanged stops terminal pause timer while keeping typed line");
    }

    // Disconnected retires the exact same grants, increments observation.generation,
    // and keeps surrounding.fresh() true (while freshForAutomatic() becomes false)
    // so manual/explicit requests work after reconnect without republishing automatically.
    {
        InputBinding binding;
        populateActiveBinding(loop, binding);
        const auto genBefore = binding.observation.generation;
        const auto res = retireBinding(binding, RetireReason::Disconnected, kContext);
        check(res.ghostHidden && res.overlayHidden, "Disconnected reports hidden ghost and overlay");
        check(!binding.broker.opened && !binding.broker.policyKnown, "Disconnected retires BrokerSession");
        check(binding.surrounding.freshness.fresh() && !binding.surrounding.freshness.freshForAutomatic(),
              "Disconnected keeps Fcitx surrounding text for explicit asks but blocks automatic republish");
        check(binding.observation.generation == genBefore + 1 && !binding.observation.zenCommit.active(),
              "Disconnected fences in-flight observer replies and clears zenCommit");
        check(!binding.typeThrough.active && !binding.typeThrough.automatic &&
                  binding.typeThrough.rest.empty() && !binding.typeThrough.heldTab &&
                  !binding.typeThrough.heldTabTimer->isEnabled(),
              "Disconnected resets TypeThroughState so Tab reaches the app immediately");
        check(!binding.revision.waiting.has_value() && !binding.revision.cycle.has_value() &&
                  !binding.revision.expiryTimer->isEnabled(),
              "Disconnected resets RevisionState");
        check(!binding.terminal.timer->isEnabled() &&
                  binding.terminal.line.text() == "git commit -m hello world",
              "Ghostty's typed line survives a broker disconnect");
    }
}

void policyDeniedObservedFieldEditingDeniedInputAndFocusRetireCleanly() {
    ::fcitx::EventLoop loop;

    // 1. PolicyDenied keeps the policy denial answer and any diagnostic Notice,
    // while closing broker.opened and wiping suggestions, revisions, carry and terminal line.
    {
        InputBinding binding;
        populateActiveBinding(loop, binding, "com.mitchellh.ghostty",
                              NativeEditTarget::DesktopApplication, NativeEditPath::TypedLine);
        binding.broker.answer(false, {}, false, true);
        binding.broker.opened = true;
        binding.panel.owner = PanelOwner::Notice;
        binding.panel.candidates.reset();
        binding.panel.overlayOwned = false;
        binding.panel.auxiliary = std::string(notice::kCheckingPermission);

        const auto res = retireBinding(binding, RetireReason::PolicyDenied, kContext);
        check(!res.ghostHidden && !res.overlayHidden, "diagnostic Notice is not a ghost");
        check(!binding.broker.opened && binding.broker.policyKnown && !binding.broker.policyAllowed &&
                  binding.broker.snoozed,
              "PolicyDenied closes opened flag while preserving policyKnown/snoozed answer");
        check(binding.noticeShown() && binding.panel.auxiliary == notice::kCheckingPermission,
              "PolicyDenied does not wipe a diagnostic Notice just shown");
        check(binding.terminal.line.text().empty() && !binding.terminal.timer->isEnabled(),
              "PolicyDenied wipes tracked terminal line per ADR 0009");
        check(!binding.typeThrough.active && !binding.typeThrough.heldTab &&
                  !binding.revision.waiting.has_value(),
              "PolicyDenied clears typeThrough and revision state");
    }

    // 2. ObservedFieldChanged preserves same-visit state (visit, zenCommit,
    // pendingBrokerStart, unnamedIbus, typeThrough, showAfterUs, dismissedContext)
    // while retiring observed focus and rolling SessionState to a fresh session.
    {
        InputBinding binding;
        populateActiveBinding(loop, binding, "zen", NativeEditTarget::BrowserOrigin, NativeEditPath::Observed);
        binding.observation.unnamedIbus = true;
        const auto oldSession = binding.state.coordinates().sessionId;
        (void)retireBinding(binding, RetireReason::ObservedFieldChanged, kContext);
        check(!binding.observed(), "ObservedFieldChanged clears observed focus");
        check(binding.state.focused() && binding.state.coordinates().sessionId != oldSession &&
                  binding.state.appId().empty() && !binding.state.editingAvailable() &&
                  binding.state.editPath() == NativeEditPath::Observed,
              "Unnamed IBus ObservedFieldChanged drops adopted appId and never falls back to Manual (ADR 0017)");
        check(binding.observation.visitId == "visit-1" &&
                  binding.observation.windowText == "nearby heading" &&
                  binding.observation.zenCommit.active() &&
                  binding.observation.pendingBrokerStart &&
                  binding.observation.unnamedIbus,
              "ObservedFieldChanged preserves visit, zenCommit (#197), pendingBrokerStart (#200) and unnamedIbus (#199)");
        check(binding.typeThrough.active && binding.typeThrough.heldTab &&
                  binding.suggestion.showAfterUs == 99'000 &&
                  binding.suggestion.dismissedContext.has_value(),
              "ObservedFieldChanged preserves type-through carry/heldTab (ADR 0004) and typing-pause gate");
    }

    // 3. EditingDenied clears pendingBrokerStart, zenCommit, visit, typeThrough,
    // suggestion, revision and terminal line, and marks editing unavailable.
    {
        InputBinding binding;
        populateActiveBinding(loop, binding, "omawrite",
                              NativeEditTarget::DesktopApplication, NativeEditPath::Observed);
        const auto genBefore = binding.observation.generation;
        (void)retireBinding(binding, RetireReason::EditingDenied, kContext);
        check(binding.state.focused() && !binding.state.editingAvailable(),
              "EditingDenied keeps focus for diagnostics with editingAvailable() false");
        check(!binding.observed() && binding.observation.visitId.empty() &&
                  !binding.observation.pendingBrokerStart && !binding.observation.zenCommit.active() &&
                  binding.observation.generation == genBefore + 1 &&
                  !binding.observation.timer->isEnabled(),
              "EditingDenied stops observation and clears pendingBrokerStart/zenCommit/visit");
        check(!binding.typeThrough.active && !binding.typeThrough.heldTab &&
                  !binding.revision.waiting.has_value() && binding.terminal.line.text().empty(),
              "EditingDenied resets typeThrough, revision and terminal line");
    }

    // 4. InputChanged fences pending observation and drops heldTab while keeping
    // typeThrough carry (for CancelTypingThrough) and showAfterUs.
    {
        InputBinding binding;
        populateActiveBinding(loop, binding);
        const auto genBefore = binding.observation.generation;
        (void)retireBinding(binding, RetireReason::InputChanged, kContext);
        check(binding.observation.generation == genBefore + 1 &&
                  !binding.observation.timer->isEnabled() &&
                  !binding.observation.zenCommit.active() &&
                  binding.observation.explicitRequest,
              "InputChanged fences observation, clears zenCommit and preserves explicitRequest");
        check(!binding.typeThrough.heldTab && binding.typeThrough.active,
              "InputChanged drops heldTab while leaving carry to the caller");
        check(binding.suggestion.showAfterUs == 99'000, "InputChanged keeps showAfterUs");
        check(!binding.revision.waiting.has_value() && !binding.revision.cycle.has_value(),
              "InputChanged resets revision waiting and cycle");
    }

    // 5. FocusReset clears every per-focus field and timer (including literalTab
    // and revision read-back) while keeping correction read-back alive (ADR 0011).
    {
        InputBinding binding;
        populateActiveBinding(loop, binding);
        binding.observation.unnamedIbus = true;
        (void)retireBinding(binding, RetireReason::FocusReset, kContext);
        check(!binding.state.focused() && !binding.broker.opened && !binding.observed(),
              "FocusReset ends SessionState focus, BrokerSession and observation");
        check(!binding.observation.pendingBrokerStart && !binding.observation.zenCommit.active() &&
                  binding.observation.visitId.empty() && binding.observation.unnamedIbus,
              "FocusReset clears pendingBrokerStart/zenCommit/visit and keeps unnamedIbus until focusIn");
        check(binding.literalTab.count == 0 && !binding.literalTab.timer->isEnabled(),
              "FocusReset drops queued literal Tabs and disables literalTab timer");
        check(binding.terminal.line.text().empty() && !binding.terminal.timer->isEnabled(),
              "FocusReset clears terminal line and disables its timer");
        check(!binding.readBack.revision.has_value() && !binding.readBack.revisionTimer->isEnabled(),
              "FocusReset clears revision read-back and disables its timer");
        check(binding.readBack.correction == std::optional<std::string>("Thank you") &&
                  binding.readBack.correctionTimer->isEnabled(),
              "FocusReset preserves IME-parity correction read-back across page-split focus (ADR 0011)");
    }
}

void revisionPreKeyClassificationPreservesInvokeAndFiltersRepeat() {
    const ::fcitx::Key invoke(FcitxKey_space,
                              ::fcitx::KeyStates{::fcitx::KeyState::Ctrl, ::fcitx::KeyState::Shift});
    const ::fcitx::Key invokeRepeat(
        FcitxKey_space,
        ::fcitx::KeyStates{::fcitx::KeyState::Ctrl, ::fcitx::KeyState::Shift, ::fcitx::KeyState::Repeat});
    const ::fcitx::Key ctrlMod(FcitxKey_Control_L, ::fcitx::KeyStates{::fcitx::KeyState::Ctrl});
    const ::fcitx::Key tab(FcitxKey_Tab);
    const ::fcitx::Key escape(FcitxKey_Escape);
    const ::fcitx::Key letterA(FcitxKey_a);

    check(classifyRevisionPreKey(invoke, true, false) == RevisionPreKey::PassInvokeToPostInput,
          "non-repeat Ctrl+Shift+Space passes through preKeyEvent to PostInputMethod revisionChord");
    check(classifyRevisionPreKey(invokeRepeat, true, false) == RevisionPreKey::FilterInvokeRepeat,
          "repeated Ctrl+Shift+Space is filtered in preKeyEvent instead of cancelling revision");
    check(classifyRevisionPreKey(invoke, false, false) == RevisionPreKey::RetireByKey,
          "already-claimed Ctrl+Shift+Space retires revision");
    check(classifyRevisionPreKey(invoke, true, true) == RevisionPreKey::YieldForeignIme,
          "foreign IME UI yields and cancels revision even on invoke chord");
    check(classifyRevisionPreKey(ctrlMod, true, false) == RevisionPreKey::KeepForModifier,
          "modifier press keeps active revision");
    check(classifyRevisionPreKey(tab, true, false) == RevisionPreKey::AcceptTab,
          "unclaimed Tab accepts or waits on active revision");
    check(classifyRevisionPreKey(escape, false, false) == RevisionPreKey::DismissEscape,
          "Escape dismisses active revision");
    check(classifyRevisionPreKey(letterA, true, false) == RevisionPreKey::RetireByKey,
          "ordinary typed key retires active revision and passes through");
}

} // namespace

int main() {
    partsResetIndependentlyAndPanelHonoursOwner();
    expiryTimersFireIndependentlyWithoutClobberingOtherParts();
    authorityChangeAndDisconnectRetireAllGrantsDeterministically();
    policyDeniedObservedFieldEditingDeniedInputAndFocusRetireCleanly();
    revisionPreKeyClassificationPreservesInvokeAndFiltersRepeat();
    std::cout << "badi-fcitx5-binding-tests: all checks passed\n";
    return 0;
}
