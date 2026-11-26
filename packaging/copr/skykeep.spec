# Skykeep for Fedora COPR, built from source the way packaging/arch/PKGBUILD builds it.
#
# npm and cargo fetch their packages during %build, so the COPR project needs "Enable internet
# access during builds" (README.md beside this file). .copr/Makefile makes the source RPM from
# the commit COPR cloned, with Version set to src-tauri/tauri.conf.json's; built by hand,
# Source0 is the tag's tarball from GitHub (spectool -g).

# The release profile (Cargo.toml) already strips the binary, so there is nothing to make a
# debuginfo package from.
%global debug_package %{nil}
# LTO in CFLAGS compiles the C inside ring and libsqlite3-sys to bitcode that Rust's linker
# cannot read (the PKGBUILD explains); Rust does its own link-time optimising.
%global _lto_cflags %{nil}

%global appid io.github.yashwanthkamireddi.Skykeep

Name:           skykeep
Version:        0.1.2
Release:        1%{?dist}
Summary:        Your iCloud on this computer: notes, reminders, calendar, contacts, Drive, photos

License:        GPL-3.0-only
URL:            https://github.com/YashwanthKamireddi/Skykeep
Source0:        %{url}/archive/v%{version}/Skykeep-%{version}.tar.gz

# aarch64 for Fedora Asahi Remix on Apple Silicon Macs.
ExclusiveArch:  x86_64 aarch64

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  /usr/bin/npm
BuildRequires:  nodejs
BuildRequires:  gcc
BuildRequires:  gcc-c++
BuildRequires:  make
BuildRequires:  pkgconfig(webkit2gtk-4.1)
BuildRequires:  pkgconfig(gtk+-3.0)
BuildRequires:  pkgconfig(librsvg-2.0)
BuildRequires:  pkgconfig(libsecret-1)
BuildRequires:  pkgconfig(libheif) >= 1.17
BuildRequires:  pkgconfig(fuse3)
BuildRequires:  pkgconfig(gstreamer-1.0)
BuildRequires:  pkgconfig(gstreamer-plugins-base-1.0)
BuildRequires:  desktop-file-utils
BuildRequires:  libappstream-glib

# The libraries the binary links are found by rpm itself. These are loaded at run time: the
# GStreamer elements behind the video player and posters (base, good, libav, and openh264
# for H.264 from Fedora's Cisco repository), the same list the release's .rpm declares.
Requires:       libheif >= 1.17
Requires:       gstreamer1-plugins-base
Requires:       gstreamer1-plugins-good
Requires:       gstreamer1-plugin-libav
Requires:       gstreamer1-plugin-openh264
# iCloud Drive and Photos as folders in the file manager.
Recommends:     fuse3
# The release's own .rpm installs the same files under the name "Skykeep".
Conflicts:      Skykeep

%description
Skykeep keeps your iCloud notes, reminders, calendar, contacts, Drive and photos on your
own computer, so they open instantly and work offline. Six apps in one, each with its own
launcher. Free and open source.

An independent project. Not affiliated with Apple. iCloud is a trademark of Apple Inc.

%prep
%autosetup -n Skykeep-%{version}

%build
# Built with the project's own release profile, as the release's .rpm is.
unset RUSTFLAGS
export CARGO_HOME="$PWD/.cargo-home"
export npm_config_cache="$PWD/.npm-cache"
npm ci --no-audit --no-fund
npm run build
cargo build --release --locked -p quince-app

%install
install -Dm755 target/release/quince-app %{buildroot}%{_bindir}/skykeep

sed 's|<launchable type="desktop-id">Skykeep.desktop</launchable>|<launchable type="desktop-id">%{appid}.desktop</launchable>|' \
    packaging/%{appid}.metainfo.xml > %{appid}.metainfo.xml
install -Dm644 %{appid}.metainfo.xml %{buildroot}%{_metainfodir}/%{appid}.metainfo.xml

sed -e '/^#/d' \
    -e 's/{{name}}/Skykeep/' \
    -e 's/{{comment}}/Your iCloud, on this computer/' \
    -e 's/{{exec}}/skykeep/g' \
    -e 's/{{icon}}/%{appid}/' \
    -e 's/{{categories}}/Office;Calendar;ContactManagement;/' \
    packaging/%{appid}.desktop.in > %{appid}.desktop
install -Dm644 %{appid}.desktop %{buildroot}%{_datadir}/applications/%{appid}.desktop
for entry in packaging/apps/*.desktop; do
  install -Dm644 "$entry" %{buildroot}%{_datadir}/applications/$(basename "$entry")
done
# Each app's own icon (packaging/icons/apps), named for its launcher; and Send to iPhone's.
for app in Notes Reminders Calendar Contacts Drive Photos Send; do
  install -Dm644 packaging/icons/apps/${app,,}.svg \
    %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/%{appid}.$app.svg
done

for size in 16x16:16x16 24x24:24x24 32x32:32x32 48x48:48x48 64x64:64x64 128x128:128x128 \
            256x256:128x128@2x 512x512:icon; do
  install -Dm644 "src-tauri/icons/${size#*:}.png" \
    %{buildroot}%{_datadir}/icons/hicolor/${size%%:*}/apps/%{appid}.png
done

sed 's|^DesktopId=Skykeep.desktop$|DesktopId=%{appid}.desktop|' \
    packaging/%{appid}.search-provider.ini > %{appid}.search-provider.ini
install -Dm644 %{appid}.search-provider.ini \
  %{buildroot}%{_datadir}/gnome-shell/search-providers/%{appid}.search-provider.ini
install -Dm644 packaging/%{appid}.SearchProvider.service \
  %{buildroot}%{_datadir}/dbus-1/services/%{appid}.SearchProvider.service
install -Dm644 packaging/%{appid}.krunner.desktop \
  %{buildroot}%{_datadir}/krunner/dbusplugins/%{appid}.krunner.desktop
# Send to iPhone at the top of the file manager's right-click menu (packaging/file-managers).
install -Dm644 packaging/file-managers/skykeep-send-to-iphone.desktop \
  %{buildroot}%{_datadir}/kio/servicemenus/skykeep-send-to-iphone.desktop
install -Dm644 packaging/file-managers/skykeep-send-to-iphone.nemo_action \
  %{buildroot}%{_datadir}/nemo/actions/skykeep-send-to-iphone.nemo_action
install -Dm644 packaging/file-managers/skykeep_send_to_iphone.py \
  %{buildroot}%{_datadir}/nautilus-python/extensions/skykeep_send_to_iphone.py

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/*.desktop
appstream-util validate-relax --nonet %{buildroot}%{_metainfodir}/%{appid}.metainfo.xml

%files
%license LICENSE
%doc README.md
%{_bindir}/skykeep
%{_datadir}/applications/%{appid}*.desktop
%{_datadir}/icons/hicolor/*/apps/%{appid}.png
%{_datadir}/icons/hicolor/scalable/apps/%{appid}.*.svg
%{_metainfodir}/%{appid}.metainfo.xml
%{_datadir}/gnome-shell/search-providers/%{appid}.search-provider.ini
%{_datadir}/dbus-1/services/%{appid}.SearchProvider.service
%dir %{_datadir}/krunner
%dir %{_datadir}/krunner/dbusplugins
%{_datadir}/krunner/dbusplugins/%{appid}.krunner.desktop
%dir %{_datadir}/kio
%dir %{_datadir}/kio/servicemenus
%{_datadir}/kio/servicemenus/skykeep-send-to-iphone.desktop
%dir %{_datadir}/nemo
%dir %{_datadir}/nemo/actions
%{_datadir}/nemo/actions/skykeep-send-to-iphone.nemo_action
%dir %{_datadir}/nautilus-python
%dir %{_datadir}/nautilus-python/extensions
%{_datadir}/nautilus-python/extensions/skykeep_send_to_iphone.py

%changelog
* Wed Nov 04 2026 Yashwanth Kamireddi <yashwanthkamireddi@users.noreply.github.com> - 0.1.2-1
- Skykeep packaged for Fedora COPR
