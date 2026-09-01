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
std::optional<std::string>
serializeSessionOpenEnvelope(const Coordinates &coordinates,
                             std::string_view appId,
                             std::string_view targetId,
                             std::uint64_t monoMs);
std::optional<std::string> serializeContextEnvelope(const ContextUpdate &update,
                                                    std::uint64_t monoMs);

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
};

bool dispatchSuggestionClear(
    const nlohmann::json &value,
    const std::function<void(const ClearNotice &)> &onClear);

struct AuthoritySnapshot {
    std::uint64_t authorityEpoch = 0;
    bool paused = true;
    bool initial = false;
};

struct WireCallbacks {
    std::function<void()> onReady;
    std::function<void(const AuthoritySnapshot &)> onAuthority;
    std::function<void(Suggestion)> onSuggestion;
    std::function<void(const ClearNotice &)> onClear;
    std::function<void(const CommitPrepare &)> onCommitPrepare;
    std::function<void()> onDisconnected;
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

    bool openSession(const Coordinates &coordinates, std::string_view appId,
                     std::string_view targetId);
    bool closeSession(const Coordinates &coordinates);
    bool publishContext(const ContextUpdate &update);
    bool requestAcceptance(const AcceptRequest &request);
    bool requestDismissal(const DismissRequest &request);
    bool reportCommit(const Coordinates &coordinates,
                      std::string_view controlId,
                      std::string_view suggestionId,
                      std::string_view status);

private:
    class Impl;
    std::unique_ptr<Impl> impl_;
};

} // namespace badi::fcitx5
