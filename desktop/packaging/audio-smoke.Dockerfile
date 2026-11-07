FROM secpar-wayland-review:local
RUN pacman -Syu --noconfirm --needed alsa-plugins pulseaudio pipewire python && useradd -m -u 1000 review
USER review
