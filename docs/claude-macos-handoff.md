# Claude macOS hosting handoff

Branch `claude/macos-host`, started from `f2cb44e`. Milestones 1 to 3 are committed.

## Done (measured)

New crate `crates/secpar-host-macos`. Everything in it except `native.rs` is platform neutral.

- `platform.rs`: the `Platform` trait. It is the only surface the host needs from macOS (permissions, displays, stream start).
- `session.rs`: per-device stream session. Checks Screen Recording consent first, then takes a slot from the shared three-device limit, then starts an independent stream. Stop and drop release everything. Input is forwarded only when Accessibility is granted.
- `backend.rs`: local command facade and agent on port 48999 (same commands as Linux, including `host_account_*` enrollment, owner sync, owner pairing links and revocation on owner change). Reuses `secpar-host-linux` identity and pairing store, which are portable, so pinned TLS identity, one-time codes, device tokens and revocation are the existing code.
- `native.rs` (macOS only): TCC preflight for Screen Recording and Accessibility, display list via CoreGraphics, ComputerName, Keychain credential storage.
- `.github/workflows/macos-arm64.yml`: the new crate's tests now run on the Mac runner.

- `media.rs`: the PRS UDP transport (probe pairing, ChaCha-style PRS crypt from `secpar-stream`, packetize/FEC, input dedup, Opus audio, Bye on drop) behind `VideoSource`, `AudioSource` and `InputSink` traits.
- `keymap.rs`, `annexb.rs`, `convert.rs`: Windows-VK to macOS key codes, AVCC to Annex-B, display fit sizing, planar/interleaved audio.
- `native_media.rs` (macOS only): ScreenCaptureKit display capture (YCbCr 420v, cursor shown, system audio 48 kHz stereo with this process excluded), VideoToolbox H.264 (hardware encoder preferred, real-time, no B-frames, forced keyframes on request).
- `native_input.rs` (macOS only): CGEvent mouse, scroll, keyboard and text injection, releasing held keys and buttons on drop. Only started when Accessibility is granted.

## Measured results

Windows 10, fake `Platform`, real loopback UDP and real PRS cipher:
`cargo test --manifest-path crates/secpar-host-macos/Cargo.toml --lib` (with `CARGO_TARGET_DIR` on a short path, the Opus CMake build fails under the long worktree path) gives 17 passed, 0 failed.
They cover permission denial reserving no slot, three streams plus a refused fourth and slot reuse, stop during startup, input gating and cleanup, identity and device-id persistence, the pairing link pinning the persisted fingerprint, and the transport: probe pairing, encrypted video arrives, input is applied, drop releases the held input, the capture source and the stream slot.
Also tested: key map (no unintended collisions), AVCC conversion (truncated/oversized lengths rejected), display fit sizing, audio interleaving.

## Mac runner results (GitHub Actions, macos-15 arm64, run 42316592862, branch claude/macos-host)

- Compiles natively: the whole crate including `native.rs`, `native_media.rs`, `native_input.rs` built without errors, and the desktop release build and dev package succeeded. This is compilation only.
- `cargo test --locked` for secpar-host-macos: 17 passed on the Mac, including the loopback transport test. The macOS-only modules have no unit tests of their own.
- The runner needs the newest installed Xcode: `screencapturekit` pulls `apple-metal`, whose Swift bridge fails on the default SDK (missing Metal 4 types). Anyone building locally needs an Xcode with that SDK. Consider this cost before keeping the dependency.
- `desktop/package.py` could not find a release tag on this branch, so the workflow now sets `SECPAR_RELEASE_TAG=v3.9.0-secpar-desktop-dev.1` (development artifact only).

## Not verified (no physical Mac)

- No capture, encode, audio or input has run, on the runner or anywhere. Headless runners have no Screen Recording consent or display session. No hardware acceleration, audible audio, end-to-end stream or latency claim is made.
- Keychain, TCC prompts, display listing, and account linking against the real service remain unverified.

## Known gaps

- H.264 only. HEVC is refused, not silently substituted.
- Primary display only; no display switching.
- A static screen delivers no new frames from ScreenCaptureKit, so a requested keyframe waits for the next change.
- Horizontal scroll direction and scroll step size (one line per 120 units) are guesses to check on a Mac.
- Account enrollment is implemented and unit-tested only for the unlinked/invalid paths with in-memory storage. A real link against accounts.secpar.win, and the Keychain storage itself, are untested. The desktop UI wiring below and moving the host TLS key into Keychain are not done.

## Desktop wiring (Mac-only files, done in this branch, opt-in)

1. `desktop/Cargo.toml`: macOS-only dependency on `secpar-host-macos` (`desktop/Cargo.lock` gains 5 packages, nothing changed or removed).
2. `desktop/src/local_host.rs`: a macOS `call()` that starts `Backend` once (`~/Library/Application Support/secpar`, port 48999, `MacPlatform`) and forwards commands. The fallback cfg is now `not(any(windows, linux, macos))`.
3. Off by default. Account sign-in calls `host_account_state` with enrollment on every platform, so without a guard a Mac would be enrolled and listening before capture is verified. The call returns the old "still in development" error unless `SECPAR_MACOS_HOST=1` is set. A tester on a physical Mac sets it to try hosting; remove the guard only after the physical tests pass.
4. Checked on Windows: desktop `local_host` tests pass (3). The macOS branch of `call()` has not been compiled yet. The Mac workflow run for this commit will show that.

## Shared integration Codex must apply later (not in this branch)

Kept out on purpose so the Host UI cannot work before capture does.

1. `desktop/src/hosting.rs` (lines ~99, ~237) and `desktop/src/lib.rs:664`: enable the Hosting page for macOS, and show a permissions row from `get_state.permissions` with `request_permission` buttons.
2. The desktop must install the rustls ring provider before hosting starts (it already does); the backend also installs it defensively.

## Next

Run the Mac workflow, fix compile errors, then manually test: permission prompts, an Android/desktop client viewing the Mac, keyboard and pointer input, audio, three simultaneous clients. Then wire the desktop app (items above) and test account linking on a real Mac.
