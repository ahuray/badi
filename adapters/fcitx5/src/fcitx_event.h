#pragma once

// Fcitx's event sources: EventSourceIO, EventSourceTime and IOEventFlag.
// Newer Fcitx (5.1.12 on Debian 13, 5.1.23 on Arch) declares them in
// eventloopinterface.h; Fcitx 5.1.7 (Ubuntu 24.04) has no such header and
// declares the same interfaces in event.h.
#include <fcitx-utils/event.h>
#if __has_include(<fcitx-utils/eventloopinterface.h>)
#include <fcitx-utils/eventloopinterface.h>
#endif

#include <cstdint>

namespace badi::fcitx5 {

// Fcitx's sd-event loop treats a timer accuracy of 0 as its 250 ms default
// slack, which delayed the 120 ms typing pause by up to 250 ms and would
// stretch the observer's 500 ms reply deadline to 750 ms.
inline constexpr std::uint64_t kTimerAccuracyUs = 1'000;

} // namespace badi::fcitx5
