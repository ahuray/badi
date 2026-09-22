#include "debug.h"

#include <nlohmann/json.hpp>
#include <sys/stat.h>

#include <cstdlib>
#include <ctime>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <optional>
#include <stdexcept>

namespace {
namespace fs = std::filesystem;

void check(bool value) {
    if (!value) throw std::runtime_error("activity debug boundary failed");
}

struct Runtime {
    std::optional<std::string> previous;
    fs::path path;
    Runtime() {
        if (const auto *value = std::getenv("XDG_RUNTIME_DIR")) previous = value;
        char name[] = "/tmp/badi-debug-test-XXXXXX";
        const auto *created = ::mkdtemp(name);
        check(created != nullptr);
        path = created;
        ::setenv("XDG_RUNTIME_DIR", path.c_str(), 1);
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
}

int main() {
    try {
        Runtime runtime;
        badi::fcitx5::ActivityDebug debug;
        const auto control = runtime.path / "badi/debug-control.json";
        const auto output = runtime.path / "badi/debug-native.json";
        const auto flag = [&](std::time_t expiry) {
            std::ofstream(control) << nlohmann::json{
                {"id", "550e8400-e29b-41d4-a716-446655440000"},
                {"expires_at", expiry}};
            fs::permissions(control, fs::perms::owner_read | fs::perms::owner_write);
        };
        debug.record("input", "omawrite", "input_received");
        check(!fs::exists(output));
        flag(std::time(nullptr) + 900);
        debug.record("tab", "not an app id or typed text", "unsupported_app");
        nlohmann::json snapshot;
        std::ifstream(output) >> snapshot;
        check(snapshot["app_id"] == "unidentified" && snapshot["counts"]["tab"] == 1);
        check(snapshot.size() == 10 && !snapshot.contains("text"));
        check(snapshot["reason_counts"]["unsupported_app"] == 1);
        struct stat info {};
        check(::stat(output.c_str(), &info) == 0 && (info.st_mode & 0777) == 0600);
        fs::remove(output);
        flag(std::time(nullptr) - 1);
        debug.record("input", "omawrite", "input_received");
        check(!fs::exists(output));
        flag(std::time(nullptr) + 900);
        fs::permissions(control, fs::perms::others_read, fs::perm_options::add);
        debug.record("input", "omawrite", "input_received");
        check(!fs::exists(output));
        fs::rename(control, control.string() + ".saved");
        fs::create_symlink(control.string() + ".saved", control);
        debug.record("input", "omawrite", "input_received");
        check(!fs::exists(output));
        fs::remove(control);
        std::ofstream(control) << "{invalid";
        fs::permissions(control, fs::perms::owner_read | fs::perms::owner_write);
        debug.record("input", "omawrite", "input_received");
        check(!fs::exists(output));
        std::cout << "Activity debug: disabled, redacted, private, expired, symlink and malformed checks passed\n";
    } catch (const std::exception &error) {
        std::cerr << error.what() << '\n';
        return 1;
    }
}
