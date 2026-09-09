#!/bin/bash
# Baut ein .deb aus dem Release-Binary. Bewusst ohne debhelper: das Paket
# besteht aus einem statischen Satz Dateien, dafür genügt dpkg-deb.
set -euo pipefail

cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH=$(dpkg --print-architecture)
PKG="jabraw_${VERSION}_${ARCH}"
ROOT="target/deb/$PKG"

cargo build --release --locked

rm -rf "$ROOT"
install -Dm755 target/release/jabraw "$ROOT/usr/bin/jabraw"
# Debug-Symbole gehoeren nicht ins Paket; das Binary bleibt im Baum wie es ist.
strip --strip-unneeded "$ROOT/usr/bin/jabraw"
install -Dm644 70-jabraw.rules                     "$ROOT/usr/lib/udev/rules.d/70-jabraw.rules"
install -Dm644 packaging/jabraw.service \
    "$ROOT/usr/lib/systemd/user/jabraw.service"
install -Dm644 packaging/jabraw-autostart.desktop \
    "$ROOT/etc/xdg/autostart/jabraw.desktop"
install -Dm644 packaging/jabraw.desktop \
    "$ROOT/usr/share/applications/jabraw.desktop"
# Symbole aus demselben Zeichencode wie das Tray-Symbol, damit der Eintrag im
# Anwendungsmenue genauso aussieht.
ICONS=$(mktemp -d)
./target/release/jabraw --write-icons "$ICONS" >/dev/null
for f in "$ICONS"/*.png; do
    dim=$(basename "$f" .png)
    install -Dm644 "$f" "$ROOT/usr/share/icons/hicolor/$dim/apps/jabraw.png"
done
rm -rf "$ICONS"

install -Dm644 packaging/50-jabraw-no-suspend.conf \
    "$ROOT/usr/share/wireplumber/wireplumber.conf.d/50-jabraw-no-suspend.conf"
install -Dm644 README.md "$ROOT/usr/share/doc/jabraw/README.md"
# Handbuchseite, Copyright und Changelog erwartet die Policy im Paket; das
# Quellpaket bekommt sie ueber debhelper, hier muessen sie von Hand hinein.
install -Dm644 debian/mans/jabraw.1 "$ROOT/usr/share/man/man1/jabraw.1"
gzip -9n "$ROOT/usr/share/man/man1/jabraw.1"
install -Dm644 debian/copyright "$ROOT/usr/share/doc/jabraw/copyright"
gzip -9nc debian/changelog > "$ROOT/usr/share/doc/jabraw/changelog.gz"
chmod 644 "$ROOT/usr/share/doc/jabraw/changelog.gz"
# Ohne diesen Eintrag ueberschreibt dpkg eine geaenderte Autostart-Datei beim
# Upgrade wortlos, statt zu fragen.
mkdir -p "$ROOT/DEBIAN"
echo "/etc/xdg/autostart/jabraw.desktop" > "$ROOT/DEBIAN/conffiles"
install -Dm755 packaging/postinst "$ROOT/DEBIAN/postinst"
install -Dm755 packaging/prerm    "$ROOT/DEBIAN/prerm"

# Bibliotheksabhängigkeiten von dpkg-shlibdeps statt von Hand: libgcc-s1 fehlte
# in der gepflegten Liste, und libc6 ohne Untergrenze lässt sich auf einem zu
# alten System installieren und scheitert dann erst beim Start. Ohne
# --ignore-missing-info und ohne Ausblenden der Meldungen: eine unvollständige
# Liste soll den Bau abbrechen, nicht ein halbes Paket ergeben.
SHLIBS=$(dpkg-shlibdeps -O "$ROOT/usr/bin/jabraw" | sed -n 's/^shlibs:Depends=//p')
if [ -z "$SHLIBS" ]; then
    echo "dpkg-shlibdeps nannte keine Abhängigkeiten" >&2
    exit 1
fi

INSTALLED_SIZE=$(du -ks "$ROOT" | cut -f1)
cat > "$ROOT/DEBIAN/control" <<EOF
Package: jabraw
Version: $VERSION
Section: sound
Priority: optional
Architecture: $ARCH
Maintainer: Michael Dold <michidold@users.noreply.github.com>
Homepage: https://github.com/michidold/jabraw
Depends: $SHLIBS, udev
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
