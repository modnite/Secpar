FROM archlinux:base-20261101.0.620205
RUN pacman -Syu --noconfirm --needed weston mesa libglvnd wayland libxkbcommon alsa-lib dbus libpulse libva
