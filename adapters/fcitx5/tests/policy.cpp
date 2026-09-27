#include "transport.h"

#include <fcitx-utils/event.h>
#include <nlohmann/json.hpp>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

#include <array>
#include <cerrno>
#include <chrono>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <functional>
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

namespace {
using namespace badi::fcitx5;
using Json = nlohmann::json;
constexpr auto kSlow = "550e8400-e29b-41d4-a716-446655440000";
constexpr auto kPrompt = "6ba7b810-9dad-41d1-80b4-00c04fd430c8";

void require(bool condition, const char *reason) {
    if (!condition) throw std::runtime_error(reason);
}

Coordinates session(const char *id) {
    Coordinates coordinates;
    coordinates.sessionId = id;
    return coordinates;
}

struct Runtime {
    std::filesystem::path path;
    Runtime() {
        char name[] = "/tmp/badi-native-policy-XXXXXX";
        const auto *created = ::mkdtemp(name);
        require(created != nullptr, "private runtime creation failed");
        path = created;
    }
    ~Runtime() {
        std::error_code ignored;
        std::filesystem::remove_all(path, ignored);
    }
};

Json status(const Json &query) {
    return Json{{"v", 2}, {"type", "policy.status"}, {"id", query.at("id")}, {"mono_ms", 0},
        {"payload", {{"authority_epoch", 1}, {"settings_revision", 1}, {"paused", false},
                     {"activation", "always"}, {"context_allowed", true}, {"display_allowed", true},
                     {"suggestions_allowed", true}, {"learning_allowed", false}, {"reason", "matched_rule"}}}};
}

// A real same-user Unix peer answers one policy query at once and holds the
// other past its two-second reply deadline.
class Harness {
public:
    Harness()
        : client_(loop_, WireCallbacks{
              .onReady = [this] { onReady(); },
              .onAuthority = [](const AuthoritySnapshot &) {},
              .onSuggestion = [](Suggestion) {},
              .onClear = [](const ClearNotice &) {},
              .onCommitPrepare = [](const CommitPrepare &) {},
              .onDisconnected = [this] { disconnected_ = true; loop_.exit(); },
              .onPolicy = [this](std::string_view session, bool allowed) { onPolicy(session, allowed); },
          }, (runtime_.path / "broker.sock").string()) {
        listener_ = ::socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0);
        require(listener_ >= 0, "listener creation failed");
        sockaddr_un address{};
        address.sun_family = AF_UNIX;
        const auto path = (runtime_.path / "broker.sock").string();
        require(path.size() < sizeof(address.sun_path), "private socket path too long");
        std::memcpy(address.sun_path, path.c_str(), path.size() + 1);
        require(::bind(listener_, reinterpret_cast<const sockaddr *>(&address), sizeof(address)) == 0 &&
                    ::chmod(path.c_str(), 0600) == 0 && ::listen(listener_, 1) == 0,
                "private listener setup failed");
        listenerEvent_ = loop_.addIOEvent(listener_, ::fcitx::IOEventFlag::In,
            [this](::fcitx::EventSourceIO *source, int, ::fcitx::IOEventFlags) {
                peer_ = ::accept4(listener_, nullptr, nullptr, SOCK_NONBLOCK | SOCK_CLOEXEC);
                require(peer_ >= 0, "private peer acceptance failed");
                source->setEnabled(false);
                peerEvent_ = loop_.addIOEvent(peer_, ::fcitx::IOEventFlag::In,
                    [this](::fcitx::EventSourceIO *, int, ::fcitx::IOEventFlags) {
                        std::array<std::uint8_t, 4096> bytes{};
                        const auto count = ::recv(peer_, bytes.data(), bytes.size(), 0);
                        if (count < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) return true;
                        if (count <= 0) { peerClosed_ = true; loop_.exit(); return true; }
                        require(decoder_.feed(std::span(bytes.data(), static_cast<std::size_t>(count))),
                                "native framing failed");
                        for (const auto &frame : decoder_.takeFrames()) received(Json::parse(frame));
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
        deadline->setNextInterval(5'000'000);
        deadline->setOneShot();
        require(client_.connect(), "native client could not connect to private peer");
        loop_.exec();
        require(!disconnected_ && !peerClosed_, "an overdue policy reply closed the broker connection");
        require(policies_ == std::vector<std::string>{kPrompt, kSlow},
                "the prompt reply and the overdue reply both reach their sessions");
        require(std::chrono::steady_clock::now() - slowQueried_ >= std::chrono::milliseconds(2'300),
                "the slow reply was not held past its deadline");
        require(queries_ == 2, "a query for a session with an owed reply was sent again");
    }

private:
    void send(const Json &value) {
        const auto frame = encodeFrame(value.dump());
        require(frame.has_value(), "private response encoding failed");
        require(::send(peer_, frame->data(), frame->size(), MSG_NOSIGNAL) == static_cast<ssize_t>(frame->size()),
                "small private response did not fit socket");
    }

    void received(const Json &value) {
        const auto type = value.at("type").get<std::string>();
        if (type == "hello") {
            send(Json{{"v", 2}, {"type", "hello.ack"}, {"id", "fcitx.hello"}, {"mono_ms", 0},
                {"payload", {{"selected_v", 2}, {"connection_id", "private-connection"},
                    {"enabled_capabilities", value.at("payload").at("capabilities")}, {"max_frame_bytes", 65536},
                    {"max_before_chars", 512}, {"max_after_chars", 128},
                    {"max_suggestion_chars", 64}, {"max_suggestion_words", 8}, {"paused", false}}}});
            send(Json{{"v", 2}, {"type", "authority.changed"}, {"mono_ms", 0},
                {"payload", {{"authority_epoch", 1}, {"settings_revision", 1}, {"paused", false}}}});
        } else if (type == "policy.query") {
            ++queries_;
            if (value.at("id") == std::string("fcitx.policy.") + kPrompt) {
                send(status(value));
                return;
            }
            slowQueried_ = std::chrono::steady_clock::now();
            slow_ = loop_.addTimeEvent(CLOCK_MONOTONIC, 0, 0,
                [this, value](::fcitx::EventSourceTime *, std::uint64_t) {
                    // Still owed after the deadline: asking again sends nothing new.
                    require(client_.queryPolicy(session(kSlow), "omawrite", "field-1"),
                            "an owed query was refused");
                    send(status(value));
                    return true;
                });
            slow_->setNextInterval(2'300'000);
            slow_->setOneShot();
        } else {
            require(type == "authority.ack", "unexpected native request");
        }
    }

    void onReady() {
        require(client_.queryPolicy(session(kSlow), "omawrite", "field-1") &&
                    client_.queryPolicy(session(kPrompt), "omawrite", "field-2"),
                "policy queries refused");
    }

    void onPolicy(std::string_view session, bool allowed) {
        require(allowed, "fixture policy must allow");
        policies_.emplace_back(session);
        if (policies_.size() == 2) loop_.exit();
    }

    Runtime runtime_;
    ::fcitx::EventLoop loop_;
    Transport client_;
    int listener_ = -1;
    int peer_ = -1;
    bool disconnected_ = false;
    bool peerClosed_ = false;
    unsigned queries_ = 0;
    std::vector<std::string> policies_;
    std::chrono::steady_clock::time_point slowQueried_{};
    FrameDecoder decoder_;
    std::unique_ptr<::fcitx::EventSourceIO> listenerEvent_;
    std::unique_ptr<::fcitx::EventSourceIO> peerEvent_;
    std::unique_ptr<::fcitx::EventSourceTime> slow_;
};
} // namespace

int main() {
    try {
        Harness().run();
        std::cout << "ok - an overdue policy reply keeps the connection and still reaches its session\n";
    } catch (const std::exception &error) {
        std::cerr << error.what() << '\n';
        return 1;
    }
    return 0;
}
