# The six app icons (packaging/icons/apps/<app>.svg), made from two sources so they never drift:
# each app's mark in the sidebar (Lucide, ISC: src/components/AppIcon.tsx), drawn in the logo's
# style (ink with a light outline, round ends) on the logo's cumulus cloud, in the app's colour.
# A seventh, send.svg, is Send to iPhone's in the file managers' menus: two rings spreading from a
# point, open at the foot, in teal, a colour none of the apps has.
# Run from the repository: python3 packaging/icons/apps/make.py
#   node_modules/lucide-react/dist/esm/icons packaging/icons/skykeep.svg packaging/icons/apps
import re, sys
ICONS, LOGO, OUT = sys.argv[1], sys.argv[2], sys.argv[3]
INK, LIGHT = "#151A22", "#EEF2F7"
src = open(LOGO).read()
CLOUD = src[src.index('<g'):src.index('<g transform="translate(530 590)')] + "</g>"
APPS = {  # app: (lucide icon, cloud body, base edge, elements left out at this weight)
  "notes": ("notebook-pen", "#FFB627", "#E39A0A", set()),
  "reminders": ("list-checks", "#FF7A2E", "#E35F12", set()),
  "calendar": ("calendar-days", "#FF5C57", "#E2413C", {"M8 17h.01", "M12 17h.01", "M16 17h.01"}),
  "contacts": ("contact-round", "#7C8798", "#626D7E", set()),
  "drive": ("folder-closed", "#2F9BFF", "#1F7FE6", set()),
  "photos": ("image", "#B65CE0", "#9A43C6", set()),
  "send": (None, "#1FC7C0", "#10A7A1", set()),
}
import math
def ring(r, a0, a1, cx=12, cy=12.5):
    """An arc of radius r round (cx, cy), clockwise from angle a0 to a1 (degrees, 0 = right)."""
    x0, y0 = cx + r * math.cos(math.radians(a0)), cy + r * math.sin(math.radians(a0))
    x1, y1 = cx + r * math.cos(math.radians(a1)), cy + r * math.sin(math.radians(a1))
    return f'<path d="M{x0:.2f} {y0:.2f} A{r} {r} 0 {1 if (a1 - a0) % 360 > 180 else 0} 1 {x1:.2f} {y1:.2f}"/>'
# Send to iPhone's mark, drawn here: no Lucide icon says it.
# Two rings, the weight of the apps' own marks, with the cloud showing round them.
SEND = ('<circle cx="12" cy="12.6" r="1.25"/>' + ring(4.4, 120, 60, cy=12.6) + ring(8.0, 128, 52, cy=12.6))
def nodes(name, drop):
    if name is None:
        return SEND
    src = open(f"{ICONS}/{name}.mjs").read()
    out = []
    for tag, attrs in re.findall(r'"(path|rect|circle)",\s*\{(.*?)\}', src, re.S):
        a = dict(re.findall(r'(\w+):\s*"([^"]*)"', attrs)); a.pop("key", None)
        if a.get("d") in drop: continue
        if tag == "circle" and name == "image": a["r"] = "2.6"   # the sun, readable at this weight
        out.append(f'<{tag} ' + " ".join(f'{k}="{v}"' for k, v in a.items()) + "/>")
    return "".join(out)
def mark(name, drop, colour, width):
    return (f'<g fill="none" stroke="{colour}" stroke-width="{width}" stroke-linecap="round" '
            f'stroke-linejoin="round">{nodes(name, drop)}</g>')
for app, (name, body, edge, drop) in APPS.items():
    cloud = CLOUD.replace('fill="#2F9BFF"', f'fill="{body}"').replace('fill="#1F7FE6"', f'fill="{edge}"')
    glyph = (f'<g transform="translate(520 450) scale(25) translate(-12 -12)">'
             f'{mark(name, drop, LIGHT, 4.6)}{mark(name, drop, INK, 2.7)}</g>')
    open(f"{OUT}/{app}.svg", "w").write(
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024">{cloud}{glyph}</svg>')
print("ok")

# Sizes made from each master (needs rsvg-convert): a 256 px PNG (the windows' own icon,
# src-tauri/src/apps.rs), a 1024 px PNG (the Store's tiles, msix/pack.ps1) and a Windows .ico of 16-256 px, every size PNG-compressed (Vista+).
import struct, subprocess
def png(svg, size):
    return subprocess.run(["rsvg-convert", "-w", str(size), "-h", str(size), svg],
                          check=True, capture_output=True).stdout
for app in APPS:
    svg = f"{OUT}/{app}.svg"
    open(f"{OUT}/{app}-256.png", "wb").write(png(svg, 256))
    # The Store package's tiles, drawn from this (its largest is about 744 px).
    open(f"{OUT}/{app}-1024.png", "wb").write(png(svg, 1024))
    images = [(s, png(svg, s)) for s in (16, 24, 32, 48, 64, 256)]
    offset, dirs, data = 6 + 16 * len(images), b"", b""
    for s, image in images:
        dirs += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(image), offset + len(data))
        data += image
    open(f"{OUT}/{app}.ico", "wb").write(struct.pack("<HHH", 0, 1, len(images)) + dirs + data)
# Each app's icon at every size the launchers ask for, as PNG, in the icon theme's own layout
# (hicolor/<n>x<n>/apps/<identifier>.<App>.png), copied whole into /usr/share/icons by every Linux
# package. The SVG alone is not enough: the icon theme spec asks each app for at least a 48 px
# PNG, and launchers that draw no SVG (or look only in sized folders) showed every app blank
# while Skykeep's own icon, PNG at every size, showed (#18).
import os
for app in APPS:
    for s in (16, 24, 32, 48, 64, 128, 256, 512):
        d = f"{OUT}/hicolor/{s}x{s}/apps"
        os.makedirs(d, exist_ok=True)
        open(f"{d}/io.github.yashwanthkamireddi.Skykeep.{app.capitalize()}.png", "wb").write(png(f"{OUT}/{app}.svg", s))
print("sizes ok")
