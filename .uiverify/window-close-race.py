#!/usr/bin/env python3
"""
A window closed the moment it opens stays closed — in the real app, on a real display.

Each app window is made hidden and shown once its page has drawn, or by a three-second fallback.
Until 0.1.27 either one showed the window again after it was closed: closed as it appeared, it
came straight back (17 Nov). This opens an app's window, closes it as soon as it is on screen
— as a window manager does, with WM_DELETE_WINDOW — and watches that it never maps again.

Runs a Skykeep binary on a headless display of its own (weston + Xwayland), with its own
session bus, a stand-in tray (so a closed window is put away, as on a desktop with a tray)
and empty data folders: never the desktop's, never the person's data.

    uv run --with python-xlib .uiverify/window-close-race.py <skykeep binary> [app ...]

Then every window is closed, and their web processes must end once the windows are let go
(about a minute; QUICK=1 skips this). Exits non-zero if any window came back or any web
process outlived its window.
"""
import os
import select
import shutil
import subprocess
import sys
import tempfile
import time

from Xlib import X, display, protocol

BIN = sys.argv[1]
APPS = sys.argv[2:] or ["reminders", "notes", "calendar"]
NAMES = {"notes": "Notes", "reminders": "Reminders", "calendar": "Calendar",
         "contacts": "Contacts", "drivews": "Drive", "photos": "Photos"}
WATCH = 6.0  # past the fallback (3 s) and a page drawing late

# A StatusNotifierWatcher with a host: what a panel with a tray puts on the session bus.
WATCHER = r'''
from gi.repository import Gio, GLib
XML = """<node><interface name="org.kde.StatusNotifierWatcher">
<method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
<method name="RegisterStatusNotifierHost"><arg type="s" direction="in"/></method>
<property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
<property name="RegisteredStatusNotifierItems" type="as" access="read"/>
<property name="ProtocolVersion" type="i" access="read"/>
<signal name="StatusNotifierItemRegistered"><arg type="s"/></signal>
<signal name="StatusNotifierHostRegistered"/>
</interface></node>"""
info = Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0]
def call(c, s, p, i, m, a, inv):
    if m == "RegisterStatusNotifierItem":
        print("item", flush=True)
    inv.return_value(None)
def prop(c, s, p, i, name):
    return {"IsStatusNotifierHostRegistered": GLib.Variant("b", True),
            "RegisteredStatusNotifierItems": GLib.Variant("as", []),
            "ProtocolVersion": GLib.Variant("i", 0)}[name]
def got(c, name):
    c.register_object("/StatusNotifierWatcher", info, call, prop, None)
Gio.bus_own_name(Gio.BusType.SESSION, "org.kde.StatusNotifierWatcher", 0, got, None, None)
print("ready", flush=True)
GLib.MainLoop().run()
'''


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


def close(d, w):
    """Closed as a window manager closes it."""
    proto = d.intern_atom("WM_PROTOCOLS")
    delete = d.intern_atom("WM_DELETE_WINDOW")
    ev = protocol.event.ClientMessage(window=w, client_type=proto,
                                      data=(32, [delete, X.CurrentTime, 0, 0, 0]))
    w.send_event(ev)
    d.flush()


def main():
    tmp = tempfile.mkdtemp(prefix="sk-close-")
    sock = f"sk-close-{os.getpid()}"
    weston = subprocess.Popen(
        ["weston", "--backend=headless", "--shell=kiosk", "--xwayland", f"--socket={sock}",
         "--width=1280", "--height=800"],
        stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True,
        env={k: v for k, v in os.environ.items() if k not in ("WAYLAND_DISPLAY", "DISPLAY")}
        | {"XDG_RUNTIME_DIR": os.environ["XDG_RUNTIME_DIR"]})
    # Xwayland starts on the first X client; weston names its display in its log.
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
        weston.kill()
        return 1
    env = {"PATH": os.environ["PATH"], "HOME": tmp, "DISPLAY": disp, "GDK_BACKEND": "x11",
           "XDG_RUNTIME_DIR": os.environ["XDG_RUNTIME_DIR"],
           "XDG_CONFIG_HOME": f"{tmp}/config", "XDG_DATA_HOME": f"{tmp}/data",
           "XDG_CACHE_HOME": f"{tmp}/cache", "XDG_STATE_HOME": f"{tmp}/state",
           "WEBKIT_DISABLE_COMPOSITING_MODE": "1", "SKYKEEP_TIMING": "1", "SKYKEEP_LOG": "1"}
    bus = subprocess.Popen(["dbus-daemon", "--session", "--print-address", "--nofork"],
                           stdout=subprocess.PIPE, text=True)
    env["DBUS_SESSION_BUS_ADDRESS"] = bus.stdout.readline().strip()
    # The system Python: GLib's bindings are not in the run's own environment.
    tray = subprocess.Popen(["/usr/bin/python3", "-c", WATCHER], env=env, stdout=subprocess.PIPE,
                            text=True)
    if tray.stdout.readline().strip() != "ready":
        print("FAIL the stand-in tray did not start")
        return 1
    d = display.Display(disp)
    failures = 0
    app = None
    tray_up = False
    try:
        for key in APPS:
            name = NAMES[key]
            log = open(f"{tmp}/{key}.log", "w")
            if app is None:
                app = subprocess.Popen([BIN, "--app", key], env=env, stdout=log, stderr=log)
            else:
                subprocess.Popen([BIN, "--app", key], env=env, stdout=log, stderr=log).wait(10)
            w, end = None, time.time() + 20
            while time.time() < end:
                w = find(d, name)
                if w is not None and mapped(w):
                    break
                time.sleep(0.02)
            if w is None or not mapped(w):
                print(f"FAIL {name}: never shown")
                failures += 1
                continue
            if not tray_up:
                # Its tray icon there first, as on a desktop: a window is put away behind it.
                ready, _, _ = select.select([tray.stdout], [], [], 15)
                if not ready or tray.stdout.readline().strip() != "item":
                    print(f"FAIL {name}: Skykeep's tray icon never came up")
                    failures += 1
                    continue
                tray_up = True
            shown = time.time()
            close(d, w)
            gone = time.time() + 3
            while time.time() < gone and mapped(w):
                time.sleep(0.01)
            if mapped(w):
                print(f"FAIL {name}: the close was never taken")
                failures += 1
                continue
            back = None
            while time.time() - shown < WATCH:
                if mapped(w):
                    back = time.time() - shown
                    break
                time.sleep(0.02)
            if back is None:
                print(f"PASS {name}: closed as it appeared and stayed closed ({WATCH:.0f} s)")
            else:
                print(f"FAIL {name}: closed as it appeared, and came back {back:.1f} s later")
                failures += 1
        # Several asked for at once, as when tiles are clicked in a row: every one opens, soon.
        if app is not None:
            many = [k for k in NAMES if k not in APPS][:3]
            start = time.time()
            for key in many:
                subprocess.Popen([BIN, "--app", key], env=env, stdout=subprocess.DEVNULL,
                                 stderr=subprocess.DEVNULL)
            waiting = {NAMES[k] for k in many}
            while waiting and time.time() - start < 15:
                for name in list(waiting):
                    w = find(d, name)
                    if w is not None and mapped(w):
                        waiting.discard(name)
                        print(f"PASS {name}: asked for with {len(many) - 1} others, on screen in "
                              f"{time.time() - start:.1f} s")
                time.sleep(0.02)
            for name in waiting:
                print(f"FAIL {name}: asked for with others, never shown")
                failures += 1
        # Every window closed: once they are let go (apps::KEPT, a minute), their web processes
        # end too. Before wry 0.57.3 (tauri-apps/wry#1939) each closed window's stayed, about
        # 100 MB apiece, until Skykeep quit.
        if app is not None and os.environ.get("QUICK") != "1":
            for w in [find(d, NAMES[k]) for k in NAMES]:
                if w is not None and mapped(w):
                    close(d, w)
            d.flush()
            def web():
                return len(subprocess.run(["pgrep", "-P", str(app.pid), "-f", "WebKitWebProcess"],
                                          capture_output=True, text=True).stdout.split())
            before = web()
            end = time.time() + 75
            while time.time() < end and web() > 0:
                time.sleep(1)
            after = web()
            if before and after == 0:
                print(f"PASS every closed window's web process ended once let go ({before} → 0)")
            else:
                print(f"FAIL closed windows' web processes still running a minute on ({before} → {after})")
                failures += 1
    finally:
        for p in (app, tray, bus, weston):
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
