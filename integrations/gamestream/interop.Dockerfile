FROM secpar-moonlight-build:local
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends libpipewire-0.3-0 libinput10 libevdev2 xvfb x11-apps pulseaudio pulseaudio-utils curl libcap2 libnuma1 libnotify4 libminiupnpc17 libayatana-appindicator3-1 && rm -rf /var/lib/apt/lists/*
