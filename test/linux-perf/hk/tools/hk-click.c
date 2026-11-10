// Moves the pointer to x y and left-clicks, through KWin's org_kde_kwin_fake_input protocol.
// Used by approve.py to press Allow in the portal dialog. KWin must run with
// KWIN_WAYLAND_NO_PERMISSION_CHECKS=1 (session.sh sets it), otherwise the bind is refused.
// Build: wayland-scanner client-header /usr/share/plasma-wayland-protocols/fake-input.xml fake-input-client.h
//        wayland-scanner private-code  /usr/share/plasma-wayland-protocols/fake-input.xml fake-input-protocol.c
//        gcc -O2 hk-click.c fake-input-protocol.c -lwayland-client -lm -o hk-click
#include <linux/input-event-codes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <wayland-client.h>
#include "fake-input-client.h"

static struct org_kde_kwin_fake_input *fake;
static void global(void *data, struct wl_registry *reg, uint32_t name, const char *iface, uint32_t version) {
    if (strcmp(iface, org_kde_kwin_fake_input_interface.name) == 0) {
        fake = wl_registry_bind(reg, name, &org_kde_kwin_fake_input_interface, version < 4 ? version : 4);
    }
}
static void global_remove(void *data, struct wl_registry *reg, uint32_t name) {}
static const struct wl_registry_listener listener = {global, global_remove};

int main(int argc, char **argv) {
    int chord_mode = argc >= 3 && strcmp(argv[1], "chord") == 0;
    int key_mode = argc == 4 && strcmp(argv[1], "key") == 0;
    if (argc != 3 && !key_mode && !chord_mode) {
        fprintf(stderr, "usage: hk-click X Y | hk-click key EVDEV_CODE DOWN\n");
        return 2;
    }
    double x = key_mode ? 0 : atof(argv[1]), y = key_mode ? 0 : atof(argv[2]);
    struct wl_display *display = wl_display_connect(NULL);
    if (!display) {
        fprintf(stderr, "no wayland display\n");
        return 1;
    }
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &listener, NULL);
    wl_display_roundtrip(display);
    if (!fake) {
        fprintf(stderr, "compositor has no org_kde_kwin_fake_input\n");
        return 1;
    }
    org_kde_kwin_fake_input_authenticate(fake, "hk-click", "headless test harness");
    if (chord_mode) {
        for (int i=2; i<argc; i++) {
            org_kde_kwin_fake_input_keyboard_key(fake, (uint32_t)strtoul(argv[i], NULL, 10), WL_KEYBOARD_KEY_STATE_PRESSED);
            wl_display_roundtrip(display); usleep(100000);
        }
        for (int i=argc-1; i>=2; i--) {
            org_kde_kwin_fake_input_keyboard_key(fake, (uint32_t)strtoul(argv[i], NULL, 10), WL_KEYBOARD_KEY_STATE_RELEASED);
            wl_display_roundtrip(display); usleep(100000);
        }
        return 0;
    }
    if (key_mode) {
        org_kde_kwin_fake_input_keyboard_key(fake, (uint32_t)strtoul(argv[2], NULL, 10),
            atoi(argv[3]) ? WL_KEYBOARD_KEY_STATE_PRESSED : WL_KEYBOARD_KEY_STATE_RELEASED);
        wl_display_roundtrip(display);
        return 0;
    }
    org_kde_kwin_fake_input_pointer_motion_absolute(fake, wl_fixed_from_double(x), wl_fixed_from_double(y));
    wl_display_roundtrip(display);
    usleep(150000);
    org_kde_kwin_fake_input_button(fake, BTN_LEFT, WL_POINTER_BUTTON_STATE_PRESSED);
    wl_display_roundtrip(display);
    usleep(80000);
    org_kde_kwin_fake_input_button(fake, BTN_LEFT, WL_POINTER_BUTTON_STATE_RELEASED);
    wl_display_roundtrip(display);
    return 0;
}
