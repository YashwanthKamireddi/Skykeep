# Send to iPhone in GNOME Files' right-click menu, for files and folders alike. Installed to
# /usr/share/nautilus-python/extensions by the distribution packages; Files loads it when its
# Python extension support (python3-nautilus, nautilus-python) is installed. Without it, Send
# to iPhone is in Open With. Sent through iCloud by Skykeep (src-tauri/src/send.rs).
import subprocess

from gi.repository import GObject, Nautilus


class SkykeepSendToIphone(GObject.GObject, Nautilus.MenuProvider):
    def get_file_items(self, *args):
        # Files 43+ passes the files alone; older versions pass the window first.
        files = args[-1]
        paths = [f.get_location().get_path() for f in files]
        if not paths or None in paths:
            return []
        item = Nautilus.MenuItem(
            name="SkykeepSendToIphone::send",
            label="Send to iPhone",
            tip="Send to your iPhone through iCloud",
        )
        item.connect("activate", lambda _item: _send(paths))
        return [item]


def _send(paths):
    # Started apart from Files, so Files never waits on it.
    subprocess.Popen(
        ["skykeep", "--send-to-iphone", *paths],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
