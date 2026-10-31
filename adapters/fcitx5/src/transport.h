#pragma once

#include "state.h"

#include <nlohmann/json_fwd.hpp>

#include <cstddef>
#include <cstdint>
#include <functional>
#include <memory>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <vector>

namespace fcitx {
class EventLoop;
}

namespace badi::fcitx5 {

constexpr std::size_t kMaxFrameBytes = 65'536;

std::optional<std::vector<std::uint8_t>> encodeFrame(std::string_view body);
bool strictBoundedJsonObject(std::string_view body);
bool strictSessionControlResult(std::string_view body);
bool strictSuggestionClear(std::string_view body);
bool strictPolicyStatus(std::string_view body);
// The broker's `error` frame refusing one request: the id that request was
// sent with and the reason code. The connection stays open.
struct RequestRefusal {
    std::string requestId;
    std::string reason;
};
// Exactly v, id, type "error", mono_ms and a payload of only a known `code`;
// any other shape closes the connection.
std::optional<RequestRefusal> strictRequestRefusal(std::string_view body);
// The broker target of a manual field: the canonical app id and the opaque
// Fcitx context id. An observed field uses the observer's inspect target.
std::optional<nlohmann::json> desktopApplicationTarget(std::string_view appId,
                                                       std::string_view targetId);
std::optional<std::string>
serializeSessionOpenEnvelope(const Coordinates &coordinates,
                             const nlohmann::json &target,
                             std::uint64_t monoMs);
std::optional<std::string> serializeContextEnvelope(const ContextUpdate &update,
                                                    std::uint64_t monoMs);
// A revision request (ADR 0012), or nullopt when the selection is not one the
// broker would accept.
std::optional<std::string> serializeRevisionEnvelope(const RevisionUpdate &update, std::uint64_t monoMs);

class FrameDecoder {
public:
    bool feed(std::span<const std::uint8_t> bytes);
    std::vector<std::string> takeFrames();
    [[nodiscard]] bool failed() const { return failed_; }

private:
    std::vector<std::uint8_t> pending_;
    std::vector<std::string> frames_;
    bool failed_ = false;
};

struct ClearNotice {
    Coordinates coordinates;
    std::optional<std::string> suggestionId;
    std::string reason;
};

bool dispatchSuggestionClear(
    const nlohmann::json &value,
    const std::function<void(const ClearNotice &)> &onClear);

struct AuthoritySnapshot {
    std::uint64_t authorityEpoch = 0;
    std::uint64_t settingsRevision = 0;
    bool paused = true;
    bool initial = false;
};

// Each connection starts with an initial snapshot. Returns true when a
// snapshot carries authority the adapter has not observed: every later epoch
// on a connection, or a reconnect whose initial authority differs from the
// last one observed on an earlier connection.
class AuthorityContinuity {
public:
    bool observe(const AuthoritySnapshot &snapshot);

private:
    std::optional<AuthoritySnapshot> observed_;
};

struct WireCallbacks {
    std::function<void()> onReady;
    std::function<void(const AuthoritySnapshot &)> onAuthority;
    std::function<void(Suggestion)> onSuggestion;
    // Alternative next words for a numbered list (ADR 0014); adapters that
    // leave this empty never negotiate alternative_list.
    std::function<void(AlternativeList)> onAlternativeList = {};
    std::function<void(const ClearNotice &)> onClear;
    std::function<void(const CommitPrepare &)> onCommitPrepare;
    std::function<void()> onDisconnected;
    // Session, allowed, the broker's resolution reason (matched_default marks
    // an allowance from the app list mode, not an exact rule), language
    // override, and whether context is on there (context_words).
    std::function<void(std::string_view, bool, std::string, std::string, bool)> onPolicy;
    // Explicit revision of one selection (ADR 0012); adapters that leave
    // these empty never ask for one.
    std::function<void(Revision)> onRevision = {};
    std::function<void(const RevisionClear &)> onRevisionClear = {};
    // A request the broker refused (an `error` frame), by the id it was sent
    // with. Ids are unique per session, not across sessions.
    std::function<void(const RequestRefusal &)> onRefusal = {};
};

class Transport {
public:
    Transport(::fcitx::EventLoop &eventLoop, WireCallbacks callbacks,
              std::string socketPath = {});
    ~Transport();
    Transport(const Transport &) = delete;
    Transport &operator=(const Transport &) = delete;

    bool connect();
    void disconnect();
    [[nodiscard]] bool ready() const;
    [[nodiscard]] std::uint64_t nowMs() const;

    bool queryPolicy(const Coordinates &coordinates, const nlohmann::json &target);
    bool openSession(const Coordinates &coordinates, const nlohmann::json &target);
    bool closeSession(const Coordinates &coordinates);
    bool publishContext(const ContextUpdate &update);
    bool requestRevision(const RevisionUpdate &update);
    // Ends the broker's work on the request at `coordinates` (a revision
    // still running).
    bool cancelRequest(const Coordinates &coordinates);
    bool requestAcceptance(const AcceptRequest &request);
    bool requestDismissal(const DismissRequest &request);
    bool requestAlternative(const DismissRequest &request);
    bool requestAlternativePick(const AlternativePickRequest &request);
    bool reportCommit(const Coordinates &coordinates,
                      std::string_view controlId,
                      std::string_view suggestionId,
                      std::string_view status);

private:
    class Impl;
    std::unique_ptr<Impl> impl_;
};

} // namespace badi::fcitx5
