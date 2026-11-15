#include "binding.h"

#include "sanitizer.h"
#include "addon_support.h"

#include <fcitx/inputpanel.h>
#include <fcitx/text.h>
#include <fcitx/userinterface.h>

namespace badi::fcitx5 {

namespace {

void rollObservationSession(InputBinding &binding, std::string_view contextId) {
    const auto session = randomUuid();
    const auto salt = randomSalt();
    const bool retired = session && salt && !contextId.empty() &&
        (binding.observation.unnamedIbus
             ? binding.state.retireUnnamedObservation(*session, std::string(contextId), *salt)
             : binding.state.retireObservation(*session, std::string(contextId), *salt));
    if (!retired) binding.state.focusOut();
}

RetireResult clearAllPanels(InputBinding &binding) {
    const bool ghostHidden = binding.panel.ownsGhost();
    const bool overlayHidden = binding.panel.clear(binding.inputContext);
    return RetireResult{.ghostHidden = ghostHidden, .overlayHidden = overlayHidden};
}

RetireResult clearEditingPanels(InputBinding &binding) {
    const bool ghostHidden = binding.panel.ownsGhost(PanelOwner::Suggestion) ||
                             binding.panel.ownsGhost(PanelOwner::Revision);
    const bool suggestionOverlay = binding.panel.clear(binding.inputContext, PanelOwner::Suggestion);
    const bool revisionOverlay = binding.panel.clear(binding.inputContext, PanelOwner::Revision);
    return RetireResult{.ghostHidden = ghostHidden,
                        .overlayHidden = suggestionOverlay || revisionOverlay};
}

} // namespace

bool PanelState::clear(::fcitx::InputContext *inputContext, std::optional<PanelOwner> matching) {
    if (matching && owner != *matching) return false;
    if (inputContext != nullptr) {
        auto &panel = inputContext->inputPanel();
        bool changed = false;
        if (candidates && panel.candidateList().get() == candidates.get()) {
            panel.setCandidateList(nullptr);
            changed = true;
        }
        if (!auxiliary.empty() && panel.auxUp().toString() == auxiliary) {
            panel.setAuxUp(::fcitx::Text());
            changed = true;
        }
        if (changed) inputContext->updateUserInterface(::fcitx::UserInterfaceComponent::InputPanel);
    }
    candidates.reset();
    auxiliary.clear();
    owner = PanelOwner::None;
    return std::exchange(overlayOwned, false);
}

RetireResult retireBinding(InputBinding &binding, RetireReason reason, std::string_view contextId) {
    switch (reason) {
    case RetireReason::InputChanged:
        binding.observation.zenCommit.clear();
        binding.observation.cancelPending();
        binding.typeThrough.dropHeldTab();
        binding.state.invalidateContext();
        binding.suggestion.clearDisplay();
        binding.revision.reset();
        binding.notice.clear();
        return clearAllPanels(binding);

    case RetireReason::PolicyDenied:
        binding.broker.opened = false;
        binding.observation.zenCommit.clear();
        binding.observation.clearWindow();
        binding.state.invalidateContext();
        binding.suggestion.reset();
        binding.typeThrough.reset();
        binding.terminal.clear();
        binding.revision.reset();
        return clearEditingPanels(binding);

    case RetireReason::AuthorityChanged:
        binding.broker.retire();
        binding.observation.zenCommit.clear();
        binding.observation.clearWindow();
        binding.observation.explicitRequest = false;
        binding.surrounding.authorityChanged();
        binding.state.invalidateContext();
        binding.suggestion.reset();
        binding.typeThrough.reset();
        binding.terminal.stopTimer();
        binding.revision.reset();
        binding.notice.clear();
        return clearAllPanels(binding);

    case RetireReason::Disconnected:
        binding.broker.retire();
        binding.observation.zenCommit.clear();
        binding.observation.clearWindow();
        ++binding.observation.generation;
        binding.observation.explicitRequest = false;
        binding.surrounding.transportLost();
        binding.state.invalidateContext();
        binding.suggestion.reset();
        binding.typeThrough.reset();
        binding.terminal.stopTimer();
        binding.revision.reset();
        binding.notice.clear();
        return clearAllPanels(binding);

    case RetireReason::ObservedFieldChanged:
        binding.broker.retire();
        binding.observation.retireField();
        binding.suggestion.clearDisplay();
        binding.suggestion.requestExplicit = false;
        binding.revision.reset();
        binding.notice.clear();
        rollObservationSession(binding, contextId);
        return clearAllPanels(binding);

    case RetireReason::EditingDenied:
        binding.broker.retire();
        binding.observation.stop();
        binding.suggestion.reset();
        binding.typeThrough.reset();
        binding.terminal.clear();
        binding.revision.reset();
        binding.notice.clear();
        if (binding.observation.unnamedIbus) rollObservationSession(binding, contextId);
        else binding.state.denyEditing();
        return clearAllPanels(binding);

    case RetireReason::FocusReset:
        binding.broker.retire();
        binding.observation.stop();
        binding.surrounding.focusOut();
        binding.state.focusOut();
        binding.suggestion.reset();
        binding.typeThrough.reset();
        binding.literalTab.reset();
        binding.terminal.clear();
        binding.revision.reset();
        binding.readBack.clearRevision();
        binding.notice.clear();
        return clearAllPanels(binding);
    }
    return {};
}

RevisionPreKey classifyRevisionPreKey(const ::fcitx::Key &rawEventKey,
                                      bool unclaimed, bool foreignIme) {
    if (foreignIme) return RevisionPreKey::YieldForeignIme;
    const auto key = rawEventKey.normalize();
    if (key.isModifier()) return RevisionPreKey::KeepForModifier;
    if (key.check(invokeChord()) && unclaimed) {
        return (rawEventKey.states() & ::fcitx::KeyState::Repeat)
                   ? RevisionPreKey::FilterInvokeRepeat
                   : RevisionPreKey::PassInvokeToPostInput;
    }
    if (key.check(::fcitx::Key(FcitxKey_Tab)) && unclaimed) return RevisionPreKey::AcceptTab;
    if (key.check(::fcitx::Key(FcitxKey_Escape))) return RevisionPreKey::DismissEscape;
    return RevisionPreKey::RetireByKey;
}

} // namespace badi::fcitx5
