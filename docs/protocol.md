# PC Remote Agent Protocol v1

Secpar uses this same protocol for Android and desktop clients. Its pairing
links use `secpar://pair`, control port 48999 and stream port 48998. The legacy
`pcremote://pair` links remain readable by clients. The upstream examples below
retain their original ports; there is no Android-specific hosting engine.

The contract between the Android app (`app/`) and PC Remote Host (`host/`). Both sides must match this file exactly; change it first, then both sides.

## Transport

- WebSocket over TLS: `wss://<host>:47999/v1`.
- The host uses a self-signed certificate generated on first run (ECDSA P-256, 10 years). The phone pins its SHA-256 fingerprint (lowercase hex of the DER) from the pairing QR code and rejects any other certificate. There is no CA validation and no plaintext fallback.
- **Text frames:** one JSON object each, with a mandatory string field `t` (the type). Unknown types are ignored. Unknown fields are ignored.
- **Binary frames:** byte 0 is the kind: `1` = file chunk, `2` = thumbnail.
- **Keepalive:** the client sends `{"t":"ping"}` every 10 s, and the host answers `{"t":"pong"}`. Either side closes the connection after 30 s of silence.

## Pairing QR code

```
pcremote://pair?n=<url-encoded PC name>&h=<ip1>,<ip2>&p=47999&fp=<64 hex>&t=<otp>
```

- `h`: the host's LAN IPv4 addresses, best first. The phone tries them in order.
- `t`: a one-time token, 22 chars base64url (128 bits). It's valid for 120 s or one use, and is regenerated when it expires or is used.

## Session start

The first frame from the client must be either `pair` or `hello`. Anything else gets `error{code:"unauthorized"}` and the connection closes.

### Pair (first time)
```json
→ {"t":"pair","otp":"<t from QR>","device":"Pixel 8","pin":"4821"}
← {"t":"paired","token":"<43 chars base64url, 256-bit>","pc":{...PcInfo}}
```
- `pin`: optional and ignored (v1 used it to pair Moonlight with Sunshine).
- `paired` is sent once the device token is stored, followed by `remote` (docs/remote-protocol.md) when a rendezvous Worker is configured.
- Errors: `bad_otp` (wrong or expired).
- After `paired`, the session is authenticated, exactly as if `hello` had succeeded.

### Hello (every later connection)
```json
→ {"t":"hello","v":1,"token":"<device token>","device":"Pixel 8"}
← {"t":"welcome","v":1,"pc":{...PcInfo},"caps":["input","media","clip","files","power","thumb","pad","net","stream","unlock","fsbrowse"],"locked":false}
```
- `locked`: the PC shows the lock or sign-in screen. Afterwards the host sends `{"t":"lock","locked":bool}` to every connection whenever that changes.
- Errors: `unauthorized` (unknown token), `revoked` (the device was removed on the PC). The host closes the connection after either one.

### PcInfo
```json
{"id":"<stable uuid>","name":"DESKTOP-ABC","user":"Sadik","macs":["AA:BB:CC:DD:EE:FF"],"os":"Windows 11 Pro 26300"}
```

## Input (cap `input`)

| Message | Meaning |
|---|---|
| `{"t":"mouse.move","dx":-3,"dy":12}` | Relative move in pixels (int, clamped to ±2000). |
| `{"t":"mouse.button","b":"left","down":true}` | `b` is one of `left`, `right`, `middle`, `x1`, `x2`. |
| `{"t":"mouse.scroll","dy":120,"dx":0}` | Wheel units (120 = one notch). Positive `dy` scrolls up. |
| `{"t":"key","vk":91,"down":true}` | Windows virtual-key code. |
| `{"t":"text","s":"héllo"}` | Types Unicode text (`KEYEVENTF_UNICODE`). Max 1000 chars. |

The host releases every key and button that is still held when a connection closes.

## Gamepad (cap `pad`)
```json
→ {"t":"pad","id":0,"btn":4096,"lt":0,"rt":255,"lx":0,"ly":-32768,"rx":1200,"ry":0}
```
- XInput state. `btn` is the XInput button mask; triggers are 0–255; sticks are int16.
- The host creates a virtual Xbox 360 controller per `id` (0–3) on the first `pad` message, and removes it 5 s after the connection closes. A phone that reconnects within those 5 s replaces its old controllers at once. Windows uses ViGEm. Linux uses `/dev/uinput` and offers `pad` only when it can write to it (`docs/linux.md`).
- `pad` also arrives inside PRS `INPUT` events (`docs/stream-protocol.md`) and drives the same controllers.
- Without a working controller driver the host answers `pad` with `error{code:"unsupported", of:"pad"}`.
- `{"t":"pad.off","id":0}` removes it immediately.
- Host → client: `{"t":"rumble","id":0,"lo":0-255,"hi":0-255}`.
- Which clients send `pad` from physical controllers, and what each host needs: `docs/gamepad.md`.

## Media (cap `media`)
```json
→ {"t":"media.sub","on":true}
← {"t":"media","title":"…","artist":"…","app":"Spotify","playing":true,"vol":45,"muted":false}
→ {"t":"media.cmd","c":"play_pause"}
```
- `c` is one of `play_pause`, `next`, `prev`, `vol_up`, `vol_down`, `mute`, `vol_set` (with `"v":0-100`).
- While subscribed, the host pushes `media` on every change, and once immediately.
- `title`, `artist` and `app` may be empty strings.
- Windows reads the current media session. Linux reads the MPRIS player that is playing (else the first with a track) and the default audio sink through `wpctl` or `pactl`. With no player, transport commands do nothing and `title`, `artist` and `app` are empty.

## Clipboard (cap `clip`)
```json
→ {"t":"clip.sub","on":true}
↔ {"t":"clip","mime":"text/plain","data":"…"}
↔ {"t":"clip","mime":"image/png","data":"<base64>"}
```
- Either side sends `clip` when its own clipboard changes (while subscribed). Receiving a `clip` sets the local clipboard without echoing it back.
- Size limits: text 1 MB, image 8 MB.

## Files (cap `files`)

**Phone → PC**
```json
→ {"t":"file.begin","id":"<uuid>","name":"photo.jpg","size":123456}
← {"t":"file.ready","id":"<uuid>","offset":0}
→ binary chunks
→ {"t":"file.end","id":"<uuid>","sha256":"<hex>"}
← {"t":"file.done","id":"<uuid>","path":"C:\\Users\\…\\Downloads\\PC Remote\\photo.jpg"}
```
- `offset > 0` in `file.ready` means a partial `<name>.part` file with the same id exists, and the phone resumes from there.
- `name` may be a relative path (`Trip/day 1/a.jpg`, `/` or `\` separators) to keep a folder's layout. Every component is sanitised: characters invalid on Windows become `_`, reserved names get a `_` prefix, and `.`, `..`, empty parts and drive/UNC prefixes are dropped, so paths are confined to the received-files folder. A clash adds ` (1)`, ` (2)` and so on.
- `{"t":"file.cancel","id"}` (either direction's id) stops a transfer; the host deletes the partial file.
- Errors: `file.error{id,code:"too_large|disk_full|bad_hash|denied"}`.

**PC → phone:** the host sends `{"t":"file.offer","id","name","size"}`. The phone replies `{"t":"file.accept","id","offset"}` or `{"t":"file.reject","id"}`. Chunks and `file.end` follow in the same format as above, in the other direction.

**Browsing the PC (cap `fsbrowse`, only while the PC allows files):**
```json
→ {"t":"fs.list","path":""}
← {"t":"fs.list","path":"","entries":[{"name":"Documents","path":"C:\\Users\\Sadik\\Documents","dir":true,"size":0,"mtime":0}, …]}
→ {"t":"fs.get","path":"C:\\Users\\Sadik\\Pictures\\Trip"}
← {"t":"file.offer","id","name":"Trip/a.jpg","size"} (one per file, at most 5000)
```
- `path: ""` lists the user's folders and the drives. Hidden and system entries are left out; folders come first.
- On Linux `path: ""` lists Home, the user's Desktop, Documents, Downloads, Pictures, Videos and Music folders, and `/`. Paths in `fs.list`, `fs.get` and `file.done` are native paths (`/home/ada/Downloads/Secpar/photo.jpg`). Dot-files are hidden in listings.
- Both hosts refuse `file.begin`, `fs.list` and `fs.get` (`file.error` `denied`, or `error` `unsupported`) while file transfer is off in the host's settings.
- Offers from `fs.get` use relative names like the uploads above.

**Binary chunk layout:** `[0x01][16-byte uuid][8-byte offset, big-endian][data ≤ 256 KiB]`.

## Unlock (cap `unlock`)
```json
→ {"t":"unlock","secret":"<PIN or password>"}
← {"t":"unlock.result","ok":true}
```
- The host wakes the display, lifts the lock curtain, types the secret (digits as number keys, the rest as Unicode) and presses Enter if still locked. `ok` says whether the PC is unlocked a few seconds later.
- The phone keeps the secret in a biometric-protected Android Keystore key and sends it only after a fingerprint check. The host never stores or logs it.

## Power (cap `power`)
```json
→ {"t":"power","a":"lock"}
← {"t":"ok","of":"power"}
```
`a` is one of `lock`, `sleep`, `restart`, `shutdown`, `signout`.

## Thumbnail (cap `thumb`)
```json
→ {"t":"thumb.sub","on":true}
```
- While subscribed, the host sends a binary frame `[0x02][JPEG]` with the primary display scaled to 320 px wide, quality 60, every 5 s, plus one immediately.
- A Linux host reuses the screen sharing permission saved by an earlier stream. With none saved, or while any stream runs, it sends no frame. It never asks for permission for a preview (`docs/linux.md`).

## Status and network (cap `net`)
```json
← {"t":"status","streaming":true,"to":"Pixel 8"}
→ {"t":"net"}
← {"t":"net","lan":["192.168.0.126"],"public":"203.0.113.7","anywhere":true,"upnp":true}
```
- `status` is reserved (v1 pushed it while Sunshine streamed); v2 hosts don't send it.
- `public` is null when unknown.

## Streaming (cap `stream`)
Session setup for the real-time media channel: `stream.start`, `stream.ready`, `stream.stop`, `stream.error` and `cursor.shape`. See `docs/stream-protocol.md` (PRS over UDP). Absolute pointer input, used by direct-touch streaming, is added:
```json
→ {"t":"mouse.pos","x":0.5,"y":0.25}
```
`x` and `y` are 0..1 fractions of the streamed display.

## LAN discovery
The host advertises mDNS `_pcremote._udp.local`, port 47999, with TXT `id=<PcInfo.id>`, `v=2` and `name=<pc name>`.

## Errors
```json
{"t":"error","code":"<code>","msg":"<human readable>","of":"<request t, optional>"}
```
Codes: `unauthorized`, `revoked`, `bad_otp`, `bad_request`, `unsupported`, `internal`.

## Managed GameStream engine (migration; not advertised by released hosts)

The optional `gamestream` capability uses the existing authenticated owner-device
agent connection. It never grants access based on a LAN address or device name.
Hosts advertise it only after a bundled backend with device-certificate binding,
revocation and process supervision is active. Original clients/hosts keep using
their negotiated capabilities during migration.

- `gamestream.info`: `{ "t":"gamestream.info", "id":"request-id" }`.
- `gamestream.pair`: `{ "t":"gamestream.pair", "id":"request-id", "client_id":"GameStream client identity", "certificate_sha256":"64 lowercase hex DER fingerprint", "pin":"four digits" }`.
- Success info: `{ "t":"gamestream.info", "id":"request-id", "http_port":48989, "https_port":48984, "certificate_sha256":"host certificate DER fingerprint" }`.
- Success enrollment: `{ "t":"gamestream.paired", "id":"request-id" }`.
- Failure: `{ "t":"gamestream.error", "id":"request-id", "code":"unavailable|bad_request|busy|...", "msg":"plain error" }`.

Request IDs are1–64 ASCII letters/digits/underscore/hyphen. Client IDs are1–128
bytes with no control characters. No management password, backend REST URL or
private key is sent. The platform's separate engine session receives the already
authenticated Secpar device ID; these messages cannot fall through to legacy input
handlers. Dropping/revoking the agent session drops that engine session so it can
cancel pending grants. Backend approval must independently bind both client ID
and certificate fingerprint atomically, journal enrollment/revocation, and resolve
cancellation races before permanent credentials are accepted. This protocol does
not itself implement or enable those backend transactions.
