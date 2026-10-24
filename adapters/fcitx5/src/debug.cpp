#include "debug.h"
#include "sanitizer.h"

#include <fcntl.h>
#include <sys/stat.h>
#include <unistd.h>

#include <array>
#include <cstdlib>
#include <ctime>

namespace badi::fcitx5 {
namespace {

// Writers stop appending once the shared trace reaches this size.
constexpr off_t kMaxTraceBytes = 16 * 1024 * 1024;

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

bool privateOwned(int fd, bool directory, struct stat *out = nullptr) {
    struct stat info {};
    const bool owned = fd >= 0 && ::fstat(fd, &info) == 0 && info.st_uid == ::getuid() &&
                       (info.st_mode & 0077) == 0 &&
                       (directory ? S_ISDIR(info.st_mode) : S_ISREG(info.st_mode));
    if (out) *out = info;
    return owned;
}

int openRuntime() {
    const auto *runtime = std::getenv("XDG_RUNTIME_DIR");
    if (!runtime || runtime[0] != '/') return -1;
    return ::open((std::string(runtime) + "/badi").c_str(), O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
}

std::int64_t nowMs() {
    return std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::system_clock::now().time_since_epoch()).count();
}

// Appends to the existing private trace below its size limit; never creates
// it, so `badi debug off` (which deletes it) stops every writer at once.
void appendTrace(const nlohmann::json &line) {
    const Descriptor directory(openRuntime());
    if (!privateOwned(directory.get(), true)) return;
    const Descriptor output(::openat(directory.get(), "debug-trace.jsonl",
                                     O_WRONLY | O_APPEND | O_NOFOLLOW | O_CLOEXEC));
    struct stat info {};
    if (!privateOwned(output.get(), false, &info) || info.st_size > kMaxTraceBytes) return;
    // One write per line: O_APPEND keeps concurrent writers' lines whole.
    const auto text = line.dump(-1, ' ', false, nlohmann::json::error_handler_t::replace) + "\n";
    [[maybe_unused]] const auto written = ::write(output.get(), text.data(), text.size());
}

nlohmann::json traceLine(std::string_view event, std::string_view app) {
    return {{"t", nowMs()}, {"src", "fcitx"}, {"ev", event},
            {"app", validLinuxAppId(app) ? std::string(app) : "unidentified"}};
}

} // namespace

std::optional<ActivityDebug::Run> ActivityDebug::readControl() const {
    const Descriptor directory(openRuntime());
    if (!privateOwned(directory.get(), true)) return std::nullopt;
    const Descriptor flag(::openat(directory.get(), "debug-control.json",
                                  O_RDONLY | O_NOFOLLOW | O_CLOEXEC));
    if (!privateOwned(flag.get(), false)) return std::nullopt;
    std::array<char, 513> buffer {};
    const auto size = ::read(flag.get(), buffer.data(), buffer.size());
    if (size <= 0 || size > 512) return std::nullopt;
    const auto control = nlohmann::json::parse(buffer.data(), buffer.data() + size,
                                              nullptr, false);
    if (!control.is_object() || !control.contains("expires_at") ||
        !control["expires_at"].is_number_integer() || !control.contains("id") ||
        !control["id"].is_string()) return std::nullopt;
    const auto now = std::time(nullptr);
    const auto expiry = control["expires_at"].get<std::int64_t>();
    const auto id = control["id"].get<std::string>();
    if (expiry <= now || expiry > now + 900 || !validSessionId(id)) return std::nullopt;
    return Run{.id = id, .text = control.value("text", nlohmann::json()) == true};
}

// A missing or invalid control is rechecked at most once a second; an
// enabled control is revalidated on every call.
std::optional<ActivityDebug::Run> ActivityDebug::run() {
    const auto checked = std::chrono::steady_clock::now();
    if (checked < disabledUntil_) return std::nullopt;
    auto current = readControl();
    if (!current) disabledUntil_ = checked + std::chrono::seconds(1);
    return current;
}

void ActivityDebug::record(std::string_view event, std::string_view app,
                           std::string_view reason, std::size_t before, std::size_t after) {
    if (const auto current = run()) write(*current, event, app, reason, before, after);
}

void ActivityDebug::trace(std::string_view event, std::string_view app, const nlohmann::json &fields,
                          const nlohmann::json &text) {
    const auto current = run();
    if (!current) return;
    auto line = traceLine(event, app);
    if (fields.is_object()) line.update(fields);
    if (current->text && text.is_object()) line.update(text);
    appendTrace(line);
}

void ActivityDebug::write(const Run &run, std::string_view event, std::string_view app,
                          std::string_view reason, std::size_t before, std::size_t after) {
    const Descriptor directory(openRuntime());
    if (!privateOwned(directory.get(), true)) return;
    const auto now = std::time(nullptr);
    if (runId_ != run.id) {
        counts_.clear();
        reasons_.clear();
        runId_ = run.id;
    }
    ++counts_[std::string(event)];
    ++reasons_[std::string(reason)];
    auto line = traceLine(event, app);
    line["reason"] = reason;
    if (before || after) line.update({{"before_bytes", before}, {"after_bytes", after}});
    appendTrace(line);
    const auto snapshot = nlohmann::json{
        {"schema", "badi.native-activity.v1"}, {"id", run.id}, {"at", now},
        {"pid", ::getpid()}, {"event", event}, {"reason", reason},
        {"app_id", validLinuxAppId(app) ? std::string(app) : "unidentified"},
        {"before_bytes", before}, {"after_bytes", after}, {"counts", counts_}, {"reason_counts", reasons_},
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
