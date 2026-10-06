#pragma once

#include <chrono>
#include <cstddef>
#include <string>
#include <string_view>
#include <unordered_map>

namespace badi::fcitx5 {

class ActivityDebug {
public:
    // Writes a content-free snapshot while `badi debug on` is active. A
    // missing or invalid control is rechecked at most once a second, or at
    // the next record after refresh().
    void record(std::string_view event, std::string_view app,
                std::string_view reason, std::size_t before = 0, std::size_t after = 0);
    void refresh() { disabledUntil_ = {}; }

private:
    bool write(std::string_view event, std::string_view app,
               std::string_view reason, std::size_t before, std::size_t after);

    std::chrono::steady_clock::time_point disabledUntil_{};
    std::string runId_;
    std::unordered_map<std::string, std::size_t> counts_;
    std::unordered_map<std::string, std::size_t> reasons_;
};

} // namespace badi::fcitx5
