NVIDIA NVENC API declarations from FFmpeg/nv-codec-headers n12.1.14.0, commit 1889e62e2d35ff7aa9baca2bceb14f053785e6f1.
Source: https://github.com/FFmpeg/nv-codec-headers/tree/1889e62e2d35ff7aa9baca2bceb14f053785e6f1
The MIT license is included at the top of nvEncodeAPI.h. No FFmpeg executable or library is linked or shipped. The encoder implementation loads the installed NVIDIA driver at runtime.

The native D3D11 resource registration, ultra-low-latency configuration and asynchronous completion follow Sunshine src/nvenc/nvenc_d3d11_native.cpp and nvenc_base.cpp (GPL-3.0).

Experimental selection: set SECPAR_WINDOWS_ENCODER=nvenc for the process being tested. Media Foundation remains the default. Explicit NVENC errors are returned rather than silently falling back, so a comparison cannot accidentally measure Media Foundation twice. H.264 and HEVC are implemented; AV1 is not yet implemented in this backend. Functional validation does not establish even frame pacing or justify changing the production default.
