#include "addon.h"

#include <fcitx-utils/event.h>
#include <fcitx-utils/eventloopinterface.h>
#include <fcitx/candidatelist.h>
#include <fcitx/inputcontext.h>
#include <fcitx/inputpanel.h>
#include <fcitx/instance.h>
#include <fcitx/text.h>
#include <fcitx/userinterface.h>

#include <cstdint>
#include <functional>
#include <memory>
#include <optional>
#include <string>
#include <utility>

namespace badi::fcitx5 {

namespace {

// The word Tab takes in the theme's highlight colour and bold, then the
// rest in its normal colour (ui/omarchy-plugin/fcitx-theme.py).
::fcitx::Text candidateText(const Suggestion &suggestion) {
    const auto parts = candidateParts(suggestion);
    ::fcitx::Text text;
    text.append(parts.strong, ::fcitx::TextFormatFlags{::fcitx::TextFormatFlag::HighLight,
                                                      ::fcitx::TextFormatFlag::Bold});
    if (!parts.rest.empty()) text.append(parts.rest);
    return text;
}

class BadiCandidate final : public ::fcitx::CandidateWord {
public:
    BadiCandidate(::fcitx::Text text,
                  std::function<void(::fcitx::InputContext *)> selected)
        : CandidateWord(std::move(text)),
          selected_(std::move(selected)) {
        setCustomLabel(::fcitx::Text());
    }

    void select(::fcitx::InputContext *inputContext) const override {
        selected_(inputContext);
    }

private:
    std::function<void(::fcitx::InputContext *)> selected_;
};

} // namespace

void BadiAddon::queryFocusedPolicies() {
    for (auto &[_, binding] : bindings_) {
        if (binding.state.focused()) queryPolicy(binding);
    }
}

void BadiAddon::onAuthority(const AuthoritySnapshot &snapshot) {
    authorityPaused_ = snapshot.paused;
    const bool changed = authorityContinuity_.observe(snapshot);
    if (snapshot.initial) {
        // onDisconnected already retired the previous connection's grants.
        // Authority that changed meanwhile also retires cached text.
        if (changed) {
            for (auto &[_, binding] : bindings_) binding.surroundingFreshness.capabilityChanged();
        }
        return;
    }
    for (auto &[_, binding] : bindings_) {
        // The broker already retired these sessions at the new authority epoch.
        binding.broker.retire();
        binding.requestExplicit = false;
        dropHeldTab(binding);
        binding.state.invalidateContext();
        binding.surroundingFreshness.capabilityChanged();
        clearOwnedPanel(binding);
    }
    if (!authorityPaused_) queryFocusedPolicies();
}

void BadiAddon::onPolicy(std::string_view session, bool allowed, const std::string &reason, std::string language) {
    auto *binding = bindingFor(session);
    if (!binding || !binding->state.focused() || !binding->state.editingAvailable()) return;
    // The app list mode opens only fields the observer corroborates; the
    // manual unknown-identity path (Tab requests) needs an exact app rule.
    if (allowed && reason == "matched_default" && !binding->observed()) {
        binding->broker.answer(false);
        debug_.record("policy", binding->state.appId(), "app_rule_required");
        return;
    }
    binding->broker.answer(allowed, std::move(language));
    // An editor with its own Badi integration (Neovim) holds the focused
    // terminal: Tab, the chord and every request stay the editor's.
    debug_.record("policy", binding->state.appId(),
                  allowed ? "app_allowed" : reason == "editor_focused" ? "editor_focused" : "app_disabled");
    if (!allowed) return;
    if (binding->observed()) requestObservedContext(*binding);
    open(*binding);
}

void BadiAddon::onSuggestion(Suggestion suggestion) {
    auto *binding = bindingFor(suggestion.coordinates.sessionId);
    if (!binding || !safeToObserve(*binding) || binding->state.acceptancePending()) return;
    const auto now = monotonicUs();
    if (binding->showAfterUs > now) {
        binding->heldSuggestion = std::move(suggestion);
        const auto session = binding->heldSuggestion->coordinates.sessionId;
        const auto id = binding->heldSuggestion->suggestionId;
        if (binding->holdTimer) binding->holdTimer->setEnabled(false);
        binding->holdTimer = oneShot(binding->showAfterUs - now, [this, session, id] {
            auto *current = bindingFor(session);
            if (!current || !current->heldSuggestion || current->heldSuggestion->suggestionId != id) return;
            if (monotonicUs() < current->showAfterUs) return;
            auto held = std::move(*current->heldSuggestion);
            current->heldSuggestion.reset();
            presentSuggestion(*current, std::move(held));
        });
        return;
    }
    presentSuggestion(*binding, std::move(suggestion));
}

void BadiAddon::presentSuggestion(Binding &binding, Suggestion suggestion) {
    if (!safeToObserve(binding) || binding.state.acceptancePending()) return;
    // SessionState keeps IME-parity apps off this unobserved display path.
    if (!binding.observed()) return displaySuggestion(std::move(suggestion));
    binding.pendingSuggestion = suggestion.suggestionId;
    const auto generation = binding.observationGeneration;
    const bool sent = accessibility_->request(snapshotRequest(binding.observedFocus),
        [this, suggestion, focus = binding.observedFocus, generation](const Json &reply) {
            auto *current = bindingFor(suggestion.coordinates.sessionId);
            if (current && safeToObserve(*current) && current->observationGeneration == generation &&
                current->state.coordinates() == suggestion.coordinates)
                previewIfCorroborated(*current, suggestion, focus, reply);
        });
    if (!sent) debug_.record("suggestion_blocked", binding.state.appId(), "observer_display_unavailable");
}

void BadiAddon::previewIfCorroborated(Binding &binding, const Suggestion &suggestion, const Json &focus, const Json &reply) {
    if (binding.state.acceptancePending()) return;
    if (!observerAnswered(reply) || !reply["focus"].is_object()) {
        return debug_.record("suggestion_blocked", binding.state.appId(), "observer_display_unavailable");
    }
    const auto context = currentContext(binding);
    const auto &observed = reply["focus"];
    if (!context || !matchesCapturedContext(binding.state.lastContext(), context) ||
        !observerCorroborates(focus, observed, *context)) {
        return debug_.record("suggestion_blocked", binding.state.appId(), "observer_display_mismatch");
    }
    binding.observedFocus = observed;
    showObservedPreview(binding, suggestion);
}

// The observer re-verifies caret and length before rendering. A verified
// reply with rendered:false means its preview is unavailable (e.g.
// uncalibrated geometry); Badi's owned Fcitx panel is then shown, which
// Wayland compositors place at the text-input caret rectangle.
void BadiAddon::showObservedPreview(Binding &binding, const Suggestion &suggestion) {
    const auto now = transport_.nowMs();
    if (!safeToObserve(binding) || suggestion.expiresAtMs <= now) return;
    const auto &focus = binding.observedFocus;
    const bool sent = accessibility_->request(
        previewRequest(focus, suggestion.text, suggestion.expiresAtMs - now, caretRect(*binding.inputContext)),
        [this, suggestion, focus, generation = binding.observationGeneration](const Json &reply) {
            const bool verified = observerAnswered(reply) && reply["focus"].is_object();
            const bool rendered = verified && reply["focus"].value("rendered", Json()) == true;
            if (!rendered) previewMayBeVisible_ = false;
            auto *current = bindingFor(suggestion.coordinates.sessionId);
            if (!current || !current->inputContext->hasFocus() ||
                current->state.coordinates() != suggestion.coordinates ||
                current->observationGeneration != generation) return hidePreview();
            previewed(*current, suggestion, focus, reply, verified, rendered);
        });
    if (sent) previewMayBeVisible_ = true;
    else debug_.record("suggestion_blocked", binding.state.appId(), "observer_display_unavailable");
}

void BadiAddon::previewed(Binding &binding, const Suggestion &suggestion, const Json &focus, const Json &reply,
               bool verified, bool rendered) {
    if (binding.state.acceptancePending()) return;
    const bool foreignIme = hasForeignImeUi(observePanel(binding));
    if (!verified || !matchesObservedFocus(focus, reply["focus"], true) ||
        !matchesCapturedContext(binding.state.lastContext(), currentContext(binding)) || foreignIme) {
        hidePreview();
        debug_.record("suggestion_blocked", binding.state.appId(),
            !verified ? "observer_display_unavailable" :
            foreignIme ? "foreign_ime_active" : "observer_display_mismatch");
        return;
    }
    displaySuggestion(suggestion, rendered);
}

void BadiAddon::displaySuggestion(Suggestion suggestion, bool overlay) {
    auto *binding = bindingFor(suggestion.coordinates.sessionId);
    if (binding && binding->state.acceptancePending()) return;
    if (!binding || !binding->inputContext->hasFocus() || hasForeignImeUi(observePanel(*binding)) ||
        !matchesCapturedContext(binding->state.lastContext(), currentContext(*binding)) ||
        !binding->state.showSuggestion(suggestion, transport_.nowMs())) {
        if (overlay) hidePreview();
        if (binding && binding->state.visibleText().empty()) clearOwnedPanel(*binding);
        return;
    }
    binding->overlayOwned = overlay;
    binding->pendingSuggestion.reset();
    binding->typedThrough = false;
    binding->typeThroughAutomatic = false;
    binding->typeThroughRest.clear();
    binding->typeThroughBase.clear();
    binding->typeThroughTyped.clear();
    binding->typeThroughWaits = 0;
    if (!overlay) showCandidate(*binding, suggestion);
    // The local lease must also hide UI if the broker stalls before its
    // suggestion.clear arrives. Only this exact candidate may be retired.
    const auto now = transport_.nowMs();
    binding->expiryTimer = oneShot((suggestion.expiresAtMs > now ? suggestion.expiresAtMs - now : 0) * 1000,
        [this, coordinates = suggestion.coordinates, suggestionId = suggestion.suggestionId] {
            auto *current = bindingFor(coordinates.sessionId);
            if (current && current->state.clearSuggestionIf(coordinates, suggestionId)) clearOwnedPanel(*current);
        });
    binding->inputContext->updateUserInterface(::fcitx::UserInterfaceComponent::InputPanel);
    debug_.record("suggestion", binding->state.appId(), overlay ? "display_overlay" : "display_dispatched");
    // The overlay is the observer's grey text at the caret; the panel is
    // Fcitx's candidate popup, Badi's fallback display.
    debug_.trace("ghost shown", binding->state.appId(),
        {{"display", overlay ? "overlay" : "panel"}, {"id", suggestion.suggestionId},
         {"bytes", suggestion.text.size()}},
        {{"text", suggestion.text}});
    // A Tab held for this remainder now takes its next word, through the
    // normal broker grant for the suggestion just displayed. A display
    // that no longer starts with that word leaves the held Tab dropped
    // and the suggestion standing.
    if (binding->heldTab) {
        const std::string rest(binding->heldTabRest);
        dropHeldTab(*binding);
        if (heldTabMatchesRest(rest, suggestion)) {
            debug_.record("decision", binding->state.appId(), "tab_held_accepted");
            requestAcceptance(*binding, true);
        }
    }
}

// Keep the last-resort panel to one quiet line; key help lives in the runbook.
void BadiAddon::showCandidate(Binding &binding, const Suggestion &suggestion) {
    auto list = std::make_unique<::fcitx::CommonCandidateList>();
    list->setPageSize(1);
    list->setLayoutHint(::fcitx::CandidateLayoutHint::Horizontal);
    list->append(std::make_unique<BadiCandidate>(candidateText(suggestion),
        [this, sessionId = suggestion.coordinates.sessionId](::fcitx::InputContext *selected) {
            auto *current = bindingFor(sessionId);
            if (current && current->inputContext == selected && selected->hasFocus()) requestAcceptance(*current);
        }));
    list->setGlobalCursorIndex(-1);
    auto &panel = binding.inputContext->inputPanel();
    binding.ownedAuxiliary.clear();
    panel.setAuxUp(::fcitx::Text(binding.ownedAuxiliary));
    panel.setCandidateList(std::move(list));
    binding.ownedCandidates = panel.candidateList();
}

void BadiAddon::onClear(const ClearNotice &clear) {
    auto *binding = bindingFor(clear.coordinates.sessionId);
    if (!binding) return;
    const bool current = binding->state.coordinates() == clear.coordinates;
    if (current) {
        // The request that would have brought a word acceptance's remainder
        // was answered without a suggestion: no Tab may be held for that
        // acceptance any more (at most one hold each), and a Tab already
        // held is dropped with it.
        if (binding->heldTab) debug_.record("decision", binding->state.appId(), "tab_held_dropped");
        binding->typeThroughAutomatic = false;
        dropHeldTab(*binding);
    }
    if (!binding->state.editingAvailable()) return clearOwnedPanel(*binding);
    debug_.record("clear", binding->state.appId(), clear.reason);
    if (current && binding->pendingSuggestion &&
        (!clear.suggestionId || *clear.suggestionId == *binding->pendingSuggestion)) {
        // A broker cancellation can arrive before the preview RPC returns.
        // Retire that asynchronous candidate even though it is not visible.
        return cancelForInput(*binding);
    }
    // A miss is announced only when the user asked: an automatic request
    // that finds nothing stays quiet on every path, terminals included.
    if (current && !clear.suggestionId && binding->requestExplicit) {
        if (const auto text = clearNoticeText(clear.reason)) return showNotice(*binding, *text);
    }
    if (binding->state.clearSuggestionIf(clear.coordinates, clear.suggestionId)) clearOwnedPanel(*binding);
}

void BadiAddon::onCommitPrepare(const CommitPrepare &prepare) {
    auto *binding = bindingFor(prepare.coordinates.sessionId);
    if (!binding || !binding->observed()) {
        // SessionState keeps IME-parity apps off the unobserved path; a
        // grant that reaches one anyway is refused.
        if (binding && (!binding->state.editingAvailable() || imeParityApp(binding->state.appId())))
            return refuseCommit(prepare, binding);
        return applyCommitPrepare(prepare);
    }
    if (!binding->state.editingAvailable() || !safeToObserve(*binding)) return refuseCommit(prepare, binding);
    corroborateCommit(*binding, prepare, monotonicUs() + 500'000);
}

// Re-snapshot immediately before commitString: the observer must still
// agree with Fcitx's live text and caret for this field.
void BadiAddon::corroborateCommit(Binding &binding, const CommitPrepare &prepare, std::uint64_t deadline) {
    if (!binding.state.acceptancePending() || binding.state.coordinates() != prepare.coordinates ||
        !safeToObserve(binding)) return refuseCommit(prepare, &binding);
    if (accessibility_->pending()) {
        // A growing preview may still own the single observer RPC. Wait
        // for it to finish before sending the one fresh commit snapshot;
        // never reuse its display check or extend either authority lease.
        if (monotonicUs() >= deadline) return refuseCommit(prepare, &binding);
        binding.commitTimer = instance_->eventLoop().addTimeEvent(CLOCK_MONOTONIC, 0, kTimerAccuracyUs,
            [this, prepare, deadline](::fcitx::EventSourceTime *timer, std::uint64_t) {
                if (accessibility_->pending() && monotonicUs() < deadline) {
                    timer->setNextInterval(5'000);
                    timer->setOneShot();
                    return true;
                }
                auto *current = bindingFor(prepare.coordinates.sessionId);
                if (current) corroborateCommit(*current, prepare, deadline);
                else reportStale(prepare);
                return true;
            });
        binding.commitTimer->setNextInterval(5'000);
        binding.commitTimer->setOneShot();
        return;
    }
    const bool sent = accessibility_->request(snapshotRequest(binding.observedFocus),
        [this, prepare, focus = binding.observedFocus, generation = binding.observationGeneration](const Json &reply) {
            auto *current = bindingFor(prepare.coordinates.sessionId);
            const auto context = current ? currentContext(*current) : std::nullopt;
            const bool answered = observerAnswered(reply) && reply["focus"].is_object();
            if (current && current->observationGeneration == generation && context && answered &&
                observerCorroborates(focus, reply["focus"], *context)) return applyCommitPrepare(prepare);
            if (current) {
                debug_.record("commit_blocked", current->state.appId(),
                    answered ? "observer_dispatch_mismatch" : "observer_dispatch_unavailable");
            }
            refuseCommit(prepare, current);
        });
    if (!sent) {
        debug_.record("commit_blocked", binding.state.appId(), "observer_dispatch_unavailable");
        refuseCommit(prepare, &binding);
    }
}

void BadiAddon::applyCommitPrepare(const CommitPrepare &prepare) {
    auto *binding = bindingFor(prepare.coordinates.sessionId);
    if (!binding || !binding->state.editingAvailable() || !binding->inputContext->hasFocus() ||
        !hasOwnedCandidate(observePanel(*binding))) return refuseCommit(prepare, binding);
    if (!matchesCapturedContext(binding->state.lastContext(), currentContext(*binding))) {
        binding->state.invalidateContext();
        return refuseCommit(prepare, binding);
    }
    const std::string shown(binding->state.visibleText());
    const auto dispatch = binding->state.authorizeCommit(prepare, transport_.nowMs(), observePanel(*binding));
    if (!dispatch) return reportStale(prepare);
    clearOwnedPanel(*binding);
    // The broker may carry the rest after a word (ADR 0004).
    const bool word = !dispatch->replaceBefore && prepare.acceptance == "word" &&
                       shown.starts_with(dispatch->text);
    binding->typedThrough = word;
    binding->typeThroughRest = word ? shown.substr(dispatch->text.size()) : std::string();
    // Only a rest that returns on its own may hold a quick second Tab.
    binding->typeThroughAutomatic =
        word && (binding->observed() || binding->state.editPath() == NativeEditPath::TypedLine);
    binding->typeThroughWaits = 0;
    if (word) {
        if (const auto previous = binding->state.lastContext())
            binding->typeThroughBase = previous->context.before;
        binding->typeThroughTyped = dispatch->text;
    } else {
        binding->typeThroughBase.clear();
        binding->typeThroughTyped.clear();
    }
    if (dispatch->replaceBefore) {
        if (imeParityApp(binding->state.appId())) return refuseCommit(prepare, binding);
        const auto context = currentContext(*binding);
        const auto count = context ? nativeReplacementChars(
                                        false, context->before, *dispatch->replaceBefore)
                                    : std::nullopt;
        if (!count) return refuseCommit(prepare, binding);
        binding->inputContext->deleteSurroundingText(-static_cast<int>(*count), *count);
    }
    // One append, like typed text. Fcitx cannot see the client apply it,
    // so the result is dispatched-unverified, never applied. A native
    // replacement deletes the exact suffix first; IME-parity stays append-only.
    binding->inputContext->commitString(dispatch->text);
    if (binding->state.editPath() == NativeEditPath::TypedLine) {
        binding->typedLine.type(dispatch->text);
        requestLineAfterPause(*binding);
    }
    debug_.record("commit", binding->state.appId(), "dispatched_unverified");
    transport_.reportCommit(dispatch->coordinates, dispatch->controlId, dispatch->suggestionId,
                            "dispatched-unverified");
}

void BadiAddon::refuseCommit(const CommitPrepare &prepare, Binding *binding) {
    if (binding) clearOwnedPanel(*binding);
    reportStale(prepare);
}

void BadiAddon::reportStale(const CommitPrepare &prepare) {
    transport_.reportCommit(prepare.coordinates, prepare.controlId, prepare.suggestionId, "stale");
}

void BadiAddon::onDisconnected() {
    authorityPaused_ = true;
    for (auto &[_, binding] : bindings_) {
        // Sessions, revisions, candidates, commit grants and in-flight
        // observer replies end with the connection. Fcitx's surrounding
        // text does not, so an explicit request after reconnect may read
        // it once fresh policy reopens the session.
        binding.broker.retire();
        ++binding.observationGeneration;
        binding.observationExplicit = false;
        binding.requestExplicit = false;
        dropHeldTab(binding);
        binding.state.invalidateContext();
        binding.surroundingFreshness.transportLost();
        clearOwnedPanel(binding);
    }
}

} // namespace badi::fcitx5
