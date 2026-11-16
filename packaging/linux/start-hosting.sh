#!/bin/sh
# Secpar: the background part. Holds this computer's identity, runs the engine
# and answers the window on $XDG_RUNTIME_DIR/secpar-service.sock. The window
# starts this same command itself when nothing answers there, so both use one
# state folder and one channel.
here=$(cd -- "$(dirname -- "$0")" && pwd)
export LD_LIBRARY_PATH="$here:$here/lib:$here/engine/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
state="$HOME/.local/share/Secpar4"
mkdir -p "$state"
channel="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/secpar-service.sock"
cd -- "$here"
exec "$here/secpar-service" \
    --state "$state" \
    --channel "$channel" \
    --engine "$here/engine/sunshine" \
    --tunnel-port 46900 "$@"
