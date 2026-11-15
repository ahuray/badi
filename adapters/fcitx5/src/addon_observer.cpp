#include "addon.h"
#include "fcitx_event.h"

#include <fcitx/inputcontext.h>
#include <fcitx/instance.h>

#include <cstdint>
#include <utility>

namespace badi::fcitx5 {

void BadiAddon::observeLater(Binding &binding, bool explicitRequest, bool retry, bool early) {
    if (!binding.inputContext->hasFocus() || !binding.state.focused() ||
        !binding.observationCandidate()) return;
    if (!retry) binding.inspection.resetBusyRetries();
    binding.observationExplicit = explicitRequest;
    if (explicitRequest) {
        binding.dismissedContext.reset();
        binding.showAfterUs = 0;
        binding.heldSuggestion.reset();
        if (binding.holdTimer) binding.holdTimer->setEnabled(false);
    }
    if (!binding.observeTimer) {
        binding.observeTimer = instance_->eventLoop().addTimeEvent(CLOCK_MONOTONIC, 0, kTimerAccuracyUs,
            [this, contextId = uuidString(binding.inputContext->uuid())](::fcitx::EventSourceTime *, std::uint64_t) {
                const auto found = bindings_.find(contextId);
                if (found != bindings_.end() && found->second.inputContext->hasFocus()) inspectField(found->second);
                return true;
            });
    }
    binding.observeTimer->setNextInterval(
        binding.inspection.delayUs(explicitRequest && !retry, binding.typedThrough, retry, early && !retry));
    binding.observeTimer->setOneShot();
}

void BadiAddon::inspectField(Binding &binding) {
    // A revision's selection is the field's own state: an inspection for a
    // caret would decline it.
    if (revisionActive(binding)) return;
    // Inspection reads only the focused accessible, so fcitx5-qt's
    // withdrawal echo does not stop it; reading text waits for observation.
    if (!binding.observationCandidate() ||
        !allowsNativeContext(observedFieldCapabilities(binding.inputContext->capabilityFlags(),
                                                       binding.surroundingEcho)) ||
        !binding.surroundingFreshness.fresh()) return;
    if (accessibility_->pending()) {
        // Bounded, so an absent service or a denied field never loops the timer.
        if (binding.inspection.retryWhileBusy()) {
            debug_.record("observer", binding.state.appId(), "observer_busy_retry");
            observeLater(binding, binding.observationExplicit, true);
        }
        else observerUnavailable(binding);
        return;
    }
    if (foreignImeActive(binding)) {
        binding.waitingForForeignUi = true;
        return;
    }
    binding.waitingForForeignUi = false;
    binding.inspection.inspecting(monotonicUs());
    debug_.record("observer", binding.state.appId(), "inspect_sent");
    const auto session = binding.state.coordinates().sessionId;
    const auto generation = binding.observationGeneration;
    const auto request = binding.unnamedIbus ? unnamedIbusInspectRequest()
                                             : inspectRequest(binding.state.appId());
    const bool sent = accessibility_->request(request,
        [this, session, generation](const Json &reply) {
            auto *current = bindingFor(session);
            if (current && current->inputContext->hasFocus() && current->observationGeneration == generation)
                inspected(*current, reply);
        });
    if (!sent) observerUnavailable(binding);
}

void BadiAddon::inspected(Binding &binding, const Json &reply) {
    debug_.record("observer", binding.state.appId(), "inspect_answered");
    if (!observerAnswered(reply)) return observerUnavailable(binding, reply.value("error", Json()));
    const auto &focus = reply["focus"];
    std::string appId = binding.state.appId();
    std::optional<NativeEditTarget> editTarget;
    if (binding.unnamedIbus) {
        if (const auto resolved = unnamedIbusEditTarget(focus)) {
            appId = resolved->first;
            editTarget = resolved->second;
        }
    } else {
        editTarget = inspectedEditTarget(focus, appId);
    }
    binding.inspection.found(editTarget.has_value());
    if (!editTarget) return denyEditing(binding);
    const bool unchanged = !binding.observationExplicit && !binding.zenCommit.active() && binding.observed() &&
        binding.state.appId() == appId &&
        matchesObservedFocus(binding.observedFocus, focus) &&
        matchesCapturedContext(binding.state.lastContext(), currentContext(binding));
    if (unchanged) return;
    // Typing in the field we already authorized is a context update, not a
    // new session or a new policy query.
    if (binding.broker.opened && binding.broker.policyAllowed &&
        binding.state.appId() == appId &&
        sameObservedField(binding.observedFocus, focus)) {
        binding.observedFocus = focus;
        debug_.record("observer", binding.state.appId(), "same_field_request");
        return requestObservedContext(binding);
    }
    bindObservedField(binding, focus, *editTarget, appId);
}

// A newly observed field gets its own session, and policy for its exact target.
void BadiAddon::bindObservedField(Binding &binding, const Json &focus, NativeEditTarget editTarget,
                                  std::string_view resolvedAppId) {
    const std::string appId(resolvedAppId.empty() ? std::string_view(binding.state.appId()) : resolvedAppId);
    if (binding.unnamedIbus && !unnamedIbusObservedApp(appId)) return denyEditing(binding);
    const auto session = randomUuid();
    const auto salt = randomSalt();
    if (!session || !salt) return;
    if (!binding.zenCommit.sameField(focus)) binding.zenCommit.clear();
    if (!sameObservedFieldIdentity(binding.fieldVisit, focus)) {
        binding.fieldVisit = focus;
        binding.fieldVisitId = *session;
        binding.dismissedContext.reset();
    }
    closeBrokerSession(binding);
    clearOwnedPanel(binding);
    if (!binding.state.focusIn(*session, binding.fieldVisitId, appId, *salt,
                               editTarget, NativeEditPath::Observed)) return;
    binding.observedFocus = focus;
    if (!binding.state.editingAvailable()) {
        binding.pendingBrokerStart = false;
        binding.zenCommit.clear();
        debug_.record("request_blocked", appId, editingUnavailableReason(appId, binding.observed()));
        return;
    }
    if (std::exchange(binding.pendingBrokerStart, false)) ensureBroker();
    queryPolicy(binding);
}

void BadiAddon::denyEditing(Binding &binding) {
    const std::string priorApp = binding.state.appId();
    binding.pendingBrokerStart = false;
    binding.zenCommit.clear();
    binding.fieldVisit = nullptr;
    binding.fieldVisitId.clear();
    if (binding.unnamedIbus) {
        retireObservedField(binding);
        ++binding.observationGeneration;
    } else {
        closeBrokerSession(binding);
        ++binding.observationGeneration;
        binding.state.denyEditing();
        binding.observedFocus = nullptr;
        clearOwnedPanel(binding);
    }
    debug_.record("request_blocked", priorApp,
        imeParityApp(priorApp) ? "ime_parity_target_invalid" : "editor_transaction_unavailable");
}

// IME-parity and unnamed-IBus contexts have no manual fallback, so an explicit
// request says why the observer produced no field: an unsupported app, the
// field's own purpose, or no observer.
void BadiAddon::observerUnavailable(Binding &binding, const Json &error) {
    const std::string priorApp = binding.state.appId();
    binding.pendingBrokerStart = false;
    binding.zenCommit.clear();
    binding.inspection.found(false);
    binding.fieldVisit = nullptr;
    binding.fieldVisitId.clear();
    // A failed inspection must not keep the previous field's grant.
    if (binding.observed() || (binding.unnamedIbus && !priorApp.empty())) retireObservedField(binding);
    if (!imeParityApp(priorApp) && !binding.unnamedIbus) return;
    if (binding.unnamedIbus && error == "unsupported_app") {
        debug_.record("request_blocked", priorApp, "unidentified_app");
        if (binding.observationExplicit) showNotice(binding, notice::kUnavailableApp);
        return;
    }
    const bool fieldDenied = observerDeniedField(error);
    debug_.record("request_blocked", priorApp,
        fieldDenied ? "ime_parity_field_denied" : "ime_parity_observer_unavailable");
    if (binding.observationExplicit)
        showNotice(binding, fieldDenied ? notice::kFieldUnreadable : notice::kObserverUnavailable);
}

void BadiAddon::observerInvalidated(const Json &event) {
    const bool fieldChanged = event.value("reason", Json()) == "field_changed";
    for (auto &[_, binding] : bindings_) {
        if (!binding.state.focused() || !invalidates(event, binding.state.appId())) continue;
        debug_.record("observer", binding.state.appId(), "observer_invalidated");
        if (!fieldChanged) {
            binding.pendingBrokerStart = false;
            binding.zenCommit.clear();
        }
        // A revision's own inspection is binding the focused field: the
        // observer may first retire the field it tracked before. Its reply
        // must still arrive.
        if (binding.revisionWaiting && !binding.observed()) continue;
        // The field changed under a revision: it ends, as for any edit.
        if (revisionActive(binding)) cancelRevision(binding);
        ++binding.observationGeneration;
        if (binding.observed()) retireObservedField(binding);
        // The user's own typing changed the field: inspect it again at once,
        // as for the keystroke itself.
        const bool typed = binding.inspection.inputSinceInspection() && binding.inspection.inspectsOnInput();
        if (typed && fieldChanged) binding.inspection.editReported();
        if (binding.inspection.reinspectAfterInvalidation(fieldChanged)) observeLater(binding, false, false, typed);
        else debug_.record("observer", binding.state.appId(), "observer_awaiting_input");
    }
}

void BadiAddon::retireObservedField(Binding &binding) {
    closeBrokerSession(binding);
    binding.observedFocus = nullptr;
    clearOwnedPanel(binding);
    // The manual path gets a session of its own: a policy answer still owed
    // to the observed field's session then finds no binding. Without fresh
    // randomness the binding stops until the next focus.
    // An unnamed-IBus context has no manual path, so retiring its observed
    // field drops the adopted app id and returns to an observed-only binding.
    const auto session = randomUuid();
    const auto salt = randomSalt();
    const auto contextId = uuidString(binding.inputContext->uuid());
    const bool retired = session && salt &&
        (binding.unnamedIbus
             ? binding.state.retireUnnamedObservation(*session, contextId, *salt)
             : binding.state.retireObservation(*session, contextId, *salt));
    if (!retired) binding.state.focusOut();
}

void BadiAddon::stopObserving(Binding &binding) {
    binding.pendingBrokerStart = false;
    binding.zenCommit.clear();
    binding.fieldVisit = nullptr;
    binding.fieldVisitId.clear();
    ++binding.observationGeneration;
    binding.observedFocus = nullptr;
    binding.waitingForForeignUi = false;
    if (binding.observeTimer) binding.observeTimer->setEnabled(false);
}

void BadiAddon::requestObservedContext(Binding &binding, bool rediscover) {
    const auto &app = binding.state.appId();
    if (!binding.observed() || !safeToObserve(binding)) {
        debug_.record("request_blocked", app, "observer_not_ready");
        return;
    }
    if (!binding.observationExplicit && !binding.surroundingFreshness.freshForAutomatic()) {
        // A reconnect alone must not republish unchanged prose automatically.
        debug_.record("request_blocked", app, "observer_awaiting_input");
        return;
    }
    if (!binding.observationExplicit) {
        // A dialog or very narrow field gets no automatic snapshot either:
        // its prose is not read until the user asks explicitly.
        if (const auto quiet = automaticQuietReason(binding.observedFocus)) {
            debug_.record("request_blocked", app, *quiet);
            return;
        }
    }
    const auto session = binding.state.coordinates().sessionId;
    const auto generation = binding.observationGeneration;
    // The window is read once per field visit, and only while context is on;
    // a snapshot that fails leaves it to the next one.
    const bool window = binding.broker.contextWords && !binding.fieldVisitId.empty() &&
        binding.windowVisitId != binding.fieldVisitId;
    const bool sent = accessibility_->request(snapshotRequest(binding.observedFocus, window),
        [this, session, generation, rediscover, window, focus = binding.observedFocus](const Json &reply) {
            auto *current = bindingFor(session);
            if (!current || !current->inputContext->hasFocus() || current->observationGeneration != generation ||
                !current->broker.policyAllowed) {
                debug_.record("request_blocked", "unidentified", "observer_authority_changed");
                return;
            }
            if (!observerAnswered(reply)) {
                current->zenCommit.clear();
                const auto error = reply.value("error", Json());
                debug_.record("request_blocked", current->state.appId(), snapshotFailureReason(error));
                // The accessible changed under a keystroke. Inspect once to
                // bind whatever is focused now; do not loop on a timeout.
                if (rediscover && (error == "stale_binding" || error == "target_mismatch"))
                    observeLater(*current, false, false, true);
                return;
            }
            publishObservedContext(*current, focus, reply, rediscover, window);
        });
    if (!sent) {
        debug_.record("request_blocked", app, "observer_busy");
        if (rediscover) observeLater(binding, false, true, true);
        else if (binding.observationExplicit) observerUnavailable(binding);
    }
}

void BadiAddon::publishObservedContext(Binding &binding, const Json &focus, const Json &reply,
                                       bool caretMayMove, bool windowAsked) {
    const auto &app = binding.state.appId();
    if (!observerAnswered(reply)) {
        binding.zenCommit.clear();
        return debug_.record("request_blocked", app, snapshotFailureReason(reply.value("error", Json())));
    }
    const auto &observed = reply["focus"];
    // After an inspection, the snapshot must still be that caret and length:
    // a repeated window can hide an absolute move. A keystroke in a field we
    // already hold has no newer inspection, so the snapshot's own caret is
    // the position, and its text still has to match Fcitx.
    const bool samePosition = caretMayMove ? continuedFieldSnapshot(focus, observed)
                                           : matchesObservedFocus(focus, observed, true);
    if (!samePosition) {
        binding.zenCommit.clear();
        return debug_.record("request_blocked", app, "observer_context_mismatch");
    }
    // The visit's window text, kept even when this context asks nothing (an
    // empty field on focus): the window is read only once per visit.
    if (windowAsked && !binding.fieldVisitId.empty()) {
        binding.windowVisitId = binding.fieldVisitId;
        binding.windowText = observedWindowText(observed);
    }
    auto context = rawContext(binding, observed);
    if (!context) {
        binding.zenCommit.clear();
        return debug_.record("request_blocked", app, unavailableContextReason(binding));
    }
    if (binding.zenCommit.active() && !binding.zenCommit.corroborate(app, *context, observed)) {
        return debug_.record("request_blocked", app, "observer_context_mismatch",
                             context->before.size(), observedAfter(*context).size());
    }
    // Escape declines one context. Deleting that text, including clearing
    // the field, ends the decline. An empty field still does not request.
    if (binding.dismissedContext == *context) return;
    binding.dismissedContext.reset();
    if (const auto blocked = observedRequestBlocked(observed, *context, hasForeignImeUi(observePanel(binding)),
                                                    !binding.observationExplicit)) {
        return debug_.record("request_blocked", app, *blocked, context->before.size(),
                             observedAfter(*context).size());
    }
    if (binding.typedThrough &&
        !typeThroughContextReady(binding.typeThroughBase, binding.typeThroughTyped, context->before)) {
        if (binding.typeThroughWaits < 10) {
            ++binding.typeThroughWaits;
            debug_.record("request_blocked", app, "type_through_pending");
            observeLater(binding, false, true);
            return;
        }
    }
    binding.typeThroughWaits = 0;
    binding.observedFocus = observed;
    auto update = binding.state.updateContext(*context);
    if (update && binding.broker.contextWords && binding.windowVisitId == binding.fieldVisitId)
        update->windowText = binding.windowText;
    if (update && open(binding) && transport_.publishContext(*update)) {
        binding.broker.coordinates = update->coordinates;
        binding.requestExplicit = update->context.explicitRequest;
        debug_.record("request", app, !context->paragraphEnd.empty() ?
            "observed_field_request_paragraph_end" : "observed_field_request", context->before.size());
    }
}

} // namespace badi::fcitx5
