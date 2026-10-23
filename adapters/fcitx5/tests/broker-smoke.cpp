#include "state.h"
#include "transport.h"

#include <fcitx-utils/event.h>
#include <fcitx-utils/eventloopinterface.h>
#include <nlohmann/json.hpp>

#include <array>
#include <cstdint>
#include <iostream>
#include <stdexcept>
#include <string>

namespace {

using namespace badi::fcitx5;
constexpr PanelObservation kPanel{.candidates = true, .candidatesOwnedByBadi = true};

void require(bool condition) {
    if (!condition) throw std::runtime_error("native broker transport contract failed");
}

nlohmann::json targetOf(const SessionState &state) {
    const auto target = desktopApplicationTarget(state.appId(), state.targetId());
    require(target.has_value());
    return *target;
}

// Exercise the shipped transport and session state against a real Rust broker.
// This fixture owns no Fcitx InputContext and therefore makes no UI/edit claim.
void run(const std::string &socket, const std::string &appId,
         const std::string &before, const std::string &expected) {
    ::fcitx::EventLoop loop;
    SessionState state;
    const auto session = appId == "omawrite"
                             ? "550e8400-e29b-41d4-a716-446655440001"
                             : "550e8400-e29b-41d4-a716-446655440000";
    require(state.focusIn(session, "smoke-context",
                          appId, "0123456789abcdef0123456789abcdef"));
    int suggestions = 0;
    int commits = 0;
    bool dismissed = false;
    std::unique_ptr<::fcitx::EventSourceTime> readingTimer;
    Transport *wire = nullptr;
    const auto publish = [&] {
        const auto update = state.updateContext(ContextWindow{
            .before = before, .after = "", .anchor = before.size(), .head = before.size(),
            .language = "en", .multiline = true,
        });
        require(update && wire->publishContext(*update));
    };
    Transport transport(loop, WireCallbacks{
        .onReady = [&] {
            require(wire->openSession(state.coordinates(), targetOf(state)));
            publish();
        },
        .onAuthority = [&](const AuthoritySnapshot &authority) {
            require(authority.initial && !authority.paused);
        },
        .onSuggestion = [&](Suggestion suggestion) {
            require(suggestion.text == expected);
            require(state.showSuggestion(std::move(suggestion), wire->nowMs()));
            ++suggestions;
            if (suggestions == 1) {
                // A person reads the panel before accepting; the candidate must
                // still be acceptable seconds later.
                readingTimer = loop.addTimeEvent(CLOCK_MONOTONIC, 0, 0,
                    [&](::fcitx::EventSourceTime *, std::uint64_t) {
                        const auto acceptance = state.requestAcceptance(wire->nowMs(), kPanel);
                        require(acceptance && wire->requestAcceptance(*acceptance));
                        return true;
                    });
                readingTimer->setNextInterval(3'200'000);
                readingTimer->setOneShot();
            } else {
                require(suggestions == 2 && commits == 1);
                const auto dismissal = state.requestDismissal(wire->nowMs(), kPanel);
                require(dismissal && wire->requestDismissal(*dismissal));
            }
        },
        .onClear = [&](const ClearNotice &notice) {
            if (suggestions != 2) return; // Superseding the first context may clear it.
            require(notice.coordinates == state.coordinates());
            dismissed = true;
            loop.exit();
        },
        .onCommitPrepare = [&](const CommitPrepare &prepare) {
            const auto dispatch = state.authorizeCommit(prepare, wire->nowMs(), kPanel);
            require(dispatch && ++commits == 1);
            require(wire->reportCommit(dispatch->coordinates, dispatch->controlId,
                                       dispatch->suggestionId, "dispatched-unverified"));
            publish();
        },
        .onDisconnected = [&] { loop.exit(); },
        .onPolicy = {},
    }, socket);
    wire = &transport;
    auto deadline = loop.addTimeEvent(
        CLOCK_MONOTONIC, 0, 0,
        [&](::fcitx::EventSourceTime *, std::uint64_t) { loop.exit(); return true; });
    deadline->setNextInterval(8'000'000);
    deadline->setOneShot();
    require(transport.connect());
    loop.exec();
    require(dismissed && suggestions == 2 && commits == 1);
    transport.disconnect();
}

void runReconnect(const std::string &socket, const std::string &appId) {
    ::fcitx::EventLoop loop;
    SessionState state;
    Transport *wire = nullptr;
    unsigned int connections = 0;
    unsigned int suggestions = 0;
    unsigned int disconnects = 0;
    Transport transport(loop, WireCallbacks{
        .onReady = [&] {
            ++connections;
            require(connections <= 2 && !state.suggestionVisible());
            require(state.focusIn(connections == 1
                ? "550e8400-e29b-41d4-a716-446655440001"
                : "550e8400-e29b-41d4-a716-446655440002",
                "recovery-context", appId, "0123456789abcdef0123456789abcdef"));
            require(wire->queryPolicy(state.coordinates(), targetOf(state)));
        },
        .onAuthority = [](const AuthoritySnapshot &authority) { require(authority.initial && !authority.paused); },
        .onSuggestion = [&](Suggestion suggestion) {
            require(suggestion.text == " for your time");
            require(state.showSuggestion(std::move(suggestion), wire->nowMs()));
            ++suggestions;
            if (suggestions == 1) std::cout << "READY_FOR_RESTART" << std::endl;
            else loop.exit();
        },
        .onClear = [](const ClearNotice &) {},
        .onCommitPrepare = [](const CommitPrepare &) { require(false); },
        .onDisconnected = [&] {
            ++disconnects;
            state.focusOut();
            require(!state.requestAcceptance(wire->nowMs(), kPanel));
        },
        .onPolicy = [&](std::string_view session, bool allowed, std::string, std::string, bool) {
            require(allowed && session == state.coordinates().sessionId);
            require(wire->openSession(state.coordinates(), targetOf(state)));
            const auto update = state.updateContext(ContextWindow{
                .before = "thank you", .after = "", .anchor = 9, .head = 9,
                .language = "en", .multiline = true,
            });
            require(update && wire->publishContext(*update));
        },
    }, socket);
    wire = &transport;
    auto deadline = loop.addTimeEvent(CLOCK_MONOTONIC, 0, 0,
        [&](::fcitx::EventSourceTime *, std::uint64_t) { loop.exit(); return true; });
    deadline->setNextInterval(12'000'000);
    deadline->setOneShot();
    transport.connect();
    loop.exec();
    require(connections == 2 && suggestions == 2 && disconnects == 1);
    transport.disconnect();
}

// Typing the suggestion's next characters gets its remainder without another
// generation, and Tab's word grant leaves the rest to carry on
// (ADR 0004). The phrase fixture would suggest differently for these texts.
void runTypeThrough(const std::string &socket, const std::string &appId) {
    ::fcitx::EventLoop loop;
    SessionState state;
    require(state.focusIn("550e8400-e29b-41d4-a716-446655440003", "type-through-context",
                          appId, "0123456789abcdef0123456789abcdef"));
    Transport *wire = nullptr;
    const std::array<std::string, 3> expected{" for your time", "r your time", " your time"};
    std::size_t shown = 0;
    bool wordGranted = false;
    bool dismissed = false;
    const auto publish = [&](const std::string &before) {
        const auto update = state.updateContext(ContextWindow{
            .before = before, .after = "", .anchor = before.size(), .head = before.size(),
            .language = "en", .multiline = true,
        });
        require(update && wire->publishContext(*update));
    };
    Transport transport(loop, WireCallbacks{
        .onReady = [&] {
            require(wire->openSession(state.coordinates(), targetOf(state)));
            publish("thank you");
        },
        .onAuthority = [](const AuthoritySnapshot &authority) { require(authority.initial && !authority.paused); },
        .onSuggestion = [&](Suggestion suggestion) {
            require(shown < expected.size() && suggestion.text == expected[shown]);
            require(state.showSuggestion(std::move(suggestion), wire->nowMs()));
            ++shown;
            if (shown == 1) return publish("thank you fo");
            if (shown == 2) {
                const auto word = state.requestAcceptance(wire->nowMs(), kPanel, true);
                require(word && word->expectedText == "r" && wire->requestAcceptance(*word));
                return;
            }
            const auto dismissal = state.requestDismissal(wire->nowMs(), kPanel);
            require(dismissal && wire->requestDismissal(*dismissal));
        },
        .onClear = [&](const ClearNotice &notice) {
            if (shown != expected.size()) return; // Superseded contexts clear earlier candidates.
            require(notice.coordinates == state.coordinates());
            dismissed = true;
            loop.exit();
        },
        .onCommitPrepare = [&](const CommitPrepare &prepare) {
            require(prepare.acceptance == "word" && prepare.text == "r");
            const auto dispatch = state.authorizeCommit(prepare, wire->nowMs(), kPanel);
            require(dispatch && !wordGranted);
            wordGranted = true;
            require(wire->reportCommit(dispatch->coordinates, dispatch->controlId,
                                       dispatch->suggestionId, "dispatched-unverified"));
            publish("thank you for");
        },
        .onDisconnected = [&] { loop.exit(); },
        .onPolicy = {},
    }, socket);
    wire = &transport;
    auto deadline = loop.addTimeEvent(CLOCK_MONOTONIC, 0, 0,
        [&](::fcitx::EventSourceTime *, std::uint64_t) { loop.exit(); return true; });
    deadline->setNextInterval(8'000'000);
    deadline->setOneShot();
    require(transport.connect());
    loop.exec();
    require(dismissed && wordGranted && shown == expected.size());
    transport.disconnect();
}

// An explicit revision of one selection (ADR 0012), against the broker's
// fixture reviser: shown, applied once over exactly the selection, refined
// and dismissed, and "nothing to change" answered without a revision.
void runRevision(const std::string &socket, const std::string &appId) {
    ::fcitx::EventLoop loop;
    SessionState state;
    require(state.focusIn("550e8400-e29b-41d4-a716-446655440003", "revision-context", appId,
                          "0123456789abcdef0123456789abcdef"));
    Transport *wire = nullptr;
    unsigned int shown = 0;
    bool applied = false;
    bool dismissed = false;
    bool unchanged = false;
    std::unique_ptr<::fcitx::EventSourceTime> readingTimer;
    const auto ask = [&](std::string_view text, std::size_t from, std::size_t to, std::string mode) {
        auto selection = captureSelectionWindow(text, to, from, true, "en");
        require(selection.has_value());
        const auto update = state.requestRevision(std::move(*selection), std::move(mode));
        require(update && wire->requestRevision(*update));
    };
    Transport transport(loop, WireCallbacks{
        .onReady = [&] {
            require(wire->openSession(state.coordinates(), targetOf(state)));
            ask("Yesterday i has went to the shop with Anna.", 10, 32, "grammar");
        },
        .onAuthority = [](const AuthoritySnapshot &authority) { require(authority.initial && !authority.paused); },
        .onSuggestion = [](Suggestion) { require(false); },
        .onClear = [](const ClearNotice &) {},
        .onCommitPrepare = [&](const CommitPrepare &prepare) {
            require(prepare.replaceSelection == SelectionRange{10, 32});
            const auto dispatch = state.authorizeRevisionCommit(prepare, wire->nowMs(), kPanel);
            require(dispatch && dispatch->text == "i went to the shop" && !applied);
            applied = true;
            require(wire->reportCommit(dispatch->coordinates, dispatch->controlId, dispatch->suggestionId,
                                       "dispatched-unverified"));
            ask("their going home", 0, 16, "refine");
        },
        .onDisconnected = [&] { loop.exit(); },
        .onPolicy = {},
        .onRevision = [&](Revision revision) {
            ++shown;
            require(state.showRevision(revision, wire->nowMs()));
            if (shown == 1) {
                require(revision.mode == "grammar" && revision.text == "i went to the shop");
                // A person reads the marked changes before applying them.
                readingTimer = loop.addTimeEvent(CLOCK_MONOTONIC, 0, 0, [&](::fcitx::EventSourceTime *, std::uint64_t) {
                    const auto acceptance = state.requestRevisionAcceptance(wire->nowMs(), kPanel);
                    require(acceptance && wire->requestAcceptance(*acceptance));
                    return true;
                });
                readingTimer->setNextInterval(1'500'000);
                readingTimer->setOneShot();
                return;
            }
            require(shown == 2 && revision.mode == "refine" && revision.text == "They're going home.");
            const auto dismissal = state.requestRevisionDismissal(wire->nowMs(), kPanel);
            require(dismissal && wire->requestDismissal(*dismissal));
        },
        .onRevisionClear = [&](const RevisionClear &clear) {
            if (shown == 2 && !dismissed) {
                require(clear.reason == "dismissed");
                dismissed = true;
                ask("Thanks for the reply.", 0, 21, "grammar");
                return;
            }
            require(dismissed && clear.reason == "no_change" && clear.coordinates == state.coordinates());
            unchanged = true;
            loop.exit();
        },
    }, socket);
    wire = &transport;
    auto deadline = loop.addTimeEvent(CLOCK_MONOTONIC, 0, 0,
        [&](::fcitx::EventSourceTime *, std::uint64_t) { loop.exit(); return true; });
    deadline->setNextInterval(8'000'000);
    deadline->setOneShot();
    require(transport.connect());
    loop.exec();
    require(applied && shown == 2 && dismissed && unchanged);
    transport.disconnect();
}

} // namespace

int main(int argc, char **argv) {
    if (argc != 3 && argc != 4 && argc != 5) return 64;
    try {
        if (argc == 4 && std::string(argv[1]) == "--reconnect") {
            runReconnect(argv[2], argv[3]);
        } else if (argc == 4 && std::string(argv[1]) == "--revision") {
            runRevision(argv[2], argv[3]);
            std::cout << "Native transport: revision shown, applied, refined, dismissed, unchanged passed\n";
            return 0;
        } else if (argc == 4 && std::string(argv[1]) == "--type-through") {
            runTypeThrough(argv[2], argv[3]);
            std::cout << "Native transport: type-through and word acceptance passed\n";
            return 0;
        } else {
            if (argc == 4) return 64;
            run(argv[1], argv[2], argc == 5 ? argv[3] : "thank you",
                argc == 5 ? argv[4] : " for your time");
        }
        std::cout << "Native transport: handshake, context, accept, report, dismiss passed\n";
        return 0;
    } catch (const std::exception &error) {
        std::cerr << error.what() << '\n';
        return 1;
    }
}
