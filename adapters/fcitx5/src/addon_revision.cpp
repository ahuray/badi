// Explicit revision of one selection (broker ADR 0012): Ctrl+Shift+Space on
// a selection asks the broker for a grammar check, and again while it shows
// for a refined wording. The revision shows with its changes marked; Tab
// applies it with one commit over exactly the revised selection, read back.

#include "addon.h"
#include "fcitx_event.h"

#include <fcitx/candidatelist.h>
#include <fcitx/inputcontext.h>
#include <fcitx/inputmethodentry.h>
#include <fcitx/inputpanel.h>
#include <fcitx/instance.h>
#include <fcitx/text.h>
#include <fcitx/userinterface.h>

#include <memory>
#include <optional>
#include <string>
#include <utility>

namespace badi::fcitx5 {

namespace {

// A revision answers within the broker's 40-second bound (cold start
// included); the notice waits as long, and a little more.
constexpr std::uint64_t kRevisionWaitUs = 45'000'000;
// The observer's one request may still be busy with an earlier read: the
// request retries within its own budget, and a commit waits until its own
// deadline, as a suggestion's does.
constexpr std::uint64_t kRevisionRetryUs = InspectionSchedule::kBusyRetryUs;
constexpr unsigned int kMaxRevisionRetries = InspectionSchedule::kMaxBusyRetries;
constexpr std::uint64_t kRevisionCommitWaitUs = 500'000;
// How long the field may take to report the applied revision.
constexpr std::uint64_t kRevisionReadBackUs = 1'000'000;

class RevisionLine final : public ::fcitx::CandidateWord {
public:
    RevisionLine(::fcitx::Text text, std::function<void(::fcitx::InputContext *)> selected)
        : CandidateWord(std::move(text)), selected_(std::move(selected)) {
        setCustomLabel(::fcitx::Text());
    }

    void select(::fcitx::InputContext *inputContext) const override { selected_(inputContext); }

private:
    std::function<void(::fcitx::InputContext *)> selected_;
};

// Removed words struck through, added ones in the highlight colour and bold.
::fcitx::Text lineText(const std::vector<DiffPart> &line, bool cut) {
    ::fcitx::Text text;
    bool first = true;
    for (const auto &part : line) {
        if (!first) text.append(" ");
        first = false;
        switch (part.kind) {
        case DiffKind::Same:
            text.append(part.text);
            break;
        case DiffKind::Removed:
            text.append(part.text, ::fcitx::TextFormatFlags{::fcitx::TextFormatFlag::Strike});
            break;
        case DiffKind::Added:
            text.append(part.text, ::fcitx::TextFormatFlags{::fcitx::TextFormatFlag::HighLight,
                                                           ::fcitx::TextFormatFlag::Bold});
            break;
        }
    }
    if (cut) text.append(" …");
    return text;
}

constexpr std::string_view waitingNotice(std::string_view mode) {
    if (mode == "synonyms") return "Badi is finding a synonym…";
    return mode == "refine" ? "Badi is refining the selection…" : "Badi is checking the selection…";
}

} // namespace

// The selection Fcitx reports now, if a revision may read it.
std::optional<SelectionWindow> BadiAddon::currentSelection(const Binding &binding) const {
    auto &input = *binding.inputContext;
    if (!input.hasFocus() || !binding.state.focused() || !binding.surrounding.freshness.fresh() ||
        !allowsNativeContext(fieldCapabilities(binding)) || instance_->isComposing(&input)) {
        return std::nullopt;
    }
    const auto &surrounding = input.surroundingText();
    const auto *method = instance_->inputMethodEntry(&input);
    if (!surrounding.isValid() || surrounding.cursor() == surrounding.anchor() || !method ||
        !validLanguageTag(method->languageCode())) {
        return std::nullopt;
    }
    auto window = captureSelectionWindow(surrounding.text(), surrounding.cursor(), surrounding.anchor(),
        !!(input.capabilityFlags() & ::fcitx::CapabilityFlag::Multiline),
        binding.broker.languageOverride.empty() ? method->languageCode() : binding.broker.languageOverride);
    if (window) window->identityKnown = binding.observed();
    return window;
}

// The chord: a revision of the selection, or the other mode of the one
// shown. False leaves the chord its usual meaning (no selection).
bool BadiAddon::revisionChord(Binding &binding) {
    const auto &app = binding.state.appId();
    if (const auto &shown = binding.state.visibleRevision()) {
        auto selection = binding.state.revisionRequest()->selection;
        const auto mode = nextRevisionMode(shown->mode, selection.selected);
        debug_.record("revision", app, mode + "_requested");
        startRevision(binding, std::move(selection), mode);
        return true;
    }
    if (binding.revisionActive()) return true;
    if (!binding.observation.unnamedIbus) {
        const auto kind = classifyNativeApp(app);
        if (kind == NativeAppClass::Terminal || kind == NativeAppClass::Unavailable) return false;
    } else if (!app.empty() && !unnamedIbusObservedApp(app)) {
        return false;
    }
    const auto &surrounding = binding.inputContext->surroundingText();
    if (!surrounding.isValid() || surrounding.cursor() == surrounding.anchor()) return false;
    if (hasForeignImeUi(observePanel(binding))) return false;
    auto selection = currentSelection(binding);
    if (!selection) {
        debug_.record("revision", app, "selection_unreadable");
        showNotice(binding, notice::kRevisionSelection);
        return true;
    }
    std::string mode = "grammar";
    if (binding.revision.cycle) {
        const auto &[cycledSelection, cycledMode] = *binding.revision.cycle;
        if (cycledSelection.before == selection->before &&
            cycledSelection.selected == selection->selected &&
            cycledSelection.after == selection->after &&
            cycledSelection.anchor == selection->anchor &&
            cycledSelection.head == selection->head &&
            cycledSelection.language == selection->language) {
            mode = cycledMode;
        }
        binding.revision.cycle.reset();
    }
    debug_.record("revision", app, mode + "_requested", selection->selected.size());
    startRevision(binding, std::move(*selection), mode);
    return true;
}

void BadiAddon::startRevision(Binding &binding, SelectionWindow selection, std::string mode) {
    binding.observation.zenCommit.clear();
    binding.typeThrough.reset();
    clearAllUi(binding);
    binding.state.clearRevision();
    ensureBroker();
    binding.revision.waiting = std::make_pair(std::move(selection), std::move(mode));
    binding.revision.retries = 0;
    binding.revision.retryStep = RevisionRetryStep::None;
    showRevisionNotice(binding, waitingNotice(binding.revision.waiting->second));
    // A field Badi already holds for this selection goes on at once; any
    // other is bound through the observer, or the manual contract.
    if (binding.observed() && binding.broker.opened && binding.broker.policyAllowed &&
        binding.observation.focus.value("selection_count", Json()) == 1) {
        return continueRevision(binding);
    }
    if (!binding.observation.unnamedIbus && !nativeObservationAvailable(binding.state.appId())) {
        return revisionByManualContract(binding);
    }
    inspectForRevision(binding);
}

// Runs `step` again for a waiting revision once the observer's one request
// may be free, within the request's retry budget. One timer per binding is
// re-armed in place: replacing it from its own callback would destroy the
// closure that is running.
bool BadiAddon::retryRevision(Binding &binding, RevisionRetryStep step) {
    if (binding.revision.retries++ >= kMaxRevisionRetries) return false;
    binding.revision.retryStep = step;
    if (!binding.revision.retryTimer) {
        binding.revision.retryTimer = instance_->eventLoop().addTimeEvent(CLOCK_MONOTONIC, 0, kTimerAccuracyUs,
            [this, context = binding.inputContext](::fcitx::EventSourceTime *, std::uint64_t) {
                auto *current = bindingFor(*context);
                if (!current || !current->revision.waiting) return true;
                switch (current->revision.retryStep) {
                case RevisionRetryStep::None:
                    break;
                case RevisionRetryStep::Inspect:
                    inspectForRevision(*current);
                    break;
                case RevisionRetryStep::Continue:
                    continueRevision(*current);
                    break;
                }
                return true;
            });
    }
    binding.revision.retryTimer->setNextInterval(kRevisionRetryUs);
    binding.revision.retryTimer->setOneShot();
    return true;
}

void BadiAddon::inspectForRevision(Binding &binding) {
    if (accessibility_->pending()) {
        if (retryRevision(binding, RevisionRetryStep::Inspect)) return;
        return endRevision(binding, notice::kObserverUnavailable, "observer_busy");
    }
    // The observer owns its reply callbacks and may run them after the input
    // context is gone, so they find the binding by session, never by pointer.
    const auto session = binding.state.coordinates().sessionId;
    const auto generation = ++binding.observation.generation;
    const auto request = binding.observation.unnamedIbus ? unnamedIbusInspectRequest(true)
                                                         : revisionInspectRequest(binding.state.appId());
    const bool sent = accessibility_->request(request,
        [this, session, generation](const Json &reply) {
            auto *current = bindingFor(session);
            if (current && current->revision.waiting && current->observation.generation == generation)
                revisionInspected(*current, reply);
        });
    if (!sent) endRevision(binding, notice::kObserverUnavailable, "observer_unavailable");
}

void BadiAddon::revisionInspected(Binding &binding, const Json &reply) {
    std::string app = binding.state.appId();
    std::optional<NativeEditTarget> target;
    if (observerAnswered(reply)) {
        if (binding.observation.unnamedIbus) {
            if (const auto resolved = unnamedIbusEditTarget(reply["focus"], true)) {
                app = resolved->first;
                target = resolved->second;
            }
        } else {
            target = revisionEditTarget(reply["focus"], app);
        }
    }
    if (!target) {
        // A native exact field the observer cannot read is still the user's
        // explicit request through the manual contract; IME-parity and
        // unnamed-IBus contexts have none.
        if (!binding.observation.unnamedIbus && classifyNativeApp(app) == NativeAppClass::NativeExact)
            return revisionByManualContract(binding);
        if (binding.observation.unnamedIbus && (binding.observed() || !binding.state.appId().empty()))
            retire(binding, RetireReason::ObservedFieldChanged);
        if (binding.observation.unnamedIbus && reply.value("error", Json()) == "unsupported_app")
            return endRevision(binding, notice::kUnavailableApp, "unidentified_app");
        return endRevision(binding, notice::kRevisionSelection, "observer_field_denied");
    }
    auto waiting = std::move(*binding.revision.waiting);
    bindObservedField(binding, reply["focus"], *target, app);
    if (!binding.state.editingAvailable()) return endRevision(binding, notice::kUnavailableApp, "no_edit_path");
    // Binding opened a new field session; the selection waits for its policy.
    waiting.first.identityKnown = true;
    binding.revision.waiting = std::move(waiting);
    showRevisionNotice(binding, waitingNotice(binding.revision.waiting->second));
    if (binding.broker.policyKnown) continueRevision(binding);
}

// An unobserved native field: the unknown-identity explicit contract.
void BadiAddon::revisionByManualContract(Binding &binding) {
    if (binding.observation.unnamedIbus) return endRevision(binding, notice::kRevisionSelection, "observer_field_denied");
    if (binding.observed()) {
        auto waiting = std::move(*binding.revision.waiting);
        retire(binding, RetireReason::ObservedFieldChanged);
        if (!binding.state.focused()) return endRevision(binding, notice::kRequestFailed, "session_unavailable");
        binding.revision.waiting = std::move(waiting);
    }
    binding.revision.waiting->first.identityKnown = false;
    showRevisionNotice(binding, waitingNotice(binding.revision.waiting->second));
    if (!binding.broker.policyKnown) {
        queryPolicy(binding);
        return;
    }
    continueRevision(binding);
}

// Policy is known for the field: open its session, confirm the selection
// with the observer when it holds the field, and ask.
void BadiAddon::continueRevision(Binding &binding) {
    if (!binding.revision.waiting) return;
    if (!binding.broker.policyAllowed) {
        return endRevision(binding, binding.broker.snoozed ? notice::kSnoozed : notice::kDisabled, "app_disabled");
    }
    if (!open(binding)) return endRevision(binding, authorityPaused_ ? notice::kPaused : notice::kModelNotConnected,
                                           "session_unavailable");
    if (!binding.observed()) return sendRevision(binding, std::nullopt);
    if (accessibility_->pending()) {
        if (retryRevision(binding, RevisionRetryStep::Continue)) return;
        return endRevision(binding, notice::kObserverUnavailable, "observer_busy");
    }
    const auto session = binding.state.coordinates().sessionId;
    const auto generation = binding.observation.generation;
    const bool sent = accessibility_->request(selectionSnapshotRequest(binding.observation.focus),
        [this, session, generation, focus = binding.observation.focus](const Json &reply) {
            auto *current = bindingFor(session);
            if (!current || !current->revision.waiting || current->observation.generation != generation) return;
            if (!observerAnswered(reply) || !sameSelectedField(focus, reply["focus"]))
                return endRevision(*current, notice::kRevisionSelection, "observer_snapshot_denied");
            sendRevision(*current, reply["focus"]);
        });
    if (!sent) endRevision(binding, notice::kObserverUnavailable, "observer_unavailable");
}

void BadiAddon::sendRevision(Binding &binding, const std::optional<Json> &observed) {
    auto [selection, mode] = std::move(*binding.revision.waiting);
    binding.revision.waiting.reset();
    const auto &app = binding.state.appId();
    // The selection is still exactly the one asked about, and the observer
    // reads the same field text around it.
    const auto current = currentSelection(binding);
    if (!current || !sameSelection(*current, selection, true)) {
        return endRevision(binding, notice::kRevisionSelection, "selection_changed");
    }
    if (observed && !observerAgreesSelection(*observed, selection)) {
        return endRevision(binding, notice::kRevisionSelection, "observer_selection_mismatch");
    }
    if (observed) binding.observation.focus = *observed;
    const auto update = binding.state.requestRevision(std::move(selection), std::move(mode));
    if (!update || !transport_.requestRevision(*update)) {
        return endRevision(binding, notice::kRequestFailed, "request_failed");
    }
    binding.broker.coordinates = update->coordinates;
    debug_.record("revision", app, update->mode + "_sent", update->selection.selected.size());
    showRevisionNotice(binding, waitingNotice(update->mode));
}

// Ends a revision before the broker had it: say why, if the user should know.
void BadiAddon::endRevision(Binding &binding, std::string_view message, std::string_view reason) {
    binding.observation.zenCommit.clear();
    binding.state.clearRevision();
    clearRevisionUi(binding);
    debug_.record("revision_blocked", binding.state.appId(), reason);
    showNotice(binding, message);
}

void BadiAddon::onRevision(Revision revision) {
    auto *binding = bindingFor(revision.coordinates.sessionId);
    if (!binding) return;
    const auto &request = binding->state.revisionRequest();
    const auto current = currentSelection(*binding);
    // The field must still hold the revised selection.
    if (!request || !current || !sameSelection(*current, request->selection, false) ||
        hasForeignImeUi(observePanel(*binding)) || !binding->state.showRevision(revision, transport_.nowMs())) {
        debug_.record("revision_blocked", binding->state.appId(), "revision_display_mismatch");
        return cancelRevision(*binding);
    }
    showRevisionCandidate(*binding, revision);
    const auto now = transport_.nowMs();
    binding->revision.expiryTimer = oneShot((revision.expiresAtMs > now ? revision.expiresAtMs - now : 0) * 1000,
        [this, coordinates = revision.coordinates, id = revision.revisionId] {
            auto *current = bindingFor(coordinates.sessionId);
            if (current && current->state.clearRevisionIf(coordinates, id)) clearRevisionUi(*current);
        });
    debug_.record("revision", binding->state.appId(), "revision_shown", revision.text.size());
    debug_.trace("revision shown", binding->state.appId(), {{"mode", revision.mode}, {"bytes", revision.text.size()}},
                 {{"text", revision.text}});
}

void BadiAddon::showRevisionCandidate(Binding &binding, const Revision &revision) {
    const auto lines = wrapDiff(revisionDiff(revision.original, revision.text), kRevisionLineScalars, kRevisionLines);
    auto list = std::make_unique<::fcitx::CommonCandidateList>();
    list->setPageSize(static_cast<int>(lines.lines.size()));
    list->setLayoutHint(::fcitx::CandidateLayoutHint::Vertical);
    for (std::size_t index = 0; index < lines.lines.size(); ++index) {
        const bool last = index + 1 == lines.lines.size();
        list->append(std::make_unique<RevisionLine>(lineText(lines.lines[index], last && !lines.complete),
            [this, sessionId = revision.coordinates.sessionId](::fcitx::InputContext *selected) {
                auto *current = bindingFor(sessionId);
                if (current && current->inputContext == selected && selected->hasFocus())
                    acceptRevision(*current);
            }));
    }
    list->setGlobalCursorIndex(-1);
    clearSuggestionUi(binding);
    clearNotice(binding);
    hidePreview();
    binding.panel.overlayOwned = false;
    auto &panel = binding.inputContext->inputPanel();
    // The candidate list replaces the waiting notice in place.
    if (binding.revision.noticeTimer) binding.revision.noticeTimer->setEnabled(false);
    binding.panel.owner = PanelOwner::Revision;
    binding.panel.auxiliary = revisionHelp(revision.mode, revision.original);
    panel.setAuxUp(::fcitx::Text(binding.panel.auxiliary));
    panel.setCandidateList(std::move(list));
    binding.panel.candidates = panel.candidateList();
    binding.inputContext->updateUserInterface(::fcitx::UserInterfaceComponent::InputPanel);
}

// The waiting notice stays until the answer, unlike a timed notice.
void BadiAddon::showRevisionNotice(Binding &binding, std::string_view message) {
    if (!binding.inputContext->hasFocus() || hasForeignImeUi(observePanel(binding))) return;
    clearSuggestionUi(binding);
    clearNotice(binding);
    hidePreview();
    binding.panel.overlayOwned = false;
    auto &panel = binding.inputContext->inputPanel();
    if (binding.panel.candidates && panel.candidateList().get() == binding.panel.candidates.get())
        panel.setCandidateList(nullptr);
    binding.panel.candidates.reset();
    binding.panel.owner = PanelOwner::Revision;
    binding.panel.auxiliary = message;
    panel.setAuxUp(::fcitx::Text(binding.panel.auxiliary));
    if (binding.revision.expiryTimer) binding.revision.expiryTimer->setEnabled(false);
    binding.revision.noticeTimer = oneShot(kRevisionWaitUs, [this, context = binding.inputContext] {
        auto *current = bindingFor(*context);
        if (current && current->revisionActive() && !current->state.visibleRevision()) cancelRevision(*current);
    });
    binding.inputContext->updateUserInterface(::fcitx::UserInterfaceComponent::InputPanel);
}

void BadiAddon::onRevisionClear(const RevisionClear &clear) {
    auto *binding = bindingFor(clear.coordinates.sessionId);
    if (!binding) return;
    // Nothing to change is not an edit or Escape: the cycle continues
    // with the next mode for this exact selection (ADR 0013).
    std::optional<std::pair<SelectionWindow, std::string>> cycle;
    if (clear.reason == "no_change") {
        if (const auto &request = binding->state.revisionRequest()) {
            cycle = std::make_pair(request->selection,
                                   nextRevisionMode(request->mode, request->selection.selected));
        }
    }
    if (!binding->state.clearRevisionIf(clear.coordinates, clear.revisionId)) return;
    binding->revision.cycle = std::move(cycle);
    debug_.record("revision_clear", binding->state.appId(), clear.reason);
    clearRevisionUi(*binding);
    if (const auto text = revisionClearNotice(clear.reason)) showNotice(*binding, *text);
}

void BadiAddon::acceptRevision(Binding &binding) {
    // A second Tab while the first is being applied is a no-op.
    if (binding.state.revisionAcceptancePending()) return;
    const auto request = binding.state.requestRevisionAcceptance(transport_.nowMs(), observePanel(binding));
    if (!request || !transport_.requestAcceptance(*request)) {
        debug_.record("revision_blocked", binding.state.appId(), "acceptance_unavailable");
        return cancelRevision(binding);
    }
    debug_.record("revision", binding.state.appId(), "revision_accepted");
}

// Escape, or any key that edits the field: tell the broker and clear Badi's
// panel. The key itself keeps its meaning.
void BadiAddon::cancelRevision(Binding &binding) {
    binding.observation.zenCommit.clear();
    binding.revision.cycle.reset();
    if (binding.state.visibleRevision() && !binding.state.revisionAcceptancePending()) {
        if (const auto request = binding.state.requestRevisionDismissal(transport_.nowMs(), observePanel(binding)))
            transport_.requestDismissal(*request);
    } else if (const auto &request = binding.state.revisionRequest(); request && !binding.state.visibleRevision()) {
        transport_.cancelRequest(request->coordinates);
    }
    binding.state.clearRevision();
    clearRevisionUi(binding);
}

// Keys while a revision waits or shows. True when the key was Badi's.
bool BadiAddon::revisionKey(::fcitx::KeyEvent &event, Binding &binding) {
    const auto panel = observePanel(binding);
    switch (classifyRevisionPreKey(event.key(), !event.filtered() && !event.accepted(),
                                   hasForeignImeUi(panel))) {
    case RevisionPreKey::YieldForeignIme:
        debug_.record("revision", binding.state.appId(), "foreign_ime_active");
        cancelRevision(binding);
        return false;
    case RevisionPreKey::KeepForModifier:
    case RevisionPreKey::PassInvokeToPostInput:
        return true;
    case RevisionPreKey::FilterInvokeRepeat:
        event.filterAndAccept();
        return true;
    case RevisionPreKey::AcceptTab:
        // Tab applies a shown revision; one that is still coming is waited
        // for, rather than letting Tab replace the selection.
        if (binding.state.visibleRevision() && hasOwnedCandidate(panel)) acceptRevision(binding);
        event.filterAndAccept();
        return true;
    case RevisionPreKey::DismissEscape:
        debug_.record("revision", binding.state.appId(), "revision_dismissed");
        cancelRevision(binding);
        event.filterAndAccept();
        return true;
    case RevisionPreKey::RetireByKey:
        // Typing replaces the selection, as it would without Badi.
        debug_.record("revision", binding.state.appId(), "revision_retired_by_key");
        cancelRevision(binding);
        return false;
    }
    return false;
}

bool BadiAddon::revisionCommit(const CommitPrepare &prepare) {
    if (!prepare.replaceSelection) return false;
    auto *binding = bindingFor(prepare.coordinates.sessionId);
    if (!binding || !binding->state.revisionAcceptancePending() || !binding->state.revisionRequest()) {
        reportStale(prepare);
        return true;
    }
    if (!binding->observed()) {
        applyRevision(*binding, prepare);
        return true;
    }
    corroborateRevisionCommit(*binding, prepare, monotonicUs() + kRevisionCommitWaitUs);
    return true;
}

// The observer confirms the exact field and selection just before the
// commit, as for every observed edit. A request still holding the observer
// is waited for on the commit's own timer, re-armed in place, until the
// grant's deadline.
void BadiAddon::corroborateRevisionCommit(Binding &binding, const CommitPrepare &prepare, std::uint64_t deadline) {
    if (accessibility_->pending()) {
        if (monotonicUs() >= deadline) return refuseRevisionCommit(prepare, &binding);
        binding.revision.commitTimer = instance_->eventLoop().addTimeEvent(CLOCK_MONOTONIC, 0, kTimerAccuracyUs,
            [this, prepare, deadline](::fcitx::EventSourceTime *timer, std::uint64_t) {
                if (accessibility_->pending() && monotonicUs() < deadline) {
                    timer->setNextInterval(kRevisionRetryUs);
                    timer->setOneShot();
                    return true;
                }
                auto *current = bindingFor(prepare.coordinates.sessionId);
                if (current && current->state.revisionAcceptancePending())
                    corroborateRevisionCommit(*current, prepare, deadline);
                else reportStale(prepare);
                return true;
            });
        binding.revision.commitTimer->setNextInterval(kRevisionRetryUs);
        binding.revision.commitTimer->setOneShot();
        return;
    }
    const bool sent = accessibility_->request(selectionSnapshotRequest(binding.observation.focus),
        [this, prepare, focus = binding.observation.focus, generation = binding.observation.generation](const Json &reply) {
            auto *current = bindingFor(prepare.coordinates.sessionId);
            if (!current) return reportStale(prepare);
            const auto &request = current->state.revisionRequest();
            if (current->observation.generation != generation || !request || !observerAnswered(reply) ||
                !sameSelectedField(focus, reply["focus"]) ||
                !observerAgreesSelection(reply["focus"], request->selection)) {
                debug_.record("commit_blocked", current->state.appId(), "observer_dispatch_mismatch");
                return refuseRevisionCommit(prepare, current);
            }
            applyRevision(*current, prepare);
        });
    if (!sent) refuseRevisionCommit(prepare, &binding);
}

void BadiAddon::applyRevision(Binding &binding, const CommitPrepare &prepare) {
    const auto request = binding.state.revisionRequest();
    const auto current = currentSelection(binding);
    if (!request || !binding.inputContext->hasFocus() || !current ||
        !sameSelection(*current, request->selection, true)) {
        debug_.record("commit_blocked", binding.state.appId(), "selection_changed");
        return refuseRevisionCommit(prepare, &binding);
    }
    const auto selection = request->selection;
    const auto dispatch = binding.state.authorizeRevisionCommit(prepare, transport_.nowMs(), observePanel(binding));
    if (!dispatch) return refuseRevisionCommit(prepare, &binding);
    clearRevisionUi(binding);
    // One commit over the selection, as typing over it would (ADR 0012).
    // Fcitx cannot see the client apply it: dispatched, never applied.
    binding.inputContext->commitString(dispatch->text);
    debug_.record("commit", binding.state.appId(), "revision_dispatched_unverified");
    transport_.reportCommit(dispatch->coordinates, dispatch->controlId, dispatch->suggestionId,
                            "dispatched-unverified");
    binding.readBack.revision = std::make_pair(selection, dispatch->text);
    binding.readBack.revisionTimer = oneShot(kRevisionReadBackUs, [this, context = binding.inputContext] {
        auto *current = bindingFor(*context);
        if (!current || !current->readBack.revision) return;
        current->readBack.revision.reset();
        debug_.record("commit", current->state.appId(), "revision_unconfirmed");
        if (current->inputContext->hasFocus()) showNotice(*current, notice::kRevisionUnconfirmed);
    });
    readBackRevision(binding);
}

void BadiAddon::refuseRevisionCommit(const CommitPrepare &prepare, Binding *binding) {
    if (binding) {
        binding->observation.zenCommit.clear();
        binding->state.clearRevision();
        clearRevisionUi(*binding);
    }
    reportStale(prepare);
}

void BadiAddon::readBackRevision(Binding &binding) {
    if (!binding.readBack.revision) return;
    const auto &surrounding = binding.inputContext->surroundingText();
    if (!surrounding.isValid()) return;
    const auto window = captureContextWindow(surrounding.text(), surrounding.cursor(), surrounding.anchor(), false,
                                             "en");
    const auto &[selection, revised] = *binding.readBack.revision;
    if (!window || !revisionLanded(window->before, window->after, selection, revised)) return;
    binding.readBack.clearRevision();
    debug_.record("commit", binding.state.appId(), "revision_confirmed");
}

// A field change while a revision waits or shows: the same selection keeps
// it; anything else retires it.
bool BadiAddon::revisionSurvives(Binding &binding) {
    if (!binding.revisionActive()) return false;
    const auto current = currentSelection(binding);
    const auto *selection = binding.revision.waiting ? &binding.revision.waiting->first
                                                     : &binding.state.revisionRequest()->selection;
    if (current && sameSelection(*current, *selection, false)) return true;
    debug_.record("revision", binding.state.appId(), "revision_retired_by_edit");
    cancelRevision(binding);
    return false;
}

} // namespace badi::fcitx5
