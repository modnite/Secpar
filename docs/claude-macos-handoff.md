# Claude macOS hosting handoff

Branch `claude/macos-host`, started from `f2cb44e`. This is milestone 1 of 2.

## Done (measured)

New crate `crates/secpar-host-macos`. Everything in it except `native.rs` is platform neutral.

- `platform.rs`: the `Platform` trait. It is the only surface the host needs from macOS (permissions, displays, stream start).
- `session.rs`: per-device stream session. Checks Screen Recording consent first, then takes a slot from the shared three-device limit, then starts an independent stream. Stop and drop release everything. Input is forwarded only when Accessibility is granted.
- `backend.rs`: local command facade and agent on port 48999 (same commands as Linux except account enrollment). Reuses `secpar-host-linux` identity and pairing store, which are portable, so pinned TLS identity, one-time codes, device tokens and revocation are the existing code.
- `native.rs` (macOS only): TCC preflight for Screen Recording and Accessibility, display list via CoreGraphics, ComputerName, Keychain credential storage.
- `.github/workflows/macos-arm64.yml`: the new crate's tests now run on the Mac runner.

Tests, run on Windows 10 with a fake `Platform`: `cargo test --manifest-path crates/secpar-host-macos/Cargo.toml` gives 6 passed, 0 failed.
They cover permission denial reserving no slot, three streams plus a refused fourth and slot reuse, stop during startup discarding the late stream, input gating and cleanup on drop, identity and device-id persistence across restarts, and the pairing link pinning the persisted fingerprint.

## Not done

- Screen capture, hardware encoding, system audio, remote input injection and the UDP media transport. `MacPlatform::start_stream` returns an explicit "not available in this build yet" error. Nothing pretends to stream.
- `native.rs` has never been compiled. This PC has no macOS target or Mac. The first Mac runner build may need small fixes.
- Account enrollment on the Mac host (`host_account_*` commands). The Keychain storage type exists but is unused. Enroll only once streaming works, otherwise the account would list a computer that cannot stream.
- Nothing was tested on physical Mac hardware. Keychain, TCC prompts, displays and permission detection are all unverified.
- Host TLS key is still a 0600 file under `~/Library/Application Support/secpar`, as on Linux. Moving it into Keychain is optional hardening.

## Shared integration Codex must apply later (not in this branch)

Kept out on purpose so the Host UI cannot work before capture does.

1. `desktop/Cargo.toml`: `[target.'cfg(target_os = "macos")'.dependencies] secpar-host-macos = { path = "../crates/secpar-host-macos" }`.
2. `desktop/src/local_host.rs`: a macOS `call()` that builds `Backend::start(config_directory(HOME), Arc::new(native::MacPlatform), "0.0.0.0:48999")` once and forwards to `Backend::command`. Replace the `not(any(windows, linux))` fallback with `not(any(windows, linux, macos))`.
3. `desktop/src/hosting.rs` (lines ~99, ~237) and `desktop/src/lib.rs:664`: enable the Hosting page for macOS, and show a permissions row from `get_state.permissions` with `request_permission` buttons.
4. Add `host_account_*` commands to the macOS backend using `KeychainStorage`.

## Next milestone

Capture via ScreenCaptureKit (macOS 12.3+, audio 13+), VideoToolbox H.264/HEVC, CGEvent input (needs Accessibility), and the UDP stream mirroring `secpar-host-linux/src/stream.rs`. Each needs a Mac runner or a real Mac to validate.
