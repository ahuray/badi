#include "transport.h"

#include "sanitizer.h"

#include <fcitx-utils/event.h>
#include <fcitx-utils/eventloopinterface.h>
#include <fcitx-utils/log.h>
#include <nlohmann/json.hpp>

#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

#include <array>
#include <algorithm>
#include <cerrno>
#include <chrono>
#include <cstring>
#include <deque>
#include <filesystem>
#include <limits>
#include <unordered_set>
#include <utility>

namespace badi::fcitx5 {
namespace {

using Json = nlohmann::json;
constexpr std::size_t kMaxQueuedFrames = 32;
constexpr std::size_t kMaxQueuedBytes = 1U << 20U;
constexpr std::size_t kMaxDecodedFrames = 32;
constexpr std::uint64_t kMaxSafeCounter = (std::uint64_t{1} << 53U) - 1U;
constexpr std::array<std::string_view, 5> kRequiredCapabilities{
    "context", "suggestion", "commit.dispatched_unverified", "control", "policy"};

bool exactKeys(const Json &value,
               std::initializer_list<std::string_view> keys) {
    if (!value.is_object() || value.size() != keys.size()) return false;
    for (const auto key : keys) {
        if (!value.contains(std::string(key))) return false;
    }
    return true;
}

bool counter(const Json &value) {
    return value.is_number_unsigned() && value.get<std::uint64_t>() <= kMaxSafeCounter;
}

bool validFingerprint(const Json &value) {
    return value.is_string() && value.get_ref<const std::string &>().size() >= 16 &&
           validOpaqueId(value.get_ref<const std::string &>());
}

bool exactCapabilities(const Json &value) {
    if (!value.is_array() || value.size() != kRequiredCapabilities.size()) return false;
    for (const auto capability : kRequiredCapabilities) {
        const auto matches = std::count_if(
            value.begin(), value.end(), [capability](const Json &candidate) {
                return candidate.is_string() && candidate == capability;
            });
        if (matches != 1) return false;
    }
    return true;
}

bool validReason(const Json &value) {
    static constexpr std::array reasons{
        "accepted", "ambiguous_session", "cancelled", "dismissed", "expired",
        "field_ambiguous", "field_not_editable", "field_sensitive", "focus_changed",
        "invalid_capability", "invalid_frame", "invalid_message", "invalid_output",
        "manual_required", "no_context", "no_suggestion", "paused", "policy_never",
        "provider_error", "provider_timeout", "session_closed", "settings_commit_unknown",
        "settings_committed_degraded", "settings_conflict", "settings_rejected", "stale",
        "superseded", "unknown_session", "unsupported_version"};
    return value.is_string() &&
           std::find(reasons.begin(), reasons.end(), value.get<std::string>()) !=
               reasons.end();
}

bool sessionControlResult(const Json &value, const Json &payload) {
    const bool knownAction = payload.contains("action") &&
                             (payload["action"] == "accept_all" ||
                              payload["action"] == "dismiss");
    return exactKeys(value, {"v", "id", "type", "mono_ms", "payload"}) &&
           value["v"] == 2 && value["type"] == "control.result" &&
           value["id"].is_string() &&
           validOpaqueId(value["id"].get_ref<const std::string &>()) &&
           counter(value["mono_ms"]) &&
           exactKeys(payload, {"action", "accepted", "reason", "paused"}) &&
           knownAction && payload["accepted"] == true &&
           payload["reason"] == "accepted" && payload["paused"].is_boolean();
}

std::optional<Coordinates> parseCoordinates(const Json &value,
                                            const Json &payload);

bool suggestionClear(const Json &value, const Json &payload) {
    const bool exactPayload =
        exactKeys(payload, {"fingerprint", "reason"}) ||
        exactKeys(payload, {"fingerprint", "suggestion_id", "reason"});
    const bool validSuggestionId =
        !payload.contains("suggestion_id") ||
        (payload["suggestion_id"].is_string() &&
         validOpaqueId(
             payload["suggestion_id"].get_ref<const std::string &>()));
    return exactKeys(value, {"v", "id", "type", "session_id",
                             "focus_epoch", "revision", "mono_ms", "payload"}) &&
           value["v"] == 2 && value["type"] == "suggestion.clear" &&
           value["id"].is_string() &&
           validOpaqueId(value["id"].get_ref<const std::string &>()) &&
           counter(value["mono_ms"]) && exactPayload && validSuggestionId &&
           validReason(payload["reason"]) && parseCoordinates(value, payload);
}

std::optional<Coordinates> parseCoordinates(const Json &value,
                                            const Json &payload) {
    if (!value.contains("session_id") || !value["session_id"].is_string() ||
        !validSessionId(value["session_id"].get_ref<const std::string &>()) ||
        !value.contains("focus_epoch") || !counter(value["focus_epoch"]) ||
        !value.contains("revision") || !counter(value["revision"]) ||
        !payload.contains("fingerprint") || !validFingerprint(payload["fingerprint"])) {
        return std::nullopt;
    }
    return Coordinates{
        .sessionId = value["session_id"].get<std::string>(),
        .focusEpoch = value["focus_epoch"].get<std::uint64_t>(),
        .revision = value["revision"].get<std::uint64_t>(),
        .fingerprint = payload["fingerprint"].get<std::string>(),
    };
}

std::optional<ClearNotice> parseClearNotice(const Json &value) {
    if (!value.contains("payload") || !value["payload"].is_object()) {
        return std::nullopt;
    }
    const auto &payload = value["payload"];
    if (!suggestionClear(value, payload)) return std::nullopt;
    const auto coordinates = parseCoordinates(value, payload);
    if (!coordinates) return std::nullopt;

    std::optional<std::string> suggestionId;
    const auto suggestion = payload.find("suggestion_id");
    if (suggestion != payload.end()) {
        suggestionId = suggestion->get<std::string>();
    }
    return ClearNotice{
        .coordinates = *coordinates,
        .suggestionId = std::move(suggestionId),
    };
}

std::optional<std::string> defaultSocketPath() {
    const auto *runtime = std::getenv("XDG_RUNTIME_DIR");
    if (runtime == nullptr || runtime[0] != '/') return std::nullopt;
    const std::filesystem::path root(runtime);
    if (!root.is_absolute() || root.lexically_normal() != root) return std::nullopt;
    return (root / "badi" / "broker.sock").string();
}

bool safeSocket(const std::string &path) {
    struct stat metadata {};
    return ::lstat(path.c_str(), &metadata) == 0 && S_ISSOCK(metadata.st_mode) &&
           metadata.st_uid == ::getuid() && (metadata.st_mode & 0777U) == 0600U;
}

Json envelope(std::string_view type, std::optional<std::string> id,
              std::uint64_t monoMs, const Json &payload,
              const Coordinates *coordinates = nullptr) {
    Json result{{"v", 2}, {"type", type}, {"mono_ms", monoMs}, {"payload", payload}};
    if (id) result["id"] = std::move(*id);
    if (coordinates != nullptr) {
        result["session_id"] = coordinates->sessionId;
        result["focus_epoch"] = coordinates->focusEpoch;
        result["revision"] = coordinates->revision;
    }
    return result;
}

std::optional<Json> parseStrictObject(std::string_view body) {
    if (body.empty() || body.size() > kMaxFrameBytes) return std::nullopt;
    bool duplicateKey = false;
    std::vector<std::unordered_set<std::string>> objectKeys;
    const auto callback = [&duplicateKey, &objectKeys](
                              int, Json::parse_event_t event, Json &parsed) {
        if (event == Json::parse_event_t::object_start) {
            objectKeys.emplace_back();
        } else if (event == Json::parse_event_t::key) {
            if (objectKeys.empty() ||
                !objectKeys.back().insert(parsed.get<std::string>()).second) {
                duplicateKey = true;
            }
        } else if (event == Json::parse_event_t::object_end) {
            if (objectKeys.empty()) {
                duplicateKey = true;
            } else {
                objectKeys.pop_back();
            }
        }
        return true;
    };
    auto value = Json::parse(body, callback, false, false);
    if (duplicateKey || !objectKeys.empty() || value.is_discarded() ||
        !value.is_object()) {
        return std::nullopt;
    }
    return value;
}

std::optional<Json> contextEnvelope(const ContextUpdate &update,
                                    std::uint64_t monoMs) {
    const auto &coordinates = update.coordinates;
    const auto &context = update.context;
    const auto before = decodeUtf8(context.before);
    const auto after = decodeUtf8(context.after);
    if (!validLinuxAppId(update.appId) || !validOpaqueId(update.targetId) ||
        !validSessionId(coordinates.sessionId) ||
        coordinates.fingerprint.size() < 16 ||
        !validOpaqueId(coordinates.fingerprint) || context.sensitive ||
        context.composing || context.anchor != context.head ||
        !validLanguageTag(context.language) ||
        !before || before->size() > kMaxBeforeScalars || !after ||
        after->size() > kMaxAfterScalars) {
        return std::nullopt;
    }
    const Json payload{
        {"fingerprint", coordinates.fingerprint},
        {"before", context.before},
        {"after", context.after},
        {"selection", Json{{"anchor", context.anchor},
                            {"head", context.head},
                            {"unit", "unicode_scalar_values"}}},
        {"field", Json{{"purpose", "unknown"},
                        {"editable", true},
                        {"multiline", context.multiline},
                        {"composing", false},
                        {"sensitive", false},
                        {"identity_known", false},
                        {"focused", true},
                        {"lock_screen", false}}},
        {"activation", "manual"},
        {"explicit", true},
        {"language", context.language},
    };
    return envelope(
        "context.changed",
        "fcitx.suggest." + std::to_string(coordinates.focusEpoch) + "." +
            std::to_string(coordinates.revision) + ".context",
        monoMs, payload, &coordinates);
}

} // namespace

bool strictBoundedJsonObject(std::string_view body) {
    return parseStrictObject(body).has_value();
}

bool strictSessionControlResult(std::string_view body) {
    const auto value = parseStrictObject(body);
    return value && value->contains("payload") && (*value)["payload"].is_object() &&
           sessionControlResult(*value, (*value)["payload"]);
}

bool strictSuggestionClear(std::string_view body) {
    const auto value = parseStrictObject(body);
    return value && parseClearNotice(*value).has_value();
}

bool dispatchSuggestionClear(
    const nlohmann::json &value,
    const std::function<void(const ClearNotice &)> &onClear) {
    const auto notice = parseClearNotice(value);
    if (!notice) return false;
    if (onClear) onClear(*notice);
    return true;
}

std::optional<std::string> serializeContextEnvelope(const ContextUpdate &update,
                                                    std::uint64_t monoMs) {
    const auto message = contextEnvelope(update, monoMs);
    if (!message) return std::nullopt;
    try {
        const auto body = message->dump();
        if (body.empty() || body.size() > kMaxFrameBytes) return std::nullopt;
        return body;
    } catch (...) {
        return std::nullopt;
    }
}

std::optional<std::string>
serializeSessionOpenEnvelope(const Coordinates &coordinates,
                             std::string_view appId,
                             std::string_view targetId,
                             std::uint64_t monoMs) {
    if (!validSessionId(coordinates.sessionId) ||
        coordinates.focusEpoch > kMaxSafeCounter || !validLinuxAppId(appId) ||
        !validOpaqueId(targetId)) {
        return std::nullopt;
    }
    auto opening = coordinates;
    opening.revision = 0;
    opening.fingerprint.clear();
    const auto message = envelope(
        "session.open",
        "fcitx.open." + std::to_string(coordinates.focusEpoch), monoMs,
        Json{{"target", Json{{"kind", "desktop_application"},
                               {"app_id", appId},
                               {"target_id", targetId}}},
             {"activation", "always"}},
        &opening);
    try {
        const auto body = message.dump();
        if (body.empty() || body.size() > kMaxFrameBytes) return std::nullopt;
        return body;
    } catch (...) {
        return std::nullopt;
    }
}

std::optional<std::vector<std::uint8_t>> encodeFrame(std::string_view body) {
    if (body.empty() || body.size() > kMaxFrameBytes) return std::nullopt;
    const auto length = static_cast<std::uint32_t>(body.size());
    std::vector<std::uint8_t> frame(4 + body.size());
    frame[0] = static_cast<std::uint8_t>(length & 0xffU);
    frame[1] = static_cast<std::uint8_t>((length >> 8U) & 0xffU);
    frame[2] = static_cast<std::uint8_t>((length >> 16U) & 0xffU);
    frame[3] = static_cast<std::uint8_t>((length >> 24U) & 0xffU);
    std::memcpy(frame.data() + 4, body.data(), body.size());
    return frame;
}

bool FrameDecoder::feed(std::span<const std::uint8_t> bytes) {
    if (failed_ || pending_.size() + bytes.size() > kMaxFrameBytes + 4U) {
        failed_ = true;
        return false;
    }
    pending_.insert(pending_.end(), bytes.begin(), bytes.end());
    while (pending_.size() >= 4) {
        const auto length = static_cast<std::uint32_t>(pending_[0]) |
                            (static_cast<std::uint32_t>(pending_[1]) << 8U) |
                            (static_cast<std::uint32_t>(pending_[2]) << 16U) |
                            (static_cast<std::uint32_t>(pending_[3]) << 24U);
        if (length == 0 || length > kMaxFrameBytes ||
            frames_.size() >= kMaxDecodedFrames) {
            failed_ = true;
            return false;
        }
        if (pending_.size() < 4U + length) break;
        frames_.emplace_back(reinterpret_cast<const char *>(pending_.data() + 4),
                             length);
        pending_.erase(pending_.begin(), pending_.begin() + 4U + length);
    }
    return true;
}

std::vector<std::string> FrameDecoder::takeFrames() {
    auto result = std::move(frames_);
    frames_.clear();
    return result;
}

class Transport::Impl {
public:
    Impl(::fcitx::EventLoop &eventLoop, WireCallbacks callbacks,
         std::string socketPath)
        : eventLoop_(eventLoop), callbacks_(std::move(callbacks)),
          socketPath_(std::move(socketPath)), started_(Clock::now()) {}

    ~Impl() { close(false); }

    bool connectSocket() {
        if (fd_ >= 0) return true;
        if (socketPath_.empty()) {
            const auto path = defaultSocketPath();
            if (!path) return false;
            socketPath_ = *path;
        }
        if (!safeSocket(socketPath_) || socketPath_.size() >= sizeof(sockaddr_un::sun_path)) {
            return false;
        }
        fd_ = ::socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0);
        if (fd_ < 0) return false;
        sockaddr_un address{};
        address.sun_family = AF_UNIX;
        std::memcpy(address.sun_path, socketPath_.c_str(), socketPath_.size() + 1);
        const auto result = ::connect(fd_, reinterpret_cast<sockaddr *>(&address),
                                      sizeof(address));
        if (result != 0 && errno != EINPROGRESS) {
            close(false);
            return false;
        }
        connecting_ = result != 0;
        io_ = eventLoop_.addIOEvent(
            fd_, ioFlags(),
            [this](::fcitx::EventSourceIO *, int, ::fcitx::IOEventFlags flags) {
                return onIo(flags);
            });
        if (!connecting_ && !finishConnect()) return false;
        return true;
    }

    void close(bool notify) {
        const bool active = fd_ >= 0;
        if (io_) io_->setEnabled(false);
        io_.reset();
        if (fd_ >= 0) ::close(std::exchange(fd_, -1));
        connecting_ = false;
        helloAcknowledged_ = false;
        authoritySeen_ = false;
        ready_ = false;
        decoder_ = FrameDecoder{};
        writes_.clear();
        writeOffset_ = 0;
        queuedBytes_ = 0;
        if (active && notify) {
            FCITX_WARN() << "Badi broker transport disconnected";
            if (callbacks_.onDisconnected) callbacks_.onDisconnected();
        }
    }

    bool queue(const Json &message) {
        std::string body;
        try {
            body = message.dump();
        } catch (...) {
            return false;
        }
        return queueBody(body);
    }

    bool queueBody(std::string_view body) {
        auto frame = encodeFrame(body);
        if (!frame || writes_.size() >= kMaxQueuedFrames ||
            queuedBytes_ + frame->size() > kMaxQueuedBytes) {
            close(true);
            return false;
        }
        queuedBytes_ += frame->size();
        writes_.push_back(std::move(*frame));
        updateEvents();
        return true;
    }

    [[nodiscard]] std::uint64_t nowMs() const {
        return static_cast<std::uint64_t>(
            std::chrono::duration_cast<std::chrono::milliseconds>(Clock::now() - started_)
                .count());
    }

    [[nodiscard]] bool ready() const { return ready_ && fd_ >= 0; }

    bool openSession(const Coordinates &coordinates, std::string_view appId,
                     std::string_view targetId) {
        if (!ready() || !validLinuxAppId(appId) || !validOpaqueId(targetId)) return false;
        const auto body =
            serializeSessionOpenEnvelope(coordinates, appId, targetId, nowMs());
        return body && queueBody(*body);
    }

    bool closeSession(const Coordinates &coordinates) {
        if (!ready()) return false;
        return queue(envelope(
            "session.close",
            "fcitx.close." + std::to_string(coordinates.focusEpoch), nowMs(),
            Json{{"reason", "session_closed"}}, &coordinates));
    }

    bool publishContext(const ContextUpdate &update) {
        if (!ready() || !validLinuxAppId(update.appId) ||
            !validOpaqueId(update.targetId) || update.context.sensitive ||
            update.context.composing ||
            update.context.anchor != update.context.head) {
            return false;
        }
        const auto &coordinates = update.coordinates;
        const auto &context = update.context;
        const auto requestId = "fcitx.suggest." +
                               std::to_string(coordinates.focusEpoch) + "." +
                               std::to_string(coordinates.revision);
        const auto serialized = serializeContextEnvelope(update, nowMs());
        if (!serialized || !queueBody(*serialized)) {
            return false;
        }
        if (context.sensitive || context.composing || context.anchor != context.head) {
            return true;
        }
        return queue(envelope(
            "suggest.request", requestId, nowMs(),
            Json{{"fingerprint", coordinates.fingerprint}, {"explicit", true}},
            &coordinates));
    }

    bool requestAcceptance(const AcceptRequest &request) {
        if (!ready()) return false;
        return queue(envelope(
            "control.request", request.controlId, nowMs(),
            Json{{"action", "accept_all"},
                 {"fingerprint", request.coordinates.fingerprint},
                 {"suggestion_id", request.suggestionId}},
            &request.coordinates));
    }

    bool requestDismissal(const DismissRequest &request) {
        if (!ready()) return false;
        return queue(envelope(
            "control.request", request.controlId, nowMs(),
            Json{{"action", "dismiss"},
                 {"fingerprint", request.coordinates.fingerprint},
                 {"suggestion_id", request.suggestionId}},
            &request.coordinates));
    }

    bool reportCommit(const Coordinates &coordinates, std::string_view controlId,
                      std::string_view suggestionId, std::string_view status) {
        if (!ready() || (status != "dispatched-unverified" && status != "stale" &&
                         status != "blocked" && status != "failed")) {
            return false;
        }
        return queue(envelope(
            "commit.result", std::string(controlId), nowMs(),
            Json{{"fingerprint", coordinates.fingerprint},
                 {"suggestion_id", suggestionId},
                 {"status", status}},
            &coordinates));
    }

private:
    using Clock = std::chrono::steady_clock;

    ::fcitx::IOEventFlags ioFlags() const {
        ::fcitx::IOEventFlags flags{::fcitx::IOEventFlag::In};
        flags |= ::fcitx::IOEventFlag::Err;
        flags |= ::fcitx::IOEventFlag::Hup;
        if (connecting_ || !writes_.empty()) flags |= ::fcitx::IOEventFlag::Out;
        return flags;
    }

    void updateEvents() {
        if (io_) io_->setEvents(ioFlags());
    }

    bool finishConnect() {
        int error = 0;
        socklen_t errorSize = sizeof(error);
        if (::getsockopt(fd_, SOL_SOCKET, SO_ERROR, &error, &errorSize) != 0 ||
            error != 0) {
            close(true);
            return false;
        }
#ifdef SO_PEERCRED
        ucred credentials{};
        socklen_t credentialSize = sizeof(credentials);
        if (::getsockopt(fd_, SOL_SOCKET, SO_PEERCRED, &credentials,
                         &credentialSize) != 0 || credentials.uid != ::getuid()) {
            close(true);
            return false;
        }
#else
        close(true);
        return false;
#endif
        connecting_ = false;
        const Json helloPayload{
            {"min_v", 2},
            {"max_v", 2},
            {"adapter", Json{{"kind", "fcitx"},
                              {"name", "badi-fcitx5"},
                              {"version", "0.1.0"}}},
            {"capabilities", Json::array({"context", "suggestion",
                                           "commit.dispatched_unverified",
                                           "control", "policy"})},
        };
        if (!queue(envelope("hello", "fcitx.hello", nowMs(), helloPayload))) {
            return false;
        }
        updateEvents();
        return true;
    }

    bool onIo(::fcitx::IOEventFlags flags) {
        if (connecting_ &&
            (!!(flags & ::fcitx::IOEventFlag::Out) ||
             !!(flags & ::fcitx::IOEventFlag::Err) ||
             !!(flags & ::fcitx::IOEventFlag::Hup)) &&
            !finishConnect()) {
            return true;
        }
        if (fd_ < 0) return true;
        if (!!(flags & ::fcitx::IOEventFlag::Out) && !flushWrites()) return true;
        if (fd_ >= 0 && !!(flags & ::fcitx::IOEventFlag::In) && !readFrames()) return true;
        if (fd_ >= 0 && (!!(flags & ::fcitx::IOEventFlag::Err) ||
                         !!(flags & ::fcitx::IOEventFlag::Hup))) {
            close(true);
        }
        return true;
    }

    bool flushWrites() {
        while (!writes_.empty()) {
            auto &frame = writes_.front();
            const auto written = ::send(fd_, frame.data() + writeOffset_,
                                        frame.size() - writeOffset_, MSG_NOSIGNAL);
            if (written < 0) {
                if (errno == EAGAIN || errno == EWOULDBLOCK) break;
                close(true);
                return false;
            }
            if (written == 0) {
                close(true);
                return false;
            }
            writeOffset_ += static_cast<std::size_t>(written);
            if (writeOffset_ == frame.size()) {
                queuedBytes_ -= frame.size();
                writes_.pop_front();
                writeOffset_ = 0;
            }
        }
        updateEvents();
        return true;
    }

    bool readFrames() {
        std::array<std::uint8_t, 8192> buffer{};
        for (;;) {
            const auto count = ::recv(fd_, buffer.data(), buffer.size(), 0);
            if (count < 0) {
                if (errno == EAGAIN || errno == EWOULDBLOCK) break;
                close(true);
                return false;
            }
            if (count == 0) {
                close(true);
                return false;
            }
            if (!decoder_.feed(std::span(buffer.data(), static_cast<std::size_t>(count)))) {
                close(true);
                return false;
            }
            for (const auto &frame : decoder_.takeFrames()) {
                if (!handleFrame(frame)) {
                    close(true);
                    return false;
                }
            }
        }
        return true;
    }

    bool handleFrame(std::string_view frame) {
        const auto parsed = parseStrictObject(frame);
        if (!parsed) {
            FCITX_ERROR() << "Badi broker rejected a malformed response frame";
            return false;
        }
        const auto &value = *parsed;
        if (
            !value.contains("v") || value["v"] != 2 ||
            !value.contains("type") || !value["type"].is_string() ||
            !value.contains("mono_ms") || !counter(value["mono_ms"]) ||
            !value.contains("payload") || !value["payload"].is_object()) {
            FCITX_ERROR() << "Badi broker rejected a non-canonical response envelope";
            return false;
        }
        const auto type = value["type"].get<std::string>();
        const auto &payload = value["payload"];
        const auto accepted = [this, &value, &payload, &type] {
            if (type == "hello.ack") return handleHelloAck(value, payload);
            if (!helloAcknowledged_) return false;
            if (type == "authority.changed") return handleAuthority(value, payload);
            if (!ready_) return false;
            if (type == "suggestion.show") return handleSuggestion(value, payload);
            if (type == "suggestion.clear") return handleClear(value);
            if (type == "control.result") {
                return sessionControlResult(value, payload);
            }
            if (type == "commit.prepare") return handleCommit(value, payload);
            return false;
        }();
        if (accepted) return true;
        // Errors are terminal for this deliberately small transport; silently
        // accepting an evolving shape would weaken the protocol boundary.
        FCITX_ERROR() << "Badi broker rejected response type: " << type;
        return false;
    }

    bool handleHelloAck(const Json &value, const Json &payload) {
        if (helloAcknowledged_ ||
            !exactKeys(value, {"v", "id", "type", "mono_ms", "payload"}) ||
            value["id"] != "fcitx.hello" ||
            !exactKeys(payload, {"selected_v", "connection_id",
                                 "enabled_capabilities", "max_frame_bytes",
                                 "max_before_chars", "max_after_chars",
                                 "max_suggestion_chars", "max_suggestion_words",
                                 "paused"}) ||
            !payload["selected_v"].is_number_unsigned() ||
            payload["selected_v"] != 2 ||
            !payload["max_frame_bytes"].is_number_unsigned() ||
            payload["max_frame_bytes"] != 65536 ||
            !payload["max_before_chars"].is_number_unsigned() ||
            payload["max_before_chars"] != 512 ||
            !payload["max_after_chars"].is_number_unsigned() ||
            payload["max_after_chars"] != 128 ||
            !payload["max_suggestion_chars"].is_number_unsigned() ||
            payload["max_suggestion_chars"] != 64 ||
            !payload["max_suggestion_words"].is_number_unsigned() ||
            payload["max_suggestion_words"] != 8 ||
            !payload["connection_id"].is_string() ||
            !validOpaqueId(payload["connection_id"].get_ref<const std::string &>()) ||
            !exactCapabilities(payload["enabled_capabilities"]) ||
            !payload["paused"].is_boolean()) {
            return false;
        }
        helloAcknowledged_ = true;
        return true;
    }

    bool handleAuthority(const Json &value, const Json &payload) {
        if (!exactKeys(value, {"v", "type", "mono_ms", "payload"}) ||
            !exactKeys(payload,
                       {"authority_epoch", "settings_revision", "paused"}) ||
            !counter(payload["authority_epoch"]) ||
            !counter(payload["settings_revision"]) || !payload["paused"].is_boolean()) {
            return false;
        }
        const auto epoch = payload["authority_epoch"].get<std::uint64_t>();
        if (authoritySeen_ && epoch <= authorityEpoch_) return false;
        const bool initial = !authoritySeen_;
        authoritySeen_ = true;
        authorityEpoch_ = epoch;
        if (!queue(envelope("authority.ack", std::nullopt, nowMs(),
                            Json{{"authority_epoch", epoch}}))) {
            return false;
        }
        ready_ = true;
        FCITX_INFO() << "Badi broker transport ready";
        if (callbacks_.onAuthority) {
            callbacks_.onAuthority(AuthoritySnapshot{
                .authorityEpoch = epoch,
                .paused = payload["paused"].get<bool>(),
                .initial = initial,
            });
        }
        if (initial && callbacks_.onReady) callbacks_.onReady();
        return true;
    }

    bool handleSuggestion(const Json &value, const Json &payload) {
        if (!exactKeys(value, {"v", "id", "type", "session_id",
                               "focus_epoch", "revision", "mono_ms", "payload"}) ||
            !value["id"].is_string() ||
            !validOpaqueId(value["id"].get_ref<const std::string &>()) ||
            !exactKeys(payload, {"fingerprint", "suggestion_id", "text",
                                 "accept_word", "ttl_ms", "provider"}) ||
            !payload["suggestion_id"].is_string() ||
            !validOpaqueId(payload["suggestion_id"].get_ref<const std::string &>()) ||
            !payload["text"].is_string() || !payload["accept_word"].is_string() ||
            !payload["ttl_ms"].is_number_unsigned() ||
            payload["ttl_ms"].get<std::uint64_t>() < 1 ||
            payload["ttl_ms"].get<std::uint64_t>() > 600 ||
            !payload["provider"].is_string() ||
            (payload["provider"] != "phrase_v1" &&
             payload["provider"] != "local_model")) {
            return false;
        }
        const auto coordinates = parseCoordinates(value, payload);
        const auto text = sanitizeSuggestion(payload["text"].get_ref<const std::string &>());
        const auto acceptWord =
            sanitizeSuggestion(payload["accept_word"].get_ref<const std::string &>());
        if (!coordinates || !text || !acceptWord ||
            !text->starts_with(*acceptWord)) {
            return false;
        }
        if (callbacks_.onSuggestion) {
            callbacks_.onSuggestion(Suggestion{
                .coordinates = *coordinates,
                .requestId = value["id"].get<std::string>(),
                .suggestionId = payload["suggestion_id"].get<std::string>(),
                .text = *text,
                .expiresAtMs = nowMs() + payload["ttl_ms"].get<std::uint64_t>(),
            });
        }
        return true;
    }

    bool handleClear(const Json &value) {
        return dispatchSuggestionClear(value, callbacks_.onClear);
    }

    bool handleCommit(const Json &value, const Json &payload) {
        if (!exactKeys(value, {"v", "id", "type", "session_id",
                               "focus_epoch", "revision", "mono_ms", "payload"}) ||
            !value["id"].is_string() ||
            !validOpaqueId(value["id"].get_ref<const std::string &>()) ||
            !exactKeys(payload, {"fingerprint", "suggestion_id", "text",
                                 "acceptance"}) ||
            !payload["suggestion_id"].is_string() ||
            !validOpaqueId(payload["suggestion_id"].get_ref<const std::string &>()) ||
            !payload["text"].is_string() || payload["acceptance"] != "all") {
            return false;
        }
        const auto coordinates = parseCoordinates(value, payload);
        const auto text = sanitizeSuggestion(payload["text"].get_ref<const std::string &>());
        if (!coordinates || !text) return false;
        if (callbacks_.onCommitPrepare) {
            callbacks_.onCommitPrepare(CommitPrepare{
                .coordinates = *coordinates,
                .controlId = value["id"].get<std::string>(),
                .suggestionId = payload["suggestion_id"].get<std::string>(),
                .text = *text,
                .acceptance = "all",
            });
        }
        return true;
    }

    ::fcitx::EventLoop &eventLoop_;
    WireCallbacks callbacks_;
    std::string socketPath_;
    Clock::time_point started_;
    int fd_ = -1;
    bool connecting_ = false;
    bool helloAcknowledged_ = false;
    bool authoritySeen_ = false;
    bool ready_ = false;
    std::uint64_t authorityEpoch_ = 0;
    std::unique_ptr<::fcitx::EventSourceIO> io_;
    FrameDecoder decoder_;
    std::deque<std::vector<std::uint8_t>> writes_;
    std::size_t writeOffset_ = 0;
    std::size_t queuedBytes_ = 0;
};

Transport::Transport(::fcitx::EventLoop &eventLoop, WireCallbacks callbacks,
                     std::string socketPath)
    : impl_(std::make_unique<Impl>(eventLoop, std::move(callbacks),
                                  std::move(socketPath))) {}

Transport::~Transport() = default;

bool Transport::connect() { return impl_->connectSocket(); }
void Transport::disconnect() { impl_->close(false); }
bool Transport::ready() const { return impl_->ready(); }
std::uint64_t Transport::nowMs() const { return impl_->nowMs(); }
bool Transport::openSession(const Coordinates &coordinates, std::string_view appId,
                            std::string_view targetId) {
    return impl_->openSession(coordinates, appId, targetId);
}
bool Transport::closeSession(const Coordinates &coordinates) {
    return impl_->closeSession(coordinates);
}
bool Transport::publishContext(const ContextUpdate &update) {
    return impl_->publishContext(update);
}
bool Transport::requestAcceptance(const AcceptRequest &request) {
    return impl_->requestAcceptance(request);
}
bool Transport::requestDismissal(const DismissRequest &request) {
    return impl_->requestDismissal(request);
}
bool Transport::reportCommit(const Coordinates &coordinates,
                             std::string_view controlId,
                             std::string_view suggestionId,
                             std::string_view status) {
    return impl_->reportCommit(coordinates, controlId, suggestionId, status);
}

} // namespace badi::fcitx5
