// Explicit revision of one selection (broker ADR 0012): Ctrl+Shift+Space on
// a selection asks the broker for a grammar check, and again while it shows
// for a refined wording. The revision shows with its changes marked; Tab
// applies it with one commit over exactly the revised selection, read back.

#include "addon.h"

#include <fcitx-utils/event.h>
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
// The observer's one request may still be busy with an earlier read.
constexpr std::uint64_t kRevisionRetryUs = 20'000;
constexpr unsigned int kMaxRevisionRetries = 25;
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
    return mode == "refine" ? "Badi is refining the selection…" : "Badi is checking the selection…";
}

} // namespace

bool BadiAddon::revisionActive(const Binding &binding) const {
    return binding.revisionWaiting || binding.state.revisionRequest();
}

// The selection Fcitx reports now, if a revision may read it.
std::optional<SelectionWindow> BadiAddon::currentSelection(const Binding &binding) const {
    auto &input = *binding.inputContext;
    if (!input.hasFocus() || !binding.state.focused() || !binding.surroundingFreshness.fresh() ||
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
        const auto mode = shown->mode == "grammar" ? "refine" : "grammar";
        auto selection = binding.state.revisionRequest()->selection;
        debug_.record("revision", app, mode == std::string_view("refine") ? "refine_requested"
                                                                         : "grammar_requested");
        startRevision(binding, std::move(selection), mode);
        return true;
    }
    if (revisionActive(binding)) return true;
    const auto kind = classifyNativeApp(app);
    if (kind == NativeAppClass::Terminal || kind == NativeAppClass::Unavailable) return false;
    const auto &surrounding = binding.inputContext->surroundingText();
    if (!surrounding.isValid() || surrounding.cursor() == surrounding.anchor()) return false;
    if (hasForeignImeUi(observePanel(binding))) return false;
    auto selection = currentSelection(binding);
    if (!selection) {
        debug_.record("revision", app, "selection_unreadable");
        showNotice(binding, notice::kRevisionSelection);
        return true;
    }
    debug_.record("revision", app, "grammar_requested", selection->selected.size());
    startRevision(binding, std::move(*selection), "grammar");
    return true;
}

void BadiAddon::startRevision(Binding &binding, SelectionWindow selection, std::string mode) {
    clearOwnedPanel(binding);
    binding.state.clearRevision();
    if (!transport_.ready()) transport_.connect();
    binding.revisionWaiting = std::make_pair(std::move(selection), std::move(mode));
    binding.revisionRetries = 0;
    showRevisionNotice(binding, waitingNotice(binding.revisionWaiting->second));
    // A field Badi already holds for this selection goes on at once; any
    // other is bound through the observer, or the manual contract.
    if (binding.observed() && binding.broker.opened && binding.broker.policyAllowed &&
        binding.observedFocus.value("selection_count", Json()) == 1) {
        return continueRevision(binding);
    }
    if (!nativeObservationAvailable(binding.state.appId())) return revisionByManualContract(binding);
    inspectForRevision(binding);
}

void BadiAddon::inspectForRevision(Binding &binding) {
    if (accessibility_->pending()) {
        if (binding.revisionRetries++ < kMaxRevisionRetries) {
            binding.revisionTimer = oneShot(kRevisionRetryUs, [this, context = binding.inputContext] {
                if (auto *current = bindingFor(*context); current && current->revisionWaiting)
                    inspectForRevision(*current);
            });
            return;
        }
        return endRevision(binding, notice::kObserverUnavailable, "observer_busy");
    }
    const auto generation = ++binding.observationGeneration;
    const bool sent = accessibility_->request(revisionInspectRequest(binding.state.appId()),
        [this, context = binding.inputContext, generation](const Json &reply) {
            auto *current = bindingFor(*context);
            if (current && current->revisionWaiting && current->observationGeneration == generation)
                revisionInspected(*current, reply);
        });
    if (!sent) endRevision(binding, notice::kObserverUnavailable, "observer_unavailable");
}

void BadiAddon::revisionInspected(Binding &binding, const Json &reply) {
    const auto &app = binding.state.appId();
    const auto target = observerAnswered(reply) ? revisionEditTarget(reply["focus"], app) : std::nullopt;
    if (!target) {
        // A native exact field the observer cannot read is still the user's
        // explicit request through the manual contract; IME-parity has none.
        if (classifyNativeApp(app) == NativeAppClass::NativeExact) return revisionByManualContract(binding);
        return endRevision(binding, notice::kRevisionSelection, "observer_field_denied");
    }
    auto waiting = std::move(*binding.revisionWaiting);
    bindObservedField(binding, reply["focus"], *target);
    if (!binding.state.editingAvailable()) return endRevision(binding, notice::kUnavailableApp, "no_edit_path");
    // Binding opened a new field session; the selection waits for its policy.
    waiting.first.identityKnown = true;
    binding.revisionWaiting = std::move(waiting);
    showRevisionNotice(binding, waitingNotice(binding.revisionWaiting->second));
    if (binding.broker.policyKnown) continueRevision(binding);
}

// An unobserved native field: the unknown-identity explicit contract.
void BadiAddon::revisionByManualContract(Binding &binding) {
    if (binding.observed()) retireObservedField(binding);
    binding.revisionWaiting->first.identityKnown = false;
    showRevisionNotice(binding, waitingNotice(binding.revisionWaiting->second));
    if (!binding.broker.policyKnown) {
        queryPolicy(binding);
        return;
    }
    continueRevision(binding);
}

// Policy is known for the field: open its session, confirm the selection
// with the observer when it holds the field, and ask.
void BadiAddon::continueRevision(Binding &binding) {
    if (!binding.revisionWaiting) return;
    if (!binding.broker.policyAllowed) {
        return endRevision(binding, binding.broker.snoozed ? notice::kSnoozed : notice::kDisabled, "app_disabled");
    }
    if (!open(binding)) return endRevision(binding, authorityPaused_ ? notice::kPaused : notice::kModelNotConnected,
                                           "session_unavailable");
    if (!binding.observed()) return sendRevision(binding, std::nullopt);
    if (accessibility_->pending()) {
        if (binding.revisionRetries++ < kMaxRevisionRetries) {
            binding.revisionTimer = oneShot(kRevisionRetryUs, [this, context = binding.inputContext] {
                if (auto *current = bindingFor(*context); current && current->revisionWaiting)
                    continueRevision(*current);
            });
            return;
        }
        return endRevision(binding, notice::kObserverUnavailable, "observer_busy");
    }
    const auto generation = binding.observationGeneration;
    const bool sent = accessibility_->request(selectionSnapshotRequest(binding.observedFocus),
        [this, context = binding.inputContext, generation, focus = binding.observedFocus](const Json &reply) {
            auto *current = bindingFor(*context);
            if (!current || !current->revisionWaiting || current->observationGeneration != generation) return;
            if (!observerAnswered(reply) || !sameSelectedField(focus, reply["focus"]))
                return endRevision(*current, notice::kRevisionSelection, "observer_snapshot_denied");
            sendRevision(*current, reply["focus"]);
        });
    if (!sent) endRevision(binding, notice::kObserverUnavailable, "observer_unavailable");
}

void BadiAddon::sendRevision(Binding &binding, const std::optional<Json> &observed) {
    auto [selection, mode] = std::move(*binding.revisionWaiting);
    binding.revisionWaiting.reset();
    const auto &app = binding.state.appId();
    // The selection is still exactly the one asked about, and the observer
    // reads the same field text around it.
    const auto current = currentSelection(binding);
    if (!current || current->selected != selection.selected || current->before != selection.before ||
        current->after != selection.after || current->anchor != selection.anchor || current->head != selection.head) {
        return endRevision(binding, notice::kRevisionSelection, "selection_changed");
    }
    if (observed && !observerAgreesSelection(*observed, selection)) {
        return endRevision(binding, notice::kRevisionSelection, "observer_selection_mismatch");
    }
    if (observed) binding.observedFocus = *observed;
    const auto update = binding.state.requestRevision(std::move(selection), std::move(mode));
    if (!update || !transport_.requestRevision(*update)) {
        return endRevision(binding, notice::kRequestFailed, "request_failed");
    }
    binding.broker.coordinates = update->coordinates;
    debug_.record("revision", app, update->mode == "refine" ? "refine_sent" : "grammar_sent",
                  update->selection.selected.size());
    showRevisionNotice(binding, waitingNotice(update->mode));
}

// Ends a revision before the broker had it: say why, if the user should know.
void BadiAddon::endRevision(Binding &binding, std::string_view message, std::string_view reason) {
    binding.revisionWaiting.reset();
    if (binding.revisionTimer) binding.revisionTimer->setEnabled(false);
    binding.state.clearRevision();
    debug_.record("revision_blocked", binding.state.appId(), reason);
    showNotice(binding, message);
}

void BadiAddon::onRevision(Revision revision) {
    auto *binding = bindingFor(revision.coordinates.sessionId);
    if (!binding) return;
    const auto &request = binding->state.revisionRequest();
    const auto current = currentSelection(*binding);
    // The field must still hold exactly the revised selection.
    if (!request || !current || current->selected != request->selection.selected ||
        current->before != request->selection.before || current->anchor != request->selection.anchor ||
        current->head != request->selection.head || hasForeignImeUi(observePanel(*binding)) ||
        !binding->state.showRevision(revision, transport_.nowMs())) {
        debug_.record("revision_blocked", binding->state.appId(), "revision_display_mismatch");
        return cancelRevision(*binding);
    }
    showRevisionCandidate(*binding, revision);
    const auto now = transport_.nowMs();
    binding->expiryTimer = oneShot((revision.expiresAtMs > now ? revision.expiresAtMs - now : 0) * 1000,
        [this, coordinates = revision.coordinates, id = revision.revisionId] {
            auto *current = bindingFor(coordinates.sessionId);
            if (current && current->state.clearRevisionIf(coordinates, id)) clearOwnedPanel(*current);
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
    auto &panel = binding.inputContext->inputPanel();
    // Badi's own panel only: clearOwnedPanel() would also retire the revision.
    if (binding.revisionNoticeTimer) binding.revisionNoticeTimer->setEnabled(false);
    binding.ownedAuxiliary = revisionHelp(revision.mode);
    panel.setAuxUp(::fcitx::Text(binding.ownedAuxiliary));
    panel.setCandidateList(std::move(list));
    binding.ownedCandidates = panel.candidateList();
    binding.inputContext->updateUserInterface(::fcitx::UserInterfaceComponent::InputPanel);
}

// The waiting notice stays until the answer, unlike a timed notice.
void BadiAddon::showRevisionNotice(Binding &binding, std::string_view message) {
    if (!binding.inputContext->hasFocus() || hasForeignImeUi(observePanel(binding))) return;
    auto &panel = binding.inputContext->inputPanel();
    if (binding.ownedCandidates && panel.candidateList().get() == binding.ownedCandidates.get())
        panel.setCandidateList(nullptr);
    binding.ownedCandidates.reset();
    binding.ownedAuxiliary = message;
    panel.setAuxUp(::fcitx::Text(binding.ownedAuxiliary));
    binding.revisionNoticeTimer = oneShot(kRevisionWaitUs, [this, context = binding.inputContext] {
        auto *current = bindingFor(*context);
        if (current && revisionActive(*current) && !current->state.visibleRevision()) cancelRevision(*current);
    });
    binding.inputContext->updateUserInterface(::fcitx::UserInterfaceComponent::InputPanel);
}

void BadiAddon::onRevisionClear(const RevisionClear &clear) {
    auto *binding = bindingFor(clear.coordinates.sessionId);
    if (!binding || !binding->state.clearRevisionIf(clear.coordinates, clear.revisionId)) return;
    debug_.record("revision_clear", binding->state.appId(), clear.reason);
    clearOwnedPanel(*binding);
    if (const auto text = revisionClearNotice(clear.reason)) showNotice(*binding, *text);
}

void BadiAddon::acceptRevision(Binding &binding) {
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
    if (binding.revisionWaiting) {
        binding.revisionWaiting.reset();
        if (binding.revisionTimer) binding.revisionTimer->setEnabled(false);
    }
    if (binding.state.visibleRevision() && !binding.state.revisionAcceptancePending()) {
        if (const auto request = binding.state.requestRevisionDismissal(transport_.nowMs(), observePanel(binding)))
            transport_.requestDismissal(*request);
    } else if (const auto &request = binding.state.revisionRequest(); request && !binding.state.visibleRevision()) {
        transport_.cancelRequest(request->coordinates);
    }
    binding.state.clearRevision();
    clearOwnedPanel(binding);
}

// Keys while a revision waits or shows. True when the key was Badi's.
bool BadiAddon::revisionKey(::fcitx::KeyEvent &event, Binding &binding) {
    const auto key = event.key().normalize();
    if (key.isModifier() || key.check(invokeChord())) return key.isModifier();
    const bool unclaimed = !event.filtered() && !event.accepted();
    if (key.check(::fcitx::Key(FcitxKey_Tab)) && unclaimed) {
        // Tab applies a shown revision; one that is still coming is waited
        // for, rather than letting Tab replace the selection.
        if (binding.state.visibleRevision() && hasOwnedCandidate(observePanel(binding))) acceptRevision(binding);
        event.filterAndAccept();
        return true;
    }
    if (key.check(::fcitx::Key(FcitxKey_Escape))) {
        debug_.record("revision", binding.state.appId(), "revision_dismissed");
        cancelRevision(binding);
        event.filterAndAccept();
        return true;
    }
    // Typing replaces the selection, as it would without Badi.
    debug_.record("revision", binding.state.appId(), "revision_retired_by_key");
    cancelRevision(binding);
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
    // The observer confirms the exact field and selection just before the
    // commit, as for every observed edit.
    if (accessibility_->pending()) {
        if (binding->revisionRetries++ < kMaxRevisionRetries) {
            binding->commitTimer = oneShot(kRevisionRetryUs, [this, prepare] { revisionCommit(prepare); });
            return true;
        }
        refuseCommit(prepare, binding);
        return true;
    }
    const bool sent = accessibility_->request(selectionSnapshotRequest(binding->observedFocus),
        [this, prepare, focus = binding->observedFocus, generation = binding->observationGeneration](const Json &reply) {
            auto *current = bindingFor(prepare.coordinates.sessionId);
            if (!current) return reportStale(prepare);
            const auto &request = current->state.revisionRequest();
            if (current->observationGeneration != generation || !request || !observerAnswered(reply) ||
                !sameSelectedField(focus, reply["focus"]) ||
                !observerAgreesSelection(reply["focus"], request->selection)) {
                debug_.record("commit_blocked", current->state.appId(), "observer_dispatch_mismatch");
                return refuseCommit(prepare, current);
            }
            applyRevision(*current, prepare);
        });
    if (!sent) refuseCommit(prepare, binding);
    return true;
}

void BadiAddon::applyRevision(Binding &binding, const CommitPrepare &prepare) {
    const auto request = binding.state.revisionRequest();
    const auto current = currentSelection(binding);
    if (!request || !binding.inputContext->hasFocus() || !current || current->selected != request->selection.selected ||
        current->before != request->selection.before || current->after != request->selection.after ||
        current->anchor != request->selection.anchor || current->head != request->selection.head) {
        debug_.record("commit_blocked", binding.state.appId(), "selection_changed");
        return refuseCommit(prepare, &binding);
    }
    const auto selection = request->selection;
    const auto dispatch = binding.state.authorizeRevisionCommit(prepare, transport_.nowMs(), observePanel(binding));
    if (!dispatch) return refuseCommit(prepare, &binding);
    clearOwnedPanel(binding);
    // One commit over the selection, as typing over it would (ADR 0012).
    // Fcitx cannot see the client apply it: dispatched, never applied.
    binding.inputContext->commitString(dispatch->text);
    debug_.record("commit", binding.state.appId(), "revision_dispatched_unverified");
    transport_.reportCommit(dispatch->coordinates, dispatch->controlId, dispatch->suggestionId,
                            "dispatched-unverified");
    binding.revisionExpected = std::make_pair(selection, dispatch->text);
    binding.revisionReadBackTimer = oneShot(kRevisionReadBackUs, [this, context = binding.inputContext] {
        auto *current = bindingFor(*context);
        if (!current || !current->revisionExpected) return;
        current->revisionExpected.reset();
        debug_.record("commit", current->state.appId(), "revision_unconfirmed");
        if (current->inputContext->hasFocus()) showNotice(*current, notice::kRevisionUnconfirmed);
    });
    readBackRevision(binding);
}

void BadiAddon::readBackRevision(Binding &binding) {
    if (!binding.revisionExpected) return;
    const auto &surrounding = binding.inputContext->surroundingText();
    if (!surrounding.isValid()) return;
    const auto window = captureContextWindow(surrounding.text(), surrounding.cursor(), surrounding.anchor(), false,
                                             "en");
    const auto &[selection, revised] = *binding.revisionExpected;
    if (!window || !revisionLanded(window->before, window->after, selection, revised)) return;
    binding.revisionExpected.reset();
    if (binding.revisionReadBackTimer) binding.revisionReadBackTimer->setEnabled(false);
    debug_.record("commit", binding.state.appId(), "revision_confirmed");
}

// A field change while a revision waits or shows: the same selection keeps
// it; anything else retires it.
bool BadiAddon::revisionSurvives(Binding &binding) {
    if (!revisionActive(binding)) return false;
    const auto current = currentSelection(binding);
    const auto *selection = binding.revisionWaiting ? &binding.revisionWaiting->first
                                                    : &binding.state.revisionRequest()->selection;
    if (current && current->selected == selection->selected && current->before == selection->before &&
        current->anchor == selection->anchor && current->head == selection->head) {
        return true;
    }
    debug_.record("revision", binding.state.appId(), "revision_retired_by_edit");
    cancelRevision(binding);
    return false;
}

} // namespace badi::fcitx5
