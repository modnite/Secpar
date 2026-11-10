FROM archlinux:latest
RUN pacman -Syu --noconfirm --needed \
      dbus pipewire pipewire-pulse wireplumber libpulse \
      kwin xdg-desktop-portal xdg-desktop-portal-kde qt6-wayland \
      mesa libva-utils intel-media-driver \
      gstreamer gst-plugins-base gst-plugins-good gst-plugins-bad gst-plugins-ugly gst-plugin-va gst-plugin-pipewire \
      opus wev weston mesa-utils procps-ng psmisc python python-dbus python-gobject python-pillow \
      rust cmake clang base-devel pkgconf wayland wayland-protocols plasma-wayland-protocols gdb strace \
 && rm -rf /var/cache/pacman/pkg/* \
 && useradd -m -u 1000 -s /bin/bash hk \
 && mkdir -p /opt/nocap && cp /usr/bin/kwin_wayland /opt/nocap/kwin_wayland
COPY tools /tools
RUN cd /tools && wayland-scanner client-header /usr/share/plasma-wayland-protocols/fake-input.xml fake-input-client.h \
 && wayland-scanner private-code /usr/share/plasma-wayland-protocols/fake-input.xml fake-input-protocol.c \
 && gcc -O2 hk-click.c fake-input-protocol.c -lwayland-client -lm -o /usr/local/bin/hk-click
COPY scripts/ /opt/hk/
RUN chmod +x /opt/hk/*.sh
ENTRYPOINT ["/opt/hk/entrypoint.sh"]
