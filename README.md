# Jabraw

*Jabra* über *hidraw* — daher der Name: die Medientasten sind nur über den
rohen HID-Kanal erreichbar, nicht über evdev (siehe unten).

Medientasten und Tray-Menü für Jabra-Headsets (Evolve2 65 über Link-380-Dongle,
USB-Vendor `0b0e`) unter Linux. Jabra bietet seine Headset-Software nur für
Windows und macOS an.

## Warum das nötig ist

Die Multifunktionstaste sendet sehr wohl ein HID-Event — die Consumer-Usages
`0xb1` (Pause) bzw. `0xb0` (Play), abwechselnd, je nachdem was das Headset
gerade für den Wiedergabezustand hält. Im Report-Descriptor des Link 380 sind
diese beiden Bits aber als

```
75 01 95 02 81 07
```

deklariert. Bit 0 des Input-Flags `0x07` bedeutet **Constant**, und
`hid-input.c` überspringt konstante Felder:

```c
if (field->flags & HID_MAIN_ITEM_CONSTANT)
        goto ignore;
```

Der Kernel legt für die Taste deshalb keinen Tastencode an — `KEY_PAUSECD`
fehlt in der Capability-Bitmap, `evtest` zeigt beim Drücken gar nichts. Windows
ignoriert das Constant-Flag, Linux nimmt es ernst.

Die Lautstärkewippe steht dagegen auf `81 02` (Data) und wird korrekt auf
`KEY_VOLUMEUP`/`KEY_VOLUMEDOWN` abgebildet. Genau deshalb funktionierte unter
Linux immer nur die Lautstärke.

Über `evdev` ist die Taste damit prinzipiell unerreichbar. Über `hidraw` ist sie
trivial erreichbar — daher dieser Daemon.

## Funktionen

* Multifunktionstaste → `PlayPause` am aktiven MPRIS-Player
* Tray-Menü (StatusNotifierItem) mit
  * Verbindungsstatus und Gerätename
  * aktuellem Player und Titel, Wiedergabe/Pause, Titelsprung
  * Lautstärke und Stummschaltung für Lautsprecher und Mikrofon
  * Auswahl des Ausgabegeräts
* Hotplug: Dongle kann jederzeit ab- und angesteckt werden

## Was nicht geht

**Akkustand.** Die Usage-Pages des Geräts sind Telephony, Consumer, Button, LED
und vier Vendor-Pages. Eine `Battery System`-Page (`0x85`) gibt es nicht, `upower`
kennt das Gerät folglich auch nicht. Der Akkustand liegt hinter Jabras
proprietärem Protokoll auf Report 5 (Usage-Page `0xff00`, 63-Byte-Puffer) und
wäre nur über Reverse Engineering erreichbar. Dasselbe gilt für Geräte-
einstellungen wie Sidetone oder Busylight.

**Titelsprung über die Headset-Taste.** Weder Doppel- noch Dreifachdruck
erzeugen die Usages `0xb5`/`0xb6`. Im Tray-Menü sind Vor/Zurück vorhanden, an
der Hardware nicht.

**Aufwachlatenz nach Pause.** WirePlumber suspendiert untätige Knoten
(`suspend-node.lua`); der Dongle baut den USB-Audiostream dann ab, und beim
nächsten Play dauert es spürbar, bis wieder etwas hörbar ist — es wirkt, als
hätte der Tastendruck nichts bewirkt. Das Paket legt deshalb
`/usr/share/wireplumber/wireplumber.conf.d/50-jabraw-no-suspend.conf` ab, das
`session.suspend-timeout-seconds = 0` für Jabra-USB-Audiogeräte setzt. Nach der
Installation einmal `systemctl --user restart wireplumber`.

**Mikroarm.** Der Arm meldet sich auf `hidraw` nicht. Report 2 Bit 2 (`Line`)
sieht auf den ersten Blick danach aus, wechselt aber im Gleichtakt mit dem
Wiedergabezustand und ist damit die Stream-Statusmeldung des Dongles. Die
Stummschaltung durch den Arm erfolgt in der Headset-Firmware; sichtbar wird sie
nur als Mute-Zustand der PipeWire-Quelle, den das Tray anzeigt.

## Tastenbelegung

| Element | Signal | Verhalten |
|---|---|---|
| Multifunktionstaste | Report 1, Bit 9/10 (Constant) | MPRIS `PlayPause` |
| Lautstärke +/− | Report 1, Bit 0/1 (Data) | Kernel/Desktop, nicht angefasst |
| Mikroarm | — | meldet über hidraw nichts |
| Next / Previous | — | sendet das Headset nicht |

Der Daemon reagiert **ausschließlich** auf die Bits, die der Kernel wegen des
Constant-Flags verwirft. Alles, was als `KEY_*` ankommt, bleibt beim Desktop —
sonst löste jede Taste doppelt aus.

Beide Bits der Multifunktionstaste werden auf `PlayPause` abgebildet, nicht
wörtlich auf `Play`/`Pause`. Das ist robuster: läuft die Zustandsannahme des
Headsets aus dem Tritt, bliebe ein wörtliches `Pause` auf einem bereits
pausierten Player wirkungslos.

## Installation

```bash
./packaging/build-deb.sh
sudo dpkg -i target/deb/jabraw_*.deb
```

Danach den Dongle einmal ab- und wieder anstecken, damit die udev-ACL gesetzt
wird, und neu anmelden (oder `/usr/bin/jabraw &` starten).

Gestartet wird über `/etc/xdg/autostart` — das funktioniert auf GNOME, KDE und
den üblichen wlroots-Panels gleichermaßen. Die mitgelieferte systemd-User-Unit
ist die Alternative für Umgebungen ohne XDG-Autostart und wird bewusst **nicht**
automatisch aktiviert; sonst liefe der Daemon doppelt. Wer sie stattdessen
nutzen will:

```bash
sudo rm /etc/xdg/autostart/jabraw.desktop
systemctl --user enable --now jabraw.service
```

Eine zweite Instanz beendet sich ohnehin von selbst: der Daemon belegt den
D-Bus-Namen `io.github.michidold.Jabraw` als Einzelinstanz-Sperre.

### Ohne Paket

```bash
sudo cp 70-jabraw.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger
cargo build --release && ./target/release/jabraw
```

Die udev-Regel **muss** eine Nummer < 73 tragen: `73-seat-late.rules` wertet den
`uaccess`-Tag aus, spätere Dateien kommen zu spät und bleiben wirkungslos.
Prüfen:

```bash
getfacl -p /dev/hidraw2 | grep '^user:'   # muss user:<name>:rw- enthalten
```

## Diagnose

```bash
jabraw --debug      # jeden Rohreport samt geänderter Bits ausgeben
jabraw --no-tray    # nur Tastensteuerung, ohne Tray
```

`--debug` zeigt Bitnummer und Usage-Namen jeder Änderung. Damit lassen sich
weitere Tasten zuordnen: Bit notieren und in `hid::action_for` eintragen. Bits,
die der Kernel bereits als `KEY_*` meldet (`evtest` zeigt sie), gehören dort
**nicht** hinein — die verarbeitet der Desktop schon.

## Aufbau

| Datei | Inhalt |
|---|---|
| `src/hid.rs` | hidraw-Suche, Lese-Threads, Bit-Dekodierung |
| `src/mpris.rs` | Player-Auswahl und -Steuerung |
| `src/audio.rs` | Lautstärke/Mute/Ausgabegerät über `wpctl` bzw. `pactl` |
| `src/tray.rs` | StatusNotifierItem-Menü |
| `packaging/` | systemd-Unit, Autostart, `.deb`-Bauskript |
