#pragma once

#include <fcitx-utils/eventloopinterface.h>
#include <fcitx-utils/event.h>
#include <nlohmann/json.hpp>

#include <functional>
#include <memory>
#include <string>

namespace badi::fcitx5 {

// Accessibility is a corroborating field observer. It never edits, decides
// policy, or blocks the input-method event loop.
class Accessibility {
public:
    using Reply = std::function<void(const nlohmann::json &)>;
    Accessibility(::fcitx::EventLoop &loop, Reply invalidated);
    ~Accessibility();
    Accessibility(const Accessibility &) = delete;
    Accessibility &operator=(const Accessibility &) = delete;
    bool request(nlohmann::json request, Reply reply);
    [[nodiscard]] bool pending() const { return static_cast<bool>(reply_); }
    void hide();
    void disconnect();

private:
    bool connect();
    bool receive();
    bool flush();
    void events();
    ::fcitx::EventLoop &loop_;
    Reply invalidated_;
    Reply reply_;
    std::unique_ptr<::fcitx::EventSourceIO> io_;
    std::unique_ptr<::fcitx::EventSourceTime> timeout_;
    int fd_ = -1;
    std::uint64_t serial_ = 0;
    std::string id_;
    std::string input_;
    std::string output_;
};

} // namespace badi::fcitx5
