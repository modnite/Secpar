#!/bin/bash
# Lists the windows KWin knows (caption | class | size | active) by running a one-shot KWin script.
# Output is read back from kwin.log, where script print() ends up.
. /tmp/hk-env
js=/tmp/hk-list.js
cat >"$js" <<'JS'
for (const w of workspace.windowList()) {
  print("WIN " + w.caption + " | " + w.resourceClass + " | " + w.frameGeometry.width + "x" + w.frameGeometry.height + " | active=" + w.active);
}
JS
mark=$(wc -l <"$HK_LOGS/kwin.log")
name=lst$RANDOM
id=$(dbus-send --session --print-reply --dest=org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript string:"$js" string:"$name" | awk '/int32/{print $2}')
dbus-send --session --print-reply --dest=org.kde.KWin "/Scripting/Script$id" org.kde.kwin.Script.run >/dev/null
sleep 0.5
tail -n +"$((mark + 1))" "$HK_LOGS/kwin.log" | grep -a "WIN " | sed 's/^.*WIN /WIN /'
dbus-send --session --dest=org.kde.KWin /Scripting org.kde.kwin.Scripting.unloadScript string:"$name" >/dev/null 2>&1
