#!/bin/bash
# Baut ein .deb aus dem Release-Binary. Bewusst ohne debhelper: das Paket
# besteht aus einem statischen Satz Dateien, dafür genügt dpkg-deb.
set -euo pipefail

cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH=$(dpkg --print-architecture)
PKG="jabra-media-daemon_${VERSION}_${ARCH}"
ROOT="target/deb/$PKG"

cargo build --release

rm -rf "$ROOT"
install -Dm755 target/release/jabra-media-daemon  "$ROOT/usr/bin/jabra-media-daemon"
install -Dm644 70-jabra.rules                     "$ROOT/usr/lib/udev/rules.d/70-jabra.rules"
install -Dm644 packaging/jabra-media-daemon.service \
    "$ROOT/usr/lib/systemd/user/jabra-media-daemon.service"
install -Dm644 packaging/jabra-media-daemon.desktop \
    "$ROOT/etc/xdg/autostart/jabra-media-daemon.desktop"
install -Dm644 packaging/jabra-media-daemon-menu.desktop \
    "$ROOT/usr/share/applications/jabra-media-daemon.desktop"
install -Dm644 README.md "$ROOT/usr/share/doc/jabra-media-daemon/README.md"
install -Dm755 packaging/postinst "$ROOT/DEBIAN/postinst"
install -Dm755 packaging/prerm    "$ROOT/DEBIAN/prerm"

INSTALLED_SIZE=$(du -ks "$ROOT" | cut -f1)
cat > "$ROOT/DEBIAN/control" <<EOF
Package: jabra-media-daemon
Version: $VERSION
Section: sound
Priority: optional
Architecture: $ARCH
Maintainer: Michael Dold <michidold@users.noreply.github.com>
Depends: libc6, udev
Recommends: pipewire, wireplumber
Suggests: gnome-shell-ubuntu-extensions | gnome-shell-extension-appindicator
Installed-Size: $INSTALLED_SIZE
Description: Medientasten und Tray-Menue fuer Jabra-Headsets
 Reicht die Multifunktionstaste von Jabra-Headsets an den aktiven
 MPRIS-Player weiter und stellt ein Tray-Menue fuer Lautstaerke,
 Stummschaltung und Ausgabegeraet bereit.
 .
 Die Taste ist ueber evdev nicht erreichbar: der Link 380 deklariert die
 zugehoerigen Bits im HID-Report-Descriptor als Constant, weshalb der
 Kernel dafuer keinen Tastencode anlegt. Der Daemon liest deshalb hidraw.
EOF

dpkg-deb --root-owner-group --build "$ROOT" "target/deb/$PKG.deb"
echo
echo "fertig: target/deb/$PKG.deb"
