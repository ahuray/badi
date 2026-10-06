#define _GNU_SOURCE
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/mman.h>
#include <wayland-server.h>
#include <xkbcommon/xkbcommon.h>
#include "input-method-server.h"
#include "virtual-keyboard-server.h"

static struct wl_display *display;
static struct wl_resource *input_method;
static uint32_t serial;
static int preedit, insertion, deletion;
static int queued_publications;
static char input_buffer[4096];
static size_t input_size;

static void release(struct wl_client *c, struct wl_resource *r) { (void)c; wl_resource_destroy(r); }
static void keymap(struct wl_resource *r, int grab) {
    struct xkb_context *context = xkb_context_new(XKB_CONTEXT_NO_FLAGS);
    struct xkb_rule_names names = {.layout = "us"};
    struct xkb_keymap *map = xkb_keymap_new_from_names(context, &names, XKB_KEYMAP_COMPILE_NO_FLAGS);
    assert(map);
    char *text = xkb_keymap_get_as_string(map, XKB_KEYMAP_FORMAT_TEXT_V1);
    int fd = memfd_create("badi-disposable-keymap", MFD_CLOEXEC);
    assert(fd >= 0 && write(fd, text, strlen(text)+1) == (ssize_t)strlen(text)+1);
    if (grab) {
        zwp_input_method_keyboard_grab_v2_send_keymap(r, 1, fd, strlen(text)+1);
        zwp_input_method_keyboard_grab_v2_send_modifiers(r, 1, 0, 0, 0, 0);
        zwp_input_method_keyboard_grab_v2_send_repeat_info(r, 25, 600);
    } else {
        wl_keyboard_send_keymap(r, WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1, fd, strlen(text)+1);
        if (wl_resource_get_version(r) >= 4) wl_keyboard_send_repeat_info(r, 25, 600);
    }
    close(fd); free(text); xkb_keymap_unref(map); xkb_context_unref(context);
}
static const struct wl_keyboard_interface keyboard_impl = {.release=release};
static void get_keyboard(struct wl_client *c, struct wl_resource *r, uint32_t id) {
    struct wl_resource *k = wl_resource_create(c, &wl_keyboard_interface, wl_resource_get_version(r), id);
    wl_resource_set_implementation(k, &keyboard_impl, NULL, NULL); keymap(k, 0);
}
static void unsupported_input(struct wl_client *c, struct wl_resource *r, uint32_t id) {
    (void)c; (void)id; wl_resource_post_error(r, 0, "probe only supports keyboards");
}
static const struct wl_seat_interface seat_impl = {
    .get_pointer=unsupported_input, .get_keyboard=get_keyboard, .get_touch=unsupported_input, .release=release};
static void bind_seat(struct wl_client *c, void *data, uint32_t version, uint32_t id) {
    (void)data;
    struct wl_resource *r=wl_resource_create(c,&wl_seat_interface,version < 7 ? version : 7,id);
    wl_resource_set_implementation(r,&seat_impl,NULL,NULL);
    wl_seat_send_capabilities(r,WL_SEAT_CAPABILITY_KEYBOARD);
    if (version>=2) wl_seat_send_name(r,"badi-private-seat");
}
static void commit_string(struct wl_client *c,struct wl_resource *r,const char *s) { (void)c;(void)r; insertion++; printf("{\"event\":\"insert\",\"bytes\":%zu}\n",strlen(s)); }
static void set_preedit(struct wl_client *c,struct wl_resource *r,const char *s,int32_t b,int32_t e) { (void)c;(void)r;(void)b;(void)e; preedit++; printf("{\"event\":\"preedit\",\"bytes\":%zu}\n",strlen(s)); }
static void delete_text(struct wl_client *c,struct wl_resource *r,uint32_t before,uint32_t after) { (void)c;(void)r; deletion++; printf("{\"event\":\"delete\",\"before\":%u,\"after\":%u}\n",before,after); }
static void commit(struct wl_client *c,struct wl_resource *r,uint32_t ack) {
    (void)c;(void)r;
    printf("{\"event\":\"commit\",\"serial\":%u,\"latest\":%u,\"preedit\":%d,\"insert\":%d,\"delete\":%d}\n",ack,serial,preedit,insertion,deletion);
    preedit=insertion=deletion=0;
    // Model Chromium's one-outstanding-publication rule: a response sends
    // another state only while a real pending client update exists.
    if (queued_publications > 0) {
        --queued_publications;
        zwp_input_method_v2_send_surrounding_text(input_method,"hello world",11,11);
        ++serial; zwp_input_method_v2_send_done(input_method);
        printf("{\"event\":\"sent_done\",\"serial\":%u}\n",serial);
    }
}
static const struct zwp_input_method_keyboard_grab_v2_interface grab_impl={.release=release};
static void grab_keyboard(struct wl_client *c,struct wl_resource *r,uint32_t id) {
    (void)r;
    struct wl_resource *g=wl_resource_create(c,&zwp_input_method_keyboard_grab_v2_interface,1,id);
    wl_resource_set_implementation(g,&grab_impl,NULL,NULL); keymap(g,1);
    puts("{\"event\":\"grab\"}");
}
static void popup(struct wl_client *c,struct wl_resource *r,uint32_t id,struct wl_resource *surface) { (void)c;(void)id;(void)surface;wl_resource_post_error(r,0,"popup unsupported by probe"); }
static const struct zwp_input_method_v2_interface im_impl={
    .commit_string=commit_string,.set_preedit_string=set_preedit,.delete_surrounding_text=delete_text,
    .commit=commit,.get_input_popup_surface=popup,.grab_keyboard=grab_keyboard,.destroy=release};
static void get_im(struct wl_client *c,struct wl_resource *r,struct wl_resource *seat,uint32_t id) {
    (void)r;(void)seat; assert(!input_method);
    input_method=wl_resource_create(c,&zwp_input_method_v2_interface,1,id);
    wl_resource_set_implementation(input_method,&im_impl,NULL,NULL);
    puts("{\"event\":\"input_method\"}");
}
static const struct zwp_input_method_manager_v2_interface manager_impl={.get_input_method=get_im,.destroy=release};
static void bind_manager(struct wl_client *c,void *data,uint32_t version,uint32_t id) {
    (void)data;(void)version;struct wl_resource *r=wl_resource_create(c,&zwp_input_method_manager_v2_interface,1,id);
    wl_resource_set_implementation(r,&manager_impl,NULL,NULL);
}
static void vk_keymap(struct wl_client *c,struct wl_resource *r,uint32_t format,int32_t fd,uint32_t size) { (void)c;(void)r;(void)format;(void)size;close(fd); }
static void vk_key(struct wl_client *c,struct wl_resource *r,uint32_t time,uint32_t key,uint32_t state) { (void)c;(void)r;(void)time;printf("{\"event\":\"key\",\"code\":%u,\"state\":%u}\n",key,state); }
static void vk_mods(struct wl_client *c,struct wl_resource *r,uint32_t d,uint32_t l,uint32_t k,uint32_t g) { (void)c;(void)r;(void)d;(void)l;(void)k;(void)g; }
static const struct zwp_virtual_keyboard_v1_interface vk_impl={.keymap=vk_keymap,.key=vk_key,.modifiers=vk_mods,.destroy=release};
static void create_vk(struct wl_client *c,struct wl_resource *r,struct wl_resource *seat,uint32_t id) {
    (void)r;(void)seat;struct wl_resource *v=wl_resource_create(c,&zwp_virtual_keyboard_v1_interface,1,id);
    wl_resource_set_implementation(v,&vk_impl,NULL,NULL);
}
static const struct zwp_virtual_keyboard_manager_v1_interface vk_manager_impl={.create_virtual_keyboard=create_vk};
static void bind_vk_manager(struct wl_client *c,void *data,uint32_t version,uint32_t id) {
    (void)data;(void)version;struct wl_resource *r=wl_resource_create(c,&zwp_virtual_keyboard_manager_v1_interface,1,id);
    wl_resource_set_implementation(r,&vk_manager_impl,NULL,NULL);
}
static int command(int fd,uint32_t mask,void *data) {
    (void)mask;(void)data;
    ssize_t n=read(fd,input_buffer+input_size,sizeof(input_buffer)-input_size-1);
    if(n<=0){wl_display_terminate(display);return 0;}
    input_size+=(size_t)n; input_buffer[input_size]=0;
    char *newline;
    while((newline=memchr(input_buffer,'\n',input_size))){
        *newline=0;
        if(!strcmp(input_buffer,"quit")){wl_display_terminate(display);return 0;}
        assert(input_method);
        if(!strcmp(input_buffer,"activate")) {
            zwp_input_method_v2_send_activate(input_method);
            zwp_input_method_v2_send_content_type(input_method,0,0);
            zwp_input_method_v2_send_surrounding_text(input_method,"hello",5,5);
        } else if(!strcmp(input_buffer,"deactivate")) zwp_input_method_v2_send_deactivate(input_method);
        else if(!strcmp(input_buffer,"chain")) { queued_publications=7; zwp_input_method_v2_send_surrounding_text(input_method,"hello world",11,11); }
        else if(!strcmp(input_buffer,"surround")) zwp_input_method_v2_send_surrounding_text(input_method,"hello world",11,11);
        else assert(!strcmp(input_buffer,"done"));
        ++serial; zwp_input_method_v2_send_done(input_method);
        printf("{\"event\":\"sent_done\",\"serial\":%u}\n",serial);
        size_t used=(size_t)(newline-input_buffer)+1; input_size-=used;memmove(input_buffer,input_buffer+used,input_size);input_buffer[input_size]=0;
    }
    wl_display_flush_clients(display);return 0;
}
int main(int argc,char **argv) {
    assert(argc==2);setvbuf(stdout,NULL,_IOLBF,0);
    display=wl_display_create();assert(display);
    if (wl_display_add_socket(display,argv[1]) != 0) {
        fputs("Unable to create private Wayland socket\n",stderr);
        wl_display_destroy(display);return 1;
    }
    assert(wl_global_create(display,&wl_seat_interface,7,NULL,bind_seat));
    assert(wl_global_create(display,&zwp_input_method_manager_v2_interface,1,NULL,bind_manager));
    assert(wl_global_create(display,&zwp_virtual_keyboard_manager_v1_interface,1,NULL,bind_vk_manager));
    assert(wl_event_loop_add_fd(wl_display_get_event_loop(display),STDIN_FILENO,WL_EVENT_READABLE,command,NULL));
    puts("{\"event\":\"ready\"}");wl_display_run(display);
    wl_display_destroy_clients(display);wl_display_destroy(display);return 0;
}
