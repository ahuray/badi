#include "addon.h"

#include <fcitx/addonfactory.h>
#include <fcitx/addoninstance.h>
#include <fcitx/addonmanager.h>
#include <fcitx/event.h>
#include <fcitx/inputcontext.h>
#include <fcitx/inputpanel.h>
#include <fcitx/instance.h>
#include <fcitx/userinterface.h>

#include <array>
#include <memory>
#include <string>
#include <utility>

namespace badi::fcitx5 {

BadiAddon::BadiAddon(::fcitx::Instance *instance)
    : instance_(instance),
      transport_(instance->eventLoop(), callbacksFor(this)) {
    accessibility_ = std::make_unique<Accessibility>(instance->eventLoop(),
        [this](const Json &event) { observerInvalidated(event); });
    watchInputContexts();
    transport_.connect();
}

BadiAddon::~BadiAddon() {
    handlers_.clear();
    for (auto &[_, binding] : bindings_) clearOwnedPanel(binding);
    accessibility_.reset();
    transport_.disconnect();
}

WireCallbacks BadiAddon::callbacksFor(BadiAddon *addon) {
    return WireCallbacks{
        .onReady = [addon] { addon->queryFocusedPolicies(); },
        .onAuthority = [addon](const AuthoritySnapshot &snapshot) {
            addon->onAuthority(snapshot);
        },
        .onSuggestion = [addon](Suggestion suggestion) {
            addon->onSuggestion(std::move(suggestion));
        },
        .onClear = [addon](const ClearNotice &notice) {
            addon->onClear(notice);
        },
        .onCommitPrepare = [addon](const CommitPrepare &prepare) {
            addon->onCommitPrepare(prepare);
        },
        .onDisconnected = [addon] { addon->onDisconnected(); },
        .onPolicy = [addon](std::string_view session, bool allowed, std::string reason, std::string language,
                            bool contextWords) {
            addon->onPolicy(session, allowed, reason, std::move(language), contextWords);
        },
        .onRevision = [addon](Revision revision) { addon->onRevision(std::move(revision)); },
        .onRevisionClear = [addon](const RevisionClear &clear) { addon->onRevisionClear(clear); },
        .onRefusal = [addon](const RequestRefusal &refusal) { addon->onRefusal(refusal); },
    };
}

void BadiAddon::watchInputContexts() {
    using ::fcitx::EventType;
    using Handler = void (BadiAddon::*)(::fcitx::InputContext &);
    const std::array<std::pair<EventType, Handler>, 5> inputContextEvents{{
        {EventType::InputContextFocusIn, &BadiAddon::focusIn},
        {EventType::InputContextFocusOut, &BadiAddon::focusOut},
        {EventType::InputContextDestroyed, &BadiAddon::destroy},
        {EventType::InputContextSurroundingTextUpdated, &BadiAddon::surroundingTextUpdated},
        {EventType::InputContextUpdatePreedit, &BadiAddon::foreignUiUpdated},
    }};
    for (const auto &entry : inputContextEvents) {
        watch(entry.first, ::fcitx::EventWatcherPhase::PostInputMethod,
              [this, handler = entry.second](::fcitx::Event &event) {
                  (this->*handler)(*static_cast<::fcitx::InputContextEvent &>(event).inputContext());
              });
    }
    watch(EventType::InputContextCapabilityChanged, ::fcitx::EventWatcherPhase::PostInputMethod,
          [this](::fcitx::Event &event) {
              auto &change = static_cast<::fcitx::CapabilityChangedEvent &>(event);
              capabilityChanged(*change.inputContext(), change.oldFlags(), change.newFlags());
          });
    watch(EventType::InputContextUpdateUI, ::fcitx::EventWatcherPhase::PostInputMethod,
          [this](::fcitx::Event &event) {
              auto &update = static_cast<::fcitx::InputContextUpdateUIEvent &>(event);
              if (update.component() == ::fcitx::UserInterfaceComponent::InputPanel)
                  foreignUiUpdated(*update.inputContext());
          });
    watch(EventType::InputContextKeyEvent, ::fcitx::EventWatcherPhase::PreInputMethod,
          [this](::fcitx::Event &event) { preKeyEvent(static_cast<::fcitx::KeyEvent &>(event)); });
    watch(EventType::InputContextKeyEvent, ::fcitx::EventWatcherPhase::PostInputMethod,
          [this](::fcitx::Event &event) { keyEvent(static_cast<::fcitx::KeyEvent &>(event)); });
}

void BadiAddon::watch(::fcitx::EventType type, ::fcitx::EventWatcherPhase phase, ::fcitx::EventHandler handler) {
    handlers_.push_back(instance_->watchEvent(type, phase, std::move(handler)));
}

BadiAddon::Binding *BadiAddon::bindingFor(::fcitx::InputContext &inputContext) {
    const auto found = bindings_.find(uuidString(inputContext.uuid()));
    if (found == bindings_.end() || found->second.inputContext != &inputContext) return nullptr;
    return &found->second;
}

BadiAddon::Binding *BadiAddon::bindingFor(std::string_view sessionId) {
    for (auto &[_, binding] : bindings_) {
        if (binding.state.coordinates().sessionId == sessionId) return &binding;
    }
    return nullptr;
}

void BadiAddon::focusIn(::fcitx::InputContext &inputContext) {
    const auto appId = canonicalProgram(inputContext);
    debug_.refresh();
    debug_.record("focus", appId, focusReason(appId));
    const auto contextId = uuidString(inputContext.uuid());
    const auto sessionId = randomUuid();
    const auto salt = randomSalt();
    if (!sessionId || !salt || appId.empty()) return;
    auto &binding = bindings_[contextId];
    closeBrokerSession(binding);
    clearOwnedPanel(binding);
    binding.inputContext = &inputContext;
    stopObserving(binding);
    binding.observationExplicit = false;
    binding.requestExplicit = false;
    dropHeldTab(binding);
    binding.dismissedContext.reset();
    clearTypeThrough(binding);
    binding.inspection.input();
    binding.revisionWaiting.reset();
    binding.revisionExpected.reset();
    // An IME-parity correction's read-back (correctionExpected) outlives
    // focus: a page that moves focus between its two edits split it, and
    // its notice must still say so (ADR 0011).
    binding.surroundingFreshness.focusIn();
    binding.surroundingEcho.reset();
    binding.typedLine.clear();
    const auto path = classifyNativeApp(appId) == NativeAppClass::Terminal ? NativeEditPath::TypedLine
                                                                            : NativeEditPath::Manual;
    if (!binding.state.focusIn(*sessionId, contextId, appId, *salt, NativeEditTarget::DesktopApplication, path)) {
        bindings_.erase(contextId);
        return;
    }
    transport_.connect();
    queryPolicy(binding);
    observeLater(binding);
}

void BadiAddon::focusOut(::fcitx::InputContext &inputContext) {
    debug_.record("blur", canonicalProgram(inputContext), "focus_left");
    auto *binding = bindingFor(inputContext);
    if (!binding) return;
    closeBrokerSession(*binding);
    dropHeldTab(*binding);
    dropLiteralTabs(*binding);
    clearOwnedPanel(*binding);
    binding->revisionWaiting.reset();
    binding->state.focusOut();
    binding->surroundingFreshness.focusOut();
    binding->surroundingEcho.reset();
    stopObserving(*binding);
}

void BadiAddon::destroy(::fcitx::InputContext &inputContext) {
    const auto found = bindings_.find(uuidString(inputContext.uuid()));
    if (found == bindings_.end()) return;
    closeBrokerSession(found->second);
    clearOwnedPanel(found->second);
    bindings_.erase(found);
}

void BadiAddon::surroundingTextUpdated(::fcitx::InputContext &inputContext) {
    const auto app = canonicalProgram(inputContext);
    auto *binding = bindingFor(inputContext);
    debug_.record("context", app, app.empty() ? "unidentified_app" :
        allowsNativeContext(binding ? fieldCapabilities(*binding) : inputContext.capabilityFlags())
            ? "context_received" : "field_denied");
    if (!binding) return;
    binding->surroundingFreshness.surroundingTextUpdated();
    binding->surroundingEcho.surroundingTextUpdated(inputContext.capabilityFlags());
    // The app's own report of its text after an IME-parity correction or a
    // revision.
    readBackCorrection(*binding);
    readBackRevision(*binding);
    const auto digest = surroundingDigest(inputContext.surroundingText());
    if (digest != binding->surroundingDigest) {
        binding->surroundingDigest = digest;
        // The observer may skip an intermediate edit during a fast
        // delete/retype. This ends only the local until-edit decline;
        // the broker still enforces timed quiet and fresh edit authority.
        binding->dismissedContext.reset();
        binding->inspection.input();
        const auto now = monotonicUs();
        if (now != 0) {
            const auto pause = binding->typedThrough ? InspectionSchedule::kTypeThroughPauseUs
                                                     : InspectionSchedule::kTypingPauseUs;
            binding->showAfterUs = now + pause;
        }
        binding->heldSuggestion.reset();
        if (binding->holdTimer) binding->holdTimer->setEnabled(false);
    }
    // Unavailable apps never hold editing context. Toolkit publication must
    // not erase their explicit, timed unavailable notice. Classify the app
    // itself: observed targets of other classes still require invalidation
    // and fresh field metadata below.
    if (binding->state.focused() && !nativeObservationAvailable(binding->state.appId())) return;
    // The selection a revision waits or shows for keeps it; nothing is
    // inspected meanwhile, since an inspection would decline the selection.
    if (revisionSurvives(*binding)) return;
    const bool unchangedObserved = binding->observed() &&
        matchesCapturedContext(binding->state.lastContext(), currentContext(*binding));
    invalidateChangedContext(*binding);
    // A field we already corroborate can start the model on this key.
    // Re-read that field directly; a second inspection would only repeat
    // the round trip. A suggestion still waits out the typing pause.
    if (!unchangedObserved) {
        // Foreign preedit must go through inspection, which yields and
        // resumes when the composition clears. A direct read would skip that.
        if (binding->observed() && binding->broker.opened && binding->broker.policyAllowed &&
            safeToObserve(*binding)) {
            debug_.record("observer", binding->state.appId(), "same_field_snapshot");
            requestObservedContext(*binding, true);
        } else {
            observeLater(*binding, false, false, binding->observed());
        }
    }
}

void BadiAddon::capabilityChanged(::fcitx::InputContext &inputContext, ::fcitx::CapabilityFlags before,
                                  ::fcitx::CapabilityFlags after) {
    const auto app = canonicalProgram(inputContext);
    auto *binding = bindingFor(inputContext);
    // fcitx5-qt's withdrawal right after sending the text changes nothing
    // the field is judged by; see SurroundingEcho.
    if (binding && !app.empty() && binding->surroundingEcho.capabilityChanged(before, after)) {
        debug_.record("authority", app, "surrounding_echo");
        return;
    }
    debug_.record("authority", app, "capabilities_changed");
    if (!binding || app.empty()) return;
    // Capability changes can leave the previous widget's buffer cached.
    binding->surroundingFreshness.capabilityChanged();
    invalidateChangedContext(*binding);
    observerInvalidated(Json{{"app_id", app}});
}

void BadiAddon::foreignUiUpdated(::fcitx::InputContext &input) {
    auto *binding = bindingFor(input);
    if (!binding || !input.hasFocus() || !binding->state.focused()) return;
    if (foreignImeActive(*binding)) {
        if (binding->state.lastContext() || binding->overlayOwned || !binding->ownedAuxiliary.empty()) {
            binding->waitingForForeignUi = true;
            cancelForInput(*binding);
        }
        return;
    }
    if (binding->waitingForForeignUi) {
        binding->waitingForForeignUi = false;
        observeLater(*binding, binding->observationExplicit);
    }
}

class BadiModuleFactory final : public ::fcitx::AddonFactory {
public:
    ::fcitx::AddonInstance *create(::fcitx::AddonManager *manager) override {
        return new BadiAddon(manager->instance());
    }
};

} // namespace badi::fcitx5


#ifdef FCITX_ADDON_FACTORY_V2
FCITX_ADDON_FACTORY_V2(badi, badi::fcitx5::BadiModuleFactory)
#else
// Fcitx 5.1.7 (Ubuntu 24.04) has no per-addon factory symbol; its loader
// resolves only the unnamed fcitx_addon_factory_instance.
FCITX_ADDON_FACTORY(badi::fcitx5::BadiModuleFactory)
#endif
