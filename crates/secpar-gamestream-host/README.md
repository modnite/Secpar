# Secpar upstream host controller

Owns a private Sunshine child process and its local management connection. No
capture, encoder, transport, frame pacing or audio engine is reimplemented here.
This crate is not wired into released Secpar and starts nothing by itself.

Implemented and checked:
- Separate state directory, exclusive instance lock, protected Windows DACL or
  Unix0700 directory/0600 secret files. Credentials are stable across restarts.
- Private configuration and desktop entry, bounded credential-helper startup,
  child readiness, explicit stop/reap and kill-on-drop for normal teardown.
- Local HTTPS management with an exact certificate pin and TLS handshake signature
  verification. No global crypto-provider assumption, proxy, redirect or system-CA
  fallback. An untrusted TLS endpoint never receives the management password.
- Bounded management responses and pending lists. Pair approval requires the
  backend's explicit device-identity-binding capability and an exact client ID /
  certificate fingerprint. It refuses the original backend rather than selecting
  an untrusted name or handing a PIN to whichever LAN request arrived first.
- State files passed to upstream cannot be symlinks/reparse points; Unix hardlinks
  are rejected before upstream can write through them.

Windows and Linux targeted tests pass. A real isolated Sunshine process review
checks authenticated readiness, duplicate-instance rejection, refusing unsupported
pair approval, stopping one owned host while another remains usable, and retaining
its TLS identity across restart. No installed host/service or user desktop is used.

Pending integration: backend certificate-binding API patch, authenticated Secpar
agent authorization, per-device enrollment/revocation journal, packaging and host
settings wiring. Abnormal parent-process death containment (Windows job / platform
service supervision) and physical capture/input/awake tests remain required before
production activation. Call explicit asynchronous stop during normal app/service
shutdown; dropping a child alone is not a crash-recovery design.

The private supervisor_review example and integrations/gamestream/supervisor-smoke.sh
are local test fixtures. They require SECPAR_SUPERVISOR_REVIEW=1 and generated X11 /
Pulse resources inside the isolated container; do not publish their debug binaries.
