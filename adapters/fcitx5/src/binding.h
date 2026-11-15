#pragma once

// Per-input-context state of the Fcitx addon, factored into small resettable
// parts with distinct expiry timers (notices, suggestions, revisions, commit
// corroboration and post-commit read-backs) and one retire function for
// input invalidation, policy denial, authority change, broker disconnect,
// observed-field change, editing denial and focus reset.

#include "fcitx_event.h"
#include "observation.h"
#include "revision.h"
#include "state.h"

#include <fcitx-utils/key.h>
#include <fcitx/candidatelist.h>
#include <fcitx/inputcontext.h>

#include <cstddef>
#include <cstdint>
#include <memory>
#include <optional>
#include <string>
#include <string_view>
#include <utility>

namespace badi::fcitx5 {

struct SurroundingState {
    SurroundingFreshness freshness;
    SurroundingEcho echo;
    std::size_t digest = 0;

    void focusIn() {
        freshness.focusIn();
        echo.reset();
    }
    void focusOut() {
        freshness.focusOut();
        echo.reset();
    }
    void capabilityChanged() { freshness.capabilityChanged(); }
    void authorityChanged() { freshness.capabilityChanged(); }
    void transportLost() { freshness.transportLost(); }
};

enum class PanelOwner { None, Suggestion, Notice, Revision };

// Badi's own Fcitx InputPanel content and grey preview ownership.
// Tagging the active owner prevents a suggestion clear or refusal from
// wiping a live notice or selection revision, and vice versa.
struct PanelState {
    PanelOwner owner = PanelOwner::None;
    std::shared_ptr<::fcitx::CandidateList> candidates;
    std::string auxiliary;
    bool overlayOwned = false;

    [[nodiscard]] bool ownsSurface(std::optional<PanelOwner> matching = std::nullopt) const {
        if (matching && owner != *matching) return false;
        return overlayOwned || candidates != nullptr || !auxiliary.empty();
    }
    [[nodiscard]] bool ownsGhost(std::optional<PanelOwner> matching = std::nullopt) const {
        if (matching && owner != *matching) return false;
        return overlayOwned || candidates != nullptr;
    }
    // Removes Badi's candidates and auxiliary text from `inputContext` when
    // `owner` matches (`nullopt` clears any owner), leaving foreign IME UI
    // untouched. Returns true if `overlayOwned` was active and cleared.
    bool clear(::fcitx::InputContext *inputContext,
               std::optional<PanelOwner> matching = std::nullopt);
};

// Timed auxiliary notices (5s). Own their own expiry timer and generation so
// a notice never disarms or fires against a suggestion or revision lease.
struct NoticeState {
    std::uint64_t generation = 0;
    std::unique_ptr<::fcitx::EventSourceTime> expiryTimer;

    void clear() {
        ++generation;
        if (expiryTimer) expiryTimer->setEnabled(false);
    }
};

// Active, pending and typing-pause-held suggestions and alternative lists,
// with suggestion-only lease and commit-corroboration timers.
struct SuggestionState {
    // A suggestion waiting for the observer before display.
    std::optional<std::string> pendingId;
    // The published request a broker clear would answer was explicit:
    // only then may a miss show its notice. Automatic misses stay quiet.
    bool requestExplicit = false;
    // Escape declined this context; it is not requested automatically again.
    std::optional<ContextWindow> dismissedContext;
    // Show a suggestion that arrives during the typing pause only after
    // that pause, and only if this keystroke is still the latest.
    std::uint64_t showAfterUs = 0;
    std::optional<Suggestion> held;
    std::unique_ptr<::fcitx::EventSourceTime> holdTimer;
    std::unique_ptr<::fcitx::EventSourceTime> expiryTimer;
    std::unique_ptr<::fcitx::EventSourceTime> commitTimer;

    void clearHeld() {
        held.reset();
        if (holdTimer) holdTimer->setEnabled(false);
    }
    // Clears displayed/in-flight suggestion UI and timers while keeping the
    // current typing-pause gate (`showAfterUs`) and Escape decline.
    void clearDisplay() {
        pendingId.reset();
        clearHeld();
        if (expiryTimer) expiryTimer->setEnabled(false);
        if (commitTimer) commitTimer->setEnabled(false);
    }
    void reset(bool keepDismissed = false) {
        clearDisplay();
        requestExplicit = false;
        showAfterUs = 0;
        if (!keepDismissed) dismissedContext.reset();
    }
};

// Suggestion remainder carried across typing or word acceptance (ADR 0004),
// plus at most one Tab held while an automatic remainder returns.
struct TypeThroughState {
    bool active = false;
    bool automatic = false;
    std::string rest;
    std::string base;
    std::string typed;
    unsigned int waits = 0;
    bool heldTab = false;
    std::string heldTabRest;
    std::unique_ptr<::fcitx::EventSourceTime> heldTabTimer;

    void clearCarry() {
        active = false;
        automatic = false;
        rest.clear();
        base.clear();
        typed.clear();
        waits = 0;
    }
    void dropHeldTab() {
        heldTab = false;
        heldTabRest.clear();
        if (heldTabTimer) heldTabTimer->setEnabled(false);
    }
    void reset() {
        clearCarry();
        dropHeldTab();
    }
};

// Ctrl+Shift+Tab presses forwarded as plain Tabs once modifiers release.
struct LiteralTabState {
    unsigned int count = 0;
    bool onKeyRelease = false;
    int code = 0;
    std::unique_ptr<::fcitx::EventSourceTime> timer;

    void stopTimer() {
        if (timer) timer->setEnabled(false);
    }
    unsigned int take() {
        stopTimer();
        return std::exchange(count, 0U);
    }
    void reset() {
        stopTimer();
        count = 0;
        onKeyRelease = false;
        code = 0;
    }
};

// A terminal's tracked typed line and its typing-pause request timer (ADR 0009).
struct TerminalLineState {
    TypedLine line;
    std::unique_ptr<::fcitx::EventSourceTime> timer;

    void stopTimer() {
        if (timer) timer->setEnabled(false);
    }
    void clear() {
        stopTimer();
        line.clear();
    }
};

// Accessibility observer state for one input context, including field-visit
// quieting, window-context cache (ADR 0010), Zen post-commit tracking
// (ADR 0003), unnamed GNOME IBus gating (ADR 0017) and deferred broker start.
struct ObservationState {
    nlohmann::json focus;
    ZenPendingCommit zenCommit;
    nlohmann::json visit;
    std::string visitId;
    std::string windowVisitId;
    std::string windowText;
    std::uint64_t generation = 0;
    bool explicitRequest = false;
    bool waitingForForeignUi = false;
    bool unnamedIbus = false;
    bool pendingBrokerStart = false;
    InspectionSchedule schedule;
    std::unique_ptr<::fcitx::EventSourceTime> timer;

    [[nodiscard]] bool active() const { return !focus.is_null(); }
    void clearWindow() {
        windowVisitId.clear();
        windowText.clear();
    }
    void clearVisit() {
        visit = nullptr;
        visitId.clear();
        clearWindow();
    }
    void cancelPending() {
        ++generation;
        if (timer) timer->setEnabled(false);
    }
    void retireField() {
        focus = nullptr;
        explicitRequest = false;
        waitingForForeignUi = false;
    }
    void stop() {
        pendingBrokerStart = false;
        zenCommit.clear();
        clearVisit();
        cancelPending();
        retireField();
    }
};

enum class RevisionRetryStep { None, Inspect, Continue };

// Explicit selection revision state and timers (ADR 0012, ADR 0013).
struct RevisionState {
    std::optional<std::pair<SelectionWindow, std::string>> waiting;
    std::optional<std::pair<SelectionWindow, std::string>> cycle;
    unsigned int retries = 0;
    RevisionRetryStep retryStep = RevisionRetryStep::None;
    std::unique_ptr<::fcitx::EventSourceTime> retryTimer;
    std::unique_ptr<::fcitx::EventSourceTime> noticeTimer;
    std::unique_ptr<::fcitx::EventSourceTime> expiryTimer;
    std::unique_ptr<::fcitx::EventSourceTime> commitTimer;

    void clearActive() {
        waiting.reset();
        retries = 0;
        retryStep = RevisionRetryStep::None;
        if (retryTimer) retryTimer->setEnabled(false);
        if (noticeTimer) noticeTimer->setEnabled(false);
        if (expiryTimer) expiryTimer->setEnabled(false);
        if (commitTimer) commitTimer->setEnabled(false);
    }
    void reset() {
        clearActive();
        cycle.reset();
    }
};

// Post-commit read-back timers for IME-parity corrections (ADR 0011) and
// selection revisions (ADR 0012). Correction read-back outlives focus changes.
struct ReadBackState {
    std::optional<std::string> correction;
    std::unique_ptr<::fcitx::EventSourceTime> correctionTimer;
    std::optional<std::pair<SelectionWindow, std::string>> revision;
    std::unique_ptr<::fcitx::EventSourceTime> revisionTimer;

    void clearCorrection() {
        correction.reset();
        if (correctionTimer) correctionTimer->setEnabled(false);
    }
    void clearRevision() {
        revision.reset();
        if (revisionTimer) revisionTimer->setEnabled(false);
    }
    void reset() {
        clearCorrection();
        clearRevision();
    }
};

struct InputBinding {
    ::fcitx::InputContext *inputContext = nullptr;
    SessionState state;
    BrokerSession broker;
    SurroundingState surrounding;
    PanelState panel;
    NoticeState notice;
    SuggestionState suggestion;
    TypeThroughState typeThrough;
    LiteralTabState literalTab;
    TerminalLineState terminal;
    ObservationState observation;
    RevisionState revision;
    ReadBackState readBack;

    [[nodiscard]] bool observed() const { return observation.active(); }
    [[nodiscard]] bool observationCandidate() const {
        return observation.unnamedIbus
            ? (state.appId().empty() || unnamedIbusObservedApp(state.appId()))
            : nativeObservationAvailable(state.appId());
    }
    [[nodiscard]] bool revisionActive() const {
        return revision.waiting.has_value() || state.revisionRequest().has_value();
    }
    [[nodiscard]] bool noticeShown() const {
        return panel.owner == PanelOwner::Notice && !panel.auxiliary.empty();
    }
};

enum class RetireReason {
    InputChanged,
    PolicyDenied,
    AuthorityChanged,
    Disconnected,
    ObservedFieldChanged,
    EditingDenied,
    FocusReset,
};

struct RetireResult {
    bool ghostHidden = false;
    bool overlayHidden = false;
};

// Retires `binding` for `reason`. `contextId` names the Fcitx input context
// when rolling `SessionState` to a fresh manual or unnamed-IBus session.
RetireResult retireBinding(InputBinding &binding, RetireReason reason,
                           std::string_view contextId = {});

enum class RevisionPreKey {
    YieldForeignIme,
    KeepForModifier,
    FilterInvokeRepeat,
    PassInvokeToPostInput,
    AcceptTab,
    DismissEscape,
    RetireByKey,
};

RevisionPreKey classifyRevisionPreKey(const ::fcitx::Key &rawEventKey,
                                      bool unclaimed, bool foreignIme);

} // namespace badi::fcitx5
