#!/usr/bin/env python3
"""Prepare pinned upstream source in a NEW short build path; never change installs."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent


def git(*args, cwd=None):
    config = ["-c", "core.longpaths=true", "-c", "advice.detachedHead=false"]
    if cwd is not None:
        # Some build volumes do not record ownership. Trust only this freshly
        # cloned build tree for this invocation, never all user repositories.
        path = Path(cwd).resolve().as_posix()
        config += ["-c", f"safe.directory={path}", "-c", f"safe.directory={path}/*"]
    return subprocess.check_output(
        ["git", *config, *args], cwd=cwd, text=True,
        encoding="utf-8",
    ).strip()


def prepare(component, destination):
    lock = json.loads((HERE / "upstream-lock.json").read_text(encoding="utf-8"))
    entry = lock["components"][component]
    destination = Path(destination).resolve()
    # A fresh directory makes retries explicit and protects any local edits.
    if destination.exists():
        raise SystemExit(f"Choose a new output directory: {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    git("clone", "--depth", "1", "--branch", entry["tag"], entry["url"], str(destination))
    actual = git("rev-parse", "HEAD", cwd=destination)
    if actual != entry["commit"]:
        raise SystemExit("Upstream tag changed; source does not match the reviewed commit")
    git("submodule", "update", "--init", "--recursive", "--depth", "1", cwd=destination)
    modules = git("submodule", "status", "--recursive", cwd=destination)
    # Do not strip the status prefix on individual lines: '+'/'-' means a mismatch.
    if any(line[:1] in ("+", "-", "U") for line in modules.splitlines()):
        raise SystemExit("Nested dependency does not match its pinned gitlink")
    git("diff", "--exit-code", "HEAD", cwd=destination)
    git("submodule", "foreach", "--recursive", "git diff --exit-code HEAD", cwd=destination)
    applied = []
    for patch in sorted((HERE / "patches" / component).glob("*.patch")):
        git("apply", "--check", str(patch), cwd=destination)
        git("apply", str(patch), cwd=destination)
        applied.append({"name": patch.name, "sha256": hashlib.sha256(patch.read_bytes()).hexdigest()})
    origin = {"component": component, "upstream": entry, "patches": applied, "submodules": modules.splitlines()}
    (destination / "SECPAR-BUILD-ORIGIN.json").write_bytes(
        (json.dumps(origin, indent=2) + "\n").encode("utf-8")
    )
    print(f"Prepared {component} at {actual}: {destination}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("component")
    parser.add_argument("destination", help="Use a short path on Windows, e.g. D:/Secpar-CI/engine-build/client")
    args = parser.parse_args()
    prepare(args.component, args.destination)
