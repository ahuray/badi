#pragma once

#include <nlohmann/json.hpp>

#include <chrono>
#include <cstddef>
#include <optional>
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
    // Appends one line to the shared debug trace (`badi debug watch`), which
    // record() also does. `text` is kept only in a `badi debug on --text`
    // run; `fields` must be content-free.
    void trace(std::string_view event, std::string_view app, const nlohmann::json &fields,
               const nlohmann::json &text = nullptr);
    void refresh() { disabledUntil_ = {}; }

private:
    struct Run {
        std::string id;
        bool text = false;
    };
    // The valid, unexpired run, or nullopt (rechecked at most once a second).
    std::optional<Run> run();
    std::optional<Run> readControl() const;
    void write(const Run &run, std::string_view event, std::string_view app,
               std::string_view reason, std::size_t before, std::size_t after);

    std::chrono::steady_clock::time_point disabledUntil_{};
    std::string runId_;
    std::unordered_map<std::string, std::size_t> counts_;
    std::unordered_map<std::string, std::size_t> reasons_;
};

} // namespace badi::fcitx5
