# Friends

How people find each other, what a host owner allows each friend, and how requests reach the owner.

## Finding people

An account is hidden until its owner turns on Show me in search. The Friends page on desktop and Android has a Find people list with a search box. The list holds only visible accounts. Search matches the start of a username and ignores case. Your own account and anyone blocked in either direction never show up.

`GET /v1/directory?q=<prefix>&after=<cursor>` returns up to 20 rows of `handle`, `name`, `cursor` and `relation`. `relation` is `none`, `requested`, `incoming` or `friend`. There are no account ids and no way to list hidden accounts. It shares a limit of 60 requests a minute per address. `PUT /v1/me/visibility` with `{"visible": true}` changes your own setting. Adding still uses `PUT /v1/friends/by-username/:name`.

## What a friend may do

Each accepted friend has these settings. Defaults are View only and Ask first.

| Setting | Values | Meaning |
| --- | --- | --- |
| `keyboard` | `true`, `false` | Allows the keyboard. |
| `mouse` | `true`, `false` | Allows the mouse. |
| `connect` | `ask`, `always` | `always` approves a request at once. `ask` waits for the owner. |
| `input` | `view`, `full` | The older single setting. `full` means keyboard and mouse both on. Kept so older apps and hosts still work. |

`GET` and `PUT /v1/friends/:id/settings` read and change them. `GET /v1/friends` includes all of them for every friend. A row saved before migration `0008_keyboard_and_mouse.sql` has no `keyboard` or `mouse` of its own and follows `input`. Choosing `input` again sets both. Choosing `keyboard` or `mouse` sets `input` to `full` only while both are on, so an older host never allows more than the owner chose.

The owner can change these from the app, or from the host itself. The host settings on Windows and Linux list each friend with Keyboard, Mouse and Connect without asking. The host uses `GET /v1/host/friends` and `PUT /v1/host/friends/:id/settings` with its own credential. A friend sees what the owner allows on a computer in `GET /v1/friend-computers` (`connect`, `keyboard`, `mouse`).

The Worker enforces them when a request is made:

- A request loses `keyboard` or `mouse` when the owner has not allowed it for that friend. A request that asks for nothing else is refused with `403 input_not_allowed`.
- The owner cannot approve keyboard or mouse for a friend who may not have it.
- Turning both off revokes the friend's open requests that include input. Turning off one of them takes effect on a running session at the host's next check, and the other keeps working.
- A friend set to `always` gets an `approved` request and claims it without waiting.

The host checks again with `secpar_host_core::guest_policy`. `Account::admit_guest` reads the owner's friend list and the request lease with the machine credential, then decides:

- The grant must match the lease exactly.
- Keyboard and mouse are removed unless the friend has `full`.
- `Allow` means start without a prompt. `AskOwner` means the host must show its own consent first.
- Each running session holds a `Gate`. Video, audio and input paths ask the gate.

Windows and Linux hosts share this code and both accept friend sessions. See `docs/guest-access.md` for the checks and `docs/guest-client.md` for how a client connects.

## Friends on a host

Up to three friends can be connected at once. One friend holds one seat. A fourth friend gets `busy` and can try again when a seat opens. Friends share the PC's three streaming slots with the owner's devices.

The owner wins. When the owner starts a stream, every friend is disconnected. A friend cannot start a stream while the owner streams.

A friend is stopped at once when the PC locks or the lock state cannot be read, when the owner removes or blocks them, when the owner lowers them to view only (keyboard and mouse stop), or when the host cannot reach the account service for 30 seconds. The Let friends connect switch in the host settings turns all of this off, and the host stops telling the service it is available.

The Friends list on the Worker shows a computer only while its host says it is online and sharing. The host says so with every heartbeat, along with the id and address of its relay Worker.

## Controllers

A friend's USB or Bluetooth controller appears on the host as a virtual Xbox 360 controller (ViGEm on Windows, uinput on Linux). Games read at most four, so the host keeps one count for the whole PC. A friend may use two, friends together may use three, and one slot stays free for the owner. The next `pad` is answered with `pad_limit`.

## Notifications

The desktop app asks the account service every 10 seconds while it is running and signed in. Android asks every 15 seconds while Notify me of requests is on.

| Platform | Friend request | Connection request | Buttons |
| --- | --- | --- | --- |
| Windows | Toast | Toast | Accept, Decline or Allow, Deny |
| Linux | `notify-send` | `notify-send` | Same, needs libnotify 0.8 or later |
| Android | Notification | Notification | Accept, Decline or Allow, Deny |
| macOS | None | None | The app is not built yet |

On Windows a button opens a `secpar-alert:` link that carries an HMAC. The key lives in the user's credential store, so a web page cannot make a link that approves a request. The link starts the app in answer mode. It answers and exits without a window. Friend requests that were already waiting when the app started are not announced, because they have no time.

Android has no push service here. Turning on Notify me of requests starts a foreground service with a low priority notice. It needs the notification permission. Android does not let that service restart itself from the background after a reboot, so open the app once after a restart.

## Previews and sending files

A friend with the view permission may subscribe to thumbnails (`thumb.sub`, listed as `thumb` in `welcome.caps`). A session that holds only `view` and never starts a stream is a preview. Clients use one for the card of a friend set to connect at any time. The host does not tell its owner about a preview. It announces the friend when they start a stream. A preview gives way to a real connection, from the same friend or from another when all three seats are taken, and the host frees one seat.

Browsing the host's disks (`fs.list`, `fs.get`) is never allowed for a friend. A friend with the `files` permission can send files to the host and receive files the host offers. The Files page of a friend's computer is therefore an upload page.


## Friends page controls

Sign in and sign out on Computers. Friends keeps Refresh. Android separates friend
requests, connection requests and accepted friends. Search has its own expandable
section. Visibility and notification switches are in Friend settings.

The glyphs beside an accepted friend's name show keyboard access, mouse access and
connection policy. Accent means allowed; muted means blocked or approval required.
A check means connect any time. A question mark means ask first. Hover or long-press
for a label. The permission menu keeps the existing controls and server checks.

Windows, Linux and Android read the same `/v1/friend-computers` directory. Desktop
refreshes it every20 seconds and Android every30 seconds while the home UI is active.
A host must send the sharing, relay and computer identity fields introduced after
3.16.8. Pairing by link does not create that account directory entry. Discovery still
requires an accepted friendship, an online host and sharing enabled.
