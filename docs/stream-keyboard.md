# Keyboard and fullscreen during a stream

Code: `desktop/src/keys.rs` (rules, no OS calls), `desktop/src/keyhook.rs` (Windows hook).

## Fullscreen

Fullscreen belongs to the stream. The app window (Computers, Files, Hosting, Friends,
Settings) never goes fullscreen and has no control for it. If the OS puts it there anyway
it is put back. Entering fullscreen from a stream remembers the window position and size.
Leaving it by any route (hotkey, menu, disconnect, error) restores them.

## Escape chord

The client keeps three hotkeys of its own. They sit behind one set of modifiers, the
escape keys. The default is Ctrl+Alt, as in 3.16 and earlier. Settings, Keyboard also offers Ctrl+Shift and Ctrl+Alt+Shift, which a host program is less likely to want.

| Keys | Action |
| --- | --- |
| escape keys + W | Fullscreen for the stream |
| escape keys + Q | Disconnect |
| escape keys + S | Show or hide the Secpar button |

The chord is never sent to the host. When it fires, the host first gets every held key
released, then the chord stays away from it until the keys are up. The stream menu shows the
chords next to Fullscreen and Disconnect, and the current escape keys in its footer. The
menu button and the mouse always work as a second way out.

## System shortcuts (Windows)

With Settings, Keyboard, "Send system shortcuts to the host" on (the default), a low-level
keyboard hook exists only while all of these hold: a stream is on screen, the window has
focus, the keys go to the host (no local stream menu open) and keyboard control is allowed.
Then every key goes to the host first, including Win and Win combos, Alt+Tab, Alt+Esc,
Alt+F4, Ctrl+Esc, Ctrl+Shift+Esc, Alt+Space, PrintScreen, the Menu key, F10, F11 and browser
and launch keys. Volume, media, sleep and input-method keys stay on this computer, because
the audio plays here.

The hook goes away at once on focus loss, disconnect, window close, error and panic. It
sends key-up for everything the host still holds. Two safeguards keep it from trapping the
keyboard. The callback passes every key through if the window thread has not sent a
heartbeat for 2 s, and when another window is in front. It never swallows the escape chord.
Keys already down when the hook comes up, and keys the OS injects, are left alone.

The wire protocol carries Windows virtual keys. The hook sends the virtual key the client
layout reports, with left and right Shift, Ctrl, Alt and Win kept apart. The host maps it to
its own layout. Auto-repeat is not resent, as before: the host keeps the key down.

## What cannot be intercepted

- Ctrl+Alt+Del. Windows handles it below any hook. The protocol has no secure-attention
  message, so there is no menu action for it.
- Win+L may not be blockable by a hook on every Windows version. Not verified.
- Linux and macOS: winit, which eframe uses, exposes no keyboard grab. X11 `XGrabKeyboard`
  and the Wayland `keyboard-shortcuts-inhibit` protocol would need extra windowing
  dependencies and were not added. The compositor keeps its own shortcuts (Alt+Tab, Super).
  The Super key is not delivered to the client by the toolkit, so it is not forwarded.
  The setting is shown on Windows only. Everything else reaches the host through the
  window as before, and the escape chord applies.
- A per-session "keyboard disabled" state does not exist in the protocol yet. The hook
  and the window path read one function (`App::keyboard_allowed`) so it has a single place
  to change.
