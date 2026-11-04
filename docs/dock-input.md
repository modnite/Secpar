# Dock input and UI changes

This fork adds physical keyboard forwarding without opening the on-screen keyboard,
Android relative pointer capture, mouse buttons, wheel scrolling, and absolute mouse
fallback for desktop environments that do not grant capture. Ctrl+Alt+Shift+Z releases
pointer capture. Keys and mouse buttons are released when window focus is lost.

Selected quality presets are no longer limited to phone pixel dimensions. Automatic
quality uses the active display dimensions. Video remains aspect-fitted; this does
not create a host virtual display or change Windows display modes like Apollo.

The Android UI uses flat tonal surfaces, standard Material typography, simpler
corners, no ambient glow or glass reflections, and shorter onboarding and mode copy.
The Android updater checks this fork.

Validation still requires a physical USB dock: verify typing, Ctrl/Alt/Shift chords,
right and middle click, wheel, capture release, reconnect, and focus loss. Test the
phone display and an external display separately. Android may reserve system
shortcuts depending on OS/device; this change does not claim full Artemis parity.
