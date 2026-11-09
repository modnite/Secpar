# Friends

How people find each other, what a host owner allows each friend, and how requests reach the owner.

## Finding people

An account is hidden until its owner turns on Show me in search. The Friends page on desktop and Android has a Find people list with a search box. The list holds only visible accounts. Search matches the start of a username and ignores case. Your own account and anyone blocked in either direction never show up.

`GET /v1/directory?q=<prefix>&after=<cursor>` returns up to 20 rows of `handle`, `name`, `cursor` and `relation`. `relation` is `none`, `requested`, `incoming` or `friend`. There are no account ids and no way to list hidden accounts. It shares a limit of 60 requests a minute per address. `PUT /v1/me/visibility` with `{"visible": true}` changes your own setting. Adding still uses `PUT /v1/friends/by-username/:name`.

## What a friend may do

Each accepted friend has two settings. Defaults are View only and Ask first.

| Setting | Values | Meaning |
| --- | --- | --- |
| `input` | `view`, `full` | `full` allows keyboard and mouse. |
| `connect` | `ask`, `always` | `always` approves a request at once. `ask` waits for the owner. |

`GET` and `PUT /v1/friends/:id/settings` read and change them. `GET /v1/friends` includes both for every friend.

The Worker enforces them when a request is made:

- A view only friend loses `keyboard` and `mouse` from the request. A request that asks for nothing else is refused with `403 input_not_allowed`.
- The owner cannot approve keyboard or mouse for a view only friend.
- Lowering a friend to view only revokes their open requests that include input.
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
