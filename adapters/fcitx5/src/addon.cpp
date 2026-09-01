#include "sanitizer.h"
#include "state.h"
#include "transport.h"

#include <fcitx-utils/capabilityflags.h>
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
        transport_.disconnect();
    }

private:
    struct Binding {
        ::fcitx::InputContext *inputContext = nullptr;
        SessionState state;
        std::shared_ptr<::fcitx::CandidateList> ownedCandidates;
        Coordinates brokerCoordinates;
        SurroundingFreshness surroundingFreshness;
        bool opened = false;
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
        const auto appId = inputContext.program();
        const auto contextId = uuidString(inputContext.uuid());
        const auto sessionId = randomUuid();
        const auto salt = randomSalt();
        if (!sessionId || !salt || !supportedAppId(appId)) return;
        const auto existing = bindings_.find(contextId);
        if (existing != bindings_.end()) {
            if (existing->second.opened) {
                transport_.closeSession(existing->second.brokerCoordinates);
            }
            clearOwnedPanel(existing->second);
        }
        auto &binding = bindings_[contextId];
        binding.inputContext = &inputContext;
        binding.surroundingFreshness.focusIn();
        if (!binding.state.focusIn(*sessionId, contextId, appId, *salt)) {
            bindings_.erase(contextId);
            return;
        }
        binding.opened = false;
        transport_.connect();
        open(binding);
    }

    void focusOut(::fcitx::InputContext &inputContext) {
        auto *binding = bindingFor(inputContext);
        if (binding == nullptr) return;
        if (binding->opened) transport_.closeSession(binding->brokerCoordinates);
        clearOwnedPanel(*binding);
        binding->state.focusOut();
        binding->surroundingFreshness.focusOut();
        binding->opened = false;
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
            // Toolkits may republish unchanged surrounding text while a chord
            // is being formed. Preserve the revision only when the complete
            // native capture still matches byte-for-byte; any unavailable or
            // changed context continues to revoke local authority immediately.
            return;
        }
        binding->state.invalidateContext();
        clearOwnedPanel(*binding);
    }

    void surroundingTextUpdated(::fcitx::InputContext &inputContext) {
        auto *binding = bindingFor(inputContext);
        if (binding == nullptr) return;
        binding->surroundingFreshness.surroundingTextUpdated();
        invalidate(inputContext);
    }

    void capabilityChanged(::fcitx::InputContext &inputContext) {
        auto *binding = bindingFor(inputContext);
        if (binding == nullptr) return;
        // A toolkit may leave the old buffer cached while changing focus
        // objects in one native window. A later surrounding-text event is the
        // only evidence that the buffer belongs to the new capability state.
        binding->surroundingFreshness.capabilityChanged();
        invalidate(inputContext);
    }

    PanelObservation observePanel(const Binding &binding) const {
        const auto &panel = binding.inputContext->inputPanel();
        const auto candidates = panel.candidateList();
        return PanelObservation{
            .preedit = !panel.preedit().empty(),
            .clientPreedit = !panel.clientPreedit().empty(),
            .candidates = candidates != nullptr && !candidates->empty(),
            .candidatesOwnedByBadi = candidates != nullptr &&
                                             candidates.get() ==
                                                 binding.ownedCandidates.get(),
        };
    }

    void clearOwnedPanel(Binding &binding) {
        if (binding.inputContext != nullptr && binding.ownedCandidates) {
            auto &panel = binding.inputContext->inputPanel();
            if (panel.candidateList().get() == binding.ownedCandidates.get()) {
                panel.setCandidateList(nullptr);
                binding.inputContext->updateUserInterface(
                    ::fcitx::UserInterfaceComponent::InputPanel);
            }
        }
        binding.ownedCandidates.reset();
        binding.state.clearSuggestion();
    }

    void preKeyEvent(::fcitx::KeyEvent &event) {
        if (event.isRelease() || event.isVirtual() ||
            !!(event.key().states() & ::fcitx::KeyState::Repeat)) {
            return;
        }
        auto *binding = bindingFor(*event.inputContext());
        if (binding == nullptr || !binding->state.focused()) return;
        const auto panel = observePanel(*binding);
        const bool escapeKey =
            event.key().normalize().check(::fcitx::Key(FcitxKey_Escape));
        const bool hasOwnedCandidate = binding->state.suggestionVisible() &&
                                       panel.candidatesOwnedByBadi;
        if (decideLocalAction(false, false, escapeKey, hasOwnedCandidate,
                              panel) != LocalAction::Dismiss) {
            return;
        }
        if (const auto request = binding->state.requestDismissal(
                transport_.nowMs(), panel)) {
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
        const bool invokeChord = key.check(::fcitx::Key("Control+Shift+space"));
        const bool acceptChord = key.check(::fcitx::Key("Control+Shift+Y"));
        const bool escapeKey = key.check(::fcitx::Key(FcitxKey_Escape));
        if (event.isVirtual() || event.filtered() || event.accepted()) return;
        const auto action = decideLocalAction(
            invokeChord, acceptChord, escapeKey,
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
        if (!binding.surroundingFreshness.fresh() ||
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
        return captureContextWindow(
            surrounding.text(), surrounding.cursor(), surrounding.anchor(), false,
            hasFlag(capabilities, ::fcitx::CapabilityFlag::Multiline), false,
            inputMethod->languageCode());
    }

    bool invoke(Binding &binding) {
        clearOwnedPanel(binding);
        const auto context = currentContext(binding);
        if (!context) {
            binding.state.invalidateContext();
            return true;
        }
        const auto update = binding.state.updateContext(*context);
        if (!update) return true;
        transport_.connect();
        if (!open(binding)) return true;
        if (transport_.publishContext(*update)) {
            binding.brokerCoordinates = update->coordinates;
        }
        return true;
    }

    bool requestAcceptance(Binding &binding) {
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
        if (binding.opened) return true;
        if (authorityPaused_ || !transport_.ready()) return false;
        if (!transport_.openSession(binding.state.coordinates(),
                                    binding.state.appId(),
                                    binding.state.targetId())) {
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
            if (!binding.state.focused() || !open(binding)) continue;
            if (binding.state.lastContext() &&
                transport_.publishContext(*binding.state.lastContext())) {
                binding.brokerCoordinates =
                    binding.state.lastContext()->coordinates;
            }
        }
    }

    void onAuthority(const AuthoritySnapshot &snapshot) {
        authorityPaused_ = snapshot.paused;
        if (snapshot.initial) return;
        for (auto &[_, binding] : bindings_) {
            // Every non-initial authority epoch is a broker-side revocation
            // boundary: all prior sessions have already been retired. Sending
            // session.close for those coordinates would be both redundant and
            // invalid, and a strict transport must not queue a known error.
            binding.opened = false;
            binding.state.invalidateContext();
            clearOwnedPanel(binding);
        }
        if (!authorityPaused_) {
            for (auto &[_, binding] : bindings_) {
                if (binding.state.focused()) open(binding);
            }
        }
    }

    void onSuggestion(Suggestion suggestion) {
        auto *binding = bindingFor(suggestion.coordinates.sessionId);
        if (binding == nullptr || !binding->inputContext->hasFocus() ||
            hasForeignImeUi(observePanel(*binding)) ||
            !binding->state.showSuggestion(suggestion, transport_.nowMs())) {
            return;
        }
        auto list = std::make_unique<::fcitx::CommonCandidateList>();
        list->setPageSize(1);
        list->setLayoutHint(::fcitx::CandidateLayoutHint::Horizontal);
        const auto sessionId = suggestion.coordinates.sessionId;
        list->append(std::make_unique<BadiCandidate>(
            suggestion.text, [this, sessionId](::fcitx::InputContext *selected) {
                auto *current = bindingFor(sessionId);
                if (current != nullptr && current->inputContext == selected &&
                    selected->hasFocus()) {
                    requestAcceptance(*current);
                }
            }));
        auto &panel = binding->inputContext->inputPanel();
        panel.setCandidateList(std::move(list));
        binding->ownedCandidates = panel.candidateList();
        binding->inputContext->updateUserInterface(
            ::fcitx::UserInterfaceComponent::InputPanel);
    }

    void onClear(const ClearNotice &notice) {
        auto *binding = bindingFor(notice.coordinates.sessionId);
        if (binding != nullptr && binding->state.clearSuggestionIf(
                                      notice.coordinates, notice.suggestionId)) {
            clearOwnedPanel(*binding);
        }
    }

    void onCommitPrepare(const CommitPrepare &prepare) {
        auto *binding = bindingFor(prepare.coordinates.sessionId);
        if (binding == nullptr || !binding->inputContext->hasFocus() ||
            hasForeignImeUi(observePanel(*binding)) ||
            !binding->ownedCandidates) {
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
            binding->state.authorizeCommit(prepare, transport_.nowMs());
        if (!dispatch) {
            transport_.reportCommit(prepare.coordinates, prepare.controlId,
                                    prepare.suggestionId, "stale");
            return;
        }
        clearOwnedPanel(*binding);
        binding->inputContext->commitString(dispatch->text);
        transport_.reportCommit(dispatch->coordinates, dispatch->controlId,
                                dispatch->suggestionId,
                                "dispatched-unverified");
    }

    void onDisconnected() {
        authorityPaused_ = true;
        for (auto &[_, binding] : bindings_) {
            binding.opened = false;
            binding.state.invalidateContext();
            clearOwnedPanel(binding);
        }
    }

    ::fcitx::Instance *instance_;
    Transport transport_;
    std::vector<std::unique_ptr<::fcitx::HandlerTableEntry<::fcitx::EventHandler>>>
        handlers_;
    std::unordered_map<std::string, Binding> bindings_;
    bool authorityPaused_ = true;
};

class BadiModuleFactory final : public ::fcitx::AddonFactory {
public:
    ::fcitx::AddonInstance *create(::fcitx::AddonManager *manager) override {
        return new BadiAddon(manager->instance());
    }
};

} // namespace badi::fcitx5

FCITX_ADDON_FACTORY_V2(badi, badi::fcitx5::BadiModuleFactory)
