FROM debian:trixie
ENV DEBIAN_FRONTEND=noninteractive
WORKDIR /bootstrap
COPY scripts/linux_build.sh scripts/linux_build.sh
COPY packaging/linux/patches/ packaging/linux/patches/
RUN bash scripts/linux_build.sh --step=deps --skip-cuda --skip-libva --skip-package --sudo-off \
    && apt-get install -y --no-install-recommends libva-dev pulseaudio pulseaudio-utils x11-apps \
    && apt-get clean && rm -rf /var/lib/apt/lists/*
ENV CC=gcc-14 CXX=g++-14
WORKDIR /source
