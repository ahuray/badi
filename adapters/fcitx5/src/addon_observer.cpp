#include "addon.h"

#include <fcitx-utils/event.h>
#include <fcitx-utils/eventloopinterface.h>
#include <fcitx/inputcontext.h>
#include <fcitx/instance.h>

#include <cstdint>

namespace badi::fcitx5 {

void BadiAddon::observeLater(Binding &binding, bool explicitRequest, bool retry, bool early) {
    if (!binding.inputContext->hasFocus() || !binding.state.focused() ||
        !nativeObservationAvailable(binding.state.appId())) return;
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
    if (!nativeObservationAvailable(binding.state.appId()) ||
        !allowsNativeContext(binding.inputContext->capabilityFlags()) ||
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
    binding.inspection.inspecting();
    debug_.record("observer", binding.state.appId(), "inspect_sent");
    const auto session = binding.state.coordinates().sessionId;
    const auto generation = binding.observationGeneration;
    const bool sent = accessibility_->request(inspectRequest(binding.state.appId()),
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
    const auto editTarget = inspectedEditTarget(focus, binding.state.appId());
    if (!editTarget) return denyEditing(binding);
    const bool unchanged = !binding.observationExplicit && binding.observed() &&
        matchesObservedFocus(binding.observedFocus, focus) &&
        matchesCapturedContext(binding.state.lastContext(), currentContext(binding));
    if (unchanged) return;
    // Typing in the field we already authorized is a context update, not a
    // new session or a new policy query.
    if (binding.broker.opened && binding.broker.policyAllowed &&
        sameObservedField(binding.observedFocus, focus)) {
        binding.observedFocus = focus;
        debug_.record("observer", binding.state.appId(), "same_field_request");
        return requestObservedContext(binding);
    }
    bindObservedField(binding, focus, *editTarget);
}

// A newly observed field gets its own session, and policy for its exact target.
void BadiAddon::bindObservedField(Binding &binding, const Json &focus, NativeEditTarget editTarget) {
    const auto session = randomUuid();
    const auto salt = randomSalt();
    if (!session || !salt) return;
    if (!sameObservedFieldIdentity(binding.fieldVisit, focus)) {
        binding.fieldVisit = focus;
        binding.fieldVisitId = *session;
        binding.dismissedContext.reset();
    }
    closeBrokerSession(binding);
    clearOwnedPanel(binding);
    const auto appId = binding.state.appId();
    if (!binding.state.focusIn(*session, binding.fieldVisitId, appId, *salt,
                               editTarget, NativeEditPath::Observed)) return;
    binding.observedFocus = focus;
    if (!binding.state.editingAvailable()) {
        debug_.record("request_blocked", appId, editingUnavailableReason(appId, binding.observed()));
        return;
    }
    queryPolicy(binding);
}

void BadiAddon::denyEditing(Binding &binding) {
    binding.fieldVisit = nullptr;
    binding.fieldVisitId.clear();
    closeBrokerSession(binding);
    ++binding.observationGeneration;
    binding.state.denyEditing();
    binding.observedFocus = nullptr;
    clearOwnedPanel(binding);
    debug_.record("request_blocked", binding.state.appId(),
        imeParityApp(binding.state.appId()) ? "ime_parity_target_invalid" : "editor_transaction_unavailable");
}

// IME-parity has no manual fallback, so an explicit request says why the
// observer produced no field: the field's own purpose, or no observer.
void BadiAddon::observerUnavailable(Binding &binding, const Json &error) {
    binding.fieldVisit = nullptr;
    binding.fieldVisitId.clear();
    // A failed inspection must not keep the previous field's grant.
    if (binding.observed()) retireObservedField(binding);
    if (!imeParityApp(binding.state.appId())) return;
    const bool fieldDenied = observerDeniedField(error);
    debug_.record("request_blocked", binding.state.appId(),
        fieldDenied ? "ime_parity_field_denied" : "ime_parity_observer_unavailable");
    if (binding.observationExplicit)
        showNotice(binding, fieldDenied ? notice::kFieldUnreadable : notice::kObserverUnavailable);
}

void BadiAddon::observerInvalidated(const Json &event) {
    const bool fieldChanged = event.value("reason", Json()) == "field_changed";
    for (auto &[_, binding] : bindings_) {
        if (!binding.state.focused() || !invalidates(event, binding.state.appId())) continue;
        debug_.record("observer", binding.state.appId(), "observer_invalidated");
        ++binding.observationGeneration;
        if (binding.observed()) retireObservedField(binding);
        if (binding.inspection.reinspectAfterInvalidation(fieldChanged)) observeLater(binding);
        else debug_.record("observer", binding.state.appId(), "observer_awaiting_input");
    }
}

void BadiAddon::retireObservedField(Binding &binding) {
    closeBrokerSession(binding);
    binding.state.retireObservation();
    binding.observedFocus = nullptr;
    clearOwnedPanel(binding);
}

void BadiAddon::stopObserving(Binding &binding) {
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
    // The window is read once per field visit, and only while context is on.
    const bool window = binding.broker.contextWords && !binding.fieldVisitId.empty() &&
        binding.windowVisitId != binding.fieldVisitId;
    if (window) {
        binding.windowVisitId = binding.fieldVisitId;
        binding.windowText.clear();
    }
    const bool sent = accessibility_->request(snapshotRequest(binding.observedFocus, window),
        [this, session, generation, rediscover, focus = binding.observedFocus](const Json &reply) {
            auto *current = bindingFor(session);
            if (!current || !current->inputContext->hasFocus() || current->observationGeneration != generation ||
                !current->broker.policyAllowed) {
                debug_.record("request_blocked", "unidentified", "observer_authority_changed");
                return;
            }
            if (!observerAnswered(reply)) {
                const auto error = reply.value("error", Json());
                debug_.record("request_blocked", current->state.appId(), snapshotFailureReason(error));
                // The accessible changed under a keystroke. Inspect once to
                // bind whatever is focused now; do not loop on a timeout.
                if (rediscover && (error == "stale_binding" || error == "target_mismatch"))
                    observeLater(*current, false, false, true);
                return;
            }
            publishObservedContext(*current, focus, reply, rediscover);
        });
    if (!sent) {
        debug_.record("request_blocked", app, "observer_busy");
        if (rediscover) observeLater(binding, false, true, true);
        else if (binding.observationExplicit) observerUnavailable(binding);
    }
}

void BadiAddon::publishObservedContext(Binding &binding, const Json &focus, const Json &reply,
                            bool caretMayMove) {
    const auto &app = binding.state.appId();
    if (!observerAnswered(reply)) {
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
        return debug_.record("request_blocked", app, "observer_context_mismatch");
    }
    auto context = currentContext(binding);
    if (!context) return debug_.record("request_blocked", app, unavailableContextReason(binding));
    if (app == "libreoffice") restoreObservedTrailingSpaces(*context, observed);
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
    if (const auto window = observedWindowText(observed); !window.empty() &&
        binding.windowVisitId == binding.fieldVisitId) binding.windowText = window;
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
