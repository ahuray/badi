#include "addon.h"
#include "fcitx_event.h"

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
    if (!parts.lead.empty()) text.append(parts.lead);
    text.append(parts.strong, ::fcitx::TextFormatFlags{::fcitx::TextFormatFlag::HighLight,
                                                      ::fcitx::TextFormatFlag::Bold});
    if (!parts.rest.empty()) text.append(parts.rest);
    return text;
}

class BadiCandidate final : public ::fcitx::CandidateWord {
public:
    BadiCandidate(::fcitx::Text text,
                  std::function<void(::fcitx::InputContext *)> selected,
                  std::string label = {})
        : CandidateWord(std::move(text)),
          selected_(std::move(selected)) {
        setCustomLabel(::fcitx::Text(label));
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
        binding.revisionWaiting.reset();
        dropHeldTab(binding);
        binding.state.invalidateContext();
        binding.surroundingFreshness.capabilityChanged();
        clearOwnedPanel(binding);
    }
    if (!authorityPaused_) queryFocusedPolicies();
}

void BadiAddon::onPolicy(std::string_view session, bool allowed, const std::string &reason, std::string language,
                         bool contextWords) {
    auto *binding = bindingFor(session);
    if (!binding || !binding->state.focused() || !binding->state.editingAvailable()) return;
    // The app list mode opens only fields the observer corroborates; the
    // manual unknown-identity path (Tab requests) needs an exact app rule.
    if (allowed && reason == "matched_default" && !binding->observed()) {
        binding->broker.answer(false);
        debug_.record("policy", binding->state.appId(), "app_rule_required");
        if (binding->revisionWaiting) continueRevision(*binding);
        return;
    }
    binding->broker.answer(allowed, std::move(language), contextWords, reason == "snoozed");
    // An editor with its own Badi integration (Neovim) holds the focused
    // terminal: Tab, the chord and every request stay the editor's. A
    // snooze withholds this app or site for a few minutes.
    debug_.record("policy", binding->state.appId(),
                  allowed                       ? "app_allowed"
                  : reason == "editor_focused" ? "editor_focused"
                  : reason == "snoozed"        ? "snoozed"
                                               : "app_disabled");
    // A revision waiting for this policy goes on instead of a context read.
    if (binding->revisionWaiting) return continueRevision(*binding);
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
    if (suggestion.replaceBefore && binding.surroundingEcho.withdrawn()) {
        return debug_.record("suggestion_blocked", binding.state.appId(), "replacement_unavailable");
    }
    // SessionState keeps IME-parity apps off this unobserved display path.
    if (!binding.observed()) return displaySuggestion(std::move(suggestion));
    const auto generation = binding.observationGeneration;
    const auto id = suggestion.suggestionId;
    const bool sent = accessibility_->request(snapshotRequest(binding.observedFocus),
        [this, suggestion, focus = binding.observedFocus, generation](const Json &reply) {
            auto *current = bindingFor(suggestion.coordinates.sessionId);
            if (current && safeToObserve(*current) && current->observationGeneration == generation &&
                current->state.coordinates() == suggestion.coordinates)
                previewIfCorroborated(*current, suggestion, focus, reply);
        });
    if (sent) binding.pendingSuggestion = id;
    else debug_.record("suggestion_blocked", binding.state.appId(), "observer_display_unavailable");
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
        previewRequest(focus, previewText(suggestion), suggestion.expiresAtMs - now, caretRect(*binding.inputContext),
                       previewStrong(suggestion)),
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
    // One display at a time: a stream may move between the panel and the
    // grey preview once the caret is calibrated, and an earlier one left
    // showing would show other text than Tab accepts.
    if (overlay) removeOwnedPanel(*binding);
    else hidePreview();
    binding->overlayOwned = overlay;
    binding->pendingSuggestion.reset();
    clearTypeThrough(*binding);
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

void BadiAddon::onAlternativeList(AlternativeList list) {
    auto *binding = bindingFor(list.coordinates.sessionId);
    if (!binding || !safeToObserve(*binding) || binding->state.acceptancePending() ||
        hasForeignImeUi(observePanel(*binding)) ||
        !binding->state.showAlternativeList(list, transport_.nowMs())) {
        return;
    }
    const auto &live = *binding->state.alternativeList();
    showAlternativeCandidates(*binding, live);
    // Hide the list when its lease ends, as for a suggestion.
    const auto now = transport_.nowMs();
    const auto suggestionId = live.suggestionId;
    binding->expiryTimer = oneShot(
        (live.expiresAtMs > now ? live.expiresAtMs - now : 0) * 1000,
        [this, coordinates = live.coordinates, suggestionId] {
            auto *current = bindingFor(coordinates.sessionId);
            if (current && current->state.alternativeList() &&
                current->state.alternativeList()->suggestionId == suggestionId) {
                current->state.clearAlternativeList();
                clearOwnedPanel(*current);
            }
        });
    binding->inputContext->updateUserInterface(::fcitx::UserInterfaceComponent::InputPanel);
    debug_.record("suggestion", binding->state.appId(), "alternative_list_shown");
}

void BadiAddon::showAlternativeCandidates(Binding &binding, const AlternativeList &list) {
    auto candidates = std::make_unique<::fcitx::CommonCandidateList>();
    candidates->setPageSize(static_cast<int>(list.words.size()));
    candidates->setLayoutHint(::fcitx::CandidateLayoutHint::Horizontal);
    for (std::size_t index = 0; index < list.words.size(); ++index) {
        candidates->append(std::make_unique<BadiCandidate>(
            ::fcitx::Text(list.words[index]),
            [this, sessionId = list.coordinates.sessionId, word = list.words[index]](
                ::fcitx::InputContext *selected) {
                auto *current = bindingFor(sessionId);
                if (current && current->inputContext == selected && selected->hasFocus())
                    requestAlternativePick(*current, word, observePanel(*current));
            },
            std::to_string(index + 1) + "."));
    }
    candidates->setGlobalCursorIndex(-1);
    auto &panel = binding.inputContext->inputPanel();
    binding.ownedAuxiliary.clear();
    panel.setAuxUp(::fcitx::Text(binding.ownedAuxiliary));
    panel.setCandidateList(std::move(candidates));
    binding.ownedCandidates = panel.candidateList();
}

bool BadiAddon::requestAlternativePick(Binding &binding, std::string word,
                                       const PanelObservation &panel) {
    const auto request = binding.state.requestAlternativePick(std::move(word), transport_.nowMs(), panel);
    if (!request || !transport_.requestAlternativePick(*request)) {
        clearOwnedPanel(binding);
        return false;
    }
    binding.requestExplicit = true;
    debug_.record("request", binding.state.appId(), "alternative_pick_requested");
    return true;
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
    if (revisionCommit(prepare)) return;
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
    std::string terminalErase;
    std::optional<std::string> correctionExpected;
    if (dispatch->replaceBefore) {
        const bool appendOnly = imeParityApp(binding->state.appId());
        if (appendOnly && !correctsAfterChecks(binding->state.appId())) return refuseCommit(prepare, binding);
        // fcitx5-qt applies the delete request to its own copy of the text,
        // which its echo cleared (SurroundingEcho): the typo would stay.
        if (binding->surroundingEcho.withdrawn()) {
            debug_.record("commit_blocked", binding->state.appId(), "replacement_unavailable");
            return refuseCommit(prepare, binding);
        }
        const auto context = currentContext(*binding);
        const auto count = context ? nativeReplacementChars(
                                        false, context->before, *dispatch->replaceBefore)
                                    : std::nullopt;
        if (!count) return refuseCommit(prepare, binding);
        if (binding->state.editPath() == NativeEditPath::TypedLine) {
            // A terminal ignores the input method's delete request. Its
            // Backspace key sends DEL to the program, so the same DELs, one
            // per tracked scalar of the typo, lead the one commit that types
            // the correction, in order (ADR 0009).
            binding->typedLine.erase(*count);
            debug_.record("commit", binding->state.appId(), "terminal_backspaces", *count);
            terminalErase = std::string(*count, '\x7f');
        } else {
            binding->inputContext->deleteSurroundingText(-static_cast<int>(*count), *count);
            // The observer confirmed this exact field just before (see
            // corroborateCommit), but an IME-parity app applies the delete
            // and the commit as two edits that page script could split
            // (ADR 0011), so its own text is read back afterwards.
            if (appendOnly) {
                correctionExpected = context->before.substr(0, context->before.size() - dispatch->replaceBefore->size()) +
                                     dispatch->text;
            }
        }
    }
    // One commit, like typed text. Fcitx cannot see the client apply it,
    // so the result is dispatched-unverified, never applied. A replacement
    // deletes the exact typed suffix first.
    binding->inputContext->commitString(terminalErase + dispatch->text);
    if (binding->state.editPath() == NativeEditPath::TypedLine) {
        binding->typedLine.type(dispatch->text);
        requestLineAfterPause(*binding);
    }
    debug_.record("commit", binding->state.appId(), "dispatched_unverified");
    transport_.reportCommit(dispatch->coordinates, dispatch->controlId, dispatch->suggestionId,
                            "dispatched-unverified");
    if (correctionExpected) expectCorrection(*binding, std::move(*correctionExpected));
}

// Waits up to a second for the app to report the corrected text. An update
// between its two edits (the typo gone, the fix not yet typed) is no verdict.
void BadiAddon::expectCorrection(Binding &binding, std::string expected) {
    binding.correctionExpected = std::move(expected);
    binding.correctionTimer = oneShot(1'000'000, [this, context = binding.inputContext] {
        auto *current = bindingFor(*context);
        if (!current || !current->correctionExpected) return;
        current->correctionExpected.reset();
        debug_.record("commit", current->state.appId(), "correction_unconfirmed");
        if (current->inputContext->hasFocus()) showNotice(*current, notice::kCorrectionUnconfirmed);
    });
    readBackCorrection(binding);
}

void BadiAddon::readBackCorrection(Binding &binding) {
    if (!binding.correctionExpected) return;
    // The app's own report of its text, whether or not Badi may still edit
    // there: the observer retires a field once its text changes. Nothing
    // read here leaves the addon; the language only satisfies the window.
    const auto &surrounding = binding.inputContext->surroundingText();
    if (!surrounding.isValid()) return;
    const auto window = captureContextWindow(surrounding.text(), surrounding.cursor(), surrounding.anchor(),
                                             false, "en");
    if (!window || !correctionLanded(window->before, *binding.correctionExpected)) return;
    binding.correctionExpected.reset();
    if (binding.correctionTimer) binding.correctionTimer->setEnabled(false);
    debug_.record("commit", binding.state.appId(), "correction_confirmed");
}

void BadiAddon::refuseCommit(const CommitPrepare &prepare, Binding *binding) {
    if (binding) clearOwnedPanel(*binding);
    reportStale(prepare);
}

void BadiAddon::reportStale(const CommitPrepare &prepare) {
    transport_.reportCommit(prepare.coordinates, prepare.controlId, prepare.suggestionId, "stale");
}

// The broker refused one request and keeps the connection. Only what that
// id names ends; ids repeat across sessions, so every binding that holds it
// retires it, which only ever withdraws.
void BadiAddon::onRefusal(const RequestRefusal &refusal) {
    for (auto &[_, binding] : bindings_) {
        switch (binding.state.refuseRequest(refusal.requestId)) {
        case RefusedRequest::None:
            break;
        case RefusedRequest::Acceptance:
            debug_.record("commit_blocked", binding.state.appId(), "broker_refused");
            clearOwnedPanel(binding);
            break;
        case RefusedRequest::Revision: {
            const auto text = revisionClearNotice(refusal.reason);
            endRevision(binding, text ? *text : notice::kRequestFailed, "broker_refused");
            break;
        }
        case RefusedRequest::Suggestion:
            debug_.record("request_blocked", binding.state.appId(), "broker_refused");
            // The user asked and nothing will answer: say so instead of
            // leaving "thinking" up.
            if (binding.requestExplicit)
                showNotice(binding, refusal.reason == "paused" ? notice::kPaused : notice::kRequestFailed);
            break;
        }
    }
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
        binding.revisionWaiting.reset();
        dropHeldTab(binding);
        binding.state.invalidateContext();
        binding.surroundingFreshness.transportLost();
        clearOwnedPanel(binding);
    }
}

} // namespace badi::fcitx5
