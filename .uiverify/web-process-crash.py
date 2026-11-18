#!/usr/bin/env python3
"""
A window whose web process is killed draws again by itself — in the real app, on a real display.

Each window's page runs in a WebKitWebProcess of its own. Killed (a crash, or the kernel's OOM
killer: WebKitGTK 2.54 has memory runaways on some GPUs), the window used to stay blank until
Skykeep was restarted. recover.rs loads the page again. This opens an app's window, kills its
web process with SIGKILL, as the OOM killer does, and checks that within about two seconds a
new web process has drawn the page again (its window_ready heard, the window's pixels not one
flat colour), that the window stayed on screen and Skykeep stayed running. Then it kills it
again, to see the second reload come after its pause.

Runs a Skykeep binary on a headless display of its own (weston + Xwayland), with its own
session bus and empty data folders: never the desktop's, never the person's data.

    uv run --with python-xlib .uiverify/web-process-crash.py <skykeep binary> [app]

Exits non-zero if the window stayed blank, went away, or Skykeep ended.
"""
import os
import shutil
import subprocess
import sys
import tempfile
import time

from Xlib import X, display

BIN = sys.argv[1]
KEY = sys.argv[2] if len(sys.argv) > 2 else "reminders"
NAMES = {"notes": "Notes", "reminders": "Reminders", "calendar": "Calendar",
         "contacts": "Contacts", "drivews": "Drive", "photos": "Photos"}
NAME = NAMES[KEY]
LABEL = f"app-{KEY}"


def find(d, title):
    """The top-level X window titled `title`, or None."""
    def walk(w):
        try:
            if w.get_wm_name() == title:
                return w
            for c in w.query_tree().children:
                f = walk(c)
                if f:
                    return f
        except Exception:
            return None
        return None
    return walk(d.screen().root)


def mapped(w):
    try:
        return w.get_attributes().map_state == X.IsViewable
    except Exception:
        return False  # gone


def colours(w):
    """How many different colours a grid of the window's pixels holds: 1 is a blank window."""
    try:
        g = w.get_geometry()
        seen = set()
        for y in range(10, g.height - 10, max(1, g.height // 24)):
            img = w.get_image(0, y, g.width, 1, X.ZPixmap, 0xFFFFFFFF)
            data = img.data
            step = max(4, (len(data) // 4 // 48) * 4)
            for i in range(0, len(data) - 3, step):
                seen.add(bytes(data[i:i + 3]))
        return len(seen)
    except Exception:
        return 0


def web(pid):
    out = subprocess.run(["pgrep", "-P", str(pid), "-f", "WebKitWebProcess"],
                         capture_output=True, text=True).stdout.split()
    return [int(p) for p in out]


def readies(path):
    with open(path, errors="replace") as f:
        return sum(1 for line in f if f"window_ready {LABEL}" in line)


def main():
    tmp = tempfile.mkdtemp(prefix="sk-crash-")
    sock = f"sk-crash-{os.getpid()}"
    weston = subprocess.Popen(
        ["weston", "--backend=headless", "--shell=kiosk", "--xwayland", f"--socket={sock}",
         "--width=1280", "--height=800"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        env={k: v for k, v in os.environ.items() if k not in ("WAYLAND_DISPLAY", "DISPLAY")}
        | {"XDG_RUNTIME_DIR": os.environ["XDG_RUNTIME_DIR"]})
    # Xwayland starts on the first X client; its display appears in /tmp/.X11-unix.
    disp = None
    end = time.time() + 10
    before = set(os.listdir("/tmp/.X11-unix"))
    while time.time() < end and not disp:
        new = set(os.listdir("/tmp/.X11-unix")) - before
        if new:
            disp = ":" + sorted(new)[0].lstrip("X")
        time.sleep(0.05)
    if not disp:
        print("FAIL no X display from weston")
        weston.terminate()
        return 1
    env = {"PATH": os.environ["PATH"], "HOME": tmp, "DISPLAY": disp, "GDK_BACKEND": "x11",
           "XDG_RUNTIME_DIR": os.environ["XDG_RUNTIME_DIR"],
           "XDG_CONFIG_HOME": f"{tmp}/config", "XDG_DATA_HOME": f"{tmp}/data",
           "XDG_CACHE_HOME": f"{tmp}/cache", "XDG_STATE_HOME": f"{tmp}/state",
           "WEBKIT_DISABLE_COMPOSITING_MODE": "1", "SKYKEEP_TIMING": "1", "SKYKEEP_LOG": "1"}
    bus = subprocess.Popen(["dbus-daemon", "--session", "--print-address", "--nofork"],
                           stdout=subprocess.PIPE, text=True)
    env["DBUS_SESSION_BUS_ADDRESS"] = bus.stdout.readline().strip()
    d = display.Display(disp)
    failures = 0
    app = None
    logpath = f"{tmp}/{KEY}.log"
    try:
        log = open(logpath, "w")
        app = subprocess.Popen([BIN, "--app", KEY], env=env, stdout=log, stderr=log)
        w, end = None, time.time() + 30
        while time.time() < end:
            w = find(d, NAME)
            if w is not None and mapped(w) and readies(logpath) and colours(w) > 1:
                break
            time.sleep(0.05)
        if w is None or not mapped(w):
            print(f"FAIL {NAME}: never shown")
            return 1
        time.sleep(1.0)  # settled: its first draw done, nothing still arriving
        print(f"     {NAME} drawn: {colours(w)} colours")

        # Killed twice: the first reload is at once, the second after recover.rs's pause (2 s).
        for round_, limit in ((1, 2.0), (2, 4.0)):
            procs = web(app.pid)
            if not procs:
                print(f"FAIL {NAME}: no web process under Skykeep ({app.pid}) to kill")
                failures += 1
                break
            seen = readies(logpath)
            for p in procs:
                os.kill(p, 9)
            killed = time.time()
            back = None
            while time.time() - killed < limit + 8:
                if app.poll() is not None:
                    break
                fresh = [p for p in web(app.pid) if p not in procs]
                if fresh and readies(logpath) > seen and mapped(w) and colours(w) > 1:
                    back = time.time() - killed
                    break
                time.sleep(0.05)
            if app.poll() is not None:
                print(f"FAIL kill {round_}: Skykeep itself ended (code {app.returncode})")
                failures += 1
                break
            if not mapped(w):
                print(f"FAIL kill {round_}: {NAME}'s window went away")
                failures += 1
            elif back is None:
                print(f"FAIL kill {round_}: {NAME} stayed blank ({colours(w)} colours, "
                      f"web processes {web(app.pid)})")
                failures += 1
            elif back > limit:
                print(f"FAIL kill {round_}: {NAME} drawn again, but only {back:.1f} s after "
                      f"(wanted within {limit:.0f} s)")
                failures += 1
            else:
                print(f"PASS kill {round_}: web process {procs} killed, {NAME} drawn again by "
                      f"a new one in {back:.1f} s; Skykeep still running")
            time.sleep(1.0)

        # The note it leaves for the person's diagnostics: why, and nothing private.
        diag = f"{tmp}/data/io.github.yashwanthkamireddi.Skykeep/diagnostics.log"
        found = [os.path.join(r, f) for r, _, fs in os.walk(tmp) for f in fs
                 if f == "diagnostics.log"]
        lines = []
        for path in found or [diag]:
            if os.path.exists(path):
                with open(path, errors="replace") as f:
                    lines += [ln.rstrip() for ln in f if "page" in ln and "window:" in ln]
        if lines:
            for ln in lines:
                print(f"     diagnostics: {ln}")
        else:
            print("FAIL no diagnostics note of the page's death")
            failures += 1
    finally:
        for p in (app, bus, weston):
            if p:
                # Asked to end, so weston takes its X display's socket and lock with it.
                p.terminate()
                try:
                    p.wait(5)
                except subprocess.TimeoutExpired:
                    p.kill()
        if not os.environ.get("KEEP"):
            shutil.rmtree(tmp, ignore_errors=True)
        else:
            print("kept", tmp)
    print(f"\n{failures} FAILED" if failures else "\nall passed")
    return 1 if failures else 0


sys.exit(main())
