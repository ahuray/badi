#pragma once

#include <cstddef>
#include <string>
#include <string_view>
#include <unordered_map>

namespace badi::fcitx5 {

class ActivityDebug {
public:
    void record(std::string_view event, std::string_view app,
                std::string_view reason, std::size_t before = 0);

private:
    std::string runId_;
    std::unordered_map<std::string, std::size_t> counts_;
    std::unordered_map<std::string, std::size_t> reasons_;
};

} // namespace badi::fcitx5
