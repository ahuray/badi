#pragma once

#include <fcitx-utils/log.h>

#include <exception>

namespace badi::fcitx5 {

/// Runs one teardown or event-loop step that must not throw, logging any
/// failure with `step`. Fcitx's event sources throw from setEnabled while
/// its loop shuts down, and an exception leaving a destructor terminates
/// fcitx5, so teardown runs each step through this and carries on.
template <typename Action>
void quietly(const char *step, const Action &action) noexcept {
    try {
        action();
    } catch (const std::exception &error) {
        try {
            FCITX_WARN() << "Badi: " << step << " failed: " << error.what();
        } catch (...) {
        }
    } catch (...) {
        try {
            FCITX_WARN() << "Badi: " << step << " failed";
        } catch (...) {
        }
    }
}

} // namespace badi::fcitx5
