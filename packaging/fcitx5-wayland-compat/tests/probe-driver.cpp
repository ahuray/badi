#include <fcitx-utils/event.h>
#include <fcitx-utils/eventloopinterface.h>
#include <fcitx-utils/keysym.h>
#include <fcitx/addonfactory.h>
#include <fcitx/addoninstance.h>
#include <fcitx/addonmanager.h>
#include <fcitx/event.h>
#include <fcitx/inputcontext.h>
#include <fcitx/inputpanel.h>
#include <fcitx/instance.h>
#include <fcitx/text.h>
#include <cstdio>
#include <cassert>
#include <cstdlib>
#include <memory>
#include <vector>
#include <unistd.h>

class ProbeDriver final : public fcitx::AddonInstance {
public:
    explicit ProbeDriver(fcitx::Instance *instance) : instance_(instance) {
        printf("PROBE_PID %ld\n",static_cast<long>(getpid()));fflush(stdout);
        events_.push_back(instance->watchEvent(fcitx::EventType::InputContextFocusIn,
            fcitx::EventWatcherPhase::PostInputMethod,[this](fcitx::Event &event){
                auto *input=static_cast<fcitx::InputContextEvent&>(event).inputContext();
                if(input->frontendName()=="wayland_v2") {
                    input_=input; puts("PROBE_FOCUS");fflush(stdout);
                    if(commitOnFocus_) {commitOnFocus_=false;input_->commitString("X");}
                }
            }));
        events_.push_back(instance->watchEvent(fcitx::EventType::InputContextFocusOut,
            fcitx::EventWatcherPhase::PostInputMethod,[this](fcitx::Event &event){
                if(input_==static_cast<fcitx::InputContextEvent&>(event).inputContext()) input_=nullptr;
            }));
        const int fd=std::stoi(std::getenv("BADI_PRIVATE_PROBE_FD"));
        io_=instance->eventLoop().addIOEvent(fd,fcitx::IOEventFlag::In,
            [this,fd](fcitx::EventSourceIO*,int,fcitx::IOEventFlags){
                char commands[32]; auto n=read(fd,commands,sizeof(commands));
                if(n<=0) return false;
                for(ssize_t i=0;i<n;i++) {
                    if(!input_) continue;
                    switch(commands[i]) {
                    case 'P': {fcitx::Text text("compose");text.setCursor(7);input_->inputPanel().setClientPreedit(text);input_->updatePreedit();break;}
                    case 'R': input_->inputPanel().setClientPreedit(fcitx::Text());input_->updatePreedit();break;
                    case 'C': input_->commitString("X");break;
                    case 'D': input_->deleteSurroundingText(-1,1);break;
                    case 'L': input_->inputPanel().setPreedit(fcitx::Text("local"));break;
                    case 'S': input_->inputPanel().setPreedit(fcitx::Text());break;
                    case 'X': instance_->processComposeString(input_,FcitxKey_Multi_key);assert(instance_->isComposing(input_));break;
                    case 'Z': instance_->resetCompose(input_);break;
                    case 'A': commitOnFocus_=true;break;
                    default: continue;
                    }
                    printf("PROBE_ACTION %c\n",commands[i]);fflush(stdout);
                }
                return true;
            });
    }
private:
    fcitx::InputContext *input_=nullptr;
    fcitx::Instance *instance_;
    bool commitOnFocus_=false;
    std::unique_ptr<fcitx::EventSourceIO> io_;
    std::vector<std::unique_ptr<fcitx::HandlerTableEntry<fcitx::EventHandler>>> events_;
};
class ProbeFactory final : public fcitx::AddonFactory {
public: fcitx::AddonInstance *create(fcitx::AddonManager *manager) override {return new ProbeDriver(manager->instance());}
};
FCITX_ADDON_FACTORY_V2(ackprobe,ProbeFactory)
