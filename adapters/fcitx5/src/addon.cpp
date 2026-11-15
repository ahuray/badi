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
    // The addon loads with the session. Attaching must not start the model.
    transport_.connect();
}

BadiAddon::~BadiAddon() {
    handlers_.clear();
    for (auto &[_, binding] : bindings_) {
        retire(binding, RetireReason::FocusReset);
        binding.readBack.clearCorrection();
    }
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
        .onAlternativeList = [addon](AlternativeList list) {
            addon->onAlternativeList(std::move(list));
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
    const bool unnamedIbus = unnamedIbusContext(inputContext);
    debug_.refresh();
    debug_.record("focus", appId, unnamedIbus ? "awaiting_observed_field" : focusReason(appId));
    const auto contextId = uuidString(inputContext.uuid());
    const auto sessionId = randomUuid();
    const auto salt = randomSalt();
    if (!sessionId || !salt || (appId.empty() && !unnamedIbus)) return;
    auto &binding = bindings_[contextId];
    // Retires all per-focus parts while keeping an IME-parity correction's
    // read-back alive across a page-split blur/focus (ADR 0011).
    retire(binding, RetireReason::FocusReset);
    binding.inputContext = &inputContext;
    binding.observation.unnamedIbus = unnamedIbus;
    binding.observation.schedule.focusIn(monotonicUs());
    binding.surrounding.focusIn();
    if (unnamedIbus) {
        if (!binding.state.focusInUnnamed(*sessionId, contextId, *salt)) {
            bindings_.erase(contextId);
            return;
        }
        transport_.connect();
        observeLater(binding);
        return;
    }
    const auto path = classifyNativeApp(appId) == NativeAppClass::Terminal ? NativeEditPath::TypedLine
                                                                            : NativeEditPath::Manual;
    if (!binding.state.focusIn(*sessionId, contextId, appId, *salt, NativeEditTarget::DesktopApplication, path)) {
        bindings_.erase(contextId);
        return;
    }
    // Focusing a field is not a request. The model starts on a typed key or
    // an explicit ask, then this connection's retries pick the socket up.
    transport_.connect();
    queryPolicy(binding);
    observeLater(binding);
}

void BadiAddon::focusOut(::fcitx::InputContext &inputContext) {
    auto *binding = bindingFor(inputContext);
    const auto app = binding && !binding->state.appId().empty() ? binding->state.appId()
                                                                : canonicalProgram(inputContext);
    debug_.record("blur", app, "focus_left");
    if (!binding) return;
    if (binding->literalTab.count > 0) debug_.record("decision", app, "literal_tab_dropped");
    retire(*binding, RetireReason::FocusReset);
}

void BadiAddon::destroy(::fcitx::InputContext &inputContext) {
    const auto found = bindings_.find(uuidString(inputContext.uuid()));
    if (found == bindings_.end()) return;
    retire(found->second, RetireReason::FocusReset);
    found->second.readBack.clearCorrection();
    bindings_.erase(found);
}

void BadiAddon::surroundingTextUpdated(::fcitx::InputContext &inputContext) {
    auto *binding = bindingFor(inputContext);
    const auto app = binding && !binding->state.appId().empty() ? binding->state.appId()
                                                                : canonicalProgram(inputContext);
    debug_.record("context", app,
        app.empty() && !(binding && binding->observation.unnamedIbus) ? "unidentified_app" :
        allowsNativeContext(binding ? fieldCapabilities(*binding) : inputContext.capabilityFlags())
            ? "context_received" : "field_denied");
    if (!binding) return;
    binding->surrounding.freshness.surroundingTextUpdated();
    binding->surrounding.echo.surroundingTextUpdated(inputContext.capabilityFlags());
    // The app's own report of its text after an IME-parity correction or a
    // revision.
    readBackCorrection(*binding);
    readBackRevision(*binding);
    const auto digest = surroundingDigest(inputContext.surroundingText());
    const bool digestChanged = digest != binding->surrounding.digest;
    if (digestChanged) {
        binding->surrounding.digest = digest;
        binding->observation.zenCommit.clear();
        // The observer may skip an intermediate edit during a fast
        // delete/retype. This ends only the local until-edit decline;
        // the broker still enforces timed quiet and fresh edit authority.
        binding->suggestion.dismissedContext.reset();
        const auto now = monotonicUs();
        binding->observation.schedule.input(now);
        if (now != 0) {
            const auto pause = binding->typeThrough.active ? InspectionSchedule::kTypeThroughPauseUs
                                                           : InspectionSchedule::kTypingPauseUs;
            binding->suggestion.showAfterUs = now + pause;
        }
        binding->suggestion.clearHeld();
    } else if (binding->observation.zenCommit.active()) {
        const auto raw = rawContext(*binding, binding->observation.focus);
        if (!raw || !binding->observation.zenCommit.matchesFcitx(*raw)) binding->observation.zenCommit.clear();
    }
    // Unavailable apps never hold editing context. Toolkit publication must
    // not erase their explicit, timed unavailable notice. Classify the app
    // itself: observed targets of other classes still require invalidation
    // and fresh field metadata below.
    if (binding->state.focused() && !binding->observationCandidate()) return;
    // The selection a revision waits or shows for keeps it; nothing is
    // inspected meanwhile, since an inspection would decline the selection.
    if (revisionSurvives(*binding)) return;
    const bool unchangedObserved = binding->observed() &&
        matchesCapturedContext(binding->state.lastContext(), currentContext(*binding));
    if (digestChanged) clearNotice(*binding);
    invalidateChangedContext(*binding);
    // A field we already corroborate can start the model on this key.
    // Re-read that field directly; a second inspection would only repeat
    // the round trip. Any other field is inspected on this key too, unless
    // the observer declined it in this visit. A field that reports the
    // user's edits is re-inspected when this key's report arrives, which
    // ends its observation anyway; the typing pause is the fallback. A
    // suggestion still waits out the typing pause.
    if (!unchangedObserved) {
        const bool awaitReport = binding->observed() && binding->observation.schedule.fieldReportsEdits();
        // Foreign preedit must go through inspection, which yields and
        // resumes when the composition clears. A direct read would skip that.
        if (binding->observed() && binding->broker.opened && binding->broker.policyAllowed &&
            safeToObserve(*binding) && !awaitReport) {
            debug_.record("observer", binding->state.appId(), "same_field_snapshot");
            requestObservedContext(*binding, true);
        } else {
            observeLater(*binding, false, false,
                         !awaitReport && (binding->observed() || binding->observation.schedule.inspectsOnInput()));
        }
    }
}

void BadiAddon::capabilityChanged(::fcitx::InputContext &inputContext, ::fcitx::CapabilityFlags before,
                                  ::fcitx::CapabilityFlags after) {
    auto *binding = bindingFor(inputContext);
    const auto app = binding && !binding->state.appId().empty() ? binding->state.appId()
                                                                : canonicalProgram(inputContext);
    const bool activeUnnamed = binding && binding->observation.unnamedIbus;
    // fcitx5-qt's withdrawal right after sending the text changes nothing
    // the field is judged by; see SurroundingEcho.
    if (binding && (!app.empty() || activeUnnamed) && binding->surrounding.echo.capabilityChanged(before, after)) {
        debug_.record("authority", app, "surrounding_echo");
        return;
    }
    debug_.record("authority", app, "capabilities_changed");
    if (!binding || (app.empty() && !activeUnnamed)) return;
    if (after & ::fcitx::CapabilityFlag::PasswordOrSensitive) binding->observation.pendingBrokerStart = false;
    binding->observation.zenCommit.clear();
    if (binding->revisionActive()) cancelRevision(*binding);
    // Capability changes can leave the previous widget's buffer cached.
    binding->surrounding.capabilityChanged();
    clearNotice(*binding);
    invalidateChangedContext(*binding);
    observerInvalidated(Json{{"app_id", app}});
}

void BadiAddon::foreignUiUpdated(::fcitx::InputContext &input) {
    auto *binding = bindingFor(input);
    if (!binding || !input.hasFocus() || !binding->state.focused()) return;
    if (foreignImeActive(*binding)) {
        binding->observation.zenCommit.clear();
        const bool hadRevision = binding->revisionActive();
        if (hadRevision) cancelRevision(*binding);
        if (hadRevision || binding->state.lastContext() || binding->panel.ownsSurface()) {
            binding->observation.waitingForForeignUi = true;
            retire(*binding, RetireReason::InputChanged);
        }
        return;
    }
    if (binding->observation.waitingForForeignUi) {
        binding->observation.waitingForForeignUi = false;
        observeLater(*binding, binding->observation.explicitRequest);
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
