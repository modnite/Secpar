# Hardware encoding

The Windows host's FFmpeg-enabled build selects native NVENC on NVIDIA, FFmpeg
AMF on AMD, and FFmpeg Quick Sync on Intel. Media Foundation encoding is excluded
from that build. The client chooses a codec it can decode; the host tries those
codecs with the selected encoder. An explicit encoder choice never falls back to
Media Foundation.

Host settings save the encoder and its preset. Changes apply to new connections.
NVIDIA presets map to P1, P4 and P7; AMF uses speed, balanced and quality; Quick
Sync uses veryfast, medium and slow. All retain low-delay settings, no B frames,
requested keyframes and BT.709 limited-range output. Resolution and frame rate
remain separate controls. A preset that exceeds the GPU's frame budget can still
cause stutter.

The implementation follows the pinned Sunshine revision recorded in
`scripts/encoder-deps.lock.json`, including its Windows hardware-frame setup and
the matching build-deps patches. FFmpeg links into the host through avcodec and
avutil. No FFmpeg command-line program is launched. The Intel dispatcher loads
the installed GPU runtime. Hardware frames remain on the GPU; the bridge does
not download pixels to the CPU.

## Build

Windows requires Visual Studio C++ build tools, CMake, Ninja, and MSYS2 with make,
nasm and pkgconf. Run `scripts/build-windows-encoder-deps.ps1`, optionally passing
`-CacheRoot` to reuse a source cache. `-Offline` requires every pinned source to
be cached. The script applies the matching Sunshine patches and builds static
libraries without programs, decoders, container formats, network support or debug
symbols. It writes source revisions and license notices beside the libraries.

Set `SECPAR_FFMPEG_DIR` to the printed prefix and use
`RUSTFLAGS=-C target-feature=+crt-static` with the `ffmpeg,gpu-isolation` Cargo
features. `scripts/build-windows-release.ps1` uses those features by default and
includes the dependency notices in the installer. Static library archives and
compiler tools are not packaged.

## Validation and platform limits

The NVIDIA tests exercise native NVENC and FFmpeg NVENC on an RTX 2060: H.264 and
HEVC headers, forced keyframes, bitrate changes and all three presets. These
checks do not establish presentation pacing on a remote device. AMD and Intel
backends compile but still require hardware validation. Availability probes open
a small H.264 session on the capture adapter; successful probing does not guarantee
every resolution or concurrent-session count.

Linux desktop navigation and settings use the same UI code. Linux hosting and
decoding still use their existing platform paths; this Windows bridge is not a
Linux engine migration. Android currently uses Secpar's inherited PRS transport,
MediaCodec decoding and AudioTrack playback. It does not run Moonlight or Artemis.
The full Android engine migration is deferred; preserve the accepted audio and
input behavior while changing the host.
