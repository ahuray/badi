#include "transport.h"

#include <fcitx-utils/event.h>
#include <nlohmann/json.hpp>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

#include <array>
#include <cerrno>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string>

namespace {
using namespace badi::fcitx5;
using Json = nlohmann::json;

void require(bool condition, const char *reason) {
    if (!condition) throw std::runtime_error(reason);
}

struct Runtime {
    std::filesystem::path path;
    Runtime() {
        char name[] = "/tmp/badi-native-wire-XXXXXX";
        const auto *created = ::mkdtemp(name);
        require(created != nullptr, "private runtime creation failed");
        path = created;
    }
    ~Runtime() {
        std::error_code ignored;
        std::filesystem::remove_all(path, ignored);
    }
};

const Json capabilities = Json::array({"context", "suggestion",
    "commit.dispatched_unverified", "control", "policy"});

Json message(const char *type) {
    Json payload{{"fingerprint", "0123456789abcdef0123456789abcdef"},
                 {"suggestion_id", "native-suggestion"}, {"text", " for your time"}};
    if (std::string_view(type) == "suggestion.show") {
        payload.update(Json{{"accept_word", " for"}, {"ttl_ms", 1000}, {"provider", "phrase_v1"}});
    } else {
        payload["acceptance"] = "all";
    }
    return Json{{"v", 2}, {"type", type}, {"id", "native-request"},
        {"session_id", "550e8400-e29b-41d4-a716-446655440000"},
        {"focus_epoch", 1}, {"revision", 1}, {"mono_ms", 0}, {"payload", payload}};
}

// A real same-user Unix peer exercises the production hello and response
// parser. No test bypass calls a private parser or creates an InputContext.
class Harness {
public:
    explicit Harness(std::string scenario)
        : scenario_(std::move(scenario)), client_(loop_, WireCallbacks{
              .onReady = [this] { onReady(); },
              .onAuthority = [](const AuthoritySnapshot &) {},
              .onSuggestion = [this](Suggestion suggestion) {
                  require(suggestion.replaceBefore.empty(), "replacement reached display callback");
                  ++suggestions_;
              },
              .onClear = [](const ClearNotice &) {},
              .onCommitPrepare = [this](const CommitPrepare &prepare) {
                  require(prepare.replaceBefore.empty(), "replacement reached commit callback");
                  ++commits_;
                  finished_ = true;
                  loop_.exit();
              },
              .onDisconnected = [this] {
                  disconnected_ = true;
                  loop_.exit();
              },
              .onPolicy = {},
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
                        require(count > 0, "native client unexpectedly closed before hello");
                        require(decoder_.feed(std::span(bytes.data(), static_cast<std::size_t>(count))),
                                "native hello framing failed");
                        for (const auto &frame : decoder_.takeFrames()) hello(Json::parse(frame));
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
        deadline->setNextInterval(2'000'000);
        deadline->setOneShot();
        require(client_.connect(), "native client could not connect to private peer");
        loop_.exec();
        require(helloSeen_, "native client never advertised its capabilities");
        if (scenario_ == "append") {
            require(ready_ && finished_ && !disconnected_ && suggestions_ == 1 && commits_ == 1,
                    "ordinary append flow regressed");
        } else {
            require(disconnected_ && !finished_ && suggestions_ == 0 && commits_ == 0,
                    "unsolicited replacement was not rejected before callbacks");
            require(ready_ == (scenario_ != "extra-capability"),
                    "replacement injection reached an unexpected handshake phase");
        }
    }

private:
    void send(const Json &value) {
        const auto frame = encodeFrame(value.dump());
        require(frame.has_value(), "private response encoding failed");
        const auto sent = ::send(peer_, frame->data(), frame->size(), MSG_NOSIGNAL);
        require(sent == static_cast<ssize_t>(frame->size()), "small private response did not fit socket");
    }

    void hello(const Json &value) {
        require(!helloSeen_ && value.at("type") == "hello", "unexpected native request");
        helloSeen_ = true;
        require(value.at("payload").at("capabilities") == capabilities,
                "native client must advertise only append capabilities");
        auto enabled = capabilities;
        if (scenario_ == "extra-capability") enabled.push_back("text_replacement");
        send(Json{{"v", 2}, {"type", "hello.ack"}, {"id", "fcitx.hello"}, {"mono_ms", 0},
            {"payload", {{"selected_v", 2}, {"connection_id", "private-connection"},
                {"enabled_capabilities", enabled}, {"max_frame_bytes", 65536},
                {"max_before_chars", 512}, {"max_after_chars", 128},
                {"max_suggestion_chars", 64}, {"max_suggestion_words", 8}, {"paused", false}}}});
        if (scenario_ != "extra-capability") {
            send(Json{{"v", 2}, {"type", "authority.changed"}, {"mono_ms", 0},
                {"payload", {{"authority_epoch", 1}, {"settings_revision", 1}, {"paused", false}}}});
        }
    }

    void onReady() {
        ready_ = true;
        if (scenario_ == "append") {
            send(message("suggestion.show"));
            send(message("commit.prepare"));
            return;
        }
        const bool suggestion = scenario_.starts_with("suggestion");
        auto response = message(suggestion ? "suggestion.show" : "commit.prepare");
        response["payload"]["replace_before"] = scenario_.ends_with("empty") ? "" : "adress";
        // These otherwise ordinary append frames must fail because of the
        // replacement field itself, not because of spelling text sanitization.
        send(response);
    }

    Runtime runtime_;
    ::fcitx::EventLoop loop_;
    std::string scenario_;
    Transport client_;
    int listener_ = -1;
    int peer_ = -1;
    bool helloSeen_ = false;
    bool ready_ = false;
    bool finished_ = false;
    bool disconnected_ = false;
    unsigned suggestions_ = 0;
    unsigned commits_ = 0;
    FrameDecoder decoder_;
    std::unique_ptr<::fcitx::EventSourceIO> listenerEvent_;
    std::unique_ptr<::fcitx::EventSourceIO> peerEvent_;
};
} // namespace

int main() {
    try {
        for (const auto *scenario : {"append", "extra-capability", "suggestion", "commit",
                                    "suggestion-empty", "commit-empty"}) {
            Harness(scenario).run();
            std::cout << "ok - native replacement boundary: " << scenario << '\n';
        }
    } catch (const std::exception &error) {
        std::cerr << error.what() << '\n';
        return 1;
    }
    return 0;
}
