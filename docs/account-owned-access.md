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
