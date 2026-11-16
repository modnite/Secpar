# Secpar 4 engine on the Secpar 3 connection

Secpar 4 is one app per OS with both roles. “Host” and “client” below are roles
inside that app, not separate products. The hosting service and the stream
component are internal parts of the one installation; there is no separate host
download and no separate client download. Android is one client-only app.

This is the integration contract for the Sunshine hosting role and Moonlight
connecting role on the existing authenticated 3.x control connection. It adds no
accounts, pairing links, knocks or relay keys. `docs/protocol.md` keeps the 3.x
rules; this file adds only the engine fields.

## Hosting role startup

- Start the bundled engine with `secpar-gamestream-host` on base port 46989. It
  listens on 127.0.0.1 only.
- Start a `secpar-tunnel` host on an ephemeral UDP port, forwarding to
  127.0.0.1:46989 with `Ports::engine(46989)` and an empty allow-list.
- Advertise the `gamestream` capability only when both are ready.
- The engine executable is `engine/sunshine.exe` in the host data directory on
  Windows and `engine/sunshine` on Linux. `SECPAR_ENGINE_PATH` overrides it for
  development only; never accept a path from a network message.
- Host state lives under `gamestream/` in the host data directory.

## `gamestream.info`

Client sends the existing request. The host answers:

```json
{"t":"gamestream.info","id":"r1",
 "http_port":46989,"https_port":46984,
 "certificate_sha256":"<engine client-certificate pin>",
 "tunnel_port":51234,
 "tunnel_certificate_sha256":"<host tunnel DER pin>",
 "tunnel_addresses":["192.168.1.20","203.0.113.7"]}
```

- `tunnel_addresses` are IPs only; `tunnel_port` is the port for all of them.
  The first is the LAN address, the second (if present) the public/UPnP one.
- The client may also use the address it used for the control connection when
  the list is empty.

## `gamestream.pair`

Client sends the existing fields plus the tunnel identity and its UDP endpoints
for punching:

```json
{"t":"gamestream.pair","id":"p1",
 "client_id":"<Moonlight unique id>",
 "certificate_sha256":"<Moonlight client certificate pin>",
 "pin":"1234",
 "tunnel_certificate_sha256":"<client tunnel DER pin>",
 "tunnel_candidates":["198.51.100.8:54321","192.168.1.30:54321"]}
```

The host:

1. approves the Moonlight certificate with the existing engine journal;
2. adds the client tunnel fingerprint to the tunnel allow-list;
3. sends small packets from the tunnel socket to `tunnel_candidates` for a few
   seconds so NATs open;
4. answers `gamestream.paired`.

Both fields are optional for old clients. A missing tunnel identity means the
host cannot let the client in and answers `gamestream.error` with
`bad_request`.

## Connecting role stream

1. `gamestream_info` on the existing control connection.
2. Create a `secpar-tunnel::Identity`. Get its public address with STUN on the
   UDP socket that will carry the tunnel. Send `gamestream.pair` with the
   identity pin and candidates.
3. Open the tunnel with that socket to the host's `tunnel_addresses` and
   `tunnel_port`, pinning `tunnel_certificate_sha256`. It listens on
   `127.0.0.2` with `Ports::engine(46989)`.
4. Start the Moonlight component against `127.0.0.2:46989` with the chosen
   resolution, FPS, bitrate, decoder, display mode and audio settings.
   Pair it with the host through `gamestream.pair` before the first stream.
5. The stream component, not the 3.x control connection, carries video, audio
   and input. The 3.x connection stays open for status, files, clipboard and
   the in-stream menu.

## Changing the stream size during a session

The engine's management port also takes a stream size change for a running
session, so a device can change resolution without ending the stream. Details,
including what it does and does not do, are in `docs/live-resolution.md`.

```json
POST /api/secpar/resolution  {"width":1280,"height":720}
```

- On the engine's own management port with the supervisor's credentials, and
  accepted from loopback only.
- The host rebuilds its encoder at that size, keeps capturing the same display
  and lets the new sequence header and keyframe reconfigure the client.
- `GET` on the same path reports the size the running stream encodes at.
- The tunnel's app channel carries the request as
  `{"app":{"resolution":{"width":1280,"height":720}}}`; only the owner's own
  devices are answered.

## Limits for the first builds

- Direct/LAN and a working public/UPnP path only. TURN relay credentials from
  the existing relay service are the next step; do not add a new key.
- The engine binds loopback; no engine port is opened on the LAN.
- One stream component per client; the host still enforces its three-device
  limit and per-device permissions.
