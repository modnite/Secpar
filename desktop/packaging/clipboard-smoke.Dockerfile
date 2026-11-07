FROM secpar-audio-review:local
USER root
RUN pacman -Syu --noconfirm --needed sway wl-clipboard
RUN setcap -r /usr/bin/sway
USER review
