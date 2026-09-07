# Jabraw

The multi-function button on Jabra headsets does nothing on Linux. Jabraw
makes it play and pause, and adds a tray menu for battery, volume, mute
and output device.

Developed against a Jabra Evolve2 65, over the Link 380 dongle and over
Bluetooth. Devices are matched by vendor id, USB `0b0e` and Bluetooth
`0067`, not by model.

## Install

```bash
./packaging/build-deb.sh
sudo apt install ./target/deb/jabraw_*.deb
systemctl --user restart wireplumber
```

That script is the quick path and what the release workflow uses. A Debian
source package lives in `debian/` for distribution through an archive:

```bash
sudo apt build-dep .
dpkg-buildpackage -b -us -uc
```

Note what still stands between that and a Debian upload: the archive builds
Rust software offline against packaged crates, so every dependency would have
to exist there as a `librust-*-dev` package and `debian/rules` would move to
`dh-cargo`. Until then the source package builds locally and in CI, but not on
a Debian buildd.

Replug the dongle so udev applies the ACL, then log out and back in.
Autostart runs the installed binary from then on, so reinstall after
rebuilding — `jabraw --version` and the device information dialog both
say which build is running.

## Use

Press the multi-function button to play or pause the active MPRIS player.
The volume rocker keeps working as before, through the desktop.

The tray icon has the rest: battery level and charging state, current
player and track, next and previous, volume and mute for speaker and
microphone, and output device selection. Two dialogs show what the
device reports about itself — model, firmware and serial number for both
headset and dongle, and an overview of its settings.

Paired straight over Bluetooth it all still works. The buttons travel
over AVRCP and the desktop forwards them to MPRIS, and jabraw reaches the
headset over RFCOMM for the rest — battery, charging state, firmware,
serial number and settings. That is also more precise than the HFP figure
BlueZ reports, 93% against 100% in one measurement.

The interface follows the locale, English by default and German on a
German `LC_ALL`, `LC_MESSAGES` or `LANG`.

## Debug

```bash
jabraw --version    # which build is running
jabraw --debug      # every raw report and the bits that changed
jabraw --no-tray    # keys only, no tray icon
```

## Background

The button does emit a HID event, consumer usage `0xb1` (Pause) or `0xb0`
(Play). But the Link 380 declares those two bits as Constant (`81 07`) in
its report descriptor, and `hid-input.c` skips constant fields, so the
kernel allocates no key code and evdev never sees the press. Jabraw reads
the raw hidraw reports instead. The volume rocker sits on `81 02` (Data),
which is why volume was the only thing that ever worked.

Battery, device data and settings come from Jabra's GNP protocol on the
vendor report:

```text
byte 0   destination      0x01 dongle, 0x04 headset (also over Bluetooth)
byte 1   source           0x00 = PC
byte 2   sequence number  mirrored in the reply
byte 3   (type << 6) | total length in bytes
byte 4   message type     2 ident, 18 status, 19 config
byte 5   subcommand
byte 6+  payload
```

The layout was derived from the traffic Jabra's own SDK produces, so
jabraw sends nothing the vendor tool does not send itself, and no
proprietary library is involved at runtime. Over hidraw the packet sits
behind report id `0x05`; over Bluetooth RFCOMM it goes out bare, with
BlueZ finding the channel through a registered Serial Port profile. Reading only — the same
subcommands are writable and would change device configuration for good.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
