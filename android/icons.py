# Skykeep for Android's icons: the desktop's own marks (Lucide, ISC, as src/components/AppIcon.tsx
# and the sidebar use them), drawn as Android vector strokes so the phone and the desktop show
# the same symbol for the same thing. Run from the repository after `npm ci`:
#   python3 android/icons.py
import re
L = "node_modules/lucide-react/dist/esm/icons"
OUT = "android/app/src/main/res/drawable"
MAP = {  # drawable: Lucide icon
  "ic_notes": "notebook-pen", "ic_reminders": "list-checks", "ic_calendar": "calendar-days",
  "ic_person": "contact-round", "ic_drive": "folder-closed", "ic_photos": "image",
  "ic_home": "house", "ic_settings": "settings", "ic_sync": "refresh-cw",
  "ic_warning": "triangle-alert", "ic_cloud_off": "cloud-off", "ic_chevron_right": "chevron-right",
  "ic_back": "chevron-left", "ic_flag": "flag", "ic_add": "plus", "ic_search": "search",
  "ic_check_circle": "circle-check",
  "ic_today": "calendar", "ic_inbox": "inbox", "ic_check": "check", "ic_list": "list",
  "ic_folder": "folder", "ic_pin": "pin", "ic_edit": "square-pen", "ic_trash": "trash-2",
  "ic_heart": "heart", "ic_play": "play",
  "ic_more": "ellipsis-vertical", "ic_upload": "upload", "ic_folder_plus": "folder-plus",
  "ic_file": "file", "ic_file_text": "file-text", "ic_file_image": "file-image",
  "ic_file_video": "file-video-camera", "ic_file_archive": "file-archive", "ic_file_audio": "file-music",
  "ic_file_sheet": "file-spreadsheet", "ic_file_slides": "presentation",
  "ic_share": "share", "ic_eye_off": "eye-off", "ic_clock": "clock", "ic_video": "video",
  "ic_screenshot": "scan", "ic_panorama": "rectangle-horizontal", "ic_portrait": "aperture",
  "ic_albums": "images", "ic_lock": "lock", "ic_info": "info", "ic_close": "x", "ic_undo": "rotate-ccw",
  "ic_heart_fill": "heart",
}
# Drawn filled as well as stroked: a favourite's heart, set apart from the outline.
FILLED = {"ic_heart_fill"}
def f(v): return float(v)
def shape(tag, a):
  if tag == "path": return a["d"]
  if tag in ("circle", "ellipse"):
    cx, cy = f(a["cx"]), f(a["cy"]); rx = f(a.get("r", a.get("rx"))); ry = f(a.get("r", a.get("ry")))
    return f"M{cx-rx},{cy}a{rx},{ry} 0 1,0 {2*rx},0a{rx},{ry} 0 1,0 {-2*rx},0"
  if tag == "rect":
    x, y, w, h = f(a["x"]), f(a["y"]), f(a["width"]), f(a["height"]); r = f(a.get("rx", a.get("ry", "0")))
    if r == 0: return f"M{x},{y}h{w}v{h}h{-w}z"
    return (f"M{x+r},{y}h{w-2*r}a{r},{r} 0 0,1 {r},{r}v{h-2*r}a{r},{r} 0 0,1 {-r},{r}"
            f"h{-(w-2*r)}a{r},{r} 0 0,1 {-r},{-r}v{-(h-2*r)}a{r},{r} 0 0,1 {r},{-r}z")
  if tag == "line": return f"M{a['x1']},{a['y1']}L{a['x2']},{a['y2']}"
  if tag in ("polyline", "polygon"):
    return "M" + "L".join(a["points"].split()) + ("z" if tag == "polygon" else "")
  raise SystemExit(f"no conversion for <{tag}>")
for out, name in MAP.items():
  src = open(f"{L}/{name}.mjs").read()
  # An alias (trash-2 is trash): the icon it re-exports.
  alias = __import__("re").search(r"export \{ default \} from '\./([\w-]+)\.mjs'", src)
  if alias: src = open(f"{L}/{alias.group(1)}.mjs").read()
  nodes = re.findall(r'\[\s*"(\w+)",\s*\{(.*?)\}\s*\]', src, re.S)
  if not nodes: raise SystemExit(f"{name}: no shapes found")
  paths = []
  for tag, attrs in nodes:
    a = dict(re.findall(r'(\w+):\s*"([^"]*)"', attrs)); a.pop("key", None)
    paths.append(shape(tag, a))
  body = "".join(
    f'\n    <path android:pathData="{d}" android:strokeColor="#FF000000" android:strokeWidth="2"'
    f'\n        android:strokeLineCap="round" android:strokeLineJoin="round" android:fillColor="{"#FF000000" if out in FILLED else "#00000000"}" />'
    for d in paths)
  open(f"{OUT}/{out}.xml", "w").write(
    f'<?xml version="1.0" encoding="utf-8"?>\n'
    f'<!-- Lucide "{name}" (ISC): the desktop\'s mark, made by android/icons.py. -->\n'
    f'<vector xmlns:android="http://schemas.android.com/apk/res/android"\n'
    f'    android:width="24dp" android:height="24dp" android:viewportWidth="24" android:viewportHeight="24">{body}\n'
    f'</vector>\n')
  print(out, name, len(paths))

# Lucide's licence, shipped with its icons and shown in Settings › Open-source licences.
import shutil
shutil.copyfile("node_modules/lucide-react/LICENSE", "android/app/src/main/res/raw/lucide_license.txt")
print("lucide licence copied")
