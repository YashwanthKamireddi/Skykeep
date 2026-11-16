#!/usr/bin/env python3
"""Bump Skykeep to a new version in every file that carries it, and add its release notes.

    python3 .github/scripts/bump-version.py 0.1.27 notes.txt

notes.txt is the release notes paragraph (plain text, wrapped as you like). This writes the
version into the workspace crates, the app, package.json and its lockfile, the Arch PKGBUILD,
packages.yml's and snap.yml's defaults, Cargo.lock, and a <release> entry at the top of the
AppStream metainfo; then checks that nothing still names the old version. Commit the result
as "Skykeep <version>", blank line, the notes: the tag's commit message is the release notes.
"""
import json
import re
import subprocess
import sys
from datetime import date
from html import escape
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CRATES = sorted(ROOT.glob("crates/quince-*/Cargo.toml")) + [ROOT / "src-tauri/Cargo.toml"]
METAINFO = ROOT / "packaging/io.github.yashwanthkamireddi.Skykeep.metainfo.xml"


def swap(path: Path, pattern: str, new: str, count: int = 1) -> None:
    text = path.read_text()
    out, n = re.subn(pattern, new, text, count=count, flags=re.M)
    if n == 0:
        sys.exit(f"{path.relative_to(ROOT)}: nothing matched {pattern!r}")
    path.write_text(out)


def main() -> None:
    if len(sys.argv) != 3 or not re.fullmatch(r"\d+\.\d+\.\d+", sys.argv[1]):
        sys.exit(__doc__)
    new = sys.argv[1]
    notes = " ".join(Path(sys.argv[2]).read_text().split())
    if not notes:
        sys.exit("the notes are empty")
    old = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text())["version"]
    if tuple(map(int, new.split("."))) <= tuple(map(int, old.split("."))):
        sys.exit(f"{new} is not newer than {old}")

    for crate in CRATES:
        swap(crate, rf'^version = "{re.escape(old)}"', f'version = "{new}"')
    for f in ("package.json", "src-tauri/tauri.conf.json"):
        swap(ROOT / f, rf'"version": "{re.escape(old)}"', f'"version": "{new}"')
    swap(ROOT / "packaging/arch/PKGBUILD", rf"^pkgver={re.escape(old)}$", f"pkgver={new}")
    swap(ROOT / ".github/workflows/packages.yml", rf'default: "{re.escape(old)}"', f'default: "{new}"')
    swap(ROOT / ".github/workflows/snap.yml", rf"v{re.escape(old)}\b", f"v{new}", count=0)
    swap(
        METAINFO,
        r"^  <releases>\n",
        f'  <releases>\n    <release version="{new}" date="{date.today().isoformat()}">\n'
        f"      <description>\n        <p>{escape(notes, quote=False)}</p>\n      </description>\n"
        "    </release>\n",
    )

    subprocess.run(["npm", "install", "--package-lock-only", "--ignore-scripts", "--no-audit", "--no-fund"],
                   cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
    subprocess.run(["cargo", "update", "--workspace", "--offline"], cwd=ROOT, check=True,
                   stderr=subprocess.DEVNULL)

    # Nothing of ours still names the old version (other crates may share the number).
    left = []
    for f in [*CRATES, ROOT / "package.json", ROOT / "src-tauri/tauri.conf.json", ROOT / "packaging/arch/PKGBUILD",
              ROOT / ".github/workflows/packages.yml", ROOT / ".github/workflows/snap.yml"]:
        if old in f.read_text():
            left.append(str(f.relative_to(ROOT)))
    lock = json.loads((ROOT / "package-lock.json").read_text())
    if lock["version"] != new or lock["packages"][""]["version"] != new:
        left.append("package-lock.json")
    bumped = {re.search(r'^name = "([^"]+)"', c.read_text(), re.M).group(1) for c in CRATES}
    locked = dict(re.findall(r'name = "([^"]+)"\nversion = "([^"]+)"', (ROOT / "Cargo.lock").read_text()))
    left += [f"Cargo.lock ({n} {locked.get(n)})" for n in sorted(bumped) if locked.get(n) != new]
    if left:
        sys.exit("still the old version: " + ", ".join(left))
    print(f"Skykeep {old} → {new}: every file bumped; commit as \"Skykeep {new}\" with the notes as its body")


if __name__ == "__main__":
    main()
