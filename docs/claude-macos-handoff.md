# Claude macOS hosting handoff

Branch `claude/macos-host`, started from `f2cb44e`. Milestones 1 and 2 are committed.

## Done (measured)

New crate `crates/secpar-host-macos`. Everything in it except `native.rs` is platform neutral.

- `platform.rs`: the `Platform` trait. It is the only surface the host needs from macOS (permissions, displays, stream start).
- `session.rs`: per-device stream session. Checks Screen Recording consent first, then takes a slot from the shared three-device limit, then starts an independent stream. Stop and drop release everything. Input is forwarded only when Accessibility is granted.
- `backend.rs`: local command facade and agent on port 48999 (same commands as Linux except account enrollment). Reuses `secpar-host-linux` identity and pairing store, which are portable, so pinned TLS identity, one-time codes, device tokens and revocation are the existing code.
- `native.rs` (macOS only): TCC preflight for Screen Recording and Accessibility, display list via CoreGraphics, ComputerName, Keychain credential storage.
- `.github/workflows/macos-arm64.yml`: the new crate's tests now run on the Mac runner.

- `media.rs`: the PRS UDP transport (probe pairing, ChaCha-style PRS crypt from `secpar-stream`, packetize/FEC, input dedup, Opus audio, Bye on drop) behind `VideoSource`, `AudioSource` and `InputSink` traits.
- `keymap.rs`, `annexb.rs`, `convert.rs`: Windows-VK to macOS key codes, AVCC to Annex-B, display fit sizing, planar/interleaved audio.
- `native_media.rs` (macOS only): ScreenCaptureKit display capture (YCbCr 420v, cursor shown, system audio 48 kHz stereo with this process excluded), VideoToolbox H.264 (hardware encoder preferred, real-time, no B-frames, forced keyframes on request).
- `native_input.rs` (macOS only): CGEvent mouse, scroll, keyboard and text injection, releasing held keys and buttons on drop. Only started when Accessibility is granted.

## Measured results

Windows 10, fake `Platform`, real loopback UDP and real PRS cipher:
`cargo test --manifest-path crates/secpar-host-macos/Cargo.toml --lib` (with `CARGO_TARGET_DIR` on a short path, the Opus CMake build fails under the long worktree path) gives 16 passed, 0 failed.
They cover permission denial reserving no slot, three streams plus a refused fourth and slot reuse, stop during startup, input gating and cleanup, identity and device-id persistence, the pairing link pinning the persisted fingerprint, and the transport: probe pairing, encrypted video arrives, input is applied, drop releases the held input, the capture source and the stream slot.
Also tested: key map (no unintended collisions), AVCC conversion (truncated/oversized lengths rejected), display fit sizing, audio interleaving.

## Not verified (no Mac available)

- `native.rs`, `native_media.rs` and `native_input.rs` have NEVER been compiled. They were written against the real `screencapturekit` 11.1.2 and `apple-cf` sources, and hand-declared VideoToolbox/CoreMedia/CoreGraphics C signatures. Expect a first round of compile fixes on the Mac runner (most likely: `SCStream`/handler `Send` bounds, exact method names).
- No capture, encode, audio or input has run. No hardware acceleration, audible audio, end-to-end stream or latency claim is made.
- Keychain, TCC prompts and display listing remain unverified.

## Known gaps

- H.264 only. HEVC is refused, not silently substituted.
- Primary display only; no display switching.
- A static screen delivers no new frames from ScreenCaptureKit, so a requested keyframe waits for the next change.
- Horizontal scroll direction and scroll step size (one line per 120 units) are guesses to check on a Mac.
- Account enrollment (`host_account_*`, `KeychainStorage` unused), the desktop UI wiring below, and the host TLS key in Keychain are still not done.

## Shared integration Codex must apply later (not in this branch)

Kept out on purpose so the Host UI cannot work before capture does.

1. `desktop/Cargo.toml`: `[target.'cfg(target_os = "macos")'.dependencies] secpar-host-macos = { path = "../crates/secpar-host-macos" }`.
2. `desktop/src/local_host.rs`: a macOS `call()` that builds `Backend::start(config_directory(HOME), Arc::new(native::MacPlatform), "0.0.0.0:48999")` once and forwards to `Backend::command`. Replace the `not(any(windows, linux))` fallback with `not(any(windows, linux, macos))`.
3. `desktop/src/hosting.rs` (lines ~99, ~237) and `desktop/src/lib.rs:664`: enable the Hosting page for macOS, and show a permissions row from `get_state.permissions` with `request_permission` buttons.
4. Add `host_account_*` commands to the macOS backend using `KeychainStorage`.

## Next

Run the Mac workflow, fix compile errors, then manually test: permission prompts, an Android/desktop client viewing the Mac, keyboard and pointer input, audio, three simultaneous clients. Then wire the desktop app and account enrollment (items above).
