#!/bin/bash
# Does Skykeep install and run on this distribution? Run as root in a clean container of it:
#
#   check.sh <package> <distro_check> <test.heic> [<video>...]
#
#   <package>       a .deb, .rpm, .pkg.tar.zst or .AppImage — installed with the system's own
#                   package manager (the .AppImage is only run), with recommends/weak
#                   dependencies off, so only what the package itself asks for is there;
#   <distro_check>  the probe's distro_check example (probe/examples/distro_check.rs), or `-`
#                   for none: then gst, heic, video and keyring are left out (a released
#                   package, checked without building anything: linux-launch.yml);
#   <test.heic>     a HEIC made for the test — never anybody's photo (`-` with no probe);
#   <video>         made-up test videos (crates/quince-media/tests/fixtures: black-start.mp4,
#                   H.264, and hevc.mov, HEVC as an iPhone records it).
#
# Checked, each on its own line, PASS or FAIL:
#   ldd       the program finds every library it links;
#   gst       the GStreamer elements the player and poster reader load at run time are there
#             (SKIP for an AppImage on a system whose gst-inspect cannot load its GStreamer:
#             the video lines below are checked through the AppImage's own GStreamer anyway);
#   heic      a HEIC decodes through the system's libheif, the app's own way — for the
#             AppImage, through the libheif and HEVC decoder it carries, found as the app
#             finds them;
#   video     each video's poster read the app's way (quince_media::poster) and the video
#             played to its end through playbin3, as the window's player plays it — through
#             the system's GStreamer, or the AppImage's own;
#   keyring   a made-up session goes into a Secret Service (gnome-keyring) and back out
#             through the app's sign-in storage;
#   launch    started headless under Xvfb on its own session bus, the page draws
#             (window_ready) and the program is still running 8 s later — with --background,
#             as the main launcher starts it, and as one app's launcher does (--app notes);
#             the AppImage with no network at all (unshare -n, so the container needs
#             CAP_SYS_ADMIN), as the AppImage catalog starts it;
#   relaunch  with no tray to click, launching again hands over to the running Skykeep and
#             its window shows;
#   wayland   the same on a Wayland compositor (weston, headless) with no X server at all, as
#             GNOME and KDE start it by default — every launch above is X11;
#   no keyring  on a session bus with no Secret Service on it and none to start (a window
#             manager with no keyring daemon), the page still draws and the program stays up;
#   no bus    with no session bus at all (a bare window manager started without one), the
#             same.
#   desktop   the launchers and the metainfo validate, the icon is in hicolor under the name
#             the launchers ask for (packages only).
# Used by .github/workflows/linux-compat.yml, and runs the same by hand under docker.
set -u
pkg=$1 probe=$2 heic=$3
videos=("${@:4}")
. /etc/os-release
like=" ${ID} ${ID_LIKE:-} "
fails=0
pass() { echo "PASS $1${2:+ — $2}"; }
fail() { echo "FAIL $1${2:+ — $2}"; fails=$((fails + 1)); }

# --- The test's own tools (never the app's dependencies: those come from the package) -------
case "$like" in
  *" debian "* | *" ubuntu "*)
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    tools="xvfb xauth dbus gnome-keyring gstreamer1.0-tools desktop-file-utils appstream weston"
    # An AppImage carries its libraries but not the ones every desktop has (linuxdeploy's
    # excludelist): GL, X11, fonts and text shaping, ALSA, DRM. Without them in this bare
    # container gst-libav (libavcodec links them) does not load and avdec_* looks missing.
    # libgpg-error too (under libgcrypt, on every system): libavcodec's libraries link it, and
    # without it gst-libav — every H.264 and HEVC decoder — does not load.
    # ALSA is libasound2t64 from Ubuntu 24.04 and Debian 13 on, libasound2 before.
    asound=libasound2t64
    apt-cache show libasound2t64 >/dev/null 2>&1 || asound=libasound2
    [[ $pkg == *.AppImage ]] && tools+=" libgl1 libegl1 libgles2 libfontconfig1 fonts-dejavu-core \
      libx11-6 libx11-xcb1 libxcb1 libxcb-dri3-0 libdrm2 libharfbuzz0b libfribidi0 libfreetype6 \
      $asound libgpg-error0"
    # shellcheck disable=SC2086
    apt-get install -y -qq --no-install-recommends $tools >/dev/null
    [[ $pkg == *.deb ]] && { apt-get install -y -q --no-install-recommends "$pkg" || exit 1; }
    ;;
  *" fedora "*)
    tools="xorg-x11-server-Xvfb xorg-x11-xauth dbus-daemon gnome-keyring gstreamer1 \
           desktop-file-utils appstream which weston"
    # As above; libGLESv2 (libglvnd-gles) too, which WebKit opens and every Fedora desktop has.
    [[ $pkg == *.AppImage ]] && tools+=" mesa-libGL mesa-libEGL libglvnd-gles fontconfig \
      dejavu-sans-fonts libX11 libX11-xcb libxcb libdrm harfbuzz fribidi freetype alsa-lib \
      libgpg-error"
    # shellcheck disable=SC2086
    dnf -y -q install --setopt=install_weak_deps=False $tools
    [[ $pkg == *.rpm ]] && { dnf -y install --setopt=install_weak_deps=False "$pkg" || exit 1; }
    ;;
  *" suse "* | *" opensuse "*)
    # dbus-run-session is in dbus-1-daemon, not dbus-1.
    tools="xvfb-run dbus-1 dbus-1-daemon gnome-keyring gstreamer-utils desktop-file-utils \
      AppStream which weston"
    [[ $pkg == *.AppImage ]] && tools+=" Mesa-libGL1 Mesa-libEGL1 fontconfig dejavu-fonts libgpg-error0"
    # shellcheck disable=SC2086
    zypper -q --non-interactive install --no-recommends $tools
    [[ $pkg == *.rpm ]] && {
      zypper --non-interactive install --no-recommends --allow-unsigned-rpm "$pkg" || exit 1
    }
    ;;
  *" arch "*)
    pacman -Syu --noconfirm --needed -q xorg-server-xvfb xorg-xauth dbus gnome-keyring \
      gstreamer desktop-file-utils appstream which weston >/dev/null
    [[ $pkg == *.pkg.tar.* ]] && { pacman -U --noconfirm "$pkg" || exit 1; }
    ;;
  *) echo "no installer for $ID"; exit 2 ;;
esac
command -v xvfb-run >/dev/null || { echo "no xvfb-run on $ID"; exit 2; }

# --- Where the program is ---------------------------------------------------------------------
if [[ $pkg == *.AppImage ]]; then
  chmod +x "$pkg"
  # No FUSE in a container: unpacked once and run from there, as --appimage-extract-and-run
  # does, so ldd can look at what it carries.
  cd /tmp && "$pkg" --appimage-extract >/dev/null && cd - >/dev/null || exit 1
  root=/tmp/squashfs-root
  prog="$root/AppRun"
  bin="$root/usr/bin/skykeep"
  libs="$root/usr/lib:$root/usr/lib/x86_64-linux-gnu"
  # The probe beside the program, in the AppDir, with APPDIR set as the AppImage's AppRun sets
  # it: it then finds the AppImage's libheif plugins exactly as the app does (only a program
  # inside APPDIR is pointed at them).
  [[ $probe == - ]] || { cp "$probe" "$root/usr/bin/distro_check" && probe="$root/usr/bin/distro_check"; }
  export APPDIR=$root
else
  prog=/usr/bin/skykeep bin=/usr/bin/skykeep libs=
fi
[[ -x $bin ]] || { echo "FAIL install — no $bin"; exit 1; }
echo "== $PRETTY_NAME — $(basename "$pkg")"

# --- ldd ------------------------------------------------------------------------------------
missing=$(LD_LIBRARY_PATH=$libs ldd "$bin" | grep "not found")
[[ -z $missing ]] && pass ldd || fail ldd "$missing"

# --- GStreamer elements (the package's, or the AppImage's own) ------------------------------
# playbin3/decodebin/videoconvert/appsink: gst-plugins-base; qtdemux, videoflip: -good;
# avdec_h264 / avdec_h265: gst-libav — an iPhone's .mov/.mp4 and its H.264/HEVC video.
if [[ $pkg == *.AppImage ]]; then
  # Exactly as the AppImage's AppRun sets GStreamer up — its own hook, sourced: its plugins
  # (usr/lib/gstreamer-1.0), its scanner — plus a registry of its own for this check. (A
  # `find -name gstreamer-1.0` here once picked the helpers' directory,
  # usr/lib/gstreamer1.0/gstreamer-1.0, and every element looked missing.)
  # shellcheck disable=SC1091
  source "$root/apprun-hooks/linuxdeploy-plugin-gstreamer.sh" || exit 1
  export GST_REGISTRY_1_0=/tmp/appimage-registry.bin
fi
media() {
  local absent= e
  # The AppImage's elements are looked at with this system's gst-inspect-1.0 on the AppImage's
  # own libgstreamer. A system GStreamer newer than the bundled one (Fedora 42's 1.26 against
  # Ubuntu 24.04's 1.24) cannot do that ("undefined symbol"), which says nothing about the
  # AppImage; then the elements are not listed here, and the video lines below — played
  # through the AppImage's own GStreamer by the probe, built against it — are the check.
  # Nor can it list them when the two are different releases even where it loads (Debian 13's
  # 1.26 against the AppImage's 1.24: every element reads as missing while the videos play).
  local own carried
  own=$(gst-inspect-1.0 --version 2>/dev/null | sed -n 's/^GStreamer \([0-9]*\.[0-9]*\).*/\1/p')
  carried=$(LD_LIBRARY_PATH=$libs gst-inspect-1.0 --version 2>/dev/null | sed -n 's/^GStreamer \([0-9]*\.[0-9]*\).*/\1/p')
  if [[ $pkg == *.AppImage ]] && ! LD_LIBRARY_PATH=$libs gst-inspect-1.0 --version >/dev/null 2>&1; then
    echo "SKIP gst — this system's gst-inspect-1.0 cannot load the AppImage's GStreamer (video below plays through it)"
  elif [[ $pkg == *.AppImage && -n $own && $own != "$carried" ]]; then
    echo "SKIP gst — this system's GStreamer is $own, the AppImage carries ${carried:-another}; its gst-inspect cannot list those (video below plays through it)"
  else
    for e in playbin3 decodebin videoconvert videoflip appsink qtdemux \
             "avdec_h264|openh264dec" avdec_h265; do
      LD_LIBRARY_PATH=$libs gst-inspect-1.0 "${e%%|*}" >/dev/null 2>&1 || { [[ $e == *"|"* ]] &&
        LD_LIBRARY_PATH=$libs gst-inspect-1.0 "${e##*|}" >/dev/null 2>&1; } || absent+=" $e"
    done
    [[ -z $absent ]] && pass gst || $1 gst "missing:$absent"
  fi
  if out=$(LD_LIBRARY_PATH=$libs "$probe" heic "$heic" 2>&1); then pass heic "$out"
  else $1 heic "$out"; fi
  local v
  for v in "${videos[@]}"; do
    if out=$(LD_LIBRARY_PATH=$libs timeout 60 "$probe" video "$v" 2>&1); then
      pass "video $(basename "$v")" "$out"
    else $1 "video $(basename "$v")" "$(tail -3 <<<"$out")"; fi
  done
}
# Fedora leaves HEVC out of its own repositories — libheif without its libde265 decoder, FFmpeg
# without H.265 — so an iPhone's HEIC photos and HEVC videos need RPM Fusion there (the note
# under the README's Linux install lines). Without it, those two are noted, not failed; then
# RPM Fusion is added exactly as the README says, and with it both must pass.
# openSUSE is the same: no libde265 and an FFmpeg without HEVC in its own repositories; H.264
# comes from the openh264 repository it has enabled out of the box, HEVC from Packman.
known() { echo "KNOWN $1 — ${2} (no HEVC decoder in $NAME's own repositories)"; }
if [[ $probe == - ]]; then
  echo "SKIP gst, heic, video — no probe given"
elif [[ $like == *" fedora "* && $pkg != *.AppImage ]]; then
  media known
  rel=$(rpm -E %fedora)
  # RPM Fusion's own instructions (rpmfusion.org/Howto/Multimedia): the full FFmpeg in place of
  # ffmpeg-free, and libheif's HEVC decoder. ffmpeg-libs alone did not give gst-libav H.264.
  dnf -y -q install "https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-$rel.noarch.rpm" &&
    dnf -y -q swap ffmpeg-free ffmpeg --allowerasing &&
    dnf -y -q install --setopt=install_weak_deps=False libheif-freeworld ||
    fail rpmfusion "could not be added"
  echo "-- with RPM Fusion:"
  # GStreamer's registry remembers gst-libav with ffmpeg-free's decoders; read it afresh.
  rm -rf "${XDG_CACHE_HOME:-$HOME/.cache}/gstreamer-1.0"
  media fail
elif [[ $like == *" suse "* && $pkg != *.AppImage ]]; then
  media known
  # Packman's FFmpeg and libheif, as Packman's own instructions put them in place.
  zypper -q --non-interactive ar -cfp 90 \
      "https://ftp.gwdg.de/pub/linux/misc/packman/suse/openSUSE_Tumbleweed/" packman &&
    zypper -q --non-interactive --gpg-auto-import-keys refresh packman &&
    zypper -q --non-interactive install --no-recommends --allow-vendor-change --from packman \
      libavcodec62 libheif1 libde265-0 libheif-HEIF ||
    fail packman "could not be added"
  echo "-- with Packman:"
  rm -rf "${XDG_CACHE_HOME:-$HOME/.cache}/gstreamer-1.0"
  media fail
else
  media fail
fi
unset GST_PLUGIN_SYSTEM_PATH_1_0 GST_PLUGIN_PATH_1_0 GST_PLUGIN_SCANNER_1_0 GST_PTP_HELPER_1_0 \
  GST_REGISTRY_1_0 GST_REGISTRY_REUSE_PLUGIN_SCANNER APPDIR

# --- Keyring and launch, in one session bus with a keyring daemon on it ---------------------
export HOME=$(mktemp -d) XDG_RUNTIME_DIR=$(mktemp -d) RUST_BACKTRACE=1 SKYKEEP_TIMING=1
chmod 700 "$XDG_RUNTIME_DIR"
cat >/tmp/session.sh <<'SH'
#!/bin/bash
# Inside dbus-run-session: a gnome-keyring with an unlocked login keyring (a throwaway
# password), then the keyring check and the two launches.
probe=$1 prog=$2 libs=$3
echo -n throwaway | gnome-keyring-daemon --unlock --components=secrets >/dev/null 2>&1
if [[ $probe == - ]]; then echo "SKIP keyring — no probe given"
elif out=$(LD_LIBRARY_PATH=$libs "$probe" keyring 2>&1); then echo "PASS keyring — $out"
else echo "FAIL keyring — $out"; fi
for how in --background launcher "--app notes"; do
  log=$(mktemp)
  # shellcheck disable=SC2206
  args=(); [[ $how == --* ]] && args=($how)
  window=main; [[ $how == "--app notes" ]] && window=app-notes
  timeout --kill-after=5 8 xvfb-run -a -s "-screen 0 1280x800x24" "$prog" "${args[@]}" >"$log" 2>&1
  rc=$?
  drawn=$(grep -m1 -o "+[0-9.]*ms window_ready $window" "$log")
  if [[ $rc -eq 124 && -n $drawn ]] && ! grep -q 'panicked at' "$log"; then
    echo "PASS launch ($how) — page drawn at ${drawn%% *}, still running at 8 s"
  else
    echo "FAIL launch ($how) — exit $rc, drawn: ${drawn:-never}"; grep -v skykeep-timing "$log" | tail -20
  fi
done
# Reachable with no tray to click (stock GNOME, and this container): launched again, as from
# the launcher, the second process hands over to the running one and leaves; that one shows
# its window.
log=$(mktemp)
timeout --kill-after=5 16 xvfb-run -a -s "-screen 0 1280x800x24" bash -c \
  '"$0" --background & sleep 6; timeout 6 "$0"; echo "second launch exited $?"; sleep 2' \
  "$prog" >"$log" 2>&1
if grep -q 'second launch exited 0' "$log" && grep -q 'handed a launch' "$log" &&
   grep -q 'main shown' "$log"; then
  echo "PASS relaunch — a second launch reopened the running one's window"
else
  echo "FAIL relaunch"; grep -v skykeep-timing "$log" | tail -20
fi
# Started by "$@", its log in $log, and stopped after 8 s (or $stop_after, for a command that
# times its own 8 s inside a wrapper that would not pass a stop on): the page drawn, still
# running, never a panic.
alive() {
  local name=$1; shift
  log=$(mktemp)
  timeout --kill-after=5 "${stop_after:-8}" "$@" >"$log" 2>&1
  local rc=$? drawn
  drawn=$(grep -m1 -o "+[0-9.]*ms window_ready main" "$log")
  if [[ $rc -eq 124 && -n $drawn ]] && ! grep -q 'panicked at' "$log"; then
    echo "PASS $name — page drawn at ${drawn%% *}, still running at 8 s"
  else
    echo "FAIL $name — exit $rc, drawn: ${drawn:-never}"; grep -v skykeep-timing "$log" | tail -20
  fi
}
# Wayland, with no X server to fall back on: weston's headless output, drawn in software.
# Weston 10 and later name the backend `headless`, older ones (Ubuntu 22.04's 9) the module.
sock=wayland-skykeep
for backend in headless headless-backend.so; do
  env -u DISPLAY -u WAYLAND_DISPLAY weston --backend=$backend --socket=$sock --idle-time=0 \
    --width=1280 --height=800 >/tmp/weston.log 2>&1 &
  weston=$!
  for _ in $(seq 50); do [[ -S $XDG_RUNTIME_DIR/$sock ]] && break; sleep 0.1; done
  [[ -S $XDG_RUNTIME_DIR/$sock ]] && break
  kill $weston 2>/dev/null; wait $weston 2>/dev/null
done
if [[ -S $XDG_RUNTIME_DIR/$sock ]]; then
  alive wayland env -u DISPLAY WAYLAND_DISPLAY=$sock GDK_BACKEND=wayland "$prog" --background
  kill $weston; wait $weston 2>/dev/null
else
  echo "FAIL wayland — weston did not start"; tail -5 /tmp/weston.log
fi
# No Secret Service: a bus of its own with nothing on it and nothing it may start (no service
# directory), where gnome-keyring's activation file would otherwise start one.
cat >/tmp/bare-bus.conf <<'XML'
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
XML
# dbus-run-session does not hand a stop on to what it runs, so the 8 s are timed inside it.
stop_after=20 alive "no keyring" dbus-run-session --config-file=/tmp/bare-bus.conf -- \
  timeout --kill-after=5 8 xvfb-run -a -s "-screen 0 1280x800x24" "$prog" --background
# No session bus at all: an address that leads nowhere, so nothing starts one either.
alive "no bus" env DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent/bus \
  xvfb-run -a -s "-screen 0 1280x800x24" "$prog" --background
SH
chmod +x /tmp/session.sh
# The AppImage offline: the whole session in a network namespace of its own, with no network.
offline=()
if [[ $pkg == *.AppImage ]]; then
  if unshare -n true 2>/dev/null; then
    offline=(unshare -n) && echo "-- launches with no network (unshare -n)"
  else
    fail offline "unshare -n is not allowed here (the container needs CAP_SYS_ADMIN)"
  fi
fi
results=$("${offline[@]}" dbus-run-session -- /tmp/session.sh "$probe" "$prog" "$libs")
echo "$results"
fails=$((fails + $(grep -c '^FAIL' <<<"$results")))

# --- Desktop integration (packages) ---------------------------------------------------------
if [[ $pkg != *.AppImage ]]; then
  apps=/usr/share/applications
  problems=
  for f in "$apps"/*Skykeep*.desktop "$apps"/skykeep*.desktop; do
    [[ -e $f ]] || continue
    m=$(desktop-file-validate "$f" 2>&1 | grep -v 'hint:')
    [[ -z $m ]] || problems+=" $(basename "$f"): $m"
    icon=$(sed -n 's/^Icon=//p' "$f" | head -1)
    sizes=$(find /usr/share/icons/hicolor -name "$icon.png" -o -name "$icon.svg" | wc -l)
    [[ $sizes -gt 0 ]] || problems+=" $(basename "$f"): no hicolor icon '$icon'"
    # The icon theme spec's one requirement of an app: a 48x48 in hicolor (or a scalable one).
    [[ -e /usr/share/icons/hicolor/48x48/apps/$icon.png ||
       -e /usr/share/icons/hicolor/scalable/apps/$icon.svg ]] ||
      problems+=" $(basename "$f"): no 48x48 hicolor icon '$icon'"
  done
  m=$(appstreamcli validate --no-net --explain /usr/share/metainfo/*Skykeep*.xml 2>&1) ||
    problems+=" metainfo: $(grep -E '^[EW]:' <<<"$m" | tr '\n' ' ')"
  launchable=$(sed -n 's:.*<launchable type="desktop-id">\(.*\)</launchable>.*:\1:p' /usr/share/metainfo/*Skykeep*.xml)
  [[ -e $apps/$launchable ]] || problems+=" metainfo launchable $launchable not installed"
  # GNOME Shell drops a search provider whose DesktopId is not an installed launcher.
  for ini in /usr/share/gnome-shell/search-providers/*Skykeep*.ini; do
    id=$(sed -n 's/^DesktopId=//p' "$ini")
    [[ -e $apps/$id ]] || problems+=" search provider DesktopId $id not installed"
  done
  icons=$(find /usr/share/icons/hicolor -path '*apps*' \( -iname '*skykeep*' \) | sed 's:/usr/share/icons/hicolor/::' | tr '\n' ' ')
  [[ -z $problems ]] && pass desktop "icons: $icons" || fail desktop "$problems"
fi

echo "== $fails failed"
exit $((fails > 0))
