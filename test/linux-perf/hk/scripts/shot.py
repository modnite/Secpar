#!/usr/bin/python3
"""Saves the compositor's whole workspace as a PNG through KWin's ScreenShot2 D-Bus interface.
Usage: shot.py out.png   (needs KWIN_WAYLAND_NO_PERMISSION_CHECKS=1 on kwin, set by session.sh)"""
import os, sys, threading
import dbus
from PIL import Image

out = sys.argv[1] if len(sys.argv) > 1 else "/tmp/shot.png"
bus = dbus.SessionBus()
iface = dbus.Interface(bus.get_object("org.kde.KWin", "/org/kde/KWin/ScreenShot2"), "org.kde.KWin.ScreenShot2")
r, w = os.pipe()
data = bytearray()
def reader():
    while True:
        chunk = os.read(r, 1 << 20)
        if not chunk:
            break
        data.extend(chunk)
t = threading.Thread(target=reader)
t.start()
meta = iface.CaptureWorkspace({"native-resolution": True, "include-cursor": True}, dbus.types.UnixFd(w))
os.close(w)
t.join()
width, height, stride = int(meta["width"]), int(meta["height"]), int(meta["stride"])
image = Image.frombuffer("RGBA", (width, height), bytes(data), "raw", "BGRA", stride, 1)
image.convert("RGB").save(out)
print(f"{out} {width}x{height}")
