#include "debug.h"
#include "sanitizer.h"

#include <nlohmann/json.hpp>
#include <fcntl.h>
#include <sys/stat.h>
#include <unistd.h>

#include <array>
#include <cstdlib>
#include <ctime>

namespace badi::fcitx5 {
namespace {

class Descriptor {
public:
    explicit Descriptor(int fd) : fd_(fd) {}
    ~Descriptor() { if (fd_ >= 0) ::close(fd_); }
    Descriptor(const Descriptor &) = delete;
    Descriptor &operator=(const Descriptor &) = delete;
    int get() const { return fd_; }

private:
    int fd_;
};

bool privateOwned(int fd, bool directory) {
    struct stat info {};
    return fd >= 0 && ::fstat(fd, &info) == 0 && info.st_uid == ::getuid() &&
           (info.st_mode & 0077) == 0 &&
           (directory ? S_ISDIR(info.st_mode) : S_ISREG(info.st_mode));
}

} // namespace

void ActivityDebug::record(std::string_view event, std::string_view app,
                           std::string_view reason, std::size_t before) {
    const auto *runtime = std::getenv("XDG_RUNTIME_DIR");
    if (!runtime || runtime[0] != '/') return;
    const Descriptor directory(::open((std::string(runtime) + "/badi").c_str(),
                                     O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC));
    if (!privateOwned(directory.get(), true)) return;
    const Descriptor flag(::openat(directory.get(), "debug-control.json",
                                  O_RDONLY | O_NOFOLLOW | O_CLOEXEC));
    if (!privateOwned(flag.get(), false)) return;
    std::array<char, 513> buffer {};
    const auto size = ::read(flag.get(), buffer.data(), buffer.size());
    if (size <= 0 || size > 512) return;
    const auto control = nlohmann::json::parse(buffer.data(), buffer.data() + size,
                                              nullptr, false);
    if (!control.is_object() || !control.contains("expires_at") ||
        !control["expires_at"].is_number_integer() || !control.contains("id") ||
        !control["id"].is_string()) return;
    const auto now = std::time(nullptr);
    const auto expiry = control["expires_at"].get<std::int64_t>();
    const auto id = control["id"].get<std::string>();
    if (expiry <= now || expiry > now + 900 || !validSessionId(id)) return;
    if (runId_ != id) {
        counts_.clear();
        reasons_.clear();
        runId_ = id;
    }
    ++counts_[std::string(event)];
    ++reasons_[std::string(reason)];
    const auto snapshot = nlohmann::json{
        {"schema", "badi.native-activity.v1"}, {"id", id}, {"at", now},
        {"pid", ::getpid()}, {"event", event}, {"reason", reason},
        {"app_id", validLinuxAppId(app) ? std::string(app) : "unidentified"},
        {"before_bytes", before}, {"counts", counts_}, {"reason_counts", reasons_},
    }.dump();
    const auto temporary = "debug-native." + std::to_string(::getpid()) + ".tmp";
    const Descriptor output(::openat(directory.get(), temporary.c_str(),
        O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0600));
    if (output.get() < 0) return;
    const auto written = ::write(output.get(), snapshot.data(), snapshot.size());
    if (written == static_cast<ssize_t>(snapshot.size())) {
        ::renameat(directory.get(), temporary.c_str(), directory.get(), "debug-native.json");
    }
    ::unlinkat(directory.get(), temporary.c_str(), 0);
}

} // namespace badi::fcitx5
