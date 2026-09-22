#include "accessibility.h"

#include <fcitx-utils/event.h>
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
#include <optional>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

namespace {
using Json = nlohmann::json;
using badi::fcitx5::Accessibility;
namespace fs = std::filesystem;
constexpr auto schema = "badi.accessibility.v1";

void require(bool condition, const char *reason) {
    if (!condition) throw std::runtime_error(reason);
}

struct Runtime {
    std::optional<std::string> previous;
    fs::path path;
    Runtime() {
        if (const auto *value = std::getenv("XDG_RUNTIME_DIR")) previous = value;
        char name[] = "/tmp/badi-accessibility-test-XXXXXX";
        const auto *created = ::mkdtemp(name);
        require(created != nullptr, "private runtime creation failed");
        path = created;
        require(::setenv("XDG_RUNTIME_DIR", path.c_str(), 1) == 0, "runtime environment failed");
        fs::create_directory(path / "badi");
        fs::permissions(path / "badi", fs::perms::owner_all);
    }
    ~Runtime() {
        if (previous) ::setenv("XDG_RUNTIME_DIR", previous->c_str(), 1);
        else ::unsetenv("XDG_RUNTIME_DIR");
        std::error_code ignored;
        fs::remove_all(path, ignored);
    }
};

// An actual Unix peer on the Fcitx event loop. It never touches the desktop,
// retains no user prose, and every scenario has an independent hard deadline.
struct Harness {
    Runtime runtime;
    ::fcitx::EventLoop loop;
    std::vector<Json> invalidations;
    std::vector<Json> requests;
    std::function<void(const Json &)> respond;
    std::function<void(const Json &)> onInvalidation;
    Accessibility client;
    int listener = -1;
    int peer = -1;
    bool finished = false;
    bool closing = false;
    bool peerClosed = false;
    std::string input;
    std::unique_ptr<::fcitx::EventSourceIO> listenerEvent;
    std::unique_ptr<::fcitx::EventSourceIO> peerEvent;
    std::vector<std::unique_ptr<::fcitx::EventSourceTime>> timers;

    Harness() : client(loop, [this](const Json &value) {
        invalidations.push_back(value);
        if (onInvalidation) onInvalidation(value);
    }) {
        listener = ::socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0);
        require(listener >= 0, "listener socket failed");
        sockaddr_un address{};
        address.sun_family = AF_UNIX;
        const auto socket = (runtime.path / "badi/accessibility.sock").string();
        require(socket.size() < sizeof(address.sun_path), "test socket path too long");
        std::memcpy(address.sun_path, socket.c_str(), socket.size() + 1);
        require(::bind(listener, reinterpret_cast<const sockaddr *>(&address), sizeof(address)) == 0,
                "private socket bind failed");
        require(::chmod(socket.c_str(), 0600) == 0 && ::listen(listener, 8) == 0, "private listener setup failed");
        listenerEvent = loop.addIOEvent(listener, ::fcitx::IOEventFlag::In,
            [this](::fcitx::EventSourceIO *, int, ::fcitx::IOEventFlags) {
                if (peer >= 0) ::close(peer);
                peer = ::accept4(listener, nullptr, nullptr, SOCK_NONBLOCK | SOCK_CLOEXEC);
                require(peer >= 0, "peer accept failed");
                peerClosed = false;
                input.clear();
                peerEvent = loop.addIOEvent(peer, ::fcitx::IOEventFlag::In,
                    [this](::fcitx::EventSourceIO *source, int, ::fcitx::IOEventFlags) {
                        std::array<char, 4096> buffer{};
                        for (;;) {
                            const auto count = ::recv(peer, buffer.data(), buffer.size(), 0);
                            if (count < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) break;
                            if (count < 0 && errno == EINTR) continue;
                            require(count >= 0, "peer read failed");
                            if (!count) { peerClosed = true; source->setEnabled(false); break; }
                            input.append(buffer.data(), static_cast<std::size_t>(count));
                            for (;;) {
                                const auto end = input.find('\n');
                                if (end == std::string::npos) break;
                                const auto value = Json::parse(input.substr(0, end));
                                input.erase(0, end + 1);
                                requests.push_back(value);
                                require(value.value("schema", "") == schema, "request schema missing");
                                if (respond) respond(value);
                            }
                        }
                        return true;
                    });
                return true;
            });
    }

    ~Harness() {
        closing = true;
        onInvalidation = {};
        client.disconnect();
        peerEvent.reset();
        listenerEvent.reset();
        if (peer >= 0) ::close(peer);
        if (listener >= 0) ::close(listener);
    }

    bool request(Json value, Accessibility::Reply reply) {
        return client.request(std::move(value), [this, reply = std::move(reply)](const Json &response) {
            if (!closing) reply(response);
        });
    }

    void limitClientSendBuffer() {
        // Find only the socket connected to this fixture's exact private path.
        // A tiny kernel queue makes partial writes deterministic without adding
        // test-only descriptor access to the product client.
        const auto path = (runtime.path / "badi/accessibility.sock").string();
        for (const auto &entry : fs::directory_iterator("/proc/self/fd")) {
            const int descriptor = std::stoi(entry.path().filename().string());
            sockaddr_un address{};
            socklen_t length = sizeof(address);
            if (::getpeername(descriptor, reinterpret_cast<sockaddr *>(&address), &length) != 0 ||
                address.sun_family != AF_UNIX || path != address.sun_path) continue;
            const int size = 4096;
            require(::setsockopt(descriptor, SOL_SOCKET, SO_SNDBUF, &size, sizeof(size)) == 0,
                    "test send buffer setup failed");
            return;
        }
        require(false, "private client descriptor was not found");
    }

    void send(std::string_view bytes) {
        require(::send(peer, bytes.data(), bytes.size(), MSG_NOSIGNAL) == static_cast<ssize_t>(bytes.size()),
                "bounded fake response write failed");
    }
    void reply(const Json &request, Json payload = Json{{"ok", true}}) {
        payload["schema"] = schema;
        payload["id"] = request.at("id");
        send(payload.dump() + "\n");
    }
    void later(std::uint64_t usec, std::function<void()> action) {
        auto timer = loop.addTimeEvent(CLOCK_MONOTONIC, 0, 0,
            [action = std::move(action)](::fcitx::EventSourceTime *, std::uint64_t) { action(); return true; });
        timer->setNextInterval(usec);
        timer->setOneShot();
        timers.push_back(std::move(timer));
    }
    void done() { finished = true; loop.exit(); }
    void run() {
        later(1'500'000, [this] { loop.exit(); });
        loop.exec();
        require(finished, "accessibility socket scenario exceeded its deadline");
    }
};

void fragmentedAndSinglePending() {
    Harness test;
    unsigned replies = 0;
    test.respond = [&](const Json &request) {
        auto wrong = Json{{"schema", schema}, {"id", "foreign"}, {"ok", true}}.dump() + "\n";
        test.send(wrong);
        const auto frame = Json{{"schema", schema}, {"id", request["id"]}, {"ok", true},
                                {"focus", Json{{"target", "synthetic"}}}}.dump() + "\n";
        test.send(frame.substr(0, 7));
        test.later(20'000, [&, frame] { test.send(frame.substr(7)); });
    };
    require(test.request(Json{{"op", "inspect"}}, [&](const Json &value) {
        require(value.value("ok", false) && value["focus"]["target"] == "synthetic", "fragmented reply changed");
        ++replies;
        test.done();
    }), "first request refused");
    require(!test.request(Json{{"op", "snapshot"}}, [](const Json &) {
        require(false, "second pending request received a callback");
    }), "overlapping request was accepted");
    test.run();
    require(replies == 1 && test.requests.size() == 1 && test.invalidations.empty(), "single pending request not preserved");
}

void invalidationDuringRequest() {
    Harness test;
    unsigned generation = 0;
    unsigned replies = 0;
    test.onInvalidation = [&](const Json &value) {
        require(value.value("event", "") == "invalidate", "wrong invalidation event");
        ++generation;
    };
    test.respond = [&](const Json &request) {
        test.send(Json{{"schema", schema}, {"event", "invalidate"}, {"reason", "focus_changed"}}.dump() + "\n");
        test.reply(request);
    };
    const auto captured = generation;
    require(test.request(Json{{"op", "snapshot"}}, [&](const Json &value) {
        require(value.value("ok", false) && generation != captured, "invalidation was not delivered before the retired reply");
        ++replies;
        test.done();
    }), "snapshot request refused");
    test.run();
    require(replies == 1 && test.invalidations.size() == 1, "pending invalidation lost or duplicated");
}

void hideDuringRequest() {
    Harness test;
    std::optional<Json> pending;
    test.respond = [&](const Json &request) {
        if (request["op"] == "snapshot") pending = request;
        else {
            require(request["op"] == "hide" && pending.has_value(), "hide damaged pending request ordering");
            test.reply(request); // Fire-and-forget acknowledgement must not steal the snapshot callback.
            test.reply(*pending);
        }
    };
    require(test.request(Json{{"op", "snapshot"}}, [&](const Json &value) {
        require(value.value("ok", false) && value["id"] == (*pending)["id"], "hide consumed the active reply");
        test.done();
    }), "snapshot refused before hide");
    test.client.hide();
    test.run();
    require(test.requests.size() == 2 && test.invalidations.empty(), "hide disconnected an active observation");
}

void timeoutDisconnects() {
    Harness test;
    const auto started = std::chrono::steady_clock::now();
    unsigned replies = 0;
    require(test.request(Json{{"op", "snapshot"}}, [&](const Json &value) {
        const auto elapsed = std::chrono::steady_clock::now() - started;
        require(value.value("ok", true) == false && value.value("error", "") == "observer_unavailable",
                "timeout must fail the pending observation");
        require(elapsed >= std::chrono::milliseconds(450) && elapsed < std::chrono::milliseconds(1000),
                "observer timeout outside its bounded window");
        ++replies;
        test.later(20'000, [&] { require(test.peerClosed, "timed-out socket remained connected"); test.done(); });
    }), "timeout fixture request refused");
    test.run();
    require(replies == 1 && test.invalidations.size() == 1, "timeout callbacks lost or repeated");
}

void backpressuredHideOrdering() {
    Harness test;
    constexpr unsigned hideCount = 64;
    unsigned hides = 0;
    unsigned inspections = 0;
    unsigned replies = 0;
    test.respond = [&](const Json &request) {
        if (request["op"] == "hide") { ++hides; return; }
        require(request["op"] == "inspect", "unexpected queued operation");
        if (++inspections == 2) require(hides == hideCount, "new request overwrote backpressured hide frames");
        test.reply(request);
    };
    require(test.request(Json{{"op", "inspect"}}, [&](const Json &value) {
        require(value.value("ok", false), "first backpressure response failed");
        ++replies;
        require(hides < hideCount, "fixture did not exert socket backpressure");
        require(test.request(Json{{"op", "inspect"}}, [&](const Json &next) {
            require(next.value("ok", false), "second backpressure response failed");
            ++replies;
            test.done();
        }), "next request refused behind bounded hide queue");
    }), "first backpressure request refused");
    test.limitClientSendBuffer();
    for (unsigned index = 0; index < hideCount; ++index) test.client.hide();
    test.run();
    require(inspections == 2 && replies == 2 && hides == hideCount && test.invalidations.empty(),
            "backpressure changed ordering or invalidated a healthy observer");
}

void malformedDisconnects(const std::string &frame) {
    Harness test;
    unsigned replies = 0;
    test.respond = [&](const Json &request) {
        auto response = frame;
        constexpr std::string_view marker = "@REQUEST_ID@";
        if (const auto position = response.find(marker); position != std::string::npos)
            response.replace(position, marker.size(), request.at("id").get<std::string>());
        test.send(response);
    };
    require(test.request(Json{{"op", "inspect"}}, [&](const Json &value) {
        require(value.value("ok", true) == false, "malformed frame produced a successful observation");
        ++replies;
        test.done();
    }), "malformed fixture request refused");
    test.run();
    require(replies == 1 && test.invalidations.size() == 1, "malformed frame did not retire authority once");
}

void shallowNestedReply() {
    Harness test;
    test.respond = [&](const Json &request) {
        test.reply(request, Json{{"ok", true}, {"extra", Json::array({Json{{"nested", Json::array({1, 2})}}})}});
    };
    require(test.request(Json{{"op", "inspect"}}, [&](const Json &value) {
        require(value.value("ok", false) && value.at("extra")[0]["nested"][1] == 2,
                "ordinary shallow JSON was rejected");
        test.done();
    }), "shallow nesting request refused");
    test.run();
    require(test.invalidations.empty(), "shallow reply unexpectedly invalidated authority");
}

void privateEndpointAndOutgoingBound() {
    Harness test;
    const auto socket = test.runtime.path / "badi/accessibility.sock";
    require(::chmod(socket.c_str(), 0666) == 0, "test chmod failed");
    require(!test.request(Json{{"op", "inspect"}}, [](const Json &) {}), "public socket accepted");
    require(::chmod(socket.c_str(), 0600) == 0, "test chmod restore failed");
    require(::chmod(socket.parent_path().c_str(), 0755) == 0, "test directory chmod failed");
    require(!test.request(Json{{"op", "inspect"}}, [](const Json &) {}), "public directory accepted");
    require(::chmod(socket.parent_path().c_str(), 0700) == 0, "test directory restore failed");
    require(!test.request(Json{{"op", "inspect"}, {"too_large", std::string(16'384, 'x')}}, [](const Json &) {
        require(false, "oversized outgoing request produced a callback");
    }), "oversized outgoing frame accepted");
    test.respond = [&](const Json &request) { test.reply(request); };
    require(test.request(Json{{"op", "inspect"}}, [&](const Json &value) {
        require(value.value("ok", false), "client did not recover after rejected outgoing frame");
        test.done();
    }), "bounded outgoing request refused after oversized frame");
    test.run();
    require(test.requests.size() == 1 && test.invalidations.empty(), "rejected request reached the observer");
}
} // namespace

int main() {
    try {
        fragmentedAndSinglePending();
        invalidationDuringRequest();
        hideDuringRequest();
        backpressuredHideOrdering();
        timeoutDisconnects();
        shallowNestedReply();
        for (const auto &frame : std::vector<std::string>{"{broken\n", "[]\n", "{\"schema\":\"foreign\"}\n",
                 "{\"schema\":\"badi.accessibility.v1\",\"schema\":\"badi.accessibility.v1\",\"id\":\"@REQUEST_ID@\",\"ok\":true}\n",
                 "{\"schema\":\"badi.accessibility.v1\",\"id\":\"@REQUEST_ID@\",\"ok\":true,\"extra\":" +
                     std::string(33, '[') + "0" + std::string(33, ']') + "}\n",
                 "{\"schema\":\"badi.accessibility.v1\",\"id\":\"@REQUEST_ID@\",\"ok\":true,\"extra\":" +
                     std::string(65, '[') + "0" + std::string(65, ']') + "}\n",
                 std::string(16'385, 'x'), std::string(16'385, 'x') + "\n"}) malformedDisconnects(frame);
        privateEndpointAndOutgoingBound();
        std::cout << "Accessibility socket: private endpoint, fragments, invalidation, one pending request, hide, timeout and frame bounds passed\n";
        return 0;
    } catch (const std::exception &error) {
        std::cerr << error.what() << '\n';
        return 1;
    }
}
