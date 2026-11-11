#!/usr/bin/env python3
"""Which Rust packages a pull request's changes can affect, for ci.yml's rust job.

Reads the changed paths on stdin and prints one line:
  all         a change every crate builds on (the lockfile, the toolchain, the test or CI
              setup): the whole workspace is tested, as on every push to main;
  none        nothing Rust compiles changed (the interface, Android, docs): no Rust tests;
  <packages>  each changed package and every workspace package that depends on it, directly
              or not, space-separated: built and tested with -p, as Zed's CI does (rdeps).

The dependency graph is cargo's own (`cargo metadata`), so a new crate or a new dependency
between crates needs no change here.
"""
import json
import re
import subprocess
import sys

# Every crate builds on these: anything here tests everything.
EVERYWHERE = re.compile(
    r"^(Cargo\.toml|Cargo\.lock|rust-toolchain\.toml|deny\.toml|\.cargo/|\.config/|\.github/)"
)


def main() -> None:
    changed = [line.strip() for line in sys.stdin if line.strip()]
    if not changed or any(EVERYWHERE.match(path) for path in changed):
        print("all")
        return

    meta = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--locked"]))
    root = meta["workspace_root"].rstrip("/") + "/"
    members = set(meta["workspace_members"])
    by_id = {p["id"]: p for p in meta["packages"] if p["id"] in members}
    # Each package's directory, longest first, so a nested crate wins over its parent.
    dirs = sorted(
        ((p["manifest_path"][len(root):].rsplit("/", 1)[0] + "/", p["name"]) for p in by_id.values()),
        key=lambda d: -len(d[0]),
    )
    hit = set()
    for path in changed:
        for d, name in dirs:
            if path.startswith(d):
                hit.add(name)
                break
    if not hit:
        print("none")
        return

    # Who depends on whom, inside the workspace, from cargo's resolved graph.
    users: dict[str, set[str]] = {p["name"]: set() for p in by_id.values()}
    for node in meta["resolve"]["nodes"]:
        if node["id"] not in by_id:
            continue
        for dep in node["deps"]:
            if dep["pkg"] in by_id:
                users[by_id[dep["pkg"]]["name"]].add(by_id[node["id"]]["name"])
    affected, todo = set(), list(hit)
    while todo:
        name = todo.pop()
        if name not in affected:
            affected.add(name)
            todo.extend(users[name])
    print(" ".join(sorted(affected)))


if __name__ == "__main__":
    main()
