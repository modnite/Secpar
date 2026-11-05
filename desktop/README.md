# Secpar Desktop development client

Native Rust client for the existing Secpar Windows host. Android remains client-only. The eventual desktop app will also host, but this implementation currently provides the client side only. Do not install this over Secpar Host or PC Remote.

Implemented: pairing links for pinned direct TLS and signed relay bootstrap, in-memory reconnect credentials, encrypted UDP video, bounded frame assembly/FEC, duplicate rejection, direct candidate probing, STUN discovery, TURN allocation/channel/permission refresh, Opus audio playback, H.264 software decode to a GPU-rendered window, host-native/720p/1080p/1440p/4K resolution choice, native cursor images, display switching, fullscreen, relative mouse/keyboard, Ctrl+F12 cursor capture/release and held-input cleanup on focus loss/disconnect.

The window uses the accepted Secpar triangle and wordmark. No account login or friend buttons are shown before those services exist.

## Run

Requires a Rust toolchain, C/C++ build tools and CMake. Linux additionally needs ALSA and window-system development libraries (for example `libasound2-dev`, `libxkbcommon-dev` and `libwayland-dev` on Debian/Ubuntu).

```sh
cargo run --manifest-path desktop/Cargo.toml
```

Generate a fresh pairing link on Secpar Host, paste it in the client and choose Pair and connect. Pairing may interrupt an existing stream because the current host has one active streaming session. Do not use your only remote connection for the first live test.

The client captures mouse and keyboard when the first decoded video frame arrives while its window has focus. Ctrl+F12 releases/reacquires them. Fullscreen capture hides the toolbar until release. Focus loss releases capture and lifts held remote keys/buttons. Disconnect returns to the connection screen. Pairings remain in memory; there is intentionally no plaintext credential file. Persistent credentials require OS credential-store integration before release.

## Validation and remaining work

The portable client has real loopback TLS/WebSocket/UDP tests for pairing/reconnect, certificate mismatch, encrypted input, audio envelopes and video recovery with packet loss and duplicates. Decoder tests exercise actual H.264 and Opus bitstreams. Input tests cover release cleanup and fractional mouse movement. The same suite runs on the Windows desktop runner, OMV Linux runner and GitHub macOS runner.

These are development checks, not a claim that all hardware/platform combinations are supported. The first Windows window was launched, but visual automation could not capture it (Windows capture helper timed out); visual review remains outstanding. Real host-to-client sessions and physical mouse-grab behavior require validation on each target OS. TURN allocation against the production relay has not been tested with this desktop client.

Remaining client work: hardware decoding, audio/video timing and jitter handling, IME and full physical-key coverage, credential-store persistence, native signed packaging and live network/performance testing. Linux/macOS hosting, the unified host UI, concurrent guest sessions and account/friend features are separate unfinished milestones. This source is not a new stable release or a replacement for Preview 10.
