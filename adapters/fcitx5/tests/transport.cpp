// The broker transport's own lifecycle against a real same-user Unix peer:
// a refused request (`error`) keeps the connection and ends only itself, a
// malformed or early refusal closes it, and frames left over from a
// connection a callback closed never reach the next one.

#include "transport.h"

#include <fcitx-utils/event.h>
#include <nlohmann/json.hpp>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

#include <array>
#include <cerrno>
#include <cstring>
#include <filesystem>
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

namespace {
using namespace badi::fcitx5;
using Json = nlohmann::json;
constexpr auto kSession = "550e8400-e29b-41d4-a716-446655440000";

void require(bool condition, const char *reason) {
    if (!condition) throw std::runtime_error(reason);
}

struct Runtime {
    std::filesystem::path path;
    Runtime() {
        char name[] = "/tmp/badi-native-transport-XXXXXX";
        const auto *created = ::mkdtemp(name);
        require(created != nullptr, "private runtime creation failed");
        path = created;
    }
    ~Runtime() {
        std::error_code ignored;
        std::filesystem::remove_all(path, ignored);
    }
};

Json helloAck(const Json &hello) {
    return Json{{"v", 2}, {"type", "hello.ack"}, {"id", "fcitx.hello"}, {"mono_ms", 0},
        {"payload", {{"selected_v", 2}, {"connection_id", "private-connection"},
            {"enabled_capabilities", hello.at("payload").at("capabilities")}, {"max_frame_bytes", 65536},
            {"max_before_chars", 512}, {"max_after_chars", 128},
            {"max_suggestion_chars", 64}, {"max_suggestion_words", 8}, {"paused", false}}}};
}

Json authority() {
    return Json{{"v", 2}, {"type", "authority.changed"}, {"mono_ms", 0},
        {"payload", {{"authority_epoch", 1}, {"settings_revision", 1}, {"paused", false}}}};
}

Json refusal(std::string_view id, std::string_view code) {
    return Json{{"v", 2}, {"id", id}, {"type", "error"}, {"mono_ms", 0}, {"payload", {{"code", code}}}};
}

Json policyStatus(const Json &query) {
    return Json{{"v", 2}, {"type", "policy.status"}, {"id", query.at("id")}, {"mono_ms", 0},
        {"payload", {{"authority_epoch", 1}, {"settings_revision", 1}, {"paused", false},
                     {"activation", "always"}, {"context_allowed", true}, {"display_allowed", true},
                     {"suggestions_allowed", true}, {"learning_allowed", false}, {"reason", "matched_rule"}}}};
}

Json suggestion() {
    return Json{{"v", 2}, {"type", "suggestion.show"}, {"id", "fcitx.suggest.1.1"}, {"session_id", kSession},
        {"focus_epoch", 1}, {"revision", 1}, {"mono_ms", 0},
        {"payload", {{"fingerprint", "0123456789abcdef0123456789abcdef"}, {"suggestion_id", "s1"},
                     {"text", " for your time"}, {"accept_word", " for"}, {"ttl_ms", 1000},
                     {"provider", "phrase_v1"}}}};
}

Coordinates session() {
    Coordinates coordinates;
    coordinates.sessionId = kSession;
    return coordinates;
}

std::vector<std::uint8_t> frame(const Json &value) {
    const auto encoded = encodeFrame(value.dump());
    require(encoded.has_value(), "private response encoding failed");
    return *encoded;
}

class Harness {
public:
    explicit Harness(std::string scenario)
        : scenario_(std::move(scenario)),
          client_(loop_, WireCallbacks{
              .onReady = [this] { onReady(); },
              .onAuthority = [](const AuthoritySnapshot &) {},
              .onSuggestion = [this](Suggestion) { onSuggestion(); },
              .onClear = [](const ClearNotice &) {},
              .onCommitPrepare = [](const CommitPrepare &) {},
              .onDisconnected = [this] { disconnected_ = true; loop_.exit(); },
              .onPolicy = [this](std::string_view session, bool allowed, std::string, std::string, bool) {
                  require(session == kSession && allowed, "the policy reached another session");
                  ++policies_;
                  loop_.exit();
              },
              .onRefusal = [this](const RequestRefusal &refused) { onRefusal(refused); },
          }, (runtime_.path / "broker.sock").string()) {
        listener_ = ::socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0);
        require(listener_ >= 0, "listener creation failed");
        sockaddr_un address{};
        address.sun_family = AF_UNIX;
        const auto path = (runtime_.path / "broker.sock").string();
        require(path.size() < sizeof(address.sun_path), "private socket path too long");
        std::memcpy(address.sun_path, path.c_str(), path.size() + 1);
        require(::bind(listener_, reinterpret_cast<const sockaddr *>(&address), sizeof(address)) == 0 &&
                    ::chmod(path.c_str(), 0600) == 0 && ::listen(listener_, 2) == 0,
                "private listener setup failed");
        listenerEvent_ = loop_.addIOEvent(listener_, ::fcitx::IOEventFlag::In,
            [this](::fcitx::EventSourceIO *, int, ::fcitx::IOEventFlags) {
                const int peer = ::accept4(listener_, nullptr, nullptr, SOCK_NONBLOCK | SOCK_CLOEXEC);
                require(peer >= 0, "private peer acceptance failed");
                peerEvent_.reset();
                if (peer_ >= 0) ::close(peer_);
                peer_ = peer;
                ++connections_;
                decoder_ = FrameDecoder{};
                peerEvent_ = loop_.addIOEvent(peer_, ::fcitx::IOEventFlag::In,
                    [this](::fcitx::EventSourceIO *source, int, ::fcitx::IOEventFlags) {
                        std::array<std::uint8_t, 4096> bytes{};
                        const auto count = ::recv(peer_, bytes.data(), bytes.size(), 0);
                        if (count < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) return true;
                        if (count <= 0) {
                            source->setEnabled(false);
                            return true;
                        }
                        require(decoder_.feed(std::span(bytes.data(), static_cast<std::size_t>(count))),
                                "native framing failed");
                        for (const auto &body : decoder_.takeFrames()) received(Json::parse(body));
                        return true;
                    });
                return true;
            });
    }

    ~Harness() {
        client_.disconnect();
        peerEvent_.reset();
        listenerEvent_.reset();
        if (peer_ >= 0) ::close(peer_);
        if (listener_ >= 0) ::close(listener_);
    }

    void run() {
        auto deadline = loop_.addTimeEvent(CLOCK_MONOTONIC, 0, 0,
            [this](::fcitx::EventSourceTime *, std::uint64_t) { loop_.exit(); return true; });
        deadline->setNextInterval(3'000'000);
        deadline->setOneShot();
        require(client_.connect(), "native client could not connect to private peer");
        loop_.exec();
        if (scenario_ == "refusals") {
            require(!disconnected_, "a refusal closed the connection");
            require(refusals_.size() == 2 && refusals_[0].requestId == std::string("fcitx.policy.") + kSession &&
                        refusals_[0].reason == "stale" && refusals_[1].requestId == "fcitx.accept.1.1" &&
                        refusals_[1].reason == "expired",
                    "each refusal reaches the adapter with its request id and reason");
            require(queries_ == 2 && policies_ == 1,
                    "a refused policy query is owed nothing: the session asks again and gets its answer");
        } else if (scenario_ == "dispatch-after-close") {
            require(!disconnected_ && suggestions_ == 1 && ready_ == 2 && connections_ == 2,
                    "a frame left from a closed connection changed the next one's handshake");
        } else {
            require(disconnected_ && refusals_.empty(), "a malformed or early refusal kept the connection");
        }
    }

private:
    void send(const std::vector<Json> &values) {
        std::vector<std::uint8_t> bytes;
        for (const auto &value : values) {
            const auto encoded = frame(value);
            bytes.insert(bytes.end(), encoded.begin(), encoded.end());
        }
        // One write, so the client reads these frames in one batch.
        require(::send(peer_, bytes.data(), bytes.size(), MSG_NOSIGNAL) == static_cast<ssize_t>(bytes.size()),
                "small private response did not fit socket");
    }

    void received(const Json &value) {
        const auto type = value.at("type").get<std::string>();
        if (type == "hello") {
            if (scenario_ == "early-refusal") return send({helloAck(value), refusal("fcitx.hello", "stale")});
            send({helloAck(value), authority()});
        } else if (type == "policy.query") {
            if (++queries_ == 1) {
                return send({refusal(value.at("id").get<std::string>(), "stale"),
                             refusal("fcitx.accept.1.1", "expired")});
            }
            send({policyStatus(value)});
        } else {
            require(type == "authority.ack", "unexpected native request");
        }
    }

    static Json field() { return *desktopApplicationTarget("omawrite", "field-1"); }

    void onReady() {
        ++ready_;
        if (scenario_ == "refusals") {
            require(client_.queryPolicy(session(), field()), "policy query refused");
        } else if (scenario_ == "malformed-refusal") {
            auto extra = refusal("fcitx.accept.1.1", "stale");
            extra["payload"]["committed"] = false;
            send({extra});
        } else if (scenario_ == "dispatch-after-close") {
            if (ready_ == 2) return loop_.exit();
            // The callback below reconnects at once; the hello.ack behind
            // the suggestion still belongs to this connection.
            send({suggestion(), helloAck(Json{{"payload", {{"capabilities", capabilities()}}}})});
        }
    }

    void onRefusal(const RequestRefusal &refused) {
        refusals_.push_back(refused);
        // Retired by its refusal, the policy query may be asked again.
        if (refused.requestId == std::string("fcitx.policy.") + kSession) {
            require(client_.queryPolicy(session(), field()),
                    "the refused session could not ask again");
        }
    }

    void onSuggestion() {
        ++suggestions_;
        if (scenario_ != "dispatch-after-close") return;
        client_.disconnect();
        require(client_.connect(), "the client could not reconnect from its callback");
    }

    static Json capabilities() {
        return Json::array({"context", "suggestion", "commit.dispatched_unverified", "text_replacement", "control",
                            "policy", "long_reading_window", "selection_revision"});
    }

    Runtime runtime_;
    ::fcitx::EventLoop loop_;
    std::string scenario_;
    Transport client_;
    int listener_ = -1;
    int peer_ = -1;
    unsigned connections_ = 0;
    unsigned ready_ = 0;
    unsigned queries_ = 0;
    unsigned policies_ = 0;
    unsigned suggestions_ = 0;
    bool disconnected_ = false;
    std::vector<RequestRefusal> refusals_;
    FrameDecoder decoder_;
    std::unique_ptr<::fcitx::EventSourceIO> listenerEvent_;
    std::unique_ptr<::fcitx::EventSourceIO> peerEvent_;
};
} // namespace

int main() {
    try {
        for (const auto *scenario : {"refusals", "malformed-refusal", "early-refusal", "dispatch-after-close"}) {
            Harness(scenario).run();
            std::cout << "ok - native transport: " << scenario << '\n';
        }
    } catch (const std::exception &error) {
        std::cerr << error.what() << '\n';
        return 1;
    }
    return 0;
}
