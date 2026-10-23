#pragma once

// Fcitx's event sources: EventSourceIO, EventSourceTime and IOEventFlag.
// Newer Fcitx (5.1.12 on Debian 13, 5.1.23 on Arch) declares them in
// eventloopinterface.h; Fcitx 5.1.7 (Ubuntu 24.04) has no such header and
// declares the same interfaces in event.h.
#include <fcitx-utils/event.h>
#if __has_include(<fcitx-utils/eventloopinterface.h>)
#include <fcitx-utils/eventloopinterface.h>
#endif
