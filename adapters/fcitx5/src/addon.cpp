#include "accessibility.h"
#include "debug.h"
#include "observation.h"
#include "sanitizer.h"
#include "state.h"
#include "transport.h"

#include <fcitx-utils/capabilityflags.h>
#include <fcitx-utils/event.h>
#include <fcitx-utils/eventloopinterface.h>
#include <fcitx-utils/key.h>
#include <fcitx-utils/keysym.h>
#include <fcitx/addonfactory.h>
#include <fcitx/addoninstance.h>
#include <fcitx/addonmanager.h>
#include <fcitx/candidatelist.h>
#include <fcitx/event.h>
#include <fcitx/inputcontext.h>
#include <fcitx/inputmethodentry.h>
#include <fcitx/inputpanel.h>
#include <fcitx/instance.h>
#include <fcitx/text.h>
#include <fcitx/userinterface.h>

#include <sys/random.h>

#include <algorithm>
#include <array>
#include <cstdint>
#include <functional>
#include <iomanip>
#include <memory>
#include <optional>
#include <sstream>
#include <string>
#include <unordered_map>
#include <utility>
#include <vector>

namespace badi::fcitx5 {
namespace {

using Json = nlohmann::json;

constexpr std::uint64_t kNoticeLifetimeUs = 5'000'000;

std::string uuidString(const ::fcitx::ICUUID &uuid) {
    static constexpr std::array<std::size_t, 4> dashes{4, 6, 8, 10};
    std::ostringstream output;
    output << std::hex << std::setfill('0');
    for (std::size_t index = 0; index < uuid.size(); ++index) {
        if (std::find(dashes.begin(), dashes.end(), index) != dashes.end()) {
            output << '-';
        }
        output << std::setw(2) << static_cast<unsigned int>(uuid[index]);
    }
    return output.str();
}

std::optional<std::array<std::uint8_t, 16>> randomBytes() {
    std::array<std::uint8_t, 16> bytes{};
    const auto count = ::getrandom(bytes.data(), bytes.size(), 0);
    if (count != static_cast<ssize_t>(bytes.size())) return std::nullopt;
    return bytes;
}

std::optional<std::string> randomSalt() {
    const auto bytes = randomBytes();
    if (!bytes) return std::nullopt;
    std::ostringstream output;
    output << std::hex << std::setfill('0');
    for (const auto byte : *bytes) output << std::setw(2) << static_cast<unsigned int>(byte);
    return output.str();
}

std::optional<std::string> randomUuid() {
    auto bytes = randomBytes();
    if (!bytes) return std::nullopt;
    auto &uuid = *bytes;
    uuid[6] = static_cast<std::uint8_t>((uuid[6] & 0x0fU) | 0x40U);  // version 4
    uuid[8] = static_cast<std::uint8_t>((uuid[8] & 0x3fU) | 0x80U);  // RFC 4122 variant
    return uuidString(uuid);
}

// The one app identity used for policy, sessions, debug and the observer.
// Empty when program() is not an app identifier.
std::string canonicalProgram(const ::fcitx::InputContext &inputContext) {
    return canonicalAppId(inputContext.program()).value_or("");
}

const ::fcitx::Key &invokeChord() {
    static const ::fcitx::Key key("Control+Shift+space");
    return key;
}

const ::fcitx::Key &acceptChord() {
    static const ::fcitx::Key key("Control+Shift+Y");
    return key;
}

std::size_t surroundingDigest(const ::fcitx::SurroundingText &surrounding) {
    return std::hash<std::string_view>{}(surrounding.text()) ^
           (static_cast<std::size_t>(surrounding.cursor()) * 0x9e3779b97f4a7c15ULL) ^ surrounding.anchor();
}

} // namespace

class BadiCandidate final : public ::fcitx::CandidateWord {
public:
    BadiCandidate(std::string text,
                  std::function<void(::fcitx::InputContext *)> selected)
        : CandidateWord(::fcitx::Text(std::move(text))),
          selected_(std::move(selected)) {}

    void select(::fcitx::InputContext *inputContext) const override {
        selected_(inputContext);
    }

private:
    std::function<void(::fcitx::InputContext *)> selected_;
};

class BadiAddon final : public ::fcitx::AddonInstance {
public:
    explicit BadiAddon(::fcitx::Instance *instance)
        : instance_(instance),
          transport_(instance->eventLoop(), callbacksFor(this)) {
        accessibility_ = std::make_unique<Accessibility>(instance->eventLoop(),
            [this](const Json &event) { observerInvalidated(event); });
        watchInputContexts();
        transport_.connect();
    }

    ~BadiAddon() override {
        handlers_.clear();
        for (auto &[_, binding] : bindings_) clearOwnedPanel(binding);
        accessibility_.reset();
        transport_.disconnect();
    }

private:
    struct Binding {
        ::fcitx::InputContext *inputContext = nullptr;
        SessionState state;
        BrokerSession broker;
        SurroundingFreshness surroundingFreshness;
        std::size_t surroundingDigest = 0;
        // Badi's own panel content, and the timer that retires it.
        std::shared_ptr<::fcitx::CandidateList> ownedCandidates;
        std::string ownedAuxiliary;
        bool overlayOwned = false;
        std::unique_ptr<::fcitx::EventSourceTime> expiryTimer;
        // A suggestion waiting for the observer before display.
        std::optional<std::string> pendingSuggestion;
        // The observed field. Replies of an older generation are ignored.
        Json observedFocus;
        std::uint64_t observationGeneration = 0;
        bool observationExplicit = false;
        bool waitingForForeignUi = false;
        InspectionSchedule inspection;
        std::unique_ptr<::fcitx::EventSourceTime> observeTimer;
        // Escape declined this context; it is not requested automatically again.
        std::optional<ContextWindow> dismissedContext;

        [[nodiscard]] bool observed() const { return !observedFocus.is_null(); }
    };

    static WireCallbacks callbacksFor(BadiAddon *addon) {
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
            .onPolicy = [addon](std::string_view session, bool allowed) {
                addon->onPolicy(session, allowed);
            },
        };
    }

    void watchInputContexts() {
        using ::fcitx::EventType;
        using Handler = void (BadiAddon::*)(::fcitx::InputContext &);
        const std::array<std::pair<EventType, Handler>, 6> inputContextEvents{{
            {EventType::InputContextFocusIn, &BadiAddon::focusIn},
            {EventType::InputContextFocusOut, &BadiAddon::focusOut},
            {EventType::InputContextDestroyed, &BadiAddon::destroy},
            {EventType::InputContextSurroundingTextUpdated, &BadiAddon::surroundingTextUpdated},
            {EventType::InputContextCapabilityChanged, &BadiAddon::capabilityChanged},
            {EventType::InputContextUpdatePreedit, &BadiAddon::foreignUiUpdated},
        }};
        for (const auto &entry : inputContextEvents) {
            watch(entry.first, ::fcitx::EventWatcherPhase::PostInputMethod,
                  [this, handler = entry.second](::fcitx::Event &event) {
                      (this->*handler)(*static_cast<::fcitx::InputContextEvent &>(event).inputContext());
                  });
        }
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

    void watch(::fcitx::EventType type, ::fcitx::EventWatcherPhase phase, ::fcitx::EventHandler handler) {
        handlers_.push_back(instance_->watchEvent(type, phase, std::move(handler)));
    }

    Binding *bindingFor(::fcitx::InputContext &inputContext) {
        const auto found = bindings_.find(uuidString(inputContext.uuid()));
        if (found == bindings_.end() || found->second.inputContext != &inputContext) return nullptr;
        return &found->second;
    }

    Binding *bindingFor(std::string_view sessionId) {
        for (auto &[_, binding] : bindings_) {
            if (binding.state.coordinates().sessionId == sessionId) return &binding;
        }
        return nullptr;
    }

    // Input context lifecycle.

    void focusIn(::fcitx::InputContext &inputContext) {
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
        binding.dismissedContext.reset();
        binding.inspection.input();
        binding.surroundingFreshness.focusIn();
        if (!binding.state.focusIn(*sessionId, contextId, appId, *salt)) {
            bindings_.erase(contextId);
            return;
        }
        transport_.connect();
        queryPolicy(binding);
        observeLater(binding);
    }

    void focusOut(::fcitx::InputContext &inputContext) {
        debug_.record("blur", canonicalProgram(inputContext), "focus_left");
        auto *binding = bindingFor(inputContext);
        if (!binding) return;
        closeBrokerSession(*binding);
        clearOwnedPanel(*binding);
        binding->state.focusOut();
        binding->surroundingFreshness.focusOut();
        stopObserving(*binding);
    }

    void destroy(::fcitx::InputContext &inputContext) {
        const auto found = bindings_.find(uuidString(inputContext.uuid()));
        if (found == bindings_.end()) return;
        closeBrokerSession(found->second);
        clearOwnedPanel(found->second);
        bindings_.erase(found);
    }

    void surroundingTextUpdated(::fcitx::InputContext &inputContext) {
        const auto app = canonicalProgram(inputContext);
        debug_.record("context", app, app.empty() ? "unidentified_app" :
            allowsNativeContext(inputContext.capabilityFlags()) ? "context_received" : "field_denied");
        auto *binding = bindingFor(inputContext);
        if (!binding) return;
        binding->surroundingFreshness.surroundingTextUpdated();
        const auto digest = surroundingDigest(inputContext.surroundingText());
        if (digest != binding->surroundingDigest) {
            binding->surroundingDigest = digest;
            binding->inspection.input();
        }
        // Unavailable apps never hold editing context. Toolkit publication must
        // not erase their explicit, timed unavailable notice. Classify the app
        // itself: observed targets of other classes still require invalidation
        // and fresh field metadata below.
        if (binding->state.focused() && !nativeObservationAvailable(binding->state.appId())) return;
        const bool unchangedObserved = binding->observed() &&
            matchesCapturedContext(binding->state.lastContext(), currentContext(*binding));
        invalidateChangedContext(*binding);
        if (!unchangedObserved) observeLater(*binding);
    }

    void capabilityChanged(::fcitx::InputContext &inputContext) {
        const auto app = canonicalProgram(inputContext);
        debug_.record("authority", app, "capabilities_changed");
        auto *binding = bindingFor(inputContext);
        if (!binding || app.empty()) return;
        // Capability changes can leave the previous widget's buffer cached.
        binding->surroundingFreshness.capabilityChanged();
        invalidateChangedContext(*binding);
        observerInvalidated(Json{{"app_id", app}});
    }

    void foreignUiUpdated(::fcitx::InputContext &input) {
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

    // Keys.

    void preKeyEvent(::fcitx::KeyEvent &event) {
        if (event.isRelease() || event.isVirtual()) return;
        const auto key = event.key().normalize();
        const bool tab = key.check(::fcitx::Key(FcitxKey_Tab));
        const auto app = canonicalProgram(*event.inputContext());
        debug_.record(tab ? "tab" : "input", app, app.empty() ? "unidentified_app" : "input_received");
        auto *binding = bindingFor(*event.inputContext());
        if (!binding || !binding->state.focused()) return;
        if (!key.isModifier()) binding->inspection.input();
        if (!binding->state.editingAvailable()) {
            debug_.record("request_blocked", app,
                editingUnavailableReason(binding->state.appId(), binding->observed()));
        }
        const auto panel = observePanel(*binding);
        const PreKey pressed{
            .modifier = key.isModifier(),
            .repeat = !!(event.key().states() & ::fcitx::KeyState::Repeat),
            .tab = tab && !event.filtered() && !event.accepted(),
            .escape = key.check(::fcitx::Key(FcitxKey_Escape)),
            .chord = key.check(invokeChord()) || key.check(acceptChord()),
        };
        switch (decidePreKey(pressed, binding->state.editingAvailable(), !binding->ownedAuxiliary.empty(),
                             binding->state.suggestionVisible(), panel)) {
        case PreKeyAction::PassThrough:
            return;
        case PreKeyAction::CancelDeclining:
            declineContext(*binding);
            [[fallthrough]];
        case PreKeyAction::Cancel:
            cancelForInput(*binding);
            return;
        case PreKeyAction::Tab:
            tabKey(event, *binding, panel);
            return;
        case PreKeyAction::CloseNotice:
            binding->state.invalidateContext();
            clearOwnedPanel(*binding);
            break;
        case PreKeyAction::Dismiss:
            dismissSuggestion(*binding, panel);
            break;
        }
        event.filterAndAccept();
    }

    // Plain Tab is claimed before Fcitx's candidate navigation only to accept
    // or to request on the manual path; otherwise it stays the application's.
    void tabKey(::fcitx::KeyEvent &event, Binding &binding, const PanelObservation &panel) {
        // An explicit key can renew an exhausted recovery budget without
        // reading the field before the new policy response arrives.
        if (!transport_.ready()) transport_.connect();
        const auto facts = requestFacts(binding);
        const auto context = facts.foreignIme ? std::nullopt : currentContext(binding);
        debug_.record("decision", binding.state.appId(), tabDecisionReason(facts, context),
                      context ? context->before.size() : 0);
        switch (decideTabAction(tabEligibleContext(context), binding.state.suggestionVisible(), panel,
                                binding.state.editPath())) {
        case LocalAction::Accept:
            requestAcceptance(binding);
            break;
        case LocalAction::Invoke:
            invoke(binding);
            break;
        case LocalAction::PassThrough:
        case LocalAction::Dismiss:
            cancelForInput(binding);
            return;
        }
        event.filterAndAccept();
    }

    // After the input method: Badi's chords, and Escape on its candidate.
    void keyEvent(::fcitx::KeyEvent &event) {
        if (event.isRelease() || !!(event.key().states() & ::fcitx::KeyState::Repeat)) return;
        auto *binding = bindingFor(*event.inputContext());
        if (!binding || !binding->state.focused()) return;
        if (event.isVirtual() || event.filtered() || event.accepted()) return;
        const auto panel = observePanel(*binding);
        const auto key = event.key().normalize();
        const bool invokeKey = key.check(invokeChord());
        if (!binding->state.editingAvailable()) {
            // Without an edit path only the invoke chord acts: it inspects or explains.
            if (invokeKey && !hasForeignImeUi(panel)) {
                invoke(*binding);
                event.filterAndAccept();
            }
            return;
        }
        const bool liveOwnedCandidate = binding->state.suggestionVisible() && panel.candidatesOwnedByBadi;
        switch (decideLocalAction(invokeKey, key.check(acceptChord()), key.check(::fcitx::Key(FcitxKey_Escape)),
                                  liveOwnedCandidate, panel)) {
        case LocalAction::Invoke:
            invoke(*binding);
            break;
        case LocalAction::Accept:
            requestAcceptance(*binding);
            break;
        case LocalAction::Dismiss:
            dismissSuggestion(*binding, panel);
            break;
        case LocalAction::PassThrough:
            if (liveOwnedCandidate && !key.isModifier()) {
                binding->state.invalidateContext();
                clearOwnedPanel(*binding);
            }
            return;
        }
        event.filterAndAccept();
    }

    // Explicit requests.

    void invoke(Binding &binding) {
        switch (routeInvoke(binding.state.appId(), binding.state.editingAvailable(), binding.observed())) {
        case InvokeRoute::InspectField:
            return inspectOnRequest(binding);
        case InvokeRoute::Unavailable:
            return showNotice(binding, notice::kUnavailableApp);
        case InvokeRoute::ObservedField:
            return observeLater(binding, true);
        case InvokeRoute::Manual:
            return requestManually(binding);
        }
    }

    // An explicit IME-parity request inspects the field; it never becomes an
    // unknown-identity manual request.
    void inspectOnRequest(Binding &binding) {
        debug_.record("observer", binding.state.appId(), "explicit_inspect_requested");
        if (const auto blocked = inspectionNotice(requestFacts(binding))) return showNotice(binding, *blocked);
        clearOwnedPanel(binding);
        transport_.connect();
        observeLater(binding, true);
    }

    void requestManually(Binding &binding) {
        clearOwnedPanel(binding);
        transport_.connect();
        queryPolicy(binding);
        if (const auto waiting = manualRequestNotice(requestFacts(binding))) return showNotice(binding, *waiting);
        const auto context = currentContext(binding);
        if (!context) {
            binding.state.invalidateContext();
            debug_.record("request_blocked", binding.state.appId(), "context_unavailable");
            return showNotice(binding, notice::kFieldUnreadable);
        }
        const auto update = binding.state.updateContext(*context);
        if (!update) return;
        if (!open(binding)) return showNotice(binding, authorityPaused_ ? notice::kPaused : notice::kModelNotConnected);
        if (!transport_.publishContext(*update)) return showNotice(binding, notice::kRequestFailed);
        debug_.record("request", binding.state.appId(), "sent_to_model", context->before.size());
        binding.broker.coordinates = update->coordinates;
        showNotice(binding, notice::kThinking);
    }

    void requestAcceptance(Binding &binding) {
        const auto request = binding.state.editingAvailable()
            ? binding.state.requestAcceptance(transport_.nowMs(), observePanel(binding))
            : std::nullopt;
        if (!request || !transport_.requestAcceptance(*request)) clearOwnedPanel(binding);
    }

    void dismissSuggestion(Binding &binding, const PanelObservation &panel) {
        if (const auto request = binding.state.requestDismissal(transport_.nowMs(), panel)) {
            declineContext(binding);
            transport_.requestDismissal(*request);
        }
        clearOwnedPanel(binding);
    }

    void declineContext(Binding &binding) {
        if (const auto &captured = binding.state.lastContext()) binding.dismissedContext = captured->context;
    }

    // The accessibility observer.

    void observeLater(Binding &binding, bool explicitRequest = false, bool retry = false) {
        if (!binding.inputContext->hasFocus() || !binding.state.focused() ||
            !nativeObservationAvailable(binding.state.appId())) return;
        if (!retry) binding.inspection.resetBusyRetries();
        binding.observationExplicit = explicitRequest;
        if (explicitRequest) binding.dismissedContext.reset();
        if (!binding.observeTimer) {
            binding.observeTimer = instance_->eventLoop().addTimeEvent(CLOCK_MONOTONIC, 0, 0,
                [this, contextId = uuidString(binding.inputContext->uuid())](::fcitx::EventSourceTime *, std::uint64_t) {
                    const auto found = bindings_.find(contextId);
                    if (found != bindings_.end() && found->second.inputContext->hasFocus()) inspectField(found->second);
                    return true;
                });
        }
        binding.observeTimer->setNextInterval(binding.inspection.delayUs(explicitRequest && !retry));
        binding.observeTimer->setOneShot();
    }

    void inspectField(Binding &binding) {
        if (!nativeObservationAvailable(binding.state.appId()) ||
            !allowsNativeContext(binding.inputContext->capabilityFlags()) ||
            !binding.surroundingFreshness.fresh()) return;
        if (accessibility_->pending()) {
            // Bounded, so an absent service or a denied field never loops the timer.
            if (binding.inspection.retryWhileBusy()) observeLater(binding, binding.observationExplicit, true);
            else observerUnavailable(binding);
            return;
        }
        if (foreignImeActive(binding)) {
            binding.waitingForForeignUi = true;
            return;
        }
        binding.waitingForForeignUi = false;
        binding.inspection.inspecting();
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

    void inspected(Binding &binding, const Json &reply) {
        if (!observerAnswered(reply)) return observerUnavailable(binding, reply.value("error", Json()));
        const auto &focus = reply["focus"];
        const auto editTarget = inspectedEditTarget(focus, binding.state.appId());
        if (!editTarget) return denyEditing(binding);
        const bool unchanged = !binding.observationExplicit && binding.observed() &&
            matchesObservedFocus(binding.observedFocus, focus) &&
            matchesCapturedContext(binding.state.lastContext(), currentContext(binding));
        if (!unchanged) bindObservedField(binding, focus, *editTarget);
    }

    // A newly observed field gets its own session, and policy for its exact target.
    void bindObservedField(Binding &binding, const Json &focus, NativeEditTarget editTarget) {
        const auto session = randomUuid();
        const auto salt = randomSalt();
        if (!session || !salt) return;
        closeBrokerSession(binding);
        clearOwnedPanel(binding);
        const auto appId = binding.state.appId();
        if (!binding.state.focusIn(*session, focus["target"]["target_id"].get<std::string>(), appId, *salt,
                                   editTarget, NativeEditPath::Observed)) return;
        binding.observedFocus = focus;
        if (!binding.state.editingAvailable()) {
            debug_.record("request_blocked", appId, editingUnavailableReason(appId, binding.observed()));
            return;
        }
        queryPolicy(binding);
    }

    void denyEditing(Binding &binding) {
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
    void observerUnavailable(Binding &binding, const Json &error = nullptr) {
        if (!imeParityApp(binding.state.appId())) return;
        const bool fieldDenied = observerDeniedField(error);
        debug_.record("request_blocked", binding.state.appId(),
            fieldDenied ? "ime_parity_field_denied" : "ime_parity_observer_unavailable");
        if (binding.observationExplicit)
            showNotice(binding, fieldDenied ? notice::kFieldUnreadable : notice::kObserverUnavailable);
    }

    void observerInvalidated(const Json &event) {
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

    void retireObservedField(Binding &binding) {
        closeBrokerSession(binding);
        binding.state.retireObservation();
        binding.observedFocus = nullptr;
        clearOwnedPanel(binding);
    }

    void stopObserving(Binding &binding) {
        ++binding.observationGeneration;
        binding.observedFocus = nullptr;
        binding.waitingForForeignUi = false;
        if (binding.observeTimer) binding.observeTimer->setEnabled(false);
    }

    void requestObservedContext(Binding &binding) {
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
        const auto session = binding.state.coordinates().sessionId;
        const auto generation = binding.observationGeneration;
        const bool sent = accessibility_->request(snapshotRequest(binding.observedFocus),
            [this, session, generation, focus = binding.observedFocus](const Json &reply) {
                auto *current = bindingFor(session);
                if (!current || !current->inputContext->hasFocus() || current->observationGeneration != generation ||
                    !current->broker.policyAllowed) {
                    debug_.record("request_blocked", "unidentified", "observer_authority_changed");
                    return;
                }
                publishObservedContext(*current, focus, reply);
            });
        if (!sent) {
            debug_.record("request_blocked", app, "observer_busy");
            if (binding.observationExplicit) observerUnavailable(binding);
        }
    }

    void publishObservedContext(Binding &binding, const Json &focus, const Json &reply) {
        const auto &app = binding.state.appId();
        if (!observerAnswered(reply)) {
            return debug_.record("request_blocked", app, snapshotFailureReason(reply.value("error", Json())));
        }
        const auto &observed = reply["focus"];
        if (!matchesObservedFocus(focus, observed, true)) {
            return debug_.record("request_blocked", app, "observer_context_mismatch");
        }
        const auto context = currentContext(binding);
        if (!context) return debug_.record("request_blocked", app, unavailableContextReason(binding));
        if (const auto blocked = observedRequestBlocked(observed, *context, hasForeignImeUi(observePanel(binding)))) {
            return debug_.record("request_blocked", app, *blocked, context->before.size());
        }
        binding.observedFocus = observed;
        if (binding.dismissedContext == *context) return;
        binding.dismissedContext.reset();
        const auto update = binding.state.updateContext(*context);
        if (update && open(binding) && transport_.publishContext(*update)) {
            binding.broker.coordinates = update->coordinates;
            debug_.record("request", app, context->paragraphEndAfter ?
                "observed_field_request_paragraph_end" : "observed_field_request", context->before.size());
        }
    }

    // Broker events.

    void queryFocusedPolicies() {
        for (auto &[_, binding] : bindings_) {
            if (binding.state.focused()) queryPolicy(binding);
        }
    }

    void onAuthority(const AuthoritySnapshot &snapshot) {
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
            binding.state.invalidateContext();
            binding.surroundingFreshness.capabilityChanged();
            clearOwnedPanel(binding);
        }
        if (!authorityPaused_) queryFocusedPolicies();
    }

    void onPolicy(std::string_view session, bool allowed) {
        auto *binding = bindingFor(session);
        if (!binding || !binding->state.focused() || !binding->state.editingAvailable()) return;
        binding->broker.answer(allowed);
        debug_.record("policy", binding->state.appId(), allowed ? "app_allowed" : "app_disabled");
        if (!allowed) return;
        if (binding->observed()) requestObservedContext(*binding);
        open(*binding);
    }

    void onSuggestion(Suggestion suggestion) {
        auto *binding = bindingFor(suggestion.coordinates.sessionId);
        if (!binding || !safeToObserve(*binding)) return;
        // SessionState keeps IME-parity apps off this unobserved display path.
        if (!binding->observed()) return displaySuggestion(std::move(suggestion));
        binding->pendingSuggestion = suggestion.suggestionId;
        const auto generation = binding->observationGeneration;
        const bool sent = accessibility_->request(snapshotRequest(binding->observedFocus),
            [this, suggestion, focus = binding->observedFocus, generation](const Json &reply) {
                auto *current = bindingFor(suggestion.coordinates.sessionId);
                if (current && safeToObserve(*current) && current->observationGeneration == generation &&
                    current->state.coordinates() == suggestion.coordinates)
                    previewIfCorroborated(*current, suggestion, focus, reply);
            });
        if (!sent) debug_.record("suggestion_blocked", binding->state.appId(), "observer_display_unavailable");
    }

    void previewIfCorroborated(Binding &binding, const Suggestion &suggestion, const Json &focus, const Json &reply) {
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
    void showObservedPreview(Binding &binding, const Suggestion &suggestion) {
        const auto now = transport_.nowMs();
        if (!safeToObserve(binding) || suggestion.expiresAtMs <= now) return;
        const auto &focus = binding.observedFocus;
        const bool sent = accessibility_->request(previewRequest(focus, suggestion.text, suggestion.expiresAtMs - now),
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

    void previewed(Binding &binding, const Suggestion &suggestion, const Json &focus, const Json &reply,
                   bool verified, bool rendered) {
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

    void displaySuggestion(Suggestion suggestion, bool overlay = false) {
        auto *binding = bindingFor(suggestion.coordinates.sessionId);
        if (!binding || !binding->inputContext->hasFocus() || hasForeignImeUi(observePanel(*binding)) ||
            !matchesCapturedContext(binding->state.lastContext(), currentContext(*binding)) ||
            !binding->state.showSuggestion(suggestion, transport_.nowMs())) {
            if (overlay) hidePreview();
            return;
        }
        binding->overlayOwned = overlay;
        binding->pendingSuggestion.reset();
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
    }

    // Badi's own Fcitx panel: the suggestion as its single candidate, with the keys.
    void showCandidate(Binding &binding, const Suggestion &suggestion) {
        auto list = std::make_unique<::fcitx::CommonCandidateList>();
        list->setPageSize(1);
        list->setLayoutHint(::fcitx::CandidateLayoutHint::Horizontal);
        list->append(std::make_unique<BadiCandidate>(suggestion.text,
            [this, sessionId = suggestion.coordinates.sessionId](::fcitx::InputContext *selected) {
                auto *current = bindingFor(sessionId);
                if (current && current->inputContext == selected && selected->hasFocus()) requestAcceptance(*current);
            }));
        auto &panel = binding.inputContext->inputPanel();
        binding.ownedAuxiliary = notice::kSuggestionKeys;
        panel.setAuxUp(::fcitx::Text(binding.ownedAuxiliary));
        panel.setCandidateList(std::move(list));
        binding.ownedCandidates = panel.candidateList();
    }

    void onClear(const ClearNotice &clear) {
        auto *binding = bindingFor(clear.coordinates.sessionId);
        if (!binding) return;
        if (!binding->state.editingAvailable()) return clearOwnedPanel(*binding);
        debug_.record("clear", binding->state.appId(), clear.reason);
        const bool current = binding->state.coordinates() == clear.coordinates;
        if (current && binding->pendingSuggestion &&
            (!clear.suggestionId || *clear.suggestionId == *binding->pendingSuggestion)) {
            // A broker cancellation can arrive before the preview RPC returns.
            // Retire that asynchronous candidate even though it is not visible.
            return cancelForInput(*binding);
        }
        if (current && !clear.suggestionId && (!binding->observed() || binding->observationExplicit)) {
            if (const auto text = clearNoticeText(clear.reason)) return showNotice(*binding, *text);
        }
        if (binding->state.clearSuggestionIf(clear.coordinates, clear.suggestionId)) clearOwnedPanel(*binding);
    }

    void onCommitPrepare(const CommitPrepare &prepare) {
        auto *binding = bindingFor(prepare.coordinates.sessionId);
        if (!binding || !binding->observed()) {
            // SessionState keeps IME-parity apps off the unobserved path; a
            // grant that reaches one anyway is refused.
            if (binding && (!binding->state.editingAvailable() || imeParityApp(binding->state.appId())))
                return refuseCommit(prepare, binding);
            return applyCommitPrepare(prepare);
        }
        if (!binding->state.editingAvailable() || !safeToObserve(*binding)) return refuseCommit(prepare, binding);
        corroborateCommit(*binding, prepare);
    }

    // Re-snapshot immediately before commitString: the observer must still
    // agree with Fcitx's live text and caret for this field.
    void corroborateCommit(Binding &binding, const CommitPrepare &prepare) {
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

    void applyCommitPrepare(const CommitPrepare &prepare) {
        auto *binding = bindingFor(prepare.coordinates.sessionId);
        if (!binding || !binding->state.editingAvailable() || !binding->inputContext->hasFocus() ||
            !hasOwnedCandidate(observePanel(*binding))) return refuseCommit(prepare, binding);
        if (!matchesCapturedContext(binding->state.lastContext(), currentContext(*binding))) {
            binding->state.invalidateContext();
            return refuseCommit(prepare, binding);
        }
        const auto dispatch = binding->state.authorizeCommit(prepare, transport_.nowMs(), observePanel(*binding));
        if (!dispatch) return reportStale(prepare);
        clearOwnedPanel(*binding);
        // One append, like typed text. Fcitx cannot see the client apply it,
        // so the result is dispatched-unverified, never applied.
        binding->inputContext->commitString(dispatch->text);
        debug_.record("commit", binding->state.appId(), "dispatched_unverified");
        transport_.reportCommit(dispatch->coordinates, dispatch->controlId, dispatch->suggestionId,
                                "dispatched-unverified");
    }

    void refuseCommit(const CommitPrepare &prepare, Binding *binding) {
        if (binding) clearOwnedPanel(*binding);
        reportStale(prepare);
    }

    void reportStale(const CommitPrepare &prepare) {
        transport_.reportCommit(prepare.coordinates, prepare.controlId, prepare.suggestionId, "stale");
    }

    void onDisconnected() {
        authorityPaused_ = true;
        for (auto &[_, binding] : bindings_) {
            // Sessions, revisions, candidates, commit grants and in-flight
            // observer replies end with the connection. Fcitx's surrounding
            // text does not, so an explicit request after reconnect may read
            // it once fresh policy reopens the session.
            binding.broker.retire();
            ++binding.observationGeneration;
            binding.observationExplicit = false;
            binding.state.invalidateContext();
            binding.surroundingFreshness.transportLost();
            clearOwnedPanel(binding);
        }
    }

    // The broker session.

    bool open(Binding &binding) {
        if (!binding.state.editingAvailable()) return false;
        if (binding.broker.opened) return true;
        if (!binding.broker.policyAllowed || authorityPaused_) return false;
        const auto target = brokerTarget(binding);
        if (!target || !transport_.openSession(binding.state.coordinates(), *target)) return false;
        binding.broker.open(binding.state.coordinates());
        return true;
    }

    void queryPolicy(Binding &binding) {
        if (!binding.state.editingAvailable() || binding.broker.policyKnown) return;
        if (const auto target = brokerTarget(binding)) transport_.queryPolicy(binding.state.coordinates(), *target);
    }

    // The observed field's inspect target, or the manual path's app and context.
    std::optional<Json> brokerTarget(const Binding &binding) const {
        if (binding.observed()) return binding.observedFocus.value("target", Json());
        return desktopApplicationTarget(binding.state.appId(), binding.state.targetId());
    }

    void closeBrokerSession(Binding &binding) {
        if (binding.broker.opened) transport_.closeSession(binding.broker.coordinates);
        binding.broker.retire();
    }

    // Fcitx state of a binding's field.

    PanelObservation observePanel(const Binding &binding) const {
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

    bool foreignImeActive(const Binding &binding) const {
        return hasForeignImeUi(observePanel(binding)) || instance_->isComposing(binding.inputContext);
    }

    RequestFacts requestFacts(const Binding &binding) const {
        return RequestFacts{
            .connected = transport_.ready(),
            .paused = authorityPaused_,
            .policyKnown = binding.broker.policyKnown,
            .policyAllowed = binding.broker.policyAllowed,
            .fresh = binding.surroundingFreshness.fresh(),
            .fieldAllowed = allowsNativeContext(binding.inputContext->capabilityFlags()),
            .foreignIme = hasForeignImeUi(observePanel(binding)),
        };
    }

    // Surrounding text may be read only from an editable, focused, permitted,
    // fresh and non-sensitive field that no input method is composing in.
    bool contextReadable(const Binding &binding) const {
        auto &input = *binding.inputContext;
        return binding.state.editingAvailable() && input.hasFocus() && binding.state.focused() &&
            binding.broker.policyAllowed && binding.surroundingFreshness.fresh() &&
            allowsNativeContext(input.capabilityFlags()) && !instance_->isComposing(&input);
    }

    bool safeToObserve(const Binding &binding) const {
        return contextReadable(binding) && !hasForeignImeUi(observePanel(binding));
    }

    std::optional<ContextWindow> currentContext(const Binding &binding) const {
        if (!contextReadable(binding)) return std::nullopt;
        auto &input = *binding.inputContext;
        const auto &surrounding = input.surroundingText();
        const auto *inputMethod = instance_->inputMethodEntry(&input);
        if (!surrounding.isValid() || !inputMethod || !validLanguageTag(inputMethod->languageCode())) {
            return std::nullopt;
        }
        auto context = captureContextWindow(surrounding.text(), surrounding.cursor(), surrounding.anchor(),
            !!(input.capabilityFlags() & ::fcitx::CapabilityFlag::Multiline), inputMethod->languageCode());
        if (!context) return context;
        context->identityKnown = binding.observed();
        context->explicitRequest = context->identityKnown ? binding.observationExplicit : true;
        // Only an observed IME-parity field treats Chromium's paragraph end
        // as end of field; every observer check still compares observedAfter().
        if (context->identityKnown && imeParityApp(binding.state.appId())) normalizeObservedParagraphEnd(*context);
        return context;
    }

    std::string_view unavailableContextReason(const Binding &binding) const {
        auto &input = *binding.inputContext;
        if (!binding.state.editingAvailable())
            return editingUnavailableReason(binding.state.appId(), binding.observed());
        if (!input.hasFocus() || !binding.state.focused() || !binding.broker.policyAllowed)
            return "native_authority_changed";
        if (!binding.surroundingFreshness.fresh()) return "native_context_stale";
        if (!allowsNativeContext(input.capabilityFlags())) return "native_field_denied";
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
    void invalidateChangedContext(Binding &binding) {
        const auto &captured = binding.state.lastContext();
        if (captured && matchesCapturedContext(captured, currentContext(binding))) return;
        binding.state.invalidateContext();
        clearOwnedPanel(binding);
    }

    // Badi's panel.

    void showNotice(Binding &binding, std::string_view message) {
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

    void clearOwnedPanel(Binding &binding) {
        binding.pendingSuggestion.reset();
        if (binding.overlayOwned) {
            binding.overlayOwned = false;
            hidePreview();
        }
        if (binding.expiryTimer) binding.expiryTimer->setEnabled(false);
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

    // One observer preview exists. It may be on screen from its request until
    // a hide is sent or the observer reports that it did not render.
    void hidePreview() {
        if (!previewMayBeVisible_) return;
        previewMayBeVisible_ = false;
        accessibility_->hide();
    }

    // Fence callbacks before the active IME can consume the key. A foreign
    // composition need not publish surrounding text or reach our post hook.
    void cancelForInput(Binding &binding) {
        ++binding.observationGeneration;
        if (binding.observeTimer) binding.observeTimer->setEnabled(false);
        binding.state.invalidateContext();
        clearOwnedPanel(binding);
        hidePreview();
    }

    std::unique_ptr<::fcitx::EventSourceTime> oneShot(std::uint64_t delayUs, std::function<void()> action) {
        auto timer = instance_->eventLoop().addTimeEvent(CLOCK_MONOTONIC, 0, 0,
            [action = std::move(action)](::fcitx::EventSourceTime *, std::uint64_t) {
                action();
                return true;
            });
        timer->setNextInterval(delayUs);
        timer->setOneShot();
        return timer;
    }

    ::fcitx::Instance *instance_;
    ActivityDebug debug_;
    Transport transport_;
    std::unique_ptr<Accessibility> accessibility_;
    std::vector<std::unique_ptr<::fcitx::HandlerTableEntry<::fcitx::EventHandler>>> handlers_;
    std::unordered_map<std::string, Binding> bindings_;
    AuthorityContinuity authorityContinuity_;
    bool authorityPaused_ = true;
    bool previewMayBeVisible_ = false;
};

class BadiModuleFactory final : public ::fcitx::AddonFactory {
public:
    ::fcitx::AddonInstance *create(::fcitx::AddonManager *manager) override {
        return new BadiAddon(manager->instance());
    }
};

} // namespace badi::fcitx5

FCITX_ADDON_FACTORY_V2(badi, badi::fcitx5::BadiModuleFactory)
