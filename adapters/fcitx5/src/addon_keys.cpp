#include "addon.h"

#include <fcitx-utils/key.h>
#include <fcitx-utils/keysym.h>
#include <fcitx/event.h>
#include <fcitx/inputcontext.h>

#include <algorithm>
#include <optional>
#include <string>
#include <utility>

namespace badi::fcitx5 {

void BadiAddon::preKeyEvent(::fcitx::KeyEvent &event) {
    if (event.isVirtual()) return;
    if (event.rawKey().isModifier()) {
        const auto states = ::fcitx::Key::keySymToStates(event.rawKey().sym());
        if (event.isRelease()) heldModifierKeys_ &= ~states;
        else heldModifierKeys_ |= states;
    }
    if (event.isRelease()) return literalTabRelease(event);
    const auto key = event.key().normalize();
    const bool tab = key.check(::fcitx::Key(FcitxKey_Tab));
    const auto app = canonicalProgram(*event.inputContext());
    debug_.record(tab ? "tab" : "input", app, app.empty() ? "unidentified_app" : "input_received");
    auto *binding = bindingFor(*event.inputContext());
    if (!binding || !binding->state.focused()) return;
    if (revisionActive(*binding) && revisionKey(event, *binding)) return;
    if (!key.isModifier()) binding->inspection.input();
    const bool literalChord = isLiteralTabChord(event.key());
    if (binding->literalTabs && !literalChord) {
        // A key after the chord's modifiers came up, even a modifier whose
        // own update follows it, lets the waiting Tabs go first; another key
        // pressed while they are still down drops them.
        if (binding->literalTabTimer && binding->literalTabTimer->isEnabled()) sendLiteralTabs(*binding);
        else if (!key.isModifier()) dropLiteralTabs(*binding);
    }
    if (!binding->state.editingAvailable()) {
        debug_.record("request_blocked", app,
            editingUnavailableReason(binding->state.appId(), binding->observed()));
    }
    const auto panel = observePanel(*binding);
    const bool unclaimed = !event.filtered() && !event.accepted();
    const auto typed = typedText(key);
    // Copied: retiring the candidate below releases its text.
    const std::string expected(binding->state.suggestionVisible() ? binding->state.visibleText()
                                                                  : binding->typeThroughRest);
    const PreKey pressed{
        .modifier = key.isModifier(),
        .repeat = !!(event.key().states() & ::fcitx::KeyState::Repeat),
        .tab = tab && unclaimed,
        .escape = key.check(::fcitx::Key(FcitxKey_Escape)),
        .chord = key.check(invokeChord()) || key.check(acceptChord()),
        .all = key.check(allKey()) && unclaimed,
        .literalTab = literalChord && unclaimed,
        .tabTaken = binding->heldTab ||
                    tabMayHoldForRest(!binding->typeThroughRest.empty(), binding->typeThroughAutomatic,
                                      hasForeignImeUi(panel), binding->state.suggestionVisible()) ||
                    (binding->state.editingAvailable() && binding->state.editPath() == NativeEditPath::Manual),
        .typesSuggestion = typesSuggestionStart(typed, expected),
    };
    const auto action = decidePreKey(pressed, binding->state.editingAvailable(), !binding->ownedAuxiliary.empty(),
                                     binding->state.suggestionVisible(), panel);
    // A password field's keys leave no line at all: even without text,
    // their count and timing would describe the password.
    const bool sensitive =
        static_cast<bool>(event.inputContext()->capabilityFlags() & ::fcitx::CapabilityFlag::PasswordOrSensitive);
    if (!pressed.modifier && !sensitive) {
        // A key that types text is named only in a text run, and only in a
        // field Badi edits for an allowed app: never a password, a denied
        // field (the observer's credential labels), nor an app not allowed.
        const auto text =
            tracedKeyText(typed, binding->broker.policyAllowed && binding->state.editingAvailable());
        debug_.trace("key", app,
            {{"key", printableKey(typed) ? "printable" : key.toString()}, {"action", preKeyActionName(action)},
             {"ghost_visible", binding->state.suggestionVisible()}},
            text ? nlohmann::json{{"typed", *text}} : nlohmann::json(nullptr));
    }
    // A Tab in the gap after a word acceptance, while its remainder is
    // still returning on its own, is held for that remainder instead of
    // reaching the app (where it would move focus or complete a command).
    // This precedes the edit-path checks: the accepted word's own edit may
    // have retired the observed field until it is bound again. At most one
    // Tab is held; further Tabs while held are swallowed with it. Any
    // other key drops the held Tab and keeps its own meaning.
    if (pressed.tab && binding->heldTab) {
        event.filterAndAccept();
        return;
    }
    if (pressed.tab &&
        tabMayHoldForRest(!binding->typeThroughRest.empty(), binding->typeThroughAutomatic,
                          hasForeignImeUi(panel), binding->state.suggestionVisible())) {
        holdTab(*binding);
        event.filterAndAccept();
        return;
    }
    if (!pressed.modifier && !pressed.tab) dropHeldTab(*binding);
    // Every key the terminal receives edits Badi's record of its line.
    const bool reachesApp = !pressed.chord && action != PreKeyAction::Tab && action != PreKeyAction::Dismiss &&
                            action != PreKeyAction::AcceptAll && action != PreKeyAction::LiteralTab;
    if (reachesApp && binding->state.editPath() == NativeEditPath::TypedLine) lineKey(*binding, key, typed);
    switch (action) {
    case PreKeyAction::PassThrough:
        return;
    case PreKeyAction::CancelDeclining:
        declineContext(*binding);
        [[fallthrough]];
    case PreKeyAction::Cancel:
        clearTypeThrough(*binding);
        cancelForInput(*binding);
        return;
    case PreKeyAction::CancelTypingThrough: {
        if (binding->typeThroughBase.empty()) {
            if (const auto previous = binding->state.lastContext())
                binding->typeThroughBase = previous->context.before;
        }
        cancelForInput(*binding);
        binding->typedThrough = true;
        binding->typeThroughTyped += std::string(typed);
        binding->typeThroughRest = expected.substr(typed.size());
        debug_.record("input", app, "typed_through");
        return;
    }
    case PreKeyAction::Tab:
        tabKey(event, *binding, panel);
        return;
    case PreKeyAction::CloseNotice:
        binding->state.invalidateContext();
        clearOwnedPanel(*binding);
        return;
    case PreKeyAction::Dismiss:
        dismissSuggestion(*binding, panel);
        break;
    case PreKeyAction::AcceptAll:
        requestAcceptance(*binding);
        break;
    case PreKeyAction::LiteralTab:
        literalTab(*binding, event.rawKey().code());
        break;
    }
    event.filterAndAccept();
}

void BadiAddon::clearTypeThrough(Binding &binding) {
    binding.typedThrough = false;
    binding.typeThroughAutomatic = false;
    binding.typeThroughRest.clear();
    binding.typeThroughBase.clear();
    binding.typeThroughTyped.clear();
    binding.typeThroughWaits = 0;
}

// Ctrl+Shift+Tab where Badi would take Tab: the suggestion retires like it
// would for any edit, and the app gets a real Tab once the chord's
// modifiers are up (see lastModifierReleased).
void BadiAddon::literalTab(Binding &binding, int code) {
    clearTypeThrough(binding);
    cancelForInput(binding);
    // A Tab may complete or indent invisibly; follow what comes next.
    binding.typedLine.restart();
    binding.literalTabs = std::min(binding.literalTabs + 1, kMaxLiteralTabs);
    binding.literalTabOnKeyRelease = !heldModifierKeys_.testAny(literalTabChord().states());
    binding.literalTabCode = code;
    debug_.record("decision", binding.state.appId(), "literal_tab_waiting");
}

void BadiAddon::literalTabRelease(::fcitx::KeyEvent &event) {
    auto *binding = bindingFor(*event.inputContext());
    if (!binding || binding->literalTabs == 0) return;
    const auto &released = event.rawKey();
    if (binding->literalTabOnKeyRelease ? released.isModifier() : !lastModifierReleased(released)) return;
    binding->literalTabTimer = oneShot(kLiteralTabDelayUs, [this, context = binding->inputContext] {
        if (auto *current = bindingFor(*context)) sendLiteralTabs(*current);
    });
}

void BadiAddon::sendLiteralTabs(Binding &binding) {
    const auto count = std::exchange(binding.literalTabs, 0U);
    if (binding.literalTabTimer) binding.literalTabTimer->setEnabled(false);
    if (count == 0) return;
    if (!binding.state.focused() || !binding.inputContext->hasFocus()) {
        debug_.record("decision", binding.state.appId(), "literal_tab_dropped");
        return;
    }
    // Wayland's input-method-v2 frontend sends the release with the press.
    // Without the chord's keycode it would look Tab up under the modifier
    // state it last heard, where Shift turns Tab into ISO_Left_Tab.
    const bool pressReleases = binding.inputContext->frontendName() == "wayland_v2";
    const ::fcitx::Key tab(FcitxKey_Tab, ::fcitx::KeyStates(), binding.literalTabCode);
    for (unsigned int sent = 0; sent < count; ++sent) {
        binding.inputContext->forwardKey(tab, false);
        if (!pressReleases) binding.inputContext->forwardKey(tab, true);
    }
    // A suggestion shown again while the Tab waited is for the field as it
    // was; the Tab changes it, so retire it like any edit.
    clearTypeThrough(binding);
    cancelForInput(binding);
    binding.typedLine.restart();
    debug_.record("decision", binding.state.appId(), "literal_tab_sent");
}

void BadiAddon::dropLiteralTabs(Binding &binding) {
    if (binding.literalTabTimer) binding.literalTabTimer->setEnabled(false);
    if (std::exchange(binding.literalTabs, 0U) == 0) return;
    debug_.record("decision", binding.state.appId(), "literal_tab_dropped");
}

void BadiAddon::holdTab(Binding &binding) {
    binding.heldTab = true;
    binding.heldTabRest = binding.typeThroughRest;
    debug_.record("decision", binding.state.appId(), "tab_held");
    binding.heldTabTimer = oneShot(kHeldTabTimeoutUs, [this, context = binding.inputContext] {
        if (auto *current = bindingFor(*context)) {
            debug_.record("decision", current->state.appId(), "tab_held_dropped");
            // At most one hold per word acceptance: the wait is over, so
            // the next Tab reaches the application.
            current->typeThroughAutomatic = false;
            dropHeldTab(*current);
        }
    });
}

void BadiAddon::dropHeldTab(Binding &binding) {
    binding.heldTab = false;
    binding.heldTabRest.clear();
    if (binding.heldTabTimer) binding.heldTabTimer->setEnabled(false);
}

// Plain Tab is claimed before Fcitx's candidate navigation only to accept
// or to request on the manual path; otherwise it stays the application's.
void BadiAddon::tabKey(::fcitx::KeyEvent &event, Binding &binding, const PanelObservation &panel) {
    // An explicit key can renew an exhausted recovery budget without
    // reading the field before the new policy response arrives.
    if (!transport_.ready()) transport_.connect();
    const auto facts = requestFacts(binding);
    const auto context = facts.foreignIme ? std::nullopt : currentContext(binding);
    debug_.record("decision", binding.state.appId(), tabDecisionReason(facts, context),
                  context ? context->before.size() : 0);
    switch (decideTabAction(tabEligibleContext(context), binding.state.suggestionVisible(), panel,
                            binding.state.editPath())) {
    case LocalAction::AcceptWord:
        requestAcceptance(binding, true);
        break;
    case LocalAction::AcceptAll:
        requestAcceptance(binding);
        break;
    case LocalAction::Invoke:
        invoke(binding);
        break;
    case LocalAction::PassThrough:
    case LocalAction::Dismiss:
        cancelForInput(binding);
        // Completion rewrites the line invisibly; follow what comes next.
        binding.typedLine.restart();
        return;
    }
    event.filterAndAccept();
}

// A key the terminal received. Typing and the line edits Readline and most
// TUIs share are followed; anything else may move the cursor or insert text
// Badi cannot see, so tracking stops until the next empty line.
void BadiAddon::lineKey(Binding &binding, const ::fcitx::Key &key, std::string_view typed) {
    if (key.isModifier()) return;
    // Until the user allows this terminal, nothing typed is held (ADR 0009):
    // the line restarts at every key and is followed from where it is once
    // the policy allows it, as a line that had text at focus is.
    if (!binding.broker.policyAllowed) {
        binding.typedLine.restart();
        return;
    }
    binding.typedLine.apply(lineEditFor(key, typed), typed);
    requestLineAfterPause(binding);
}

void BadiAddon::requestLineAfterPause(Binding &binding) {
    if (binding.lineTimer) binding.lineTimer->setEnabled(false);
    if (!binding.typedLine.tracking() || !binding.typedLine.prose()) return;
    const auto pause = binding.typedThrough ? InspectionSchedule::kTypeThroughPauseUs
                                            : InspectionSchedule::kTypingPauseUs;
    binding.lineTimer = oneShot(pause, [this, context = binding.inputContext] {
        if (auto *current = bindingFor(*context)) requestTypedLine(*current, false);
    });
}

// A terminal's typed line as the context. Automatic requests need prose;
// the chord asks for any tracked line.
void BadiAddon::requestTypedLine(Binding &binding, bool explicitRequest) {
    const auto &app = binding.state.appId();
    if (explicitRequest) {
        clearOwnedPanel(binding);
        transport_.connect();
        queryPolicy(binding);
        if (const auto waiting = manualRequestNotice(requestFacts(binding))) return showNotice(binding, *waiting);
    }
    auto context = currentContext(binding);
    if (!context || context->before.empty() || (!explicitRequest && !binding.typedLine.prose())) {
        debug_.record("request_blocked", app, binding.typedLine.tracking() ? "context_unavailable" : "line_unknown");
        if (explicitRequest) showNotice(binding, binding.typedLine.tracking() ? notice::kFieldUnreadable
                                                                               : notice::kLineUnknown);
        return;
    }
    context->explicitRequest = explicitRequest;
    const auto update = binding.state.updateContext(*context);
    if (!update || !open(binding) || !transport_.publishContext(*update)) return;
    binding.broker.coordinates = update->coordinates;
    binding.requestExplicit = update->context.explicitRequest;
    debug_.record("request", app, explicitRequest ? "terminal_explicit_request" : "terminal_line_request",
                  context->before.size());
}

// After the input method: Badi's chords, and Escape on its candidate.
void BadiAddon::keyEvent(::fcitx::KeyEvent &event) {
    if (event.isRelease() || !!(event.key().states() & ::fcitx::KeyState::Repeat)) return;
    auto *binding = bindingFor(*event.inputContext());
    if (!binding || !binding->state.focused()) return;
    if (event.isVirtual() || event.filtered() || event.accepted()) return;
    const auto panel = observePanel(*binding);
    const auto key = event.key().normalize();
    const bool invokeChordPressed = key.check(invokeChord());
    const bool foreignIme = hasForeignImeUi(panel);
    // On a selection the chord asks for a revision of it (ADR 0012).
    if (invokeChordPressed && !foreignIme && revisionChord(*binding)) {
        event.filterAndAccept();
        return;
    }
    const bool claimInvoke = claimInvokeChord(binding->state.editingAvailable(),
                                               binding->broker.policyAllowed, foreignIme);
    if (invokeChordPressed && !claimInvoke && !foreignIme) {
        debug_.record("decision", binding->state.appId(),
                      binding->state.editingAvailable() ? "app_disabled" : "no_edit_path");
    }
    if (!binding->state.editingAvailable()) return;
    const bool liveOwnedCandidate = binding->state.suggestionVisible() && panel.candidatesOwnedByBadi;
    switch (decideLocalAction(claimInvoke && invokeChordPressed, key.check(acceptChord()),
                              key.check(::fcitx::Key(FcitxKey_Escape)),
                              liveOwnedCandidate, panel)) {
    case LocalAction::Invoke:
        invoke(*binding);
        break;
    case LocalAction::AcceptAll:
        requestAcceptance(*binding);
        break;
    case LocalAction::AcceptWord:
        requestAcceptance(*binding, true);
        break;
    case LocalAction::Dismiss:
        dismissSuggestion(*binding, panel);
        break;
    case LocalAction::PassThrough:
        if (liveOwnedCandidate && !key.isModifier()) {
            binding->state.invalidateContext();
            clearOwnedPanel(*binding);
        }
        return;
    }
    event.filterAndAccept();
}

// Explicit requests.
void BadiAddon::invoke(Binding &binding) {
    if (binding.state.editingAvailable() && binding.state.editPath() == NativeEditPath::TypedLine)
        return requestTypedLine(binding, true);
    switch (routeInvoke(binding.state.appId(), binding.state.editingAvailable(), binding.observed())) {
    case InvokeRoute::InspectField:
        return inspectOnRequest(binding);
    case InvokeRoute::Unavailable:
        return showNotice(binding, notice::kUnavailableApp);
    case InvokeRoute::ObservedField:
        return observeLater(binding, true);
    case InvokeRoute::Manual:
        return requestManually(binding);
    }
}

// An explicit IME-parity request inspects the field; it never becomes an
// unknown-identity manual request.
void BadiAddon::inspectOnRequest(Binding &binding) {
    debug_.record("observer", binding.state.appId(), "explicit_inspect_requested");
    if (const auto blocked = inspectionNotice(requestFacts(binding))) return showNotice(binding, *blocked);
    clearOwnedPanel(binding);
    transport_.connect();
    observeLater(binding, true);
}

void BadiAddon::requestManually(Binding &binding) {
    clearOwnedPanel(binding);
    transport_.connect();
    queryPolicy(binding);
    if (const auto waiting = manualRequestNotice(requestFacts(binding))) return showNotice(binding, *waiting);
    const auto context = currentContext(binding);
    if (!context) {
        binding.state.invalidateContext();
        debug_.record("request_blocked", binding.state.appId(), "context_unavailable");
        return showNotice(binding, notice::kFieldUnreadable);
    }
    const auto update = binding.state.updateContext(*context);
    if (!update) return;
    if (!open(binding)) return showNotice(binding, authorityPaused_ ? notice::kPaused : notice::kModelNotConnected);
    if (!transport_.publishContext(*update)) return showNotice(binding, notice::kRequestFailed);
    binding.requestExplicit = update->context.explicitRequest;
    debug_.record("request", binding.state.appId(), "sent_to_model", context->before.size());
    binding.broker.coordinates = update->coordinates;
    showNotice(binding, notice::kThinking);
}

void BadiAddon::requestAcceptance(Binding &binding, bool word) {
    // A second Tab or Ctrl+→ while the first acceptance is in flight is a
    // no-op. Clearing the panel here would cancel that acceptance.
    if (binding.state.acceptancePending()) return;
    const auto request = binding.state.editingAvailable()
        ? binding.state.requestAcceptance(transport_.nowMs(), observePanel(binding), word)
        : std::nullopt;
    if (!request || !transport_.requestAcceptance(*request)) return clearOwnedPanel(binding);
    // Retain local ownership until the grant is checked, but stop painting
    // extensions once the user has chosen this immutable candidate.
    binding.heldSuggestion.reset();
    if (binding.holdTimer) binding.holdTimer->setEnabled(false);
    hidePreview();
}

void BadiAddon::dismissSuggestion(Binding &binding, const PanelObservation &panel) {
    if (const auto request = binding.state.requestDismissal(transport_.nowMs(), panel)) {
        ++binding.observationGeneration;
        declineContext(binding);
        transport_.requestDismissal(*request);
    }
    clearOwnedPanel(binding);
}

void BadiAddon::declineContext(Binding &binding) {
    if (const auto &captured = binding.state.lastContext()) binding.dismissedContext = captured->context;
}

} // namespace badi::fcitx5
