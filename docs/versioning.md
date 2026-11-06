# Secpar release versions

Public versions use x.x.x. The first number advances for a major milestone; the
second for features and bug fixes; the third for minor fixes and visual polish.
The next Windows release is 3.1.0, above the installed legacy 3.0.3 version so
upgrades do not appear to be downgrades. Applications can advance when their own
changes ship; public release tags remain plain vX.Y.Z and select assets per OS.
Existing preview tags stay historical and updaters must continue accepting them.
Android package version codes remain increasing internal install identifiers.

Publish optimized release-profile binaries only. Debug, UI-review and test builds
remain local; never bundle them in a public installer. The Windows release builder
is scripts/build-windows-release.ps1, which checks version agreement and builds
both the UI and host in release mode with symbols stripped. No --debug switch.
The public Windows asset remains one complete installer with both roles.
