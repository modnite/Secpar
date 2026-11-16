# What the Secpar Linux application contains

Secpar itself is GPL-3.0 (see `Secpar-GPL-3.0.txt`). The application is one
file, and these parts of it come from other projects.

| Part | Licence | Where |
| --- | --- | --- |
| Qt 6.12 (libraries, QML modules, plugins) | LGPL-3.0 | `usr/lib/secpar/lib`, `usr/lib/secpar/qt6` |
| Sunshine, the hosting engine | GPL-3.0 | `usr/lib/secpar/engine` |
| Moonlight-qt stream component, with Secpar's in-stream menu | GPL-3.0 | `usr/lib/secpar/stream` |
| FFmpeg, as Sunshine packages it | LGPL-2.1 or later | linked into the engine |
| SDL2 | zlib | linked into the stream component |
| OpenH264 | BSD-2-Clause | linked into the stream component |
| Lucide icons | ISC | `Lucide-ISC.txt` |

Qt is shipped as separate, replaceable shared libraries, which is how the LGPL
asks for it to be used. The source of Sunshine, Moonlight and Secpar is at
<https://github.com/modnite/Secpar>, including the pinned upstream commits and
the patch set Secpar applies.

Libraries the operating system provides are not bundled: the graphics drivers,
Wayland and X11, PipeWire and PulseAudio. Their licences are the
distribution's.
