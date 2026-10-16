#include "addon.h"

#include <fcitx-utils/key.h>
#include <fcitx-utils/keysym.h>
#include <fcitx/event.h>
#include <fcitx/inputcontext.h>

#include <optional>
#include <string>

namespace badi::fcitx5 {

void BadiAddon::preKeyEvent(::fcitx::KeyEvent &event) {
    if (event.isRelease() || event.isVirtual()) return;
    const auto key = event.key().normalize();
    const bool tab = key.check(::fcitx::Key(FcitxKey_Tab));
    const auto app = canonicalProgram(*event.inputContext());
    debug_.record(tab ? "tab" : "input", app, app.empty() ? "unidentified_app" : "input_received");
    auto *binding = bindingFor(*event.inputContext());
    if (!binding || !binding->state.focused()) return;
    if (!key.isModifier()) binding->inspection.input();
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
        .typesSuggestion = typesSuggestionStart(typed, expected),
    };
    const auto action = decidePreKey(pressed, binding->state.editingAvailable(), !binding->ownedAuxiliary.empty(),
                                     binding->state.suggestionVisible(), panel);
    if (!pressed.modifier) {
        // A key that types text is named only in a text run; control keys
        // such as Tab and Escape also "type" a control character.
        const bool printable = !typed.empty() && static_cast<unsigned char>(typed[0]) >= 0x20 && typed != "\x7f";
        debug_.trace("key", app,
            {{"key", printable ? "printable" : key.toString()}, {"action", preKeyActionName(action)},
             {"ghost_visible", binding->state.suggestionVisible()}},
            printable ? nlohmann::json{{"typed", typed}} : nlohmann::json(nullptr));
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
    const bool reachesApp = !pressed.chord && action != PreKeyAction::Tab &&
                            action != PreKeyAction::Dismiss && action != PreKeyAction::AcceptAll;
    if (reachesApp && binding->state.editPath() == NativeEditPath::TypedLine) lineKey(*binding, key, typed);
    switch (action) {
    case PreKeyAction::PassThrough:
        return;
    case PreKeyAction::CancelDeclining:
        declineContext(*binding);
        [[fallthrough]];
    case PreKeyAction::Cancel:
        binding->typedThrough = false;
        binding->typeThroughAutomatic = false;
        binding->typeThroughRest.clear();
        binding->typeThroughBase.clear();
        binding->typeThroughTyped.clear();
        binding->typeThroughWaits = 0;
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
    }
    event.filterAndAccept();
}

void BadiAddon::holdTab(Binding &binding) {
    binding.heldTab = true;
    binding.heldTabRest = binding.typeThroughRest;
    debug_.record("decision", binding.state.appId(), "tab_held");
    binding.heldTabTimer = oneShot(kHeldTabTimeoutUs, [this, context = binding.inputContext] {
        if (auto *current = bindingFor(*context)) {
            debug_.record("decision", current->state.appId(), "tab_held_dropped");
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
        // Completion may rewrite a terminal's line in ways Badi cannot see.
        binding.typedLine.lose();
        return;
    }
    event.filterAndAccept();
}

// A key the terminal received. Typing and the line edits Readline and most
// TUIs share are followed; anything else may move the cursor or insert text
// Badi cannot see, so tracking stops until the next empty line.
void BadiAddon::lineKey(Binding &binding, const ::fcitx::Key &key, std::string_view typed) {
    if (key.isModifier()) return;
    const auto states = key.states();
    const bool ctrl = !!(states & ::fcitx::KeyState::Ctrl);
    const bool alt = !!(states & ::fcitx::KeyState::Alt);
    const auto sym = key.sym();
    // Fcitx may report a Ctrl letter in either case.
    const auto letter = [sym](::fcitx::KeySym lower, ::fcitx::KeySym upper) { return sym == lower || sym == upper; };
    auto &line = binding.typedLine;
    if (sym == FcitxKey_Return || sym == FcitxKey_KP_Enter ||
        (ctrl && !alt && (letter(FcitxKey_c, FcitxKey_C) || letter(FcitxKey_u, FcitxKey_U)))) {
        line.clear();
    } else if (sym == FcitxKey_BackSpace && !ctrl && !alt) {
        line.erase();
    } else if ((ctrl && letter(FcitxKey_w, FcitxKey_W)) || ((ctrl || alt) && sym == FcitxKey_BackSpace)) {
        line.eraseWord();
    } else if (!typed.empty() && static_cast<unsigned char>(typed[0]) >= 0x20 && typed != "\x7f") {
        line.type(typed);
    } else {
        line.lose();
    }
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
