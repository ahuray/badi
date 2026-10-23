#include "addon.h"
#include "fcitx_event.h"

#include <fcitx-utils/capabilityflags.h>
#include <fcitx/candidatelist.h>
#include <fcitx/inputcontext.h>
#include <fcitx/inputmethodentry.h>
#include <fcitx/inputpanel.h>
#include <fcitx/instance.h>
#include <fcitx/text.h>
#include <fcitx/userinterface.h>

#include <algorithm>
#include <cstdint>
#include <functional>
#include <memory>
#include <optional>
#include <utility>

namespace badi::fcitx5 {

bool BadiAddon::open(Binding &binding) {
    if (!binding.state.editingAvailable()) return false;
    if (binding.broker.opened) return true;
    if (!binding.broker.policyAllowed || authorityPaused_) return false;
    const auto target = brokerTarget(binding);
    if (!target || !transport_.openSession(binding.state.coordinates(), *target)) return false;
    binding.broker.open(binding.state.coordinates());
    return true;
}

void BadiAddon::queryPolicy(Binding &binding) {
    if (!binding.state.editingAvailable() || binding.broker.policyKnown) return;
    if (const auto target = brokerTarget(binding)) transport_.queryPolicy(binding.state.coordinates(), *target);
}

// The observed field's inspect target, or the manual path's app and context.
std::optional<Json> BadiAddon::brokerTarget(const Binding &binding) const {
    if (binding.observed()) {
        auto target = binding.observedFocus.value("target", Json());
        target["target_id"] = binding.fieldVisitId;
        return target;
    }
    return desktopApplicationTarget(binding.state.appId(), binding.state.targetId());
}

void BadiAddon::closeBrokerSession(Binding &binding) {
    if (binding.broker.opened) transport_.closeSession(binding.broker.coordinates);
    binding.broker.retire();
}

// Fcitx state of a binding's field.
PanelObservation BadiAddon::observePanel(const Binding &binding) const {
    const auto &panel = binding.inputContext->inputPanel();
    const auto candidates = panel.candidateList();
    const bool anyCandidates = candidates != nullptr && !candidates->empty();
    return PanelObservation{
        .preedit = !panel.preedit().empty(),
        .clientPreedit = !panel.clientPreedit().empty(),
        .candidates = binding.overlayOwned || anyCandidates,
        .candidatesOwnedByBadi = (binding.overlayOwned && !anyCandidates) ||
            (candidates != nullptr && candidates.get() == binding.ownedCandidates.get()),
        .foreignAuxiliary = (!panel.auxUp().empty() && panel.auxUp().toString() != binding.ownedAuxiliary) ||
            !panel.auxDown().empty(),
    };
}

bool BadiAddon::foreignImeActive(const Binding &binding) const {
    return hasForeignImeUi(observePanel(binding)) || instance_->isComposing(binding.inputContext);
}

RequestFacts BadiAddon::requestFacts(const Binding &binding) const {
    return RequestFacts{
        .connected = transport_.ready(),
        .paused = authorityPaused_,
        .policyKnown = binding.broker.policyKnown,
        .policyAllowed = binding.broker.policyAllowed,
        .snoozed = binding.broker.snoozed,
        // A terminal has no surrounding text; its typed line is the context.
        .fresh = binding.state.editPath() == NativeEditPath::TypedLine ? binding.typedLine.tracking()
                                                                       : binding.surroundingFreshness.fresh(),
        .fieldAllowed = binding.state.editPath() == NativeEditPath::TypedLine
            ? !(binding.inputContext->capabilityFlags() & ::fcitx::CapabilityFlag::PasswordOrSensitive)
            : allowsNativeContext(fieldCapabilities(binding)),
        .foreignIme = hasForeignImeUi(observePanel(binding)),
    };
}

// An observed field is judged with fcitx5-qt's withdrawal echo undone
// (SurroundingEcho): its observer still corroborates text and caret before
// display and dispatch. The unobserved manual path keeps Fcitx's flags.
::fcitx::CapabilityFlags BadiAddon::fieldCapabilities(const Binding &binding) const {
    const auto flags = binding.inputContext->capabilityFlags();
    return binding.observed() ? observedFieldCapabilities(flags, binding.surroundingEcho) : flags;
}

// Surrounding text may be read only from an editable, focused, permitted,
// fresh and non-sensitive field that no input method is composing in.
bool BadiAddon::contextReadable(const Binding &binding) const {
    auto &input = *binding.inputContext;
    if (binding.state.editPath() == NativeEditPath::TypedLine) {
        return binding.state.editingAvailable() && input.hasFocus() && binding.state.focused() &&
            binding.broker.policyAllowed && binding.typedLine.tracking() &&
            !(input.capabilityFlags() & ::fcitx::CapabilityFlag::PasswordOrSensitive) &&
            !instance_->isComposing(&input);
    }
    return binding.state.editingAvailable() && input.hasFocus() && binding.state.focused() &&
        binding.broker.policyAllowed && binding.surroundingFreshness.fresh() &&
        allowsNativeContext(fieldCapabilities(binding)) && !instance_->isComposing(&input);
}

bool BadiAddon::safeToObserve(const Binding &binding) const {
    return contextReadable(binding) && !hasForeignImeUi(observePanel(binding));
}

std::optional<ContextWindow> BadiAddon::currentContext(const Binding &binding) const {
    if (!contextReadable(binding)) return std::nullopt;
    auto &input = *binding.inputContext;
    if (binding.state.editPath() == NativeEditPath::TypedLine) {
        const auto *method = instance_->inputMethodEntry(&input);
        if (!method || !validLanguageTag(method->languageCode())) return std::nullopt;
        // The rest of the line after a moved caret is the context's `after`.
        const auto caret = binding.typedLine.caretScalars();
        auto context = captureContextWindow(binding.typedLine.text(), caret, caret, false,
            binding.broker.languageOverride.empty() ? method->languageCode() : binding.broker.languageOverride);
        if (context) {
            context->identityKnown = true;
            context->explicitRequest = false;
        }
        return context;
    }
    const auto &surrounding = input.surroundingText();
    const auto *inputMethod = instance_->inputMethodEntry(&input);
    if (!surrounding.isValid() || !inputMethod || !validLanguageTag(inputMethod->languageCode())) {
        return std::nullopt;
    }
    auto context = captureContextWindow(surrounding.text(), surrounding.cursor(), surrounding.anchor(),
        !!(input.capabilityFlags() & ::fcitx::CapabilityFlag::Multiline),
        binding.broker.languageOverride.empty() ? inputMethod->languageCode() : binding.broker.languageOverride);
    if (!context) return context;
    context->identityKnown = binding.observed();
    context->explicitRequest = context->identityKnown ? binding.observationExplicit : true;
    // Only an observed IME-parity field treats Chromium's paragraph end
    // as end of field; every observer check still compares observedAfter().
    if (context->identityKnown && imeParityApp(binding.state.appId())) normalizeObservedParagraphEnd(*context);
    if (context->identityKnown && binding.state.appId() == "libreoffice")
        restoreObservedTrailingSpaces(*context, binding.observedFocus);
    return context;
}

std::string_view BadiAddon::unavailableContextReason(const Binding &binding) const {
    auto &input = *binding.inputContext;
    if (!binding.state.editingAvailable())
        return editingUnavailableReason(binding.state.appId(), binding.observed());
    if (!input.hasFocus() || !binding.state.focused() || !binding.broker.policyAllowed)
        return "native_authority_changed";
    if (!binding.surroundingFreshness.fresh()) return "native_context_stale";
    if (!allowsNativeContext(fieldCapabilities(binding))) return "native_field_denied";
    if (instance_->isComposing(&input)) return "foreign_ime_active";
    const auto &surrounding = input.surroundingText();
    if (!surrounding.isValid()) return "native_surrounding_invalid";
    const auto *method = instance_->inputMethodEntry(&input);
    if (!method || !validLanguageTag(method->languageCode())) return "native_language_unavailable";
    if (surrounding.cursor() != surrounding.anchor()) return "native_selection_present";
    return "native_context_invalid";
}

// Chord formation may republish an unchanged toolkit buffer; only a
// changed context retires the captured revision and Badi's panel.
void BadiAddon::invalidateChangedContext(Binding &binding) {
    const auto &captured = binding.state.lastContext();
    if (captured && matchesCapturedContext(captured, currentContext(binding))) return;
    binding.state.invalidateContext();
    clearOwnedPanel(binding);
}

// Badi's panel.
void BadiAddon::showNotice(Binding &binding, std::string_view message) {
    if (!binding.inputContext->hasFocus() || hasForeignImeUi(observePanel(binding))) return;
    clearOwnedPanel(binding);
    binding.ownedAuxiliary = message;
    binding.inputContext->inputPanel().setAuxUp(::fcitx::Text(binding.ownedAuxiliary));
    binding.expiryTimer = oneShot(kNoticeLifetimeUs, [this, coordinates = binding.state.coordinates()] {
        auto *current = bindingFor(coordinates.sessionId);
        if (current && current->state.coordinates() == coordinates) clearOwnedPanel(*current);
    });
    binding.inputContext->updateUserInterface(::fcitx::UserInterfaceComponent::InputPanel);
}

void BadiAddon::clearOwnedPanel(Binding &binding) {
    // The display Badi owns, not the suggestion state: a broker clear or the
    // local lease retires the state before the display is cleared here.
    if (binding.overlayOwned || binding.ownedCandidates) {
        debug_.trace("ghost hidden", binding.state.appId(),
            {{"display", binding.overlayOwned ? "overlay" : "panel"}});
    }
    binding.pendingSuggestion.reset();
    binding.heldSuggestion.reset();
    if (binding.holdTimer) binding.holdTimer->setEnabled(false);
    if (binding.overlayOwned) {
        binding.overlayOwned = false;
        hidePreview();
    }
    if (binding.expiryTimer) binding.expiryTimer->setEnabled(false);
    if (binding.commitTimer) binding.commitTimer->setEnabled(false);
    if (binding.inputContext != nullptr) {
        auto &panel = binding.inputContext->inputPanel();
        bool changed = false;
        if (binding.ownedCandidates && panel.candidateList().get() == binding.ownedCandidates.get()) {
            panel.setCandidateList(nullptr);
            changed = true;
        }
        if (!binding.ownedAuxiliary.empty() && panel.auxUp().toString() == binding.ownedAuxiliary) {
            panel.setAuxUp(::fcitx::Text());
            changed = true;
        }
        if (changed) binding.inputContext->updateUserInterface(::fcitx::UserInterfaceComponent::InputPanel);
    }
    binding.ownedCandidates.reset();
    binding.ownedAuxiliary.clear();
    binding.state.clearSuggestion();
}

// Qt's Fcitx module reports the caret relative to its window (RelativeRect);
// other frontends report none (wayland_v2 sends 0,0,0,0).
std::optional<CaretRect> BadiAddon::caretRect(const ::fcitx::InputContext &input) {
    const auto &rect = input.cursorRect();
    if (!input.capabilityFlags().test(::fcitx::CapabilityFlag::RelativeRect) || rect.height() <= 0 ||
        rect.left() < 0 || rect.top() < 0 || rect.width() < 0) return std::nullopt;
    return CaretRect{.x = rect.left(), .y = rect.top(), .width = rect.width(), .height = rect.height(),
                     .scale = input.scaleFactor()};
}

// One observer preview exists. It may be on screen from its request until
// a hide is sent or the observer reports that it did not render.
void BadiAddon::hidePreview() {
    if (!previewMayBeVisible_) return;
    previewMayBeVisible_ = false;
    accessibility_->hide();
}

// Fence callbacks before the active IME can consume the key. A foreign
// composition need not publish surrounding text or reach our post hook.
void BadiAddon::cancelForInput(Binding &binding) {
    dropHeldTab(binding);
    ++binding.observationGeneration;
    if (binding.observeTimer) binding.observeTimer->setEnabled(false);
    binding.state.invalidateContext();
    clearOwnedPanel(binding);
    hidePreview();
}

std::unique_ptr<::fcitx::EventSourceTime> BadiAddon::oneShot(std::uint64_t delayUs, std::function<void()> action) {
    auto timer = instance_->eventLoop().addTimeEvent(CLOCK_MONOTONIC, 0, kTimerAccuracyUs,
        [action = std::move(action)](::fcitx::EventSourceTime *, std::uint64_t) {
            action();
            return true;
        });
    timer->setNextInterval(delayUs);
    timer->setOneShot();
    return timer;
}

} // namespace badi::fcitx5
