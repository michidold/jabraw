# Jabraw

The multi-function button on Jabra headsets does nothing on Linux. Jabraw
makes it play and pause, and adds a tray menu for volume, mute and output
device.

Developed against a Jabra Evolve2 65 on a Link 380 dongle. Devices are
matched by USB vendor `0b0e`, so other Jabra hardware may work.

## Install

```bash
./packaging/build-deb.sh
sudo apt install ./target/deb/jabraw_*.deb
systemctl --user restart wireplumber
```

Replug the dongle so udev applies the ACL, then log out and back in.
Jabraw starts on its own from then on.

## Use

Press the multi-function button to play or pause the active MPRIS player.
The volume rocker keeps working as before, through the desktop.

The tray icon has the rest: headset battery level, current player and
track, next and previous, volume and mute for both speaker and
microphone, and output device selection. "Geräteinformationen" opens a
dialog with model, firmware and serial number of both the headset and
the dongle.

Skipping tracks from the headset itself is not possible — it never sends
those HID usages.

## Debug

```bash
jabraw --debug      # print every raw report and the bits that changed
jabraw --no-tray    # keys only, no tray icon
```

## Without the dongle

Paired straight over Bluetooth, the headset has no HID device, so none of
the above applies. Nothing needs doing: the buttons travel over AVRCP and the
desktop forwards them to MPRIS, and BlueZ reports the battery over HFP.
Jabraw picks the headset up through BlueZ so it still shows in the tray with
its name and battery, and falls back to the USB path the moment a dongle
appears. Devices are matched by vendor id, USB `0b0e` and Bluetooth `0067`,
not by model.

## Battery level

The device exposes no HID battery usage page, so `upower` does not see it.
The level comes from Jabra's own GNP protocol on the vendor report instead:

```text
byte 0   destination      0x01 = device
byte 1   source           0x00 = PC
byte 2   sequence number  mirrored in the reply
byte 3   (type << 6) | total length in bytes
byte 4   message type     18 = status
byte 5   subcommand       2 = headset battery
byte 6+  payload
```

A read of status/2 answers with four payload bytes: byte 1 is the percentage,
byte 0 signals that the headset is charging. Both were matched against the
hardware, `00 20 00 00` at 32 % off the cable against `01 3a 00 00` at 58 % on
it. Bluetooth carries the charge level over HFP, which has no charging flag
and only coarse steps, so "charging" shows only on the USB path.

Byte 0 addresses the target, which is how headset and dongle are told apart
on the one hidraw node: `0x01` is the dongle, `0x04` the headset. Both answer
the ident group, so each reports its own name, firmware version and serial. The packet layout was taken from the traffic Jabra's own SDK
produces, so jabraw sends nothing the vendor tool does not send itself. No
proprietary library is involved at runtime.

## Why this is needed

The button does emit a HID event, consumer usage `0xb1` (Pause) or `0xb0`
(Play). But the Link 380 declares those two bits as Constant (`81 07`) in
its report descriptor, and `hid-input.c` skips constant fields, so the
kernel allocates no key code and evdev never sees the press. Jabraw reads
the raw hidraw reports instead. The volume rocker sits on `81 02` (Data),
which is why volume was the only thing that ever worked.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
