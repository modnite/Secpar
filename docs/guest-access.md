# Guest admission boundary

Host enrollment is deployed; the desktop can link/unlink the local Windows host.
Friend streaming is still disabled. `crate::guest_grant` verifies the account grant
format and provides a bounded, atomic one-use admission cache. It is not wired to
an incoming transport and is not sufficient to authorize a session on its own.

Before exposing a guest connection path:

- Fetch signing keys only from accounts.secpar.win over normally verified HTTPS.
  Bound responses and key count. Never follow token-supplied key URLs or keys.
- Match the grant audience against the installed machine registration. Verify the
  intended guest and request against `/v1/host/requests/:id` using the machine-only
  credential. Require a currently claimed, unexpired, unblocked friendship lease
  with exactly the signed permission set, and local sharing enabled.
- Consume the signed request atomically after the online/busy checks and before
  registering a guest session. A friend grant must never become an owner device,
  export an owner pairing token, or receive a rendezvous owner ticket.
- Use the same immutable permission policy for WebSocket commands and encrypted
  PRS input. A WebSocket-only check leaves a UDP input bypass. Deny power/unlock
  and filesystem browsing, even when generic file sharing was requested.
- Enforce video/audio/clipboard/file/controller permissions on outgoing data too.
  No-view and no-audio grants must not receive those streams. Support gamepad-only
  grants without silently adding desktop view or keyboard/mouse privileges.
- Stop guest capture/input on lock, sign-out, secure/UAC desktop or inability to
  determine the active desktop. The SYSTEM host currently follows Winlogon for
  owners; guest capture must never follow that owner path. Check before sending
  frames and before injecting input, not only in the one-second UI lock watcher.
- Recheck the online lease frequently with a short local deadline. Offline errors,
  unlinking, blocking or revocation must stop output and release held input/pads.
- Preserve the existing busy rejection. A guest cannot displace an owner's active
  stream. Fan-out and controller seats require separate ownership and cleanup.

Validation must include signed wrong-host/expired/unsupported grants, replay races,
revocation during both TCP control and UDP input, audio/view denial, secure-desktop
transitions, held-input cleanup, and owner/guest stream contention. Compilation or
signature tests alone are not end-to-end guest admission evidence.

## Friend settings

`crate::guest_policy` in `secpar-host-core` decides what a friend may do. It strips keyboard and mouse from a view only friend and says whether the host may start without asking. It is built and tested, and both hosts can call `Account::admit_guest`. No host transport accepts friend sessions yet, so the checks above still apply before it is wired in. See `docs/friends.md`.
