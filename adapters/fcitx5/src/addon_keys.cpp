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
    auto *binding = bindingFor(*event.inputContext());
    const auto app = binding && !binding->state.appId().empty() ? binding->state.appId()
                                                                : canonicalProgram(*event.inputContext());
    debug_.record(tab ? "tab" : "input", app,
                  app.empty() && !(binding && binding->observation.unnamedIbus) ? "unidentified_app"
                                                                                : "input_received");
    if (!binding || !binding->state.focused()) return;
    // A password field's keys leave no line, carry or held Tab at all: even
    // without text, their count and timing would describe the password.
    const bool sensitive =
        static_cast<bool>(event.inputContext()->capabilityFlags() & ::fcitx::CapabilityFlag::PasswordOrSensitive);
    if (sensitive) {
        binding->observation.pendingBrokerStart = false;
        binding->typeThrough.reset();
    }
    if (binding->revisionActive() && revisionKey(event, *binding)) return;
    if (!key.isModifier()) binding->observation.schedule.input(monotonicUs());
    const bool literalChord = isLiteralTabChord(event.key());
    if (binding->literalTab.count && !literalChord) {
        // A key after the chord's modifiers came up, even a modifier whose
        // own update follows it, lets the waiting Tabs go first; another key
        // pressed while they are still down drops them.
        if (binding->literalTab.timer && binding->literalTab.timer->isEnabled()) sendLiteralTabs(*binding);
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
                                                                  : binding->typeThrough.rest);
    // Only the literal-Tab chord asks whether Tab would be taken; the manual
    // path's answer reads Fcitx's text, so other keys skip it.
    const bool tabTaken = literalChord && unclaimed &&
        (binding->typeThrough.heldTab ||
         tabMayHoldForRest(!binding->typeThrough.rest.empty(), binding->typeThrough.automatic,
                           hasForeignImeUi(panel), binding->state.suggestionVisible()) ||
         manualTabRequests(binding->state.editPath(), binding->state.editingAvailable(),
                           binding->broker.policyAllowed,
                           hasForeignImeUi(panel) ? std::nullopt : currentContext(*binding)));
    const PreKey pressed{
        .modifier = key.isModifier(),
        .repeat = !!(event.key().states() & ::fcitx::KeyState::Repeat),
        .tab = tab && unclaimed,
        .escape = key.check(::fcitx::Key(FcitxKey_Escape)),
        .chord = key.check(invokeChord()) || key.check(acceptChord()),
        .all = key.check(allKey()) && unclaimed,
        .literalTab = literalChord && unclaimed,
        .tabTaken = tabTaken,
        .typesSuggestion = typesSuggestionStart(typed, expected),
    };
    // A live numbered list takes its digits and Escape here, before a typed
    // line could record the digit as text; no other key is captured.
    if (binding->state.alternativeList() && !hasForeignImeUi(panel) && unclaimed) {
        if (pressed.escape) {
            binding->observation.zenCommit.clear();
            binding->state.clearAlternativeList();
            clearSuggestionUi(*binding);
            event.filterAndAccept();
            return;
        }
        const auto sym = static_cast<int>(key.sym());
        const int first = static_cast<int>(FcitxKey_1);
        const bool plain = !(key.states() & ::fcitx::KeyState::Ctrl) &&
                           !(key.states() & ::fcitx::KeyState::Alt) &&
                           !(key.states() & ::fcitx::KeyState::Super) &&
                           !(key.states() & ::fcitx::KeyState::Shift);
        if (plain && sym >= first && sym <= first + 8) {
            if (const auto word = alternativeListPick(
                    binding->state.alternativeList()->words,
                    static_cast<char>('1' + (sym - first)));
                word && requestAlternativePick(*binding, *word, panel)) {
                event.filterAndAccept();
                return;
            }
        }
    }
    const auto action = decidePreKey(pressed, binding->state.editingAvailable(), binding->noticeShown(),
                                     binding->state.suggestionVisible(), panel);
    // A typed key is the moment an app needs the model. Modifiers, repeats of
    // a key that already asked, and password fields do not start it. Observed
    // apps and unnamed GNOME IBus contexts defer the start until async inspect
    // confirms an editable field, so password fields and urlbars never wake it.
    if (!sensitive && !pressed.modifier && !pressed.repeat && printableKey(typed)) {
        // Manual native apps suggest only on explicit Tab/Ctrl+Shift+Space, so
        // ordinary typing there must not cold-start the model.
        if (binding->state.editingAvailable() && binding->state.editPath() != NativeEditPath::Manual) {
            binding->observation.pendingBrokerStart = false;
            ensureBroker();
        } else if (binding->observationCandidate()) {
            binding->observation.pendingBrokerStart = true;
        }
    }
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
    if (pressed.tab && binding->typeThrough.heldTab) {
        event.filterAndAccept();
        return;
    }
    if (pressed.tab &&
        tabMayHoldForRest(!binding->typeThrough.rest.empty(), binding->typeThrough.automatic,
                          hasForeignImeUi(panel), binding->state.suggestionVisible())) {
        holdTab(*binding);
        event.filterAndAccept();
        return;
    }
    if (!pressed.modifier && !pressed.tab) binding->typeThrough.dropHeldTab();
    // Every key the terminal receives edits Badi's record of its line.
    const bool reachesApp = !pressed.chord && action != PreKeyAction::Tab && action != PreKeyAction::Dismiss &&
                            action != PreKeyAction::AcceptAll && action != PreKeyAction::LiteralTab;
    if (reachesApp && binding->state.editPath() == NativeEditPath::TypedLine) lineKey(*binding, key, typed);
    switch (action) {
    case PreKeyAction::PassThrough:
        if (!pressed.modifier &&
            !(unclaimed && !pressed.repeat && binding->state.editingAvailable() &&
              binding->state.suggestionVisible() && panel.candidatesOwnedByBadi && hasOwnedCandidate(panel) &&
              !hasForeignImeUi(panel) &&
              (key.check(acceptChord()) || (key.check(invokeChord()) && binding->broker.policyAllowed)))) {
            binding->observation.zenCommit.clear();
        }
        return;
    case PreKeyAction::CancelDeclining:
        declineContext(*binding);
        [[fallthrough]];
    case PreKeyAction::Cancel:
        binding->typeThrough.clearCarry();
        retire(*binding, RetireReason::InputChanged);
        return;
    case PreKeyAction::CancelTypingThrough: {
        if (binding->typeThrough.base.empty()) {
            if (const auto previous = binding->state.lastContext())
                binding->typeThrough.base = previous->context.before;
        }
        retire(*binding, RetireReason::InputChanged);
        binding->typeThrough.active = true;
        binding->typeThrough.typed += std::string(typed);
        binding->typeThrough.rest = expected.substr(typed.size());
        debug_.record("input", app, "typed_through");
        return;
    }
    case PreKeyAction::Tab:
        tabKey(event, *binding, panel);
        return;
    case PreKeyAction::CloseNotice:
        binding->observation.zenCommit.clear();
        binding->state.invalidateContext();
        clearNotice(*binding);
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

// Ctrl+Shift+Tab where Badi would take Tab: the suggestion retires like it
// would for any edit, and the app gets a real Tab once the chord's
// modifiers are up (see lastModifierReleased).
void BadiAddon::literalTab(Binding &binding, int code) {
    binding.typeThrough.clearCarry();
    retire(binding, RetireReason::InputChanged);
    // A Tab may complete or indent invisibly; follow what comes next.
    binding.terminal.clear();
    binding.literalTab.count = std::min(binding.literalTab.count + 1, kMaxLiteralTabs);
    binding.literalTab.onKeyRelease = !heldModifierKeys_.testAny(literalTabChord().states());
    binding.literalTab.code = code;
    debug_.record("decision", binding.state.appId(), "literal_tab_waiting");
}

void BadiAddon::literalTabRelease(::fcitx::KeyEvent &event) {
    auto *binding = bindingFor(*event.inputContext());
    if (!binding || binding->literalTab.count == 0) return;
    const auto &released = event.rawKey();
    if (binding->literalTab.onKeyRelease ? released.isModifier() : !lastModifierReleased(released)) return;
    binding->literalTab.timer = oneShot(kLiteralTabDelayUs, [this, context = binding->inputContext] {
        if (auto *current = bindingFor(*context)) sendLiteralTabs(*current);
    });
}

void BadiAddon::sendLiteralTabs(Binding &binding) {
    const auto count = binding.literalTab.take();
    if (count == 0) return;
    if (!binding.state.focused() || !binding.inputContext->hasFocus()) {
        debug_.record("decision", binding.state.appId(), "literal_tab_dropped");
        return;
    }
    // Wayland's input-method-v2 frontend sends the release with the press.
    // Without the chord's keycode it would look Tab up under the modifier
    // state it last heard, where Shift turns Tab into ISO_Left_Tab.
    const bool pressReleases = binding.inputContext->frontendName() == "wayland_v2";
    const ::fcitx::Key tab(FcitxKey_Tab, ::fcitx::KeyStates(), binding.literalTab.code);
    for (unsigned int sent = 0; sent < count; ++sent) {
        binding.inputContext->forwardKey(tab, false);
        if (!pressReleases) binding.inputContext->forwardKey(tab, true);
    }
    // A suggestion shown again while the Tab waited is for the field as it
    // was; the Tab changes it, so retire it like any edit.
    binding.typeThrough.clearCarry();
    retire(binding, RetireReason::InputChanged);
    binding.terminal.clear();
    debug_.record("decision", binding.state.appId(), "literal_tab_sent");
}

void BadiAddon::dropLiteralTabs(Binding &binding) {
    if (binding.literalTab.take() == 0) return;
    debug_.record("decision", binding.state.appId(), "literal_tab_dropped");
}

void BadiAddon::holdTab(Binding &binding) {
    binding.typeThrough.heldTab = true;
    binding.typeThrough.heldTabRest = binding.typeThrough.rest;
    debug_.record("decision", binding.state.appId(), "tab_held");
    binding.typeThrough.heldTabTimer = oneShot(kHeldTabTimeoutUs, [this, context = binding.inputContext] {
        if (auto *current = bindingFor(*context)) {
            debug_.record("decision", current->state.appId(), "tab_held_dropped");
            // At most one hold per word acceptance: the wait is over, so
            // the next Tab reaches the application.
            current->typeThrough.automatic = false;
            current->typeThrough.dropHeldTab();
        }
    });
}

// Plain Tab is claimed before Fcitx's candidate navigation only to accept
// or to request on the manual path; otherwise it stays the application's.
void BadiAddon::tabKey(::fcitx::KeyEvent &event, Binding &binding, const PanelObservation &panel) {
    const auto facts = requestFacts(binding);
    const auto context = facts.foreignIme ? std::nullopt : currentContext(binding);
    debug_.record("decision", binding.state.appId(), tabDecisionReason(facts, context),
                  context ? context->before.size() : 0);
    switch (decideTabAction(tabEligibleContext(context), binding.state.suggestionVisible(), panel,
                            binding.state.editPath())) {
    case TabAction::AcceptWord:
        ensureBroker();
        requestAcceptance(binding, true);
        break;
    case TabAction::Invoke:
        ensureBroker();
        invoke(binding);
        break;
    case TabAction::PassThrough:
        retire(binding, RetireReason::InputChanged);
        // Completion rewrites the line invisibly; follow what comes next.
        binding.terminal.clear();
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
        binding.terminal.clear();
        return;
    }
    binding.terminal.line.apply(lineEditFor(key, typed), typed);
    requestLineAfterPause(binding);
}

void BadiAddon::requestLineAfterPause(Binding &binding) {
    binding.terminal.stopTimer();
    if (!binding.terminal.line.tracking() || !binding.terminal.line.prose()) return;
    const auto pause = binding.typeThrough.active ? InspectionSchedule::kTypeThroughPauseUs
                                                  : InspectionSchedule::kTypingPauseUs;
    binding.terminal.timer = oneShot(pause, [this, context = binding.inputContext] {
        if (auto *current = bindingFor(*context)) requestTypedLine(*current, false);
    });
}

// A terminal's typed line as the context. Automatic requests need prose;
// the chord asks for any tracked line.
void BadiAddon::requestTypedLine(Binding &binding, bool explicitRequest) {
    const auto &app = binding.state.appId();
    if (explicitRequest) {
        clearAllUi(binding);
        ensureBroker();
        queryPolicy(binding);
        if (const auto waiting = manualRequestNotice(requestFacts(binding))) return showNotice(binding, *waiting);
    }
    auto context = currentContext(binding);
    if (!context || context->before.empty() || (!explicitRequest && !binding.terminal.line.prose())) {
        debug_.record("request_blocked", app,
                      binding.terminal.line.tracking() ? "context_unavailable" : "line_unknown");
        if (explicitRequest) {
            showNotice(binding, binding.terminal.line.tracking() ? notice::kFieldUnreadable
                                                                 : notice::kLineUnknown);
        }
        return;
    }
    context->explicitRequest = explicitRequest;
    const auto update = binding.state.updateContext(*context);
    if (!update || !open(binding) || !transport_.publishContext(*update)) return;
    binding.broker.coordinates = update->coordinates;
    binding.suggestion.requestExplicit = update->context.explicitRequest;
    debug_.record("request", app, explicitRequest ? "terminal_explicit_request" : "terminal_line_request",
                  context->before.size());
}

// After the input method: Badi's chords.
void BadiAddon::keyEvent(::fcitx::KeyEvent &event) {
    if (event.isRelease() || !!(event.key().states() & ::fcitx::KeyState::Repeat)) return;
    auto *binding = bindingFor(*event.inputContext());
    if (!binding || !binding->state.focused()) return;
    if (event.isVirtual()) return;
    const auto key = event.key().normalize();
    if (event.filtered() || event.accepted()) {
        if (!key.isModifier()) binding->observation.zenCommit.clear();
        return;
    }
    const auto panel = observePanel(*binding);
    const bool invokeChordPressed = key.check(invokeChord());
    const bool foreignIme = hasForeignImeUi(panel);
    // On a selection the chord asks for a revision of it (ADR 0012).
    if (invokeChordPressed && !foreignIme && revisionChord(*binding)) {
        binding->observation.zenCommit.clear();
        event.filterAndAccept();
        return;
    }
    const bool claimInvoke = claimInvokeChord(binding->state.editingAvailable(),
                                               binding->broker.policyAllowed, foreignIme);
    // On a shown continuation the chord asks for another one.
    if (invokeChordPressed && claimInvoke && requestAlternative(*binding, panel)) {
        event.filterAndAccept();
        return;
    }
    if (invokeChordPressed && !claimInvoke && !foreignIme) {
        debug_.record("decision", binding->state.appId(),
                      binding->state.editingAvailable() ? "app_disabled" : "no_edit_path");
    }
    if (!binding->state.editingAvailable()) {
        if (!key.isModifier()) binding->observation.zenCommit.clear();
        return;
    }
    // Escape, Tab and Ctrl+Right were decided before the input method.
    const bool liveOwnedCandidate = binding->state.suggestionVisible() && panel.candidatesOwnedByBadi;
    switch (decideLocalAction(claimInvoke && invokeChordPressed, key.check(acceptChord()), liveOwnedCandidate,
                              panel)) {
    case LocalAction::Invoke:
        invoke(*binding);
        break;
    case LocalAction::AcceptAll:
        requestAcceptance(*binding);
        break;
    case LocalAction::PassThrough:
        if (!key.isModifier()) binding->observation.zenCommit.clear();
        if ((liveOwnedCandidate ||
             (binding->state.alternativeList() && panel.candidatesOwnedByBadi)) &&
            !key.isModifier()) {
            binding->state.invalidateContext();
            clearSuggestionUi(*binding);
        }
        return;
    }
    event.filterAndAccept();
}

// Explicit requests.
void BadiAddon::invoke(Binding &binding) {
    binding.observation.zenCommit.clear();
    if (binding.state.editingAvailable() && binding.state.editPath() == NativeEditPath::TypedLine) {
        ensureBroker();
        return requestTypedLine(binding, true);
    }
    switch (routeInvoke(binding.state.appId(), binding.state.editingAvailable(), binding.observed())) {
    case InvokeRoute::InspectField:
        return inspectOnRequest(binding);
    case InvokeRoute::Unavailable:
        return showNotice(binding, notice::kUnavailableApp);
    case InvokeRoute::ObservedField:
        ensureBroker();
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
    clearAllUi(binding);
    ensureBroker();
    observeLater(binding, true);
}

void BadiAddon::requestManually(Binding &binding) {
    clearAllUi(binding);
    ensureBroker();
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
    binding.suggestion.requestExplicit = update->context.explicitRequest;
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
    if (!request || !transport_.requestAcceptance(*request)) {
        binding.observation.zenCommit.clear();
        return clearSuggestionUi(binding);
    }
    // Retain local ownership until the grant is checked, but stop painting
    // extensions once the user has chosen this immutable candidate.
    binding.suggestion.clearHeld();
    hidePreview();
}

bool BadiAddon::requestAlternative(Binding &binding, const PanelObservation &panel) {
    const auto request = binding.state.requestAlternative(transport_.nowMs(), panel);
    if (!request || !transport_.requestAlternative(*request)) return false;
    // A press that finds nothing new says so, as any explicit request.
    binding.suggestion.requestExplicit = true;
    debug_.record("request", binding.state.appId(), "alternative_requested");
    return true;
}

void BadiAddon::dismissSuggestion(Binding &binding, const PanelObservation &panel) {
    binding.observation.zenCommit.clear();
    if (const auto request = binding.state.requestDismissal(transport_.nowMs(), panel)) {
        ++binding.observation.generation;
        declineContext(binding);
        transport_.requestDismissal(*request);
    }
    clearSuggestionUi(binding);
}

void BadiAddon::declineContext(Binding &binding) {
    if (const auto &captured = binding.state.lastContext()) binding.suggestion.dismissedContext = captured->context;
}

} // namespace badi::fcitx5
