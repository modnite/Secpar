#!/usr/bin/python3
"""Presses Allow on every KDE portal consent dialog that appears. Runs until killed.

xdg-desktop-portal-kde shows "Remote Control Requested" (RemoteDesktop) and, for a plain ScreenCast, a
"Screen Sharing" chooser. This finds the window through KWin's WindowsRunner D-Bus service, reads its geometry
with getWindowInfo and clicks the button at a fixed offset from the window's bottom-right corner using
hk-click (fake input)."""
import subprocess, sys, time
import dbus

CAPTIONS = {"Remote Control Requested": (-141, -25)}  # caption -> offset of the Allow button from the bottom-right corner
bus = dbus.SessionBus()
runner = dbus.Interface(bus.get_object("org.kde.KWin", "/WindowsRunner"), "org.kde.krunner1")
kwin = dbus.Interface(bus.get_object("org.kde.KWin", "/KWin"), "org.kde.KWin")
done = set()
while True:
    try:
        for caption, (dx, dy) in CAPTIONS.items():
            for match in runner.Match(caption.split()[0]):
                if str(match[1]) != caption or str(match[0]) in done:
                    continue
                info = kwin.getWindowInfo(str(match[0]).split("_", 1)[1])
                x = float(info["x"]) + float(info["width"]) + dx
                y = float(info["y"]) + float(info["height"]) + dy
                time.sleep(1.0)  # let the dialog finish painting
                subprocess.run(["hk-click", str(x), str(y)], check=False)
                done.add(str(match[0]))
                print(f"approved '{caption}' at {x:.0f},{y:.0f}", flush=True)
    except dbus.DBusException as error:
        print("approve:", error, file=sys.stderr, flush=True)
    time.sleep(0.5)
