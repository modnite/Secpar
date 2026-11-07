FROM ubuntu:24.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    build-essential git ca-certificates pkg-config python3 \
    libegl1-mesa-dev libgl1-mesa-dev libopus-dev libsdl2-dev libsdl2-ttf-dev \
    libssl-dev libavcodec-dev libavformat-dev libswscale-dev libva-dev libvdpau-dev \
    libxkbcommon-dev libdrm-dev libwayland-dev wayland-protocols \
    qt6-base-dev qt6-declarative-dev libqt6svg6-dev qt6-wayland \
    qml6-module-qtquick-controls qml6-module-qtquick-templates qml6-module-qtquick-layouts \
    qml6-module-qtqml-workerscript qml6-module-qtquick-window qml6-module-qtquick \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /source
