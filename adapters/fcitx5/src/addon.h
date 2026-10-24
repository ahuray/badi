#pragma once

// The Fcitx addon instance: one binding per input context, the broker
// transport and the accessibility observer. Method bodies live in
// addon.cpp (lifecycle), addon_keys.cpp, addon_observer.cpp,
// addon_broker.cpp (broker events and display), addon_session.cpp and
// addon_revision.cpp (selection revision).

#include "accessibility.h"
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
    struct Binding {
        ::fcitx::InputContext *inputContext = nullptr;
        SessionState state;
        BrokerSession broker;
        SurroundingFreshness surroundingFreshness;
        SurroundingEcho surroundingEcho;
        std::size_t surroundingDigest = 0;
        // Badi's own panel content, and the timer that retires it.
        std::shared_ptr<::fcitx::CandidateList> ownedCandidates;
        std::string ownedAuxiliary;
        bool overlayOwned = false;
        std::unique_ptr<::fcitx::EventSourceTime> expiryTimer;
        std::unique_ptr<::fcitx::EventSourceTime> commitTimer;
        // A suggestion waiting for the observer before display.
        std::optional<std::string> pendingSuggestion;
        // The observed field. Replies of an older generation are ignored.
        Json observedFocus;
        // Identity for a field visit, used only to preserve broker quieting.
        // Observation epochs and editing sessions still retire on every
        // invalidation; this contains no text or reusable editing grant.
        Json fieldVisit;
        std::string fieldVisitId;
        // The observer's window text for this field visit (ADR 0010): asked
        // for once per visit while the broker says context_words.
        std::string windowVisitId;
        std::string windowText;
        std::uint64_t observationGeneration = 0;
        bool observationExplicit = false;
        // The published request a broker clear would answer was explicit:
        // only then may a miss show its notice. Automatic misses (a typed
        // terminal line above all) stay quiet.
        bool requestExplicit = false;
        bool waitingForForeignUi = false;
        InspectionSchedule inspection;
        std::unique_ptr<::fcitx::EventSourceTime> observeTimer;
        // Escape declined this context; it is not requested automatically again.
        std::optional<ContextWindow> dismissedContext;
        // The last key typed the suggestion's next characters, or a word was
        // accepted: the broker may carry the remainder at once (ADR 0004), so
        // the next inspection skips most of the typing pause.
        bool typedThrough = false;
        // The suggestion text not yet typed, until it is shown again: keys
        // typed faster than the remainder returns still count as type-through.
        std::string typeThroughRest;
        // The field text the suggestion was shown for, plus the characters
        // typed since. A snapshot from before those characters land must not
        // ask the model; the broker can carry the remainder only once they
        // are in the field.
        std::string typeThroughBase;
        std::string typeThroughTyped;
        unsigned int typeThroughWaits = 0;
        // The pending rest of a word acceptance returns on its own (it was
        // accepted on an observed field or a typed line), so one Tab may be
        // held for it; see tabMayHoldForRest.
        bool typeThroughAutomatic = false;
        // At most one Tab held for that rest, and the timer that drops it.
        bool heldTab = false;
        std::string heldTabRest;
        std::unique_ptr<::fcitx::EventSourceTime> heldTabTimer;
        // Ctrl+Shift+Tab presses that reach the app as plain Tabs once the
        // chord's modifiers are up, and the timer that sends them.
        unsigned int literalTabs = 0;
        std::unique_ptr<::fcitx::EventSourceTime> literalTabTimer;
        // The chord's modifiers were state, not keys: no modifier release
        // will come, so the chord key's own release sends the Tab.
        bool literalTabOnKeyRelease = false;
        // The chord's Tab key in the sending keyboard's keymap, so the
        // frontend need not look Tab up under modifiers still in effect.
        int literalTabCode = 0;
        // Show a suggestion that arrives during the typing pause only after
        // that pause, and only if this keystroke is still the latest.
        std::uint64_t showAfterUs = 0;
        std::optional<Suggestion> heldSuggestion;
        std::unique_ptr<::fcitx::EventSourceTime> holdTimer;
        // A terminal's typed line and its typing-pause request.
        TypedLine typedLine;
        std::unique_ptr<::fcitx::EventSourceTime> lineTimer;
        // An IME-parity correction's expected text before the caret, until
        // the app's own text confirms it (ADR 0011), and the timer that
        // gives up on it.
        std::optional<std::string> correctionExpected;
        std::unique_ptr<::fcitx::EventSourceTime> correctionTimer;
        // An explicit revision on its way (ADR 0012): the selection and mode
        // waiting for the observer's field, its policy or its session.
        std::optional<std::pair<SelectionWindow, std::string>> revisionWaiting;
        // Retries of that request's observer steps while the observer is
        // busy, the step to retry, and the timer that runs it.
        unsigned int revisionRetries = 0;
        void (BadiAddon::*revisionStep)(Binding &) = nullptr;
        std::unique_ptr<::fcitx::EventSourceTime> revisionTimer;
        std::unique_ptr<::fcitx::EventSourceTime> revisionNoticeTimer;
        // An applied revision's selection and text, until the field reads so.
        std::optional<std::pair<SelectionWindow, std::string>> revisionExpected;
        std::unique_ptr<::fcitx::EventSourceTime> revisionReadBackTimer;

        [[nodiscard]] bool observed() const { return !observedFocus.is_null(); }
    };

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
    void dropHeldTab(Binding &binding);
    void clearTypeThrough(Binding &binding);
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
    void dismissSuggestion(Binding &binding, const PanelObservation &panel);
    void declineContext(Binding &binding);
    void lineKey(Binding &binding, const ::fcitx::Key &key, std::string_view typed);
    void requestTypedLine(Binding &binding, bool explicitRequest);
    void requestLineAfterPause(Binding &binding);

    // The accessibility observer.
    void observeLater(Binding &binding, bool explicitRequest = false, bool retry = false, bool early = false);
    void inspectField(Binding &binding);
    void inspected(Binding &binding, const Json &reply);
    void bindObservedField(Binding &binding, const Json &focus, NativeEditTarget editTarget);
    void denyEditing(Binding &binding);
    void observerUnavailable(Binding &binding, const Json &error = nullptr);
    void observerInvalidated(const Json &event);
    void retireObservedField(Binding &binding);
    void stopObserving(Binding &binding);
    void requestObservedContext(Binding &binding, bool rediscover = false);
    void publishObservedContext(Binding &binding, const Json &focus, const Json &reply, bool caretMayMove,
                                bool windowAsked);

    // Broker events.
    void queryFocusedPolicies();
    void onAuthority(const AuthoritySnapshot &snapshot);
    void onPolicy(std::string_view session, bool allowed, const std::string &reason, std::string language,
                  bool contextWords);
    void onSuggestion(Suggestion suggestion);
    void presentSuggestion(Binding &binding, Suggestion suggestion);
    void previewIfCorroborated(Binding &binding, const Suggestion &suggestion, const Json &focus, const Json &reply);
    void showObservedPreview(Binding &binding, const Suggestion &suggestion);
    void previewed(Binding &binding, const Suggestion &suggestion, const Json &focus, const Json &reply,
                   bool verified, bool rendered);
    void displaySuggestion(Suggestion suggestion, bool overlay = false);
    void showCandidate(Binding &binding, const Suggestion &suggestion);
    void onClear(const ClearNotice &clear);
    void onCommitPrepare(const CommitPrepare &prepare);
    void corroborateCommit(Binding &binding, const CommitPrepare &prepare, std::uint64_t deadline);
    void applyCommitPrepare(const CommitPrepare &prepare);
    void refuseCommit(const CommitPrepare &prepare, Binding *binding);
    void reportStale(const CommitPrepare &prepare);
    void onRefusal(const RequestRefusal &refusal);
    void onDisconnected();

    // Explicit revision of one selection (addon_revision.cpp).
    bool revisionActive(const Binding &binding) const;
    std::optional<SelectionWindow> currentSelection(const Binding &binding) const;
    bool revisionChord(Binding &binding);
    void startRevision(Binding &binding, SelectionWindow selection, std::string mode);
    bool retryRevision(Binding &binding, void (BadiAddon::*step)(Binding &));
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
    void readBackRevision(Binding &binding);
    bool revisionSurvives(Binding &binding);

    // The broker session.
    bool open(Binding &binding);
    void queryPolicy(Binding &binding);
    std::optional<Json> brokerTarget(const Binding &binding) const;
    void closeBrokerSession(Binding &binding);
    PanelObservation observePanel(const Binding &binding) const;
    bool foreignImeActive(const Binding &binding) const;
    RequestFacts requestFacts(const Binding &binding) const;
    ::fcitx::CapabilityFlags fieldCapabilities(const Binding &binding) const;
    bool contextReadable(const Binding &binding) const;
    bool safeToObserve(const Binding &binding) const;
    std::optional<ContextWindow> currentContext(const Binding &binding) const;
    std::optional<ContextWindow> currentContext(const Binding &binding, const Json &observed) const;
    std::string_view unavailableContextReason(const Binding &binding) const;
    void invalidateChangedContext(Binding &binding);
    void showNotice(Binding &binding, std::string_view message);
    void expectCorrection(Binding &binding, std::string expected);
    void readBackCorrection(Binding &binding);
    void clearOwnedPanel(Binding &binding);
    void removeOwnedPanel(Binding &binding);
    static std::optional<CaretRect> caretRect(const ::fcitx::InputContext &input);
    void hidePreview();
    void cancelForInput(Binding &binding);
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
