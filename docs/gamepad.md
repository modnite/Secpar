# Controllers

Secpar passes physical controllers from a client to the host. The host shows them to games as
Xbox 360 controllers. A client sends the `pad` messages described in `docs/protocol.md`.

## What each client reads

| Client | Source | Notes |
| --- | --- | --- |
| Windows | XInput (`xinput1_4.dll`, loaded at run time) | Up to four controllers, polled at about 125 Hz. Reads the Guide button and sends rumble back. |
| Linux | `/dev/input/event*` (evdev) | Up to four controllers. Rumble through force feedback when the pad supports it. |
| Android | Android input events | Bluetooth and USB controllers in Screen, Remote and Gamepad modes. Rumble goes to the controller's own motors. |
| macOS | none yet | The on-screen pad on Android still works. |

Controllers are read only while a stream is on screen and the Secpar window has focus. When
focus is lost or the stream ends, the host gets `pad.off` for every controller, so nothing stays held.
Only changes are sent. A held state is repeated every 250 ms and a change is repeated twice
quickly, because input travels over UDP.

### Windows

XInput covers Xbox controllers and anything that presents as one: Steam Input, DS4Windows,
8BitDo in XInput mode and similar. A bare DualShock or DualSense is not an XInput device.
Use Steam Input or DS4Windows to pass it. Secpar does not read `Windows.Gaming.Input`, to keep the installer small.

### Linux

The client opens every readable `/dev/input/event*` node that has a south face button and a left
stick. It does not grab the device. Virtual devices are skipped, so controllers that Secpar or
Steam Input create are never sent back. Buttons follow the kernel names: the xpad driver
(Xbox) and the PlayStation and Nintendo drivers are handled, and the PlayStation and
Nintendo pads map by position (cross is A). Pads with the right stick on Z and RZ, hat or button
d-pads and digital triggers also work. Sticks have a 3.6 % dead zone that still reaches full range.

If no pad is found, check that you can read the node: `ls -l /dev/input/event*`. Most desktops
give the logged in user access. Otherwise add your user to the `input` group.

### Android

Controller keys and sticks are taken while a Secpar stream, Remote or Gamepad screen is open
and the host offers `pad`. In Gamepad mode the on-screen pad keeps id 0 and physical controllers
take ids 1 to 3. In the other modes physical controllers take ids from 0.
A controller that is unplugged sends `pad.off` for its id.

## What the host needs

| Host | Needs | If missing |
| --- | --- | --- |
| Windows | The ViGEmBus driver | Hosting settings show "Gamepad unavailable: the ViGEmBus driver is not installed". The host does not offer `pad`, Android hides Gamepad mode, and the desktop client shows a notice when a controller is plugged in. |
| Linux | Write access to `/dev/uinput` | Hosting settings show the reason. See `docs/linux.md` for the udev rule. An AppImage cannot install the rule. |

The Windows installer does not bundle ViGEmBus, because it is a kernel driver from another
project. Install it from the ViGEmBus releases page (Nefarius). The host checks for it on every
connection, so no restart is needed after installing.

Each client device gets up to four controllers, ids 0 to 3. Controllers from one device are
released at once when it reconnects, and kept for 5 s after a dropped connection.

## Not covered

- macOS clients.
- DualShock and DualSense on Windows without a wrapper.
- Motion sensors, touchpads and gyro of any pad.
- A Windows PC that hosts a friend while you stream another PC with a controller: the virtual
  controllers the host creates look like real XInput controllers to your own client.
