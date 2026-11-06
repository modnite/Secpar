# Application updates

Android Preview 16 and Windows Desktop Development 6 add app-level update flows.
Existing older previews need one manual upgrade to obtain this updater.

Both apps check the Secpar GitHub release list on launch and every six hours.
They select their own product and architecture, handle numeric preview versions,
ignore drafts, and never select a legacy host/client-only package. Stable desktop
builds ignore development releases. Current previews follow the preview channel.

Android: Settings → Check for updates opens the update screen. Background checks
also post an update notification. Download & install verifies the GitHub SHA-256
and exact length, package ID, increasing Android version code and the installed
signing certificate. Android's system installer requires confirmation. The screen
works even when notifications are denied; no installation starts from a background
check. Allow Secpar under Install unknown apps when Android requests it.

Desktop: Updates appears in the toolbar, changing to Update available when a
newer compatible release exists. Download update checks the exact length and
GitHub SHA-256 before enabling Install update. Installation opens the native
installer with its normal system approval; a client stream must first disconnect.
The Windows installer updates the whole single Secpar app, including the hosting
service, and restarts that service. Review the warning and keep another route to
the PC available before updating remotely. The bundled legacy service's silent
update timer is disabled when the unified desktop UI is present.

Downloads use public HTTPS GitHub release metadata and its asset digest. This
provides transport/integrity verification, not independent publisher signing for
the current unsigned Windows installer. Android additionally requires its existing
APK signing identity. Release publishing access must remain restricted.

Linux and macOS use the same discovery code but reserve unified package names;
old client-only archives are deliberately ineligible. Native installation on those
platforms remains part of the unfinished unified host/client packages. They are
not being advertised as completed auto-updating products.

Release preparation: bump Android versionName/versionCode and the desktop tag in
desktop/build.rs (or provide SECPAR_RELEASE_TAG at build time). Desktop public
Windows asset: Secpar-Windows-x64-setup.exe. Android asset: Secpar-previewN.apk
(or Secpar.apk for stable). GitHub must expose the SHA-256 asset digest before
clients can offer a release. Verify release assets after upload. Never publish
an installer that updates only an internal desktop component.

Validation: 190 Android unit tests; the Updates activity launches in the isolated
emulator. Desktop tests cover product/channel/architecture asset filtering and
rejection of invalid or corrupted downloads using a loopback HTTP server. Native
system installation/relaunch still requires user validation; development work does
not replace the user's live installation.
