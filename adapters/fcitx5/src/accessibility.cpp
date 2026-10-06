#include "accessibility.h"
#include "transport.h"

#include <fcitx-utils/event.h>

#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

#include <array>
#include <cerrno>
#include <cstdlib>
#include <cstring>
#include <utility>

namespace badi::fcitx5 {
namespace {
constexpr std::size_t maxFrame = 16'384;
constexpr auto schema = "badi.accessibility.v1";
}

Accessibility::Accessibility(::fcitx::EventLoop &loop, Reply invalidated)
    : loop_(loop), invalidated_(std::move(invalidated)) {}

Accessibility::~Accessibility() {
    invalidated_ = {};
    disconnect();
}

void Accessibility::disconnect() {
    const bool wasConnected = fd_ >= 0 || static_cast<bool>(reply_);
    if (timeout_) timeout_->setEnabled(false);
    if (io_) io_->setEnabled(false);
    if (fd_ >= 0) ::close(fd_);
    fd_ = -1;
    input_.clear();
    output_.clear();
    id_.clear();
    auto pending = std::move(reply_);
    reply_ = {};
    if (wasConnected && invalidated_) invalidated_(nlohmann::json{{"event", "invalidate"}, {"reason", "observer_unavailable"}});
    if (pending) pending(nlohmann::json{{"ok", false}, {"error", "observer_unavailable"}});
}

void Accessibility::hide() {
    if (fd_ < 0) return;
    const nlohmann::json message{{"schema", schema}, {"id", "hide." + std::to_string(++serial_)}, {"op", "hide"}};
    if (output_.size() + message.dump().size() + 1 > maxFrame) { disconnect(); return; }
    output_ += message.dump() + "\n";
    flush();
}

bool Accessibility::connect() {
    if (fd_ >= 0) return true;
    const auto *runtime = std::getenv("XDG_RUNTIME_DIR");
    if (!runtime || runtime[0] != '/') return false;
    const std::string directory = std::string(runtime) + "/badi";
    const auto path = directory + "/accessibility.sock";
    struct stat metadata {};
    if (::lstat(directory.c_str(), &metadata) || !S_ISDIR(metadata.st_mode) ||
        metadata.st_uid != ::getuid() || (metadata.st_mode & 0077) ||
        ::lstat(path.c_str(), &metadata) || !S_ISSOCK(metadata.st_mode) ||
        metadata.st_uid != ::getuid() || (metadata.st_mode & 0777) != 0600) return false;
    sockaddr_un address{};
    address.sun_family = AF_UNIX;
    if (path.size() >= sizeof(address.sun_path)) return false;
    std::memcpy(address.sun_path, path.c_str(), path.size() + 1);
    fd_ = ::socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0);
    if (fd_ < 0) return false;
    // Local Unix connects normally complete immediately; an overloaded observer
    // is unavailable for this epoch instead of delaying the user's keystroke.
    if (::connect(fd_, reinterpret_cast<const sockaddr *>(&address), sizeof(address))) {
        ::close(fd_);
        fd_ = -1;
        return false;
    }
    ucred credentials{};
    socklen_t length = sizeof(credentials);
    if (::getsockopt(fd_, SOL_SOCKET, SO_PEERCRED, &credentials, &length) || credentials.uid != ::getuid()) {
        disconnect();
        return false;
    }
    io_ = loop_.addIOEvent(fd_, ::fcitx::IOEventFlag::In,
        [this](::fcitx::EventSourceIO *, int, ::fcitx::IOEventFlags flags) {
            if (!!(flags & ::fcitx::IOEventFlag::In) && !receive()) return true;
            if (fd_ >= 0 && !!(flags & ::fcitx::IOEventFlag::Out) && !flush()) return true;
            if (fd_ >= 0 && (!!(flags & ::fcitx::IOEventFlag::Err) || !!(flags & ::fcitx::IOEventFlag::Hup))) disconnect();
            return true;
        });
    events();
    return true;
}

void Accessibility::events() {
    if (!io_) return;
    ::fcitx::IOEventFlags flags{::fcitx::IOEventFlag::In};
    flags |= ::fcitx::IOEventFlag::Err;
    flags |= ::fcitx::IOEventFlag::Hup;
    if (!output_.empty()) flags |= ::fcitx::IOEventFlag::Out;
    io_->setEvents(flags);
}

bool Accessibility::request(nlohmann::json value, Reply reply) {
    // Only the current focused field may have an outstanding observation.
    if (reply_ || !value.is_object() || !connect()) return false;
    const auto id = "fcitx." + std::to_string(++serial_);
    value["id"] = id;
    value["schema"] = schema;
    const auto frame = value.dump() + "\n";
    if (frame.size() + output_.size() > maxFrame) return false;
    output_ += frame;
    id_ = id;
    reply_ = std::move(reply);
    if (!timeout_) timeout_ = loop_.addTimeEvent(CLOCK_MONOTONIC, 0, 0,
        [this](::fcitx::EventSourceTime *, std::uint64_t) { disconnect(); return true; });
    timeout_->setNextInterval(500'000);
    timeout_->setOneShot();
    return flush();
}

bool Accessibility::flush() {
    while (!output_.empty()) {
        const auto count = ::send(fd_, output_.data(), output_.size(), MSG_NOSIGNAL);
        if (count < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) break;
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) { disconnect(); return false; }
        output_.erase(0, static_cast<std::size_t>(count));
    }
    events();
    return true;
}

bool Accessibility::receive() {
    std::array<char, 4096> buffer{};
    for (std::size_t turn = 0; turn < 8; ++turn) {
        const auto count = ::recv(fd_, buffer.data(), buffer.size(), 0);
        if (count < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) break;
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) { disconnect(); return false; }
        input_.append(buffer.data(), static_cast<std::size_t>(count));
        for (;;) {
            const auto newline = input_.find('\n');
            if (newline == std::string::npos) break;
            if (newline >= maxFrame) { disconnect(); return false; }
            const auto frame = input_.substr(0, newline);
            if (!strictBoundedJsonObject(frame)) { disconnect(); return false; }
            const auto message = nlohmann::json::parse(frame, nullptr, false);
            input_.erase(0, newline + 1);
            if (!message.is_object() || message.value("schema", nlohmann::json()) != schema) {
                disconnect(); return false;
            }
            if (message.value("event", nlohmann::json()) == "invalidate") {
                if (invalidated_) invalidated_(message);
            } else if (message.value("id", nlohmann::json()) == id_ && reply_) {
                if (timeout_) timeout_->setEnabled(false);
                auto reply = std::move(reply_);
                reply_ = {};
                id_.clear();
                reply(message);
            }
            if (fd_ < 0) return false;
        }
        if (input_.size() >= maxFrame) { disconnect(); return false; }
    }
    return true;
}

} // namespace badi::fcitx5
