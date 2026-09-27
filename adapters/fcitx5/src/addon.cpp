#include "debug.h"
#include "accessibility.h"
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

std::optional<std::string> randomSalt() {
    std::array<std::uint8_t, 16> bytes{};
    const auto count = ::getrandom(bytes.data(), bytes.size(), 0);
    if (count != static_cast<ssize_t>(bytes.size())) return std::nullopt;
    std::ostringstream output;
    output << std::hex << std::setfill('0');
    for (const auto byte : bytes) {
        output << std::setw(2) << static_cast<unsigned int>(byte);
    }
    return output.str();
}

std::optional<std::string> randomUuid() {
    std::array<std::uint8_t, 16> bytes{};
    const auto count = ::getrandom(bytes.data(), bytes.size(), 0);
    if (count != static_cast<ssize_t>(bytes.size())) return std::nullopt;
    bytes[6] = static_cast<std::uint8_t>((bytes[6] & 0x0fU) | 0x40U);
    bytes[8] = static_cast<std::uint8_t>((bytes[8] & 0x3fU) | 0x80U);
    return uuidString(bytes);
}

bool hasFlag(::fcitx::CapabilityFlags flags, ::fcitx::CapabilityFlag flag) {
    return !!(flags & flag);
}

// The one app identity used for policy, sessions, debug and the observer.
// Empty when program() is not an app identifier.
std::string canonicalProgram(const ::fcitx::InputContext &inputContext) {
    return canonicalAppId(inputContext.program()).value_or("");
}

constexpr auto kObserverUnavailableNotice = "Badi cannot see this text field — check badi doctor";
// Automatic inspection waits for typing to pause.
constexpr std::uint64_t kObserveDelayUs = 120'000;
// Invalidations without Fcitx input back off, then wait for input.
constexpr unsigned int kMaxIdleReinspections = 3;

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

class BadiAddon;

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
            [this](const nlohmann::json &event) { observerInvalidated(event); });
        handlers_.push_back(instance_->watchEvent(
            ::fcitx::EventType::InputContextFocusIn,
            ::fcitx::EventWatcherPhase::PostInputMethod,
            [this](::fcitx::Event &event) {
                focusIn(*static_cast<::fcitx::InputContextEvent &>(event).inputContext());
            }));
        handlers_.push_back(instance_->watchEvent(
            ::fcitx::EventType::InputContextFocusOut,
            ::fcitx::EventWatcherPhase::PostInputMethod,
            [this](::fcitx::Event &event) {
                focusOut(*static_cast<::fcitx::InputContextEvent &>(event).inputContext());
            }));
        handlers_.push_back(instance_->watchEvent(
            ::fcitx::EventType::InputContextDestroyed,
            ::fcitx::EventWatcherPhase::PostInputMethod,
            [this](::fcitx::Event &event) {
                destroy(*static_cast<::fcitx::InputContextEvent &>(event).inputContext());
            }));
        handlers_.push_back(instance_->watchEvent(
            ::fcitx::EventType::InputContextSurroundingTextUpdated,
            ::fcitx::EventWatcherPhase::PostInputMethod,
            [this](::fcitx::Event &event) {
                surroundingTextUpdated(
                    *static_cast<::fcitx::InputContextEvent &>(event).inputContext());
            }));
        handlers_.push_back(instance_->watchEvent(
            ::fcitx::EventType::InputContextCapabilityChanged,
            ::fcitx::EventWatcherPhase::PostInputMethod,
            [this](::fcitx::Event &event) {
                capabilityChanged(
                    *static_cast<::fcitx::InputContextEvent &>(event).inputContext());
            }));
        handlers_.push_back(instance_->watchEvent(
            ::fcitx::EventType::InputContextUpdateUI,
            ::fcitx::EventWatcherPhase::PostInputMethod,
            [this](::fcitx::Event &event) {
                auto &update = static_cast<::fcitx::InputContextUpdateUIEvent &>(event);
                if (update.component() == ::fcitx::UserInterfaceComponent::InputPanel)
                    foreignUiUpdated(*update.inputContext());
            }));
        handlers_.push_back(instance_->watchEvent(
            ::fcitx::EventType::InputContextUpdatePreedit,
            ::fcitx::EventWatcherPhase::PostInputMethod,
            [this](::fcitx::Event &event) {
                foreignUiUpdated(*static_cast<::fcitx::InputContextEvent &>(event).inputContext());
            }));
        handlers_.push_back(instance_->watchEvent(
            ::fcitx::EventType::InputContextKeyEvent,
            ::fcitx::EventWatcherPhase::PreInputMethod,
            [this](::fcitx::Event &event) {
                preKeyEvent(static_cast<::fcitx::KeyEvent &>(event));
            }));
        handlers_.push_back(instance_->watchEvent(
            ::fcitx::EventType::InputContextKeyEvent,
            ::fcitx::EventWatcherPhase::PostInputMethod,
            [this](::fcitx::Event &event) {
                keyEvent(static_cast<::fcitx::KeyEvent &>(event));
            }));
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
        std::shared_ptr<::fcitx::CandidateList> ownedCandidates;
        std::string ownedAuxiliary;
        std::unique_ptr<::fcitx::EventSourceTime> expiryTimer;
        Coordinates brokerCoordinates;
        SurroundingFreshness surroundingFreshness;
        bool opened = false;
        bool policyKnown = false;
        bool policyAllowed = false;
        nlohmann::json observedFocus;
        std::unique_ptr<::fcitx::EventSourceTime> observeTimer;
        std::uint64_t observationGeneration = 0;
        bool observationExplicit = false;
        bool overlayOwned = false;
        bool waitingForForeignUi = false;
        std::optional<std::string> pendingSuggestion;
        unsigned int observeRetries = 0;
        std::optional<ContextWindow> dismissedContext;
        // Fcitx input (keys and surrounding-text changes) since the last inspection.
        std::uint64_t inputSerial = 0;
        std::uint64_t inspectedSerial = 0;
        std::size_t surroundingDigest = 0;
        unsigned int idleInvalidations = 0;

        void noteInput() {
            ++inputSerial;
            idleInvalidations = 0;
        }
    };

    static WireCallbacks callbacksFor(BadiAddon *addon) {
        return WireCallbacks{
            .onReady = [addon] { addon->onReady(); },
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

    Binding *bindingFor(::fcitx::InputContext &inputContext) {
        const auto key = uuidString(inputContext.uuid());
        const auto iterator = bindings_.find(key);
        if (iterator == bindings_.end() ||
            iterator->second.inputContext != &inputContext) {
            return nullptr;
        }
        return &iterator->second;
    }

    Binding *bindingFor(std::string_view sessionId) {
        for (auto &[_, binding] : bindings_) {
            if (binding.state.coordinates().sessionId == sessionId) return &binding;
        }
        return nullptr;
    }

    void focusIn(::fcitx::InputContext &inputContext) {
        const auto appId = canonicalProgram(inputContext);
        debug_.refresh();
        debug_.record("focus", appId, focusReason(appId));
        const auto contextId = uuidString(inputContext.uuid());
        const auto sessionId = randomUuid();
        const auto salt = randomSalt();
        if (!sessionId || !salt || appId.empty()) return;
        const auto existing = bindings_.find(contextId);
        if (existing != bindings_.end()) {
            if (existing->second.opened) {
                transport_.closeSession(existing->second.brokerCoordinates);
            }
            clearOwnedPanel(existing->second);
        }
        auto &binding = bindings_[contextId];
        binding.inputContext = &inputContext;
        binding.observationGeneration++;
        binding.observedFocus = nullptr;
        binding.observationExplicit = false;
        binding.waitingForForeignUi = false;
        binding.dismissedContext.reset();
        binding.noteInput();
        if (binding.observeTimer) binding.observeTimer->setEnabled(false);
        binding.surroundingFreshness.focusIn();
        if (!binding.state.focusIn(*sessionId, contextId, appId, *salt)) {
            bindings_.erase(contextId);
            return;
        }
        binding.opened = false;
        binding.policyKnown = false;
        binding.policyAllowed = false;
        transport_.connect();
        queryPolicy(binding);
        observeLater(binding);
    }

    void focusOut(::fcitx::InputContext &inputContext) {
        debug_.record("blur", canonicalProgram(inputContext), "focus_left");
        auto *binding = bindingFor(inputContext);
        if (binding == nullptr) return;
        if (binding->opened) transport_.closeSession(binding->brokerCoordinates);
        clearOwnedPanel(*binding);
        binding->state.focusOut();
        binding->surroundingFreshness.focusOut();
        binding->opened = false;
        binding->policyAllowed = false;
        binding->observationGeneration++;
        binding->observedFocus = nullptr;
        binding->waitingForForeignUi = false;
        if (binding->observeTimer) binding->observeTimer->setEnabled(false);
    }

    void destroy(::fcitx::InputContext &inputContext) {
        const auto key = uuidString(inputContext.uuid());
        auto iterator = bindings_.find(key);
        if (iterator == bindings_.end()) return;
        if (iterator->second.opened) {
            transport_.closeSession(iterator->second.brokerCoordinates);
        }
        clearOwnedPanel(iterator->second);
        bindings_.erase(iterator);
    }

    void invalidate(::fcitx::InputContext &inputContext) {
        auto *binding = bindingFor(inputContext);
        if (binding == nullptr) return;
        const auto &captured = binding->state.lastContext();
        const auto current = captured ? currentContext(*binding) : std::nullopt;
        if (matchesCapturedContext(captured, current)) {
            // Chord formation may republish an unchanged toolkit buffer.
            return;
        }
        binding->state.invalidateContext();
        clearOwnedPanel(*binding);
    }

    void surroundingTextUpdated(::fcitx::InputContext &inputContext) {
        const auto app = canonicalProgram(inputContext);
        debug_.record("context", app, app.empty() ? "unidentified_app" :
            allowsNativeContext(inputContext.capabilityFlags()) ? "context_received" : "field_denied");
        auto *binding = bindingFor(inputContext);
        if (binding == nullptr) return;
        binding->surroundingFreshness.surroundingTextUpdated();
        const auto digest = surroundingDigest(inputContext.surroundingText());
        if (digest != binding->surroundingDigest) {
            binding->surroundingDigest = digest;
            binding->noteInput();
        }
        // Unavailable apps never hold editing context. Toolkit publication must
        // not erase their explicit, timed unavailable notice. Classify the app
        // itself: observed targets of other classes still require invalidation
        // and fresh field metadata below.
        if (binding->state.focused() && !nativeObservationAvailable(binding->state.appId())) return;
        const bool unchangedObserved = !binding->observedFocus.is_null() &&
            matchesCapturedContext(binding->state.lastContext(), currentContext(*binding));
        invalidate(inputContext);
        if (!unchangedObserved) observeLater(*binding);
    }

    void capabilityChanged(::fcitx::InputContext &inputContext) {
        const auto app = canonicalProgram(inputContext);
        debug_.record("authority", app, "capabilities_changed");
        auto *binding = bindingFor(inputContext);
        if (binding == nullptr || app.empty()) return;
        // Capability changes can leave the previous widget's buffer cached.
        binding->surroundingFreshness.capabilityChanged();
        invalidate(inputContext);
        observerInvalidated(nlohmann::json{{"app_id", app}});
    }

    PanelObservation observePanel(const Binding &binding) const {
        const auto &panel = binding.inputContext->inputPanel();
        const auto candidates = panel.candidateList();
        return PanelObservation{
            .preedit = !panel.preedit().empty(),
            .clientPreedit = !panel.clientPreedit().empty(),
            .candidates = binding.overlayOwned || (candidates != nullptr && !candidates->empty()),
            .candidatesOwnedByBadi = (binding.overlayOwned && (candidates == nullptr || candidates->empty())) || (candidates != nullptr &&
                                             candidates.get() ==
                                                 binding.ownedCandidates.get()),
            .foreignAuxiliary = (!panel.auxUp().empty() &&
                                  panel.auxUp().toString() != binding.ownedAuxiliary) ||
                                 !panel.auxDown().empty(),
        };
    }

    // One observer preview exists. It may be on screen from its request until
    // a hide is sent or the observer reports that it did not render.
    void hidePreview() {
        if (!previewMayBeVisible_) return;
        previewMayBeVisible_ = false;
        accessibility_->hide();
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
            if (changed) {
                binding.inputContext->updateUserInterface(
                    ::fcitx::UserInterfaceComponent::InputPanel);
            }
        }
        binding.ownedCandidates.reset();
        binding.ownedAuxiliary.clear();
        binding.state.clearSuggestion();
    }

    void denyEditing(Binding &binding) {
        if (binding.opened) transport_.closeSession(binding.brokerCoordinates);
        binding.opened = false;
        binding.policyAllowed = false;
        binding.observationGeneration++;
        binding.state.denyEditing();
        binding.observedFocus = nullptr;
        clearOwnedPanel(binding);
        debug_.record("request_blocked", binding.state.appId(),
            imeParityApp(binding.state.appId()) ? "ime_parity_target_invalid" : "editor_transaction_unavailable");
    }

    // IME-parity has no manual fallback, so an explicit request says why the
    // observer produced no field: the field's own purpose, or no observer.
    void observerUnavailable(Binding &binding, const nlohmann::json &error = nullptr) {
        if (!imeParityApp(binding.state.appId())) return;
        const bool fieldDenied = observerDeniedField(error);
        debug_.record("request_blocked", binding.state.appId(),
            fieldDenied ? "ime_parity_field_denied" : "ime_parity_observer_unavailable");
        if (binding.observationExplicit)
            showNotice(binding, fieldDenied ? "Badi cannot read this text field — run badi debug status"
                                            : kObserverUnavailableNotice);
    }

    void observerInvalidated(const nlohmann::json &event) {
        const auto app = event.value("app_id", nlohmann::json());
        const bool fieldChanged = event.value("reason", nlohmann::json()) == "field_changed";
        for (auto &[_, binding] : bindings_) {
            if (!binding.state.focused() || (app.is_string() && app != "" && app != binding.state.appId())) continue;
            debug_.record("observer", binding.state.appId(), "observer_invalidated");
            binding.observationGeneration++;
            if (!binding.observedFocus.is_null()) {
                if (binding.opened) transport_.closeSession(binding.brokerCoordinates);
                binding.opened = false;
                binding.policyKnown = false;
                binding.policyAllowed = false;
                binding.state.retireObservation();
                binding.observedFocus = nullptr;
                clearOwnedPanel(binding);
            }
            // Without Fcitx input since the last inspection, a changed field
            // waits for input and other invalidations back off, so a busy page
            // cannot drive an inspection loop. The next input inspects again.
            if (binding.inputSerial == binding.inspectedSerial) {
                if (fieldChanged || binding.idleInvalidations == kMaxIdleReinspections) {
                    debug_.record("observer", binding.state.appId(), "observer_awaiting_input");
                    continue;
                }
                ++binding.idleInvalidations;
            }
            observeLater(binding);
        }
    }

    void observeLater(Binding &binding, bool explicitRequest = false, bool retry = false) {
        if (!binding.inputContext->hasFocus() || !binding.state.focused() ||
            !nativeObservationAvailable(binding.state.appId())) return;
        if (!retry) binding.observeRetries = 0;
        binding.observationExplicit = explicitRequest;
        if (explicitRequest) binding.dismissedContext.reset();
        const auto contextId = uuidString(binding.inputContext->uuid());
        if (!binding.observeTimer) binding.observeTimer = instance_->eventLoop().addTimeEvent(CLOCK_MONOTONIC, 0, 0,
            [this, contextId](::fcitx::EventSourceTime *, std::uint64_t) {
                auto found = bindings_.find(contextId);
                if (found != bindings_.end() && found->second.inputContext->hasFocus()) inspectField(found->second);
                return true;
            });
        binding.observeTimer->setNextInterval(explicitRequest && !retry ? 1 : kObserveDelayUs << binding.idleInvalidations);
        binding.observeTimer->setOneShot();
    }

    void inspectField(Binding &binding) {
        if (!nativeObservationAvailable(binding.state.appId()) ||
            !allowsNativeContext(binding.inputContext->capabilityFlags()) ||
            !binding.surroundingFreshness.fresh()) return;
        if (accessibility_->pending()) {
            // A previous field's RPC can outlive a rapid focus switch. Retry
            // this latest field within the bounded RPC deadline, never an
            // absent service or a denied field in an unbounded timer loop.
            if (binding.observeRetries++ < 5)
                observeLater(binding, binding.observationExplicit, true);
            else
                observerUnavailable(binding);
            return;
        }
        if (hasForeignImeUi(observePanel(binding)) || instance_->isComposing(binding.inputContext)) {
            binding.waitingForForeignUi = true;
            return;
        }
        binding.waitingForForeignUi = false;
        binding.inspectedSerial = binding.inputSerial;
        const auto session = binding.state.coordinates().sessionId;
        const auto generation = binding.observationGeneration;
        const bool sent = accessibility_->request(nlohmann::json{{"op", "inspect"}, {"app_id", binding.state.appId()}},
            [this, session, generation](const nlohmann::json &response) {
                auto *current = bindingFor(session);
                if (!current || !current->inputContext->hasFocus() || current->observationGeneration != generation) return;
                if (!observerAnswered(response)) {
                    observerUnavailable(*current, response.value("error", nlohmann::json()));
                    return;
                }
                const auto &focus = response["focus"];
                const auto editTarget = inspectedEditTarget(focus, current->state.appId());
                if (!editTarget) {
                    denyEditing(*current);
                    return;
                }
                if (!current->observationExplicit && !current->observedFocus.is_null() &&
                    matchesObservedFocus(current->observedFocus, focus) &&
                    matchesCapturedContext(current->state.lastContext(), currentContext(*current))) return;
                const auto newSession = randomUuid();
                const auto salt = randomSalt();
                if (!newSession || !salt) return;
                if (current->opened) transport_.closeSession(current->brokerCoordinates);
                clearOwnedPanel(*current);
                current->opened = false;
                current->policyKnown = false;
                current->policyAllowed = false;
                const auto appId = current->state.appId();
                if (!current->state.focusIn(*newSession, focus["target"]["target_id"].get<std::string>(), appId, *salt,
                                            *editTarget, NativeEditPath::Observed)) return;
                current->observedFocus = focus;
                if (!current->state.editingAvailable()) {
                    debug_.record("request_blocked", appId, editingUnavailableReason(appId, !current->observedFocus.is_null()));
                    return;
                }
                queryPolicy(*current);
            });
        if (!sent) observerUnavailable(binding);
    }

    void requestObservedContext(Binding &binding) {
        const auto session = binding.state.coordinates().sessionId;
        const auto generation = binding.observationGeneration;
        const auto focus = binding.observedFocus;
        if (focus.is_null() || !safeToObserve(binding)) {
            debug_.record("request_blocked", binding.state.appId(), "observer_not_ready");
            return;
        }
        if (!binding.observationExplicit && !binding.surroundingFreshness.freshForAutomatic()) {
            // A reconnect alone must not republish unchanged prose automatically.
            debug_.record("request_blocked", binding.state.appId(), "observer_awaiting_input");
            return;
        }
        const bool requested = accessibility_->request(nlohmann::json{{"op", "snapshot"}, {"binding", focus["binding"]}, {"policy_target", focus["target"]}},
            [this, session, generation, focus](const nlohmann::json &response) {
                auto *current = bindingFor(session);
                if (!current || !current->inputContext->hasFocus() || current->observationGeneration != generation ||
                    !current->policyAllowed) {
                    debug_.record("request_blocked", "unidentified", "observer_authority_changed");
                    return;
                }
                if (!observerAnswered(response)) {
                    debug_.record("request_blocked", current->state.appId(),
                        snapshotFailureReason(response.value("error", nlohmann::json())));
                    return;
                }
                const auto &observed = response["focus"];
                if (!matchesObservedFocus(focus, observed, true)) {
                    debug_.record("request_blocked", current->state.appId(), "observer_context_mismatch");
                    return;
                }
                const auto context = currentContext(*current);
                if (!context || !context->identityKnown || !context->after.empty() || context->before.empty() ||
                    !observerAgrees(observed, *context) || hasForeignImeUi(observePanel(*current))) {
                    debug_.record("request_blocked", current->state.appId(),
                        !context ? unavailableContextReason(*current) :
                        !context->identityKnown ? "field_identity_unknown" :
                        context->before.empty() ? "empty_prefix" :
                        !context->after.empty() ? "caret_not_at_end" :
                        hasForeignImeUi(observePanel(*current)) ? "foreign_ime_active" :
                        "observer_context_mismatch", context ? context->before.size() : 0);
                    return;
                }
                current->observedFocus = observed;
                if (current->dismissedContext && *current->dismissedContext == *context) return;
                current->dismissedContext.reset();
                const auto update = current->state.updateContext(*context);
                if (update && open(*current) && transport_.publishContext(*update)) {
                    current->brokerCoordinates = update->coordinates;
                    debug_.record("request", current->state.appId(), context->paragraphEndAfter ?
                        "observed_field_request_paragraph_end" : "observed_field_request", context->before.size());
                }
            });
        if (!requested) {
            debug_.record("request_blocked", binding.state.appId(), "observer_busy");
            if (binding.observationExplicit) observerUnavailable(binding);
        }
    }

    void showNotice(Binding &binding, std::string message) {
        if (!binding.inputContext->hasFocus() || hasForeignImeUi(observePanel(binding))) return;
        clearOwnedPanel(binding);
        binding.ownedAuxiliary = std::move(message);
        binding.inputContext->inputPanel().setAuxUp(::fcitx::Text(binding.ownedAuxiliary));
        const auto coordinates = binding.state.coordinates();
        binding.expiryTimer = instance_->eventLoop().addTimeEvent(CLOCK_MONOTONIC, 0, 0,
            [this, coordinates](::fcitx::EventSourceTime *, std::uint64_t) {
                auto *current = bindingFor(coordinates.sessionId);
                if (current && current->state.coordinates() == coordinates) clearOwnedPanel(*current);
                return true;
            });
        binding.expiryTimer->setNextInterval(5'000'000);
        binding.expiryTimer->setOneShot();
        binding.inputContext->updateUserInterface(::fcitx::UserInterfaceComponent::InputPanel);
    }

    void foreignUiUpdated(::fcitx::InputContext &input) {
        auto *binding = bindingFor(input);
        if (!binding || !input.hasFocus() || !binding->state.focused()) return;
        if (hasForeignImeUi(observePanel(*binding)) || instance_->isComposing(&input)) {
            if (binding->state.lastContext() || binding->overlayOwned ||
                !binding->ownedAuxiliary.empty()) {
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

    void cancelForInput(Binding &binding) {
        // Fence callbacks before the active IME can consume the key. A foreign
        // composition need not publish surrounding text or reach our post hook.
        binding.observationGeneration++;
        if (binding.observeTimer) binding.observeTimer->setEnabled(false);
        binding.state.invalidateContext();
        clearOwnedPanel(binding);
        hidePreview();
    }

    void preKeyEvent(::fcitx::KeyEvent &event) {
        if (event.isRelease() || event.isVirtual()) {
            return;
        }
        const auto app = canonicalProgram(*event.inputContext());
        const auto tab = event.key().normalize().check(::fcitx::Key(FcitxKey_Tab));
        debug_.record(tab ? "tab" : "input", app, app.empty() ? "unidentified_app" : "input_received");
        auto *binding = bindingFor(*event.inputContext());
        if (binding == nullptr || !binding->state.focused()) return;
        if (!event.key().isModifier()) binding->noteInput();
        if (!binding->state.editingAvailable()) {
            // Preserve original Tab/navigation. Only the explicit invocation
            // chord may display a notice or request an observed field below.
            if (!event.key().isModifier()) cancelForInput(*binding);
            debug_.record("request_blocked", app,
                editingUnavailableReason(binding->state.appId(), !binding->observedFocus.is_null()));
            return;
        }
        const auto panel = observePanel(*binding);
        const auto key = event.key().normalize();
        if (!!(event.key().states() & ::fcitx::KeyState::Repeat)) {
            if (!key.isModifier()) cancelForInput(*binding);
            return;
        }
        if (key.check(::fcitx::Key(FcitxKey_Tab)) && !event.filtered() && !event.accepted()) {
            // An explicit key can renew an exhausted recovery budget without
            // reading the field before the new policy response arrives.
            if (!transport_.ready()) transport_.connect();
            // Claim plain Tab before Fcitx's candidate-navigation fallback, but
            // leave indentation/navigation and foreign IMEs alone elsewhere.
            const auto context = hasForeignImeUi(panel) ? std::nullopt : currentContext(*binding);
            const bool eligible = tabEligibleContext(context);
            const auto action = decideTabAction(eligible, binding->state.suggestionVisible(), panel,
                                                binding->state.editPath());
            debug_.record("decision", app, !binding->policyKnown ? "checking_app_policy" :
                !binding->policyAllowed ? "app_disabled" : hasForeignImeUi(panel) ? "foreign_ime" :
                !allowsNativeContext(event.inputContext()->capabilityFlags()) ? "field_denied" :
                !binding->surroundingFreshness.fresh() ? "no_fresh_context" :
                !context ? "context_unavailable" :
                !context->after.empty() ? "caret_not_at_end" :
                !supportedWritingLanguage(context->language) ? "language_unsupported" :
                !eligible ? "empty_context" : "eligible", context ? context->before.size() : 0);
            if (action == LocalAction::Accept) {
                requestAcceptance(*binding);
                event.filterAndAccept();
            } else if (action == LocalAction::Invoke) {
                invoke(*binding);
                event.filterAndAccept();
            } else {
                cancelForInput(*binding);
            }
            return;
        }
        const bool escapeKey =
            event.key().normalize().check(::fcitx::Key(FcitxKey_Escape));
        if (escapeKey && !binding->ownedAuxiliary.empty() && !binding->state.suggestionVisible() && !hasForeignImeUi(panel)) {
            binding->state.invalidateContext();
            clearOwnedPanel(*binding);
            event.filterAndAccept();
            return;
        }
        const bool hasOwnedCandidate = binding->state.suggestionVisible() &&
                                       panel.candidatesOwnedByBadi;
        if (decideLocalAction(false, false, escapeKey, hasOwnedCandidate,
                              panel) != LocalAction::Dismiss) {
            if (!key.isModifier() && !key.check(invokeChord()) && !key.check(acceptChord())) {
                if (escapeKey && binding->state.lastContext())
                    binding->dismissedContext = binding->state.lastContext()->context;
                cancelForInput(*binding);
            }
            return;
        }
        if (const auto request = binding->state.requestDismissal(
                transport_.nowMs(), panel)) {
            if (binding->state.lastContext()) binding->dismissedContext = binding->state.lastContext()->context;
            transport_.requestDismissal(*request);
        }
        clearOwnedPanel(*binding);
        event.filterAndAccept();
    }

    void keyEvent(::fcitx::KeyEvent &event) {
        if (event.isRelease() ||
            !!(event.key().states() & ::fcitx::KeyState::Repeat)) {
            return;
        }
        auto *binding = bindingFor(*event.inputContext());
        if (binding == nullptr || !binding->state.focused()) return;
        const auto panel = observePanel(*binding);
        const auto key = event.key().normalize();
        const bool invokeKey = key.check(invokeChord());
        const bool acceptKey = key.check(acceptChord());
        const bool escapeKey = key.check(::fcitx::Key(FcitxKey_Escape));
        if (event.isVirtual() || event.filtered() || event.accepted()) return;
        if (!binding->state.editingAvailable()) {
            if (invokeKey && !hasForeignImeUi(panel) && invoke(*binding)) event.filterAndAccept();
            return;
        }
        const auto action = decideLocalAction(
            invokeKey, acceptKey, escapeKey,
            binding->state.suggestionVisible() && panel.candidatesOwnedByBadi,
            panel);
        switch (action) {
        case LocalAction::Invoke:
            if (invoke(*binding)) event.filterAndAccept();
            return;
        case LocalAction::Accept:
            requestAcceptance(*binding);
            event.filterAndAccept();
            return;
        case LocalAction::Dismiss:
            if (const auto request = binding->state.requestDismissal(
                    transport_.nowMs(), panel)) {
                if (binding->state.lastContext()) binding->dismissedContext = binding->state.lastContext()->context;
                transport_.requestDismissal(*request);
            }
            clearOwnedPanel(*binding);
            event.filterAndAccept();
            return;
        case LocalAction::PassThrough:
            if (binding->state.suggestionVisible() &&
                panel.candidatesOwnedByBadi && !key.isModifier()) {
                binding->state.invalidateContext();
                clearOwnedPanel(*binding);
            }
            return;
        }
    }

    std::optional<ContextWindow> currentContext(Binding &binding) const {
        auto &inputContext = *binding.inputContext;
        const auto capabilities = inputContext.capabilityFlags();
        if (!binding.state.editingAvailable() || !inputContext.hasFocus() || !binding.state.focused() || !binding.policyAllowed || !binding.surroundingFreshness.fresh() ||
            !allowsNativeContext(capabilities) ||
            instance_->isComposing(&inputContext)) {
            return std::nullopt;
        }
        const auto &surrounding = inputContext.surroundingText();
        const auto *inputMethod = instance_->inputMethodEntry(&inputContext);
        if (!surrounding.isValid() || inputMethod == nullptr ||
            !validLanguageTag(inputMethod->languageCode())) {
            return std::nullopt;
        }
        auto context = captureContextWindow(
            surrounding.text(), surrounding.cursor(), surrounding.anchor(),
            hasFlag(capabilities, ::fcitx::CapabilityFlag::Multiline), inputMethod->languageCode());
        if (context) {
            context->identityKnown = !binding.observedFocus.is_null();
            context->explicitRequest = context->identityKnown ? binding.observationExplicit : true;
            // Only an observed IME-parity field treats Chromium's paragraph end
            // as end of field; every observer check still compares observedAfter().
            if (context->identityKnown && imeParityApp(binding.state.appId()))
                normalizeObservedParagraphEnd(*context);
        }
        return context;
    }

    std::string_view unavailableContextReason(const Binding &binding) const {
        auto &input = *binding.inputContext;
        if (!binding.state.editingAvailable())
            return editingUnavailableReason(binding.state.appId(), !binding.observedFocus.is_null());
        if (!input.hasFocus() || !binding.state.focused() || !binding.policyAllowed)
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

    bool safeToObserve(const Binding &binding) const {
        auto &input = *binding.inputContext;
        return binding.state.editingAvailable() && input.hasFocus() && binding.state.focused() && binding.policyAllowed &&
            binding.surroundingFreshness.fresh() && allowsNativeContext(input.capabilityFlags()) &&
            !instance_->isComposing(&input) && !hasForeignImeUi(observePanel(binding));
    }

    bool invoke(Binding &binding) {
        if (!binding.state.editingAvailable() && imeParityApp(binding.state.appId()) &&
            binding.observedFocus.is_null()) {
            // An explicit IME-parity request inspects the field; it never
            // becomes an unknown-identity manual request.
            debug_.record("observer", binding.state.appId(), "explicit_inspect_requested");
            if (!binding.surroundingFreshness.fresh()) {
                showNotice(binding, "Badi needs fresh context — type, then invoke again");
            } else if (!allowsNativeContext(binding.inputContext->capabilityFlags())) {
                showNotice(binding, "Badi cannot read this text field — run badi debug status");
            } else {
                clearOwnedPanel(binding);
                transport_.connect();
                observeLater(binding, true);
            }
            return true;
        }
        if (!binding.state.editingAvailable()) {
            showNotice(binding, "Badi cannot safely insert suggestions in this app yet");
            return true;
        }
        if (!binding.observedFocus.is_null()) {
            observeLater(binding, true);
            return true;
        }
        clearOwnedPanel(binding);
        transport_.connect();
        queryPolicy(binding);
        if (!transport_.ready()) {
            showNotice(binding, "Badi is reconnecting — check badi doctor if this persists");
            return true;
        }
        if (!binding.policyKnown) {
            showNotice(binding, "Badi is checking this application's permission");
            return true;
        }
        if (!binding.policyAllowed) {
            showNotice(binding, authorityPaused_ ? "Badi paused — resume from the tray"
                                               : "Badi is disabled for this application");
            return true;
        }
        if (!binding.surroundingFreshness.fresh()) {
            showNotice(binding, "Badi needs fresh context — type, then invoke again");
            return true;
        }
        const auto context = currentContext(binding);
        if (!context) {
            binding.state.invalidateContext();
            debug_.record("request_blocked", binding.state.appId(), "context_unavailable");
            showNotice(binding, "Badi cannot read this text field — run badi debug status");
            return true;
        }
        const auto update = binding.state.updateContext(*context);
        if (!update) return true;
        if (!open(binding)) {
            showNotice(binding, authorityPaused_ ? "Badi paused — resume from the tray" : "Badi model is not connected");
            return true;
        }
        if (transport_.publishContext(*update)) {
            debug_.record("request", binding.state.appId(), "sent_to_model", context->before.size());
            binding.brokerCoordinates = update->coordinates;
            showNotice(binding, "Badi is thinking…");
        } else {
            showNotice(binding, "Badi request failed — check the tray status");
        }
        return true;
    }

    bool requestAcceptance(Binding &binding) {
        if (!binding.state.editingAvailable()) {
            clearOwnedPanel(binding);
            return false;
        }
        const auto request = binding.state.requestAcceptance(
            transport_.nowMs(), observePanel(binding));
        if (!request || !transport_.requestAcceptance(*request)) {
            binding.state.clearSuggestion();
            clearOwnedPanel(binding);
            return false;
        }
        return true;
    }

    bool open(Binding &binding) {
        if (!binding.state.editingAvailable()) return false;
        if (binding.opened) return true;
        if (!binding.policyAllowed || authorityPaused_ || !transport_.ready()) return false;
        const bool opened = binding.observedFocus.is_null()
            ? transport_.openSession(binding.state.coordinates(), binding.state.appId(), binding.state.targetId())
            : transport_.openTargetSession(binding.state.coordinates(), binding.observedFocus["target"]);
        if (!opened) {
            return false;
        }
        binding.brokerCoordinates = binding.state.coordinates();
        binding.brokerCoordinates.revision = 0;
        binding.brokerCoordinates.fingerprint.clear();
        binding.opened = true;
        return true;
    }

    void onReady() {
        for (auto &[_, binding] : bindings_) {
            if (binding.state.focused()) queryPolicy(binding);
        }
    }

    void queryPolicy(Binding &binding) {
        if (binding.state.editingAvailable() && !binding.policyKnown && transport_.ready()) {
            if (!binding.observedFocus.is_null())
                transport_.queryTargetPolicy(binding.state.coordinates(), binding.observedFocus["target"]);
            else
                transport_.queryPolicy(binding.state.coordinates(), binding.state.appId(), binding.state.targetId());
        }
    }

    void onPolicy(std::string_view session, bool allowed) {
        auto *binding = bindingFor(session);
        if (!binding || !binding->state.focused() || !binding->state.editingAvailable()) return;
        binding->policyKnown = true;
        binding->policyAllowed = allowed;
        debug_.record("policy", binding->state.appId(), allowed ? "app_allowed" : "app_disabled");
        if (allowed && !binding->observedFocus.is_null()) requestObservedContext(*binding);
        if (allowed) open(*binding);
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
            binding.opened = false;
            binding.policyKnown = false;
            binding.policyAllowed = false;
            binding.state.invalidateContext();
            binding.surroundingFreshness.capabilityChanged();
            clearOwnedPanel(binding);
        }
        if (!authorityPaused_) {
            for (auto &[_, binding] : bindings_) {
                if (binding.state.focused()) queryPolicy(binding);
            }
        }
    }

    void onSuggestion(Suggestion suggestion) {
        auto *binding = bindingFor(suggestion.coordinates.sessionId);
        if (!binding || !safeToObserve(*binding)) return;
        // SessionState keeps IME-parity apps off this unobserved display path.
        if (binding->observedFocus.is_null()) { displaySuggestion(std::move(suggestion)); return; }
        binding->pendingSuggestion = suggestion.suggestionId;
        const auto focus = binding->observedFocus;
        const auto generation = binding->observationGeneration;
        const bool sent = accessibility_->request(nlohmann::json{{"op", "snapshot"}, {"binding", focus["binding"]}, {"policy_target", focus["target"]}},
            [this, suggestion, focus, generation](const nlohmann::json &response) {
                auto *current = bindingFor(suggestion.coordinates.sessionId);
                if (!current || !safeToObserve(*current) || current->observationGeneration != generation ||
                    current->state.coordinates() != suggestion.coordinates) return;
                if (!observerAnswered(response) || !response["focus"].is_object()) {
                    debug_.record("suggestion_blocked", current->state.appId(), "observer_display_unavailable");
                    return;
                }
                const auto context = currentContext(*current);
                const auto &observed = response["focus"];
                if (!context || !matchesCapturedContext(current->state.lastContext(), context) ||
                    !matchesObservedFocus(focus, observed, true) || !observerAgrees(observed, *context)) {
                    debug_.record("suggestion_blocked", current->state.appId(), "observer_display_mismatch");
                    return;
                }
                current->observedFocus = observed;
                showObservedPreview(*current, suggestion);
            });
        if (!sent) debug_.record("suggestion_blocked", binding->state.appId(), "observer_display_unavailable");
    }

    void displaySuggestion(Suggestion suggestion, bool overlay = false) {
        auto *binding = bindingFor(suggestion.coordinates.sessionId);
        if (binding == nullptr || !binding->inputContext->hasFocus() ||
            hasForeignImeUi(observePanel(*binding)) ||
            !matchesCapturedContext(binding->state.lastContext(), currentContext(*binding)) ||
            !binding->state.showSuggestion(suggestion, transport_.nowMs())) {
            if (overlay) hidePreview();
            return;
        }
        binding->overlayOwned = overlay;
        binding->pendingSuggestion.reset();
        const auto sessionId = suggestion.coordinates.sessionId;
        if (!overlay) {
            auto list = std::make_unique<::fcitx::CommonCandidateList>();
            list->setPageSize(1);
            list->setLayoutHint(::fcitx::CandidateLayoutHint::Horizontal);
            list->append(std::make_unique<BadiCandidate>(
                suggestion.text,
                [this, sessionId](::fcitx::InputContext *selected) {
                    auto *current = bindingFor(sessionId);
                    if (current != nullptr && current->inputContext == selected &&
                        selected->hasFocus()) {
                        requestAcceptance(*current);
                    }
                }));
            auto &panel = binding->inputContext->inputPanel();
            binding->ownedAuxiliary = "Badi · Tab to accept · Escape to dismiss";
            panel.setAuxUp(::fcitx::Text(binding->ownedAuxiliary));
            panel.setCandidateList(std::move(list));
            binding->ownedCandidates = panel.candidateList();
        }
        // The local lease must also hide UI if the broker stalls before its
        // suggestion.clear arrives. Only this exact candidate may be retired.
        binding->expiryTimer = instance_->eventLoop().addTimeEvent(
            CLOCK_MONOTONIC, 0, 0,
            [this, sessionId, coordinates = suggestion.coordinates,
             suggestionId = suggestion.suggestionId](
                ::fcitx::EventSourceTime *, std::uint64_t) {
                auto *current = bindingFor(sessionId);
                if (current != nullptr &&
                    current->state.clearSuggestionIf(coordinates, suggestionId)) {
                    clearOwnedPanel(*current);
                }
                return true;
            });
        const auto now = transport_.nowMs();
        binding->expiryTimer->setNextInterval(
            (suggestion.expiresAtMs > now ? suggestion.expiresAtMs - now : 0) * 1000);
        binding->expiryTimer->setOneShot();
        binding->inputContext->updateUserInterface(
            ::fcitx::UserInterfaceComponent::InputPanel);
        debug_.record("suggestion", binding->state.appId(), overlay ? "display_overlay" : "display_dispatched");
    }

    void showObservedPreview(Binding &binding, const Suggestion &suggestion) {
        if (!safeToObserve(binding)) return;
        const auto coordinates = suggestion.coordinates;
        const auto focus = binding.observedFocus;
        const auto generation = binding.observationGeneration;
        const auto now = transport_.nowMs();
        if (suggestion.expiresAtMs <= now) return;
        // The observer re-verifies caret and length before rendering. A
        // verified reply with rendered:false means its preview is unavailable
        // (e.g. uncalibrated geometry); Badi's owned Fcitx panel is then shown,
        // which Wayland compositors place at the text-input caret rectangle.
        const bool sent = accessibility_->request(nlohmann::json{{"op", "preview"}, {"binding", focus["binding"]},
            {"policy_target", focus["target"]}, {"text", suggestion.text},
            {"expected_caret", focus["caret"]}, {"expected_total_chars", focus["total_chars"]},
            {"ttl_ms", suggestion.expiresAtMs - now}},
            [this, coordinates, suggestion, focus, generation](const nlohmann::json &response) {
                const bool verified = observerAnswered(response) && response["focus"].is_object();
                const bool rendered = verified && response["focus"].value("rendered", nlohmann::json()) == true;
                if (!rendered) previewMayBeVisible_ = false;
                auto *current = bindingFor(coordinates.sessionId);
                if (!current || !current->inputContext->hasFocus() || current->state.coordinates() != coordinates ||
                    current->observationGeneration != generation) {
                    hidePreview();
                    return;
                }
                if (!verified || !matchesObservedFocus(focus, response["focus"], true) ||
                    !matchesCapturedContext(current->state.lastContext(), currentContext(*current)) ||
                    hasForeignImeUi(observePanel(*current))) {
                    hidePreview();
                    debug_.record("suggestion_blocked", current->state.appId(),
                        !verified ? "observer_display_unavailable" :
                        hasForeignImeUi(observePanel(*current)) ? "foreign_ime_active" : "observer_display_mismatch");
                    return;
                }
                displaySuggestion(suggestion, rendered);
            });
        if (sent) previewMayBeVisible_ = true;
        else debug_.record("suggestion_blocked", binding.state.appId(), "observer_display_unavailable");
    }

    void onClear(const ClearNotice &notice) {
        auto *binding = bindingFor(notice.coordinates.sessionId);
        if (binding && !binding->state.editingAvailable()) {
            clearOwnedPanel(*binding);
            return;
        }
        if (binding) debug_.record("clear", binding->state.appId(), notice.reason);
        if (binding && binding->state.coordinates() == notice.coordinates && binding->pendingSuggestion &&
            (!notice.suggestionId || *notice.suggestionId == *binding->pendingSuggestion)) {
            // A broker cancellation can arrive before the preview RPC returns.
            // Retire that asynchronous candidate even though it is not visible.
            cancelForInput(*binding);
            return;
        }
        if (binding && !notice.suggestionId && binding->state.coordinates() == notice.coordinates &&
            (binding->observedFocus.is_null() || binding->observationExplicit)) {
            if (const auto text = clearNoticeText(notice.reason)) {
                showNotice(*binding, std::string(*text));
                return;
            }
        }
        if (binding != nullptr && binding->state.clearSuggestionIf(
                                      notice.coordinates, notice.suggestionId)) {
            clearOwnedPanel(*binding);
        }
    }

    void onCommitPrepare(const CommitPrepare &prepare) {
        auto *binding = bindingFor(prepare.coordinates.sessionId);
        if ((binding && !binding->state.editingAvailable()) ||
            (binding && binding->observedFocus.is_null() && imeParityApp(binding->state.appId()))) {
            if (binding) clearOwnedPanel(*binding);
            transport_.reportCommit(prepare.coordinates, prepare.controlId, prepare.suggestionId, "stale");
            return;
        }
        if (binding && !binding->observedFocus.is_null()) {
            if (!safeToObserve(*binding)) {
                clearOwnedPanel(*binding);
                transport_.reportCommit(prepare.coordinates, prepare.controlId, prepare.suggestionId, "stale");
                return;
            }
            // Re-snapshot immediately before commitString: the observer must
            // still agree with Fcitx's live text and caret for this field.
            const auto focus = binding->observedFocus;
            const auto generation = binding->observationGeneration;
            const bool sent = accessibility_->request(nlohmann::json{{"op", "snapshot"},
                {"binding", focus["binding"]}, {"policy_target", focus["target"]}},
                [this, prepare, focus, generation](const nlohmann::json &response) {
                    auto *current = bindingFor(prepare.coordinates.sessionId);
                    const auto context = current ? currentContext(*current) : std::nullopt;
                    const bool answered = observerAnswered(response) && response["focus"].is_object();
                    const bool valid = current && current->observationGeneration == generation && context &&
                        answered && matchesObservedFocus(focus, response["focus"], true) &&
                        observerAgrees(response["focus"], *context);
                    if (valid) applyCommitPrepare(prepare);
                    else {
                        if (current) {
                            debug_.record("commit_blocked", current->state.appId(),
                                answered ? "observer_dispatch_mismatch" : "observer_dispatch_unavailable");
                            clearOwnedPanel(*current);
                        }
                        transport_.reportCommit(prepare.coordinates, prepare.controlId, prepare.suggestionId, "stale");
                    }
                });
            if (!sent) {
                debug_.record("commit_blocked", binding->state.appId(), "observer_dispatch_unavailable");
                clearOwnedPanel(*binding);
                transport_.reportCommit(prepare.coordinates, prepare.controlId, prepare.suggestionId, "stale");
            }
            return;
        }
        applyCommitPrepare(prepare);
    }

    void applyCommitPrepare(const CommitPrepare &prepare) {
        auto *binding = bindingFor(prepare.coordinates.sessionId);
        if (binding == nullptr || !binding->state.editingAvailable() || !binding->inputContext->hasFocus() ||
            !hasOwnedCandidate(observePanel(*binding))) {
            if (binding != nullptr) clearOwnedPanel(*binding);
            transport_.reportCommit(prepare.coordinates, prepare.controlId,
                                    prepare.suggestionId, "stale");
            return;
        }
        const auto current = currentContext(*binding);
        const auto &captured = binding->state.lastContext();
        if (!current || !captured || *current != captured->context) {
            binding->state.invalidateContext();
            clearOwnedPanel(*binding);
            transport_.reportCommit(prepare.coordinates, prepare.controlId,
                                    prepare.suggestionId, "stale");
            return;
        }
        const auto dispatch =
            binding->state.authorizeCommit(prepare, transport_.nowMs(),
                                           observePanel(*binding));
        if (!dispatch) {
            transport_.reportCommit(prepare.coordinates, prepare.controlId,
                                    prepare.suggestionId, "stale");
            return;
        }
        clearOwnedPanel(*binding);
        binding->inputContext->commitString(dispatch->text);
        debug_.record("commit", binding->state.appId(), "dispatched_unverified");
        transport_.reportCommit(dispatch->coordinates, dispatch->controlId,
                                dispatch->suggestionId,
                                "dispatched-unverified");
    }

    void onDisconnected() {
        authorityPaused_ = true;
        for (auto &[_, binding] : bindings_) {
            // Sessions, revisions, candidates, commit grants and in-flight
            // observer replies end with the connection. Fcitx's surrounding
            // text does not, so an explicit request after reconnect may read
            // it once fresh policy reopens the session.
            binding.opened = false;
            binding.policyKnown = false;
            binding.policyAllowed = false;
            binding.observationGeneration++;
            binding.observationExplicit = false;
            binding.state.invalidateContext();
            binding.surroundingFreshness.transportLost();
            clearOwnedPanel(binding);
        }
    }

    ::fcitx::Instance *instance_;
    ActivityDebug debug_;
    Transport transport_;
    std::unique_ptr<Accessibility> accessibility_;
    std::vector<std::unique_ptr<::fcitx::HandlerTableEntry<::fcitx::EventHandler>>>
        handlers_;
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
