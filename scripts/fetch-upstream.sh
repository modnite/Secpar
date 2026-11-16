#!/bin/sh
# Fetch the pinned upstream sources into upstream/src/<name> at the exact commits
# in upstream/pins.json. Usage: scripts/fetch-upstream.sh [name ...]
# SECPAR_UPSTREAM_DIR=<dir> fetches somewhere else; on Windows use a short path,
# because the submodules exceed the path length limit under a deep directory.
# SECPAR_NO_SUBMODULES=1 skips submodules (enough for reading the source).
# With --check it only confirms that each pinned commit exists upstream.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
check=0
[ "${1:-}" = --check ] && { check=1; shift; }
python3 - "$root/upstream/pins.json" "$@" <<'PY' | while IFS='	' read -r name url commit subs; do
import json, sys
wanted = set(sys.argv[2:])
for s in json.load(open(sys.argv[1], encoding="utf-8"))["sources"]:
    if not wanted or s["name"] in wanted:
        print(s["name"], s["url"], s["commit"], "1" if s["submodules"] else "0", sep="\t")
PY
    subs=$(printf '%s' "$subs" | tr -d '\r')
    dir="${SECPAR_UPSTREAM_DIR:-$root/upstream/src}/$name"
    if [ "$check" = 1 ]; then
        tmp=$(mktemp -d "$root/upstream/.check.XXXXXX")
        git -C "$tmp" init -q
        if git -C "$tmp" fetch -q --depth 1 "$url" "$commit" 2>/dev/null; then echo "ok      $name $commit"; else echo "MISSING $name $commit"; fi
        rm -rf "$tmp"
        continue
    fi
    mkdir -p "$dir"
    [ -d "$dir/.git" ] || git -C "$dir" init -q
    git -C "$dir" fetch -q --depth 1 "$url" "$commit"
    git -C "$dir" checkout -q --detach FETCH_HEAD
    [ "$subs" = 1 ] && [ -z "${SECPAR_NO_SUBMODULES:-}" ] && git -C "$dir" -c core.longpaths=true submodule update -q --init --recursive --depth 1
    echo "fetched $name $(git -C "$dir" rev-parse HEAD)"
done
