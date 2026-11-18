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
    if (!retry) binding.observation.schedule.resetBusyRetries();
    binding.observation.explicitRequest = explicitRequest;
    if (explicitRequest) {
        binding.suggestion.dismissedContext.reset();
        binding.suggestion.showAfterUs = 0;
        binding.suggestion.clearHeld();
    }
    if (!binding.observation.timer) {
        binding.observation.timer = instance_->eventLoop().addTimeEvent(CLOCK_MONOTONIC, 0, kTimerAccuracyUs,
            [this, contextId = uuidString(binding.inputContext->uuid())](::fcitx::EventSourceTime *, std::uint64_t) {
                const auto found = bindings_.find(contextId);
                if (found != bindings_.end() && found->second.inputContext->hasFocus()) inspectField(found->second);
                return true;
            });
    }
    binding.observation.timer->setNextInterval(
        binding.observation.schedule.delayUs(explicitRequest && !retry, binding.typeThrough.active, retry,
                                             early && !retry));
    binding.observation.timer->setOneShot();
}

void BadiAddon::inspectField(Binding &binding) {
    // A revision's selection is the field's own state: an inspection for a
    // caret would decline it.
    if (binding.revisionActive()) return;
    // Inspection reads only the focused accessible, so fcitx5-qt's
    // withdrawal echo does not stop it; reading text waits for observation.
    if (!binding.observationCandidate() ||
        !allowsNativeContext(observedFieldCapabilities(binding.inputContext->capabilityFlags(),
                                                       binding.surrounding.echo)) ||
        !binding.surrounding.freshness.fresh()) return;
    if (accessibility_->pending()) {
        // Bounded, so an absent service or a denied field never loops the timer.
        if (binding.observation.schedule.retryWhileBusy()) {
            debug_.record("observer", binding.state.appId(), "observer_busy_retry");
            observeLater(binding, binding.observation.explicitRequest, true);
        } else {
            observerUnavailable(binding);
        }
        return;
    }
    if (foreignImeActive(binding)) {
        binding.observation.waitingForForeignUi = true;
        return;
    }
    binding.observation.waitingForForeignUi = false;
    binding.observation.schedule.inspecting(monotonicUs());
    debug_.record("observer", binding.state.appId(), "inspect_sent");
    const auto session = binding.state.coordinates().sessionId;
    const auto generation = binding.observation.generation;
    const auto request = binding.observation.unnamedIbus ? unnamedIbusInspectRequest()
                                                         : inspectRequest(binding.state.appId());
    const bool sent = accessibility_->request(request,
        [this, session, generation](const Json &reply) {
            auto *current = bindingFor(session);
            if (current && current->inputContext->hasFocus() && current->observation.generation == generation)
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
    if (binding.observation.unnamedIbus) {
        if (const auto resolved = unnamedIbusEditTarget(focus)) {
            appId = resolved->first;
            editTarget = resolved->second;
        }
    } else {
        editTarget = inspectedEditTarget(focus, appId);
    }
    binding.observation.schedule.found(editTarget.has_value());
    if (!editTarget) return denyEditing(binding);
    const bool unchanged = !binding.observation.explicitRequest && !binding.observation.zenCommit.active() &&
        binding.observed() &&
        binding.state.appId() == appId &&
        matchesObservedFocus(binding.observation.focus, focus) &&
        matchesCapturedContext(binding.state.lastContext(), currentContext(binding));
    if (unchanged) return;
    // Typing in the field we already authorized is a context update, not a
    // new session or a new policy query.
    if (binding.broker.opened && binding.broker.policyAllowed &&
        binding.state.appId() == appId &&
        sameObservedField(binding.observation.focus, focus)) {
        binding.observation.focus = focus;
        debug_.record("observer", binding.state.appId(), "same_field_request");
        return requestObservedContext(binding);
    }
    bindObservedField(binding, focus, *editTarget, appId);
}

// A newly observed field gets its own session, and policy for its exact target.
void BadiAddon::bindObservedField(Binding &binding, const Json &focus, NativeEditTarget editTarget,
                                  std::string_view resolvedAppId) {
    const std::string appId(resolvedAppId.empty() ? std::string_view(binding.state.appId()) : resolvedAppId);
    if (binding.observation.unnamedIbus && !unnamedIbusObservedApp(appId)) return denyEditing(binding);
    const auto session = randomUuid();
    const auto salt = randomSalt();
    if (!session || !salt) return;
    if (!binding.observation.zenCommit.sameField(focus)) binding.observation.zenCommit.clear();
    if (!sameObservedFieldIdentity(binding.observation.visit, focus)) {
        binding.observation.clearVisit();
        binding.observation.visit = focus;
        binding.observation.visitId = *session;
        binding.suggestion.dismissedContext.reset();
        binding.typeThrough.reset();
    }
    closeBrokerSession(binding);
    clearAllUi(binding);
    if (!binding.state.focusIn(*session, binding.observation.visitId, appId, *salt,
                               editTarget, NativeEditPath::Observed)) return;
    binding.observation.focus = focus;
    if (!binding.state.editingAvailable()) {
        binding.observation.pendingBrokerStart = false;
        binding.observation.zenCommit.clear();
        debug_.record("request_blocked", appId, editingUnavailableReason(appId, binding.observed()));
        return;
    }
    if (std::exchange(binding.observation.pendingBrokerStart, false)) ensureBroker();
    queryPolicy(binding);
}

void BadiAddon::denyEditing(Binding &binding) {
    const std::string priorApp = binding.state.appId();
    retire(binding, RetireReason::EditingDenied);
    debug_.record("request_blocked", priorApp,
        imeParityApp(priorApp) ? "ime_parity_target_invalid" : "editor_transaction_unavailable");
}

// IME-parity and unnamed-IBus contexts have no manual fallback, so an explicit
// request says why the observer produced no field: an unsupported app, the
// field's own purpose, or no observer.
void BadiAddon::observerUnavailable(Binding &binding, const Json &error) {
    const std::string priorApp = binding.state.appId();
    const bool explicitRequest = binding.observation.explicitRequest;
    binding.observation.pendingBrokerStart = false;
    binding.observation.zenCommit.clear();
    binding.typeThrough.reset();
    binding.observation.schedule.found(false);
    binding.observation.clearVisit();
    // A failed inspection must not keep the previous field's grant.
    if (binding.observed() || (binding.observation.unnamedIbus && !priorApp.empty()))
        retire(binding, RetireReason::ObservedFieldChanged);
    if (!imeParityApp(priorApp) && !binding.observation.unnamedIbus) return;
    if (binding.observation.unnamedIbus && error == "unsupported_app") {
        debug_.record("request_blocked", priorApp, "unidentified_app");
        if (explicitRequest) showNotice(binding, notice::kUnavailableApp);
        return;
    }
    const bool fieldDenied = observerDeniedField(error);
    debug_.record("request_blocked", priorApp,
        fieldDenied ? "ime_parity_field_denied" : "ime_parity_observer_unavailable");
    if (explicitRequest)
        showNotice(binding, fieldDenied ? notice::kFieldUnreadable : notice::kObserverUnavailable);
}

void BadiAddon::observerInvalidated(const Json &event) {
    const auto reason = event.value("reason", Json());
    const bool fieldChanged = reason == "field_changed";
    const bool windowLeft = reason == "window_changed" || reason == "desktop_locked";
    for (auto &[_, binding] : bindings_) {
        if (!binding.state.focused() || !invalidates(event, binding.state.appId())) continue;
        debug_.record("observer", binding.state.appId(), "observer_invalidated");
        if (!fieldChanged) {
            binding.observation.pendingBrokerStart = false;
            binding.observation.zenCommit.clear();
        }
        if (windowLeft) binding.typeThrough.reset();
        // A revision's own inspection is binding the focused field: the
        // observer may first retire the field it tracked before. Its reply
        // must still arrive.
        if (binding.revision.waiting && !binding.observed()) continue;
        // The field changed under a revision: it ends, as for any edit.
        if (binding.revisionActive()) cancelRevision(binding);
        ++binding.observation.generation;
        if (binding.observed()) retire(binding, RetireReason::ObservedFieldChanged);
        // The user's own typing changed the field: inspect it again at once,
        // as for the keystroke itself.
        const bool typed = binding.observation.schedule.inputSinceInspection() &&
                           binding.observation.schedule.inspectsOnInput();
        if (typed && fieldChanged) binding.observation.schedule.editReported();
        if (binding.observation.schedule.reinspectAfterInvalidation(fieldChanged))
            observeLater(binding, false, false, typed);
        else
            debug_.record("observer", binding.state.appId(), "observer_awaiting_input");
    }
}

void BadiAddon::requestObservedContext(Binding &binding, bool rediscover) {
    const auto &app = binding.state.appId();
    if (!binding.observed() || !safeToObserve(binding)) {
        debug_.record("request_blocked", app, "observer_not_ready");
        return;
    }
    if (!binding.observation.explicitRequest && !binding.surrounding.freshness.freshForAutomatic()) {
        // A reconnect alone must not republish unchanged prose automatically.
        debug_.record("request_blocked", app, "observer_awaiting_input");
        return;
    }
    if (!binding.observation.explicitRequest) {
        // A dialog or very narrow field gets no automatic snapshot either:
        // its prose is not read until the user asks explicitly.
        if (const auto quiet = automaticQuietReason(binding.observation.focus)) {
            debug_.record("request_blocked", app, *quiet);
            return;
        }
    }
    const auto session = binding.state.coordinates().sessionId;
    const auto generation = binding.observation.generation;
    // The window is read once per field visit, and only while context is on;
    // a snapshot that fails leaves it to the next one.
    const bool window = binding.broker.contextWords && !binding.observation.visitId.empty() &&
        binding.observation.windowVisitId != binding.observation.visitId;
    const bool sent = accessibility_->request(snapshotRequest(binding.observation.focus, window),
        [this, session, generation, rediscover, window, focus = binding.observation.focus](const Json &reply) {
            auto *current = bindingFor(session);
            if (!current || !current->inputContext->hasFocus() || current->observation.generation != generation ||
                !current->broker.policyAllowed) {
                debug_.record("request_blocked", "unidentified", "observer_authority_changed");
                return;
            }
            if (!observerAnswered(reply)) {
                current->observation.zenCommit.clear();
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
        else if (binding.observation.explicitRequest) observerUnavailable(binding);
    }
}

void BadiAddon::publishObservedContext(Binding &binding, const Json &focus, const Json &reply,
                                       bool caretMayMove, bool windowAsked) {
    const auto &app = binding.state.appId();
    if (!observerAnswered(reply)) {
        binding.observation.zenCommit.clear();
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
        binding.observation.zenCommit.clear();
        return debug_.record("request_blocked", app, "observer_context_mismatch");
    }
    // The visit's window text, kept even when this context asks nothing (an
    // empty field on focus): the window is read only once per visit.
    if (windowAsked && !binding.observation.visitId.empty()) {
        binding.observation.windowVisitId = binding.observation.visitId;
        binding.observation.windowText = observedWindowText(observed);
    }
    auto context = rawContext(binding, observed);
    if (!context) {
        binding.observation.zenCommit.clear();
        return debug_.record("request_blocked", app, unavailableContextReason(binding));
    }
    if (binding.observation.zenCommit.active() &&
        !binding.observation.zenCommit.corroborate(app, *context, observed)) {
        return debug_.record("request_blocked", app, "observer_context_mismatch",
                             context->before.size(), observedAfter(*context).size());
    }
    // Escape declines one context. Deleting that text, including clearing
    // the field, ends the decline. An empty field still does not request.
    if (binding.suggestion.dismissedContext == *context) return;
    binding.suggestion.dismissedContext.reset();
    if (const auto blocked = observedRequestBlocked(observed, *context, hasForeignImeUi(observePanel(binding)),
                                                    !binding.observation.explicitRequest)) {
        return debug_.record("request_blocked", app, *blocked, context->before.size(),
                             observedAfter(*context).size());
    }
    if (binding.typeThrough.active &&
        !typeThroughContextReady(binding.typeThrough.base, binding.typeThrough.typed, context->before)) {
        if (binding.typeThrough.waits < 10) {
            ++binding.typeThrough.waits;
            debug_.record("request_blocked", app, "type_through_pending");
            observeLater(binding, false, true);
            return;
        }
    }
    binding.typeThrough.waits = 0;
    binding.observation.focus = observed;
    auto update = binding.state.updateContext(*context);
    if (update && binding.broker.contextWords &&
        binding.observation.windowVisitId == binding.observation.visitId) {
        update->windowText = binding.observation.windowText;
    }
    if (update && open(binding) && transport_.publishContext(*update)) {
        binding.broker.coordinates = update->coordinates;
        binding.suggestion.requestExplicit = update->context.explicitRequest;
        debug_.record("request", app, !context->paragraphEnd.empty() ?
            "observed_field_request_paragraph_end" : "observed_field_request", context->before.size());
    }
}

} // namespace badi::fcitx5
