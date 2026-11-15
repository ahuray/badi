#pragma once

// The Fcitx addon instance: one binding per input context, the broker
// transport and the accessibility observer. Per-context state and retirement
// live in binding.{h,cpp}; method bodies live in addon.cpp (lifecycle),
// addon_keys.cpp, addon_observer.cpp, addon_broker.cpp (broker events and
// display), addon_session.cpp and addon_revision.cpp (selection revision).

#include "accessibility.h"
#include "binding.h"
#include "debug.h"
#include "observation.h"
#include "sanitizer.h"
#include "state.h"
#include "transport.h"

#include <fcitx-utils/event.h>
#include <fcitx/addoninstance.h>
#include <fcitx/candidatelist.h>
#include <fcitx/event.h>
#include <fcitx/inputcontext.h>
#include <fcitx/instance.h>

#include <cstdint>
#include <functional>
#include <memory>
#include <optional>
#include <string>
#include <string_view>
#include <unordered_map>
#include <utility>
#include <vector>

#include "addon_support.h"

namespace badi::fcitx5 {

class BadiAddon final : public ::fcitx::AddonInstance {
public:
    explicit BadiAddon(::fcitx::Instance *instance);
    ~BadiAddon() override;

private:
    using Binding = InputBinding;

    static WireCallbacks callbacksFor(BadiAddon *addon);
    void watchInputContexts();
    void watch(::fcitx::EventType type, ::fcitx::EventWatcherPhase phase, ::fcitx::EventHandler handler);
    Binding *bindingFor(::fcitx::InputContext &inputContext);
    Binding *bindingFor(std::string_view sessionId);

    // Input context lifecycle.
    void focusIn(::fcitx::InputContext &inputContext);
    void focusOut(::fcitx::InputContext &inputContext);
    void destroy(::fcitx::InputContext &inputContext);
    void surroundingTextUpdated(::fcitx::InputContext &inputContext);
    void capabilityChanged(::fcitx::InputContext &inputContext, ::fcitx::CapabilityFlags before,
                           ::fcitx::CapabilityFlags after);
    void foreignUiUpdated(::fcitx::InputContext &input);

    // Keys.
    void preKeyEvent(::fcitx::KeyEvent &event);
    void holdTab(Binding &binding);
    void literalTab(Binding &binding, int code);
    void literalTabRelease(::fcitx::KeyEvent &event);
    void sendLiteralTabs(Binding &binding);
    void dropLiteralTabs(Binding &binding);
    void tabKey(::fcitx::KeyEvent &event, Binding &binding, const PanelObservation &panel);
    void keyEvent(::fcitx::KeyEvent &event);
    void invoke(Binding &binding);
    void inspectOnRequest(Binding &binding);
    void requestManually(Binding &binding);
    void requestAcceptance(Binding &binding, bool word = false);
    // True when the chord asked for another continuation than the shown one.
    bool requestAlternative(Binding &binding, const PanelObservation &panel);
    void dismissSuggestion(Binding &binding, const PanelObservation &panel);
    void declineContext(Binding &binding);
    void lineKey(Binding &binding, const ::fcitx::Key &key, std::string_view typed);
    void requestTypedLine(Binding &binding, bool explicitRequest);
    void requestLineAfterPause(Binding &binding);

    // The accessibility observer.
    void observeLater(Binding &binding, bool explicitRequest = false, bool retry = false, bool early = false);
    void inspectField(Binding &binding);
    void inspected(Binding &binding, const Json &reply);
    void bindObservedField(Binding &binding, const Json &focus, NativeEditTarget editTarget,
                           std::string_view resolvedAppId = {});
    void denyEditing(Binding &binding);
    void observerUnavailable(Binding &binding, const Json &error = nullptr);
    void observerInvalidated(const Json &event);
    void requestObservedContext(Binding &binding, bool rediscover = false);
    void publishObservedContext(Binding &binding, const Json &focus, const Json &reply, bool caretMayMove,
                                bool windowAsked);

    // Broker events.
    void queryFocusedPolicies();
    void onAuthority(const AuthoritySnapshot &snapshot);
    void onPolicy(std::string_view session, bool allowed, const std::string &reason, std::string language,
                  bool contextWords);
    void onSuggestion(Suggestion suggestion);
    void onAlternativeList(AlternativeList list);
    void presentSuggestion(Binding &binding, Suggestion suggestion);
    void previewIfCorroborated(Binding &binding, const Suggestion &suggestion, const Json &focus, const Json &reply);
    void showObservedPreview(Binding &binding, const Suggestion &suggestion);
    void previewed(Binding &binding, const Suggestion &suggestion, const Json &focus, const Json &reply,
                   bool verified, bool rendered);
    void displaySuggestion(Suggestion suggestion, bool overlay = false);
    void showCandidate(Binding &binding, const Suggestion &suggestion);
    void showAlternativeCandidates(Binding &binding, const AlternativeList &list);
    bool requestAlternativePick(Binding &binding, std::string word, const PanelObservation &panel);
    void onClear(const ClearNotice &clear);
    void onCommitPrepare(const CommitPrepare &prepare);
    void corroborateCommit(Binding &binding, const CommitPrepare &prepare, std::uint64_t deadline);
    void applyCommitPrepare(const CommitPrepare &prepare);
    void refuseCommit(const CommitPrepare &prepare, Binding *binding);
    void reportStale(const CommitPrepare &prepare);
    void onRefusal(const RequestRefusal &refusal);
    void onDisconnected();

    // Explicit revision of one selection (addon_revision.cpp).
    std::optional<SelectionWindow> currentSelection(const Binding &binding) const;
    bool revisionChord(Binding &binding);
    void startRevision(Binding &binding, SelectionWindow selection, std::string mode);
    bool retryRevision(Binding &binding, RevisionRetryStep step);
    void inspectForRevision(Binding &binding);
    void revisionInspected(Binding &binding, const Json &reply);
    void revisionByManualContract(Binding &binding);
    void continueRevision(Binding &binding);
    void sendRevision(Binding &binding, const std::optional<Json> &observed);
    void endRevision(Binding &binding, std::string_view message, std::string_view reason);
    void onRevision(Revision revision);
    void showRevisionCandidate(Binding &binding, const Revision &revision);
    void showRevisionNotice(Binding &binding, std::string_view message);
    void onRevisionClear(const RevisionClear &clear);
    void acceptRevision(Binding &binding);
    void cancelRevision(Binding &binding);
    bool revisionKey(::fcitx::KeyEvent &event, Binding &binding);
    bool revisionCommit(const CommitPrepare &prepare);
    void corroborateRevisionCommit(Binding &binding, const CommitPrepare &prepare, std::uint64_t deadline);
    void applyRevision(Binding &binding, const CommitPrepare &prepare);
    void refuseRevisionCommit(const CommitPrepare &prepare, Binding *binding);
    void readBackRevision(Binding &binding);
    bool revisionSurvives(Binding &binding);

    // The broker session, panel ownership and unified retirement.
    bool open(Binding &binding);
    // Asks systemd to start the model, then attaches. Login and focus only
    // attach, so a down model stays down until somebody types or asks.
    void ensureBroker();
    void queryPolicy(Binding &binding);
    std::optional<Json> brokerTarget(const Binding &binding) const;
    void closeBrokerSession(Binding &binding);
    void retire(Binding &binding, RetireReason reason);
    PanelObservation observePanel(const Binding &binding) const;
    bool foreignImeActive(const Binding &binding) const;
    RequestFacts requestFacts(const Binding &binding) const;
    ::fcitx::CapabilityFlags fieldCapabilities(const Binding &binding) const;
    bool contextReadable(const Binding &binding) const;
    bool safeToObserve(const Binding &binding) const;
    std::optional<ContextWindow> currentContext(const Binding &binding) const;
    std::optional<ContextWindow> rawContext(const Binding &binding, const Json &observed) const;
    std::string_view unavailableContextReason(const Binding &binding) const;
    void invalidateChangedContext(Binding &binding);
    void showNotice(Binding &binding, std::string_view message);
    void expectCorrection(Binding &binding, std::string expected);
    void readBackCorrection(Binding &binding);
    void clearSuggestionUi(Binding &binding);
    void clearNotice(Binding &binding);
    void clearRevisionUi(Binding &binding);
    void clearAllUi(Binding &binding);
    static std::optional<CaretRect> caretRect(const ::fcitx::InputContext &input);
    void hidePreview();
    std::unique_ptr<::fcitx::EventSourceTime> oneShot(std::uint64_t delayUs, std::function<void()> action);

    ::fcitx::Instance *instance_;
    ActivityDebug debug_;
    Transport transport_;
    std::unique_ptr<Accessibility> accessibility_;
    std::vector<std::unique_ptr<::fcitx::HandlerTableEntry<::fcitx::EventHandler>>> handlers_;
    std::unordered_map<std::string, Binding> bindings_;
    AuthorityContinuity authorityContinuity_;
    bool authorityPaused_ = true;
    // Modifiers held as keys (a physical keyboard's Ctrl, Shift, Alt, Super).
    // A virtual keyboard may set modifiers as state alone, without key events.
    ::fcitx::KeyStates heldModifierKeys_;
    bool previewMayBeVisible_ = false;
};

} // namespace badi::fcitx5
