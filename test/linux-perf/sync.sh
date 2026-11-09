#!/bin/bash
# Copies the worktree sources to OMV (/root/perf/src) and builds the tools there. Usage: sync.sh [build]
cd "$(dirname "$0")/../.." || exit 1
tar --exclude=target -cf - crates test | ssh cdsrv 'mkdir -p /root/perf/src && tar -C /root/perf/src -xf -'
[ "$1" = build ] && ssh cdsrv '/root/perf/b.sh'
