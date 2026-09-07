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

The tray icon has the rest: current player and track, next and previous,
volume and mute for both speaker and microphone, and output device
selection.

Skipping tracks from the headset itself is not possible — it never sends
those HID usages.

## Debug

```bash
jabraw --debug      # print every raw report and the bits that changed
jabraw --no-tray    # keys only, no tray icon
```

## Why this is needed

The button does emit a HID event, consumer usage `0xb1` (Pause) or `0xb0`
(Play). But the Link 380 declares those two bits as Constant (`81 07`) in
its report descriptor, and `hid-input.c` skips constant fields, so the
kernel allocates no key code and evdev never sees the press. Jabraw reads
the raw hidraw reports instead. The volume rocker sits on `81 02` (Data),
which is why volume was the only thing that ever worked.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
