# Account-owned computer access

Status: server and Windows/Linux/Android native integration shipped in3.9.0.
User confirms Android3.9 sign-in lists the Windows PC and connects without QR.
Windows3.9 auto-enrollment is verified live. Linux remote hosting and
macOS hosting remain incomplete; do not describe them as supported remote hosts.
Owner pairing acceptance cases live in cloud/accounts/test/service.test.ts and
cloud/accounts/test/runtime-smoke.mjs.
The service only transports one-use host pairing codes, never paired-device tokens.
The recipient is the native account session that created the request. Enrolled
host identity, enrollment generation, account ownership, expiration and one-time
consumption are checked independently. Results are AES-GCM encrypted in D1 with
request context bound as authenticated data; the key is purpose-derived from the
existing private signing scalar. A key rotation safely expires in-flight results.
Native consumers must also compare the returned pairing fingerprint with the
account computer record before using the existing pinned-TLS pairing protocol.
Native3.9 clients use this API for owned pairing, storing per-client credentials
separately from independent manual pairings.

`GET /v1/computers` lists only the account's computers, including enrollment,
certificate fingerprint and heartbeat presence. Presence does not authorize a
connection. `POST /v1/owner-pairings` takes a computer ID and device name, scoped to
the calling native session. Its enrolled host polls `/v1/host/owner-pairings` and
POSTs a fresh pairing link to the request ID. Only the requesting session can GET
the result, once, before the two-minute expiry. Signing out cancels the effective
authorization; unlinking/relinking the host invalidates its pending work. Limits
bound concurrent requests; invalid links cannot replace an existing result.

Next: portable host enrollment and polling; independent bounded owner pairing
codes alongside manual QR codes; desktop and Android computer lists; local vault
entries per computer/client/account; LAN-first connection and authenticated relay
fallback. Never reuse one paired device identity on multiple clients.

## Pairing happens by itself (desktop)

There is no pairing step for a person to do. After sign-in the desktop app looks at the
account's online computers. For each one it has no credential for, it makes an owner
pairing request in the background, spends the one-use link once, and keeps the device
credential in the vault. Opening the computer later needs no pairing. One request runs at
a time and a computer that cannot answer is left alone for five minutes. The pairing link
box on the Computers page is gone. Android and the host's own QR page are unchanged.

Opening a computer that has a credential does not wait for the account. The pinned
fingerprint inside the credential is checked by the TLS handshake, so an account round
trip first adds only delay. The account can still rule a credential out: a computer that
is no longer linked, or has another identity in the last list, gets a new pairing. A
computer that answers with another certificate is treated like a refused credential and
renewed once. While the Computers list loads, a sync, a pairing in the background or a
slow local service never holds a command back. The Files page takes over the
authenticated connection the live preview already has, when there is one.

## Connection log

`%LOCALAPPDATA%\Secpar\logs\connect.log` on Windows and `$XDG_STATE_HOME/secpar/logs/connect.log`
(default `~/.local/state/secpar/logs/connect.log`) on Linux. One line per step:
`timestamp computer step detail ms=<took> at=<since the attempt began>`. Steps are
account_fetch, credential, pair_request, pair_wait, auto_pair, handoff, schedule, route_try
(lan or relay, with the outcome), tcp, tls, ws_upgrade, hello, welcome and first_reply.
`first_reply click_to_listing` is the time from the click on Open to the first folder on
screen. Computers show as the first eight characters of their id and an address as its kind
and last part (`lan4:.213`). Tokens, keys, pairing links and credentials never reach the
file, which rotates to `connect.log.1` and stays under 256 KB.
