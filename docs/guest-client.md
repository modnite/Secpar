# Connecting to a friend's PC

What a client does to use a friend's computer. The host side is in `docs/friends.md` and `docs/guest-access.md`.

## Steps

1. `GET /v1/friend-computers` lists computers of accepted friends that are online and have Let friends connect on. Each row has `id`, `name`, `platform`, `owner` and `owner_handle`.
2. `POST /v1/requests` with `{"computer": id, "permissions": [...]}`. Permissions are `view`, `audio`, `gamepad`, `keyboard`, `mouse`, `clipboard`, `files`. The Worker removes `keyboard` and `mouse` for a view only friend and answers `403 input_not_allowed` when nothing else is left. The answer has `id` and `state`.
3. `state` is `approved` at once when the owner set this friend to connect at any time. Otherwise it is `pending`. Poll `GET /v1/requests/:id` every 2 seconds until it is `approved`. `rejected` and `revoked` are final. A request that stays pending past its `expires` time (a Unix second in the answer) is dead.
4. `POST /v1/requests/:id/claim` answers `{"grant", "expires_at", "host": {"pc", "relay", "fingerprint"}}`. The grant lasts 60 seconds and works once. `409 host_not_sharing` means the host went offline or switched friends off. `409 grant_not_available` means it was already claimed.
5. Open a WebSocket to `<relay with wss>/v1/guest?pc=<host.pc>`. `relay` is an https origin. Use normal certificate checking, not pinning.
6. Send `{"t":"guest","grant":"<grant>"}` as the first frame. Nothing else may come first. The relay closes the socket with code 4401 otherwise.
7. The host answers `welcome` and the connection works like a paired device from then on. `welcome.caps` lists only what the grant allows: `stream`, `input`, `clip`, `files`, `pad`. Audio and view are requested in `stream.start`.
8. An `error` frame with one of these codes means the host refused:

| Code | Meaning |
| --- | --- |
| `guest_refused` | Bad or used grant, friend removed, friends switched off |
| `busy` | Three friends are already connected, or the owner is using the PC |
| `locked` | The PC is locked |
| `forbidden` | A message the grant does not allow. The session stays open |
| `guest_ended` | The host ended the session. `msg` says why |

## Streaming

`stream.start`, `stream.ready`, the UDP stream and `stream.cand` are the same as for a paired device (`docs/stream-protocol.md`). Two differences:

- A friend has no relay ticket, so there is no TURN allocation of its own. The client punches towards the host candidates in `stream.ready`. That list includes the host's own relayed address.
- `stream.start` is refused with `busy` while the owner streams. The host ends a friend's session when the owner starts a stream.

A view only session shows the screen and sends nothing. The host drops keyboard and mouse input, and does not answer it.

## Controllers

Plug a USB or Bluetooth controller into the client and it appears as a controller on the host. The client sends `pad` messages as in `docs/protocol.md`:

`{"t":"pad","id":0,"btn":4096,"lt":0,"rt":255,"lx":0,"ly":-32768,"rx":1200,"ry":0}`

- `id` is 0 to 3, one per local controller, in the order they were plugged in. Reuse the id when the same controller comes back.
- `btn` is the XInput button mask. Sticks are -32768 to 32767 with up positive on `ly` and `ry` as XInput defines. Triggers are 0 to 255.
- Send a state on every change and once a second while idle. Send `pad.off` when a controller is unplugged.
- Rumble arrives as `rumble {id, lo, hi}`. Play it on the matching local controller.
- The host gives a friend at most 2 controllers, and at most 4 across everyone on that PC. A `pad` that does not fit is answered with `error{code:"pad_limit"}`.
- A grant needs `gamepad`. Without it the host ignores `pad`.

Mapping is the client's job. Use a maintained library with a controller database: `gilrs` on Windows and Linux, and the platform input framework on Android. DualShock, DualSense, Xbox, Switch Pro and 8BitDo pads should all arrive as the XInput layout above. Gyro, touchpad and adaptive triggers are not forwarded.
