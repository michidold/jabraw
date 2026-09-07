# Jabraw

Media keys and a tray menu for Jabra headsets on Linux. *Jabra* over
*hidraw* — which is the whole point: the multi-function button cannot be
reached through evdev.

Developed against a Jabra Evolve2 65 on a Link 380 dongle. Devices are matched
by USB vendor `0b0e`, so other Jabra hardware may work.

## Why hidraw

The button does emit a HID event: consumer usage `0xb1` (Pause) or `0xb0`
(Play), alternating with whatever state the headset assumes. The Link 380 report
descriptor declares those two bits as

    75 01 95 02 81 07

Bit 0 of input flag `0x07` means **Constant**, and `hid-input.c` skips constant
fields:

```c
if (field->flags & HID_MAIN_ITEM_CONSTANT)
        goto ignore;
```

So the kernel never allocates a key code. `KEY_PAUSECD` is missing from the
device capability bitmap and `evtest` prints nothing on press. Volume sits on
`81 02` (Data) and maps correctly, which is why volume was the only thing that
ever worked on Linux.

## Install

```bash
./packaging/build-deb.sh
sudo apt install ./target/deb/jabraw_*.deb
```

Replug the dongle so udev applies the ACL, then log out and back in. To start it
in the current session instead, run `jabraw &`.

| Path | Purpose |
|---|---|
| `/usr/bin/jabraw` | the daemon |
| `/usr/lib/udev/rules.d/70-jabraw.rules` | `uaccess` ACL on the Jabra hidraw node |
| `/etc/xdg/autostart/jabraw.desktop` | autostart |
| `/usr/share/applications/jabraw.desktop` | application menu entry |
| `/usr/lib/systemd/user/jabraw.service` | alternative to XDG autostart, not enabled |
| `/usr/share/wireplumber/wireplumber.conf.d/50-jabraw-no-suspend.conf` | keeps the sink out of suspend |

The udev rule has to sort before `73-seat-late.rules`, which is what turns the
`uaccess` tag into an ACL — hence the `70-` prefix. A rule numbered above
73 is read too late and has no effect.

Only the XDG entry starts the daemon, because it works across desktops. The
systemd unit ships for environments without XDG autostart and is deliberately
left disabled; enabling both would start the daemon twice. A second instance
exits by itself regardless: the daemon holds the D-Bus name
`io.github.michidold.Jabraw` as a single-instance lock.

Without the WirePlumber drop-in, `suspend-node.lua` parks idle nodes after a few
seconds, the dongle tears down the USB audio stream, and the next play takes
long enough to look like a lost key press. Run `systemctl --user restart
wireplumber` once after installing.

### From source

```bash
sudo cp 70-jabraw.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger
cargo build --release && ./target/release/jabraw
```

## What the controls do

| Control | Signal | Behaviour |
|---|---|---|
| Multi-function button | report 1, bits 9/10 (Constant) | MPRIS `PlayPause` |
| Volume rocker | report 1, bits 0/1 (Data) | left to the kernel and desktop |
| Boom arm | — | reports nothing over hidraw |
| Next / previous | — | the headset never sends these |

The daemon acts **only** on bits the kernel discards. Anything arriving as
`KEY_*` stays with the desktop; handling it here as well would fire every key
twice.

Both button bits map to `PlayPause` rather than to a literal `Play` and `Pause`.
When the headset's idea of the state drifts, a literal `Pause` on an already
paused player does nothing, while a toggle still works.

The tray menu offers, beyond play/pause: connection status and device name, the
active player and track, next/previous, volume and mute for both speaker and
microphone, and output device selection. Hotplug is handled, so the dongle can
be pulled and reinserted at any time.

## Troubleshooting

```bash
jabraw --debug      # print every raw report and the bits that changed
jabraw --no-tray    # keys only, no tray icon
```

`--debug` names the bit and its usage on every change, which is how to map
further buttons: note the bit and add it to `hid::action_for`. Bits the kernel
already reports as `KEY_*` — the ones `evtest` shows — do not belong there,
because the desktop handles them.

Nothing happens at all: check that the ACL was applied.

```bash
getfacl -p /dev/hidraw* 2>/dev/null | grep -B6 "^user:$USER:"
```

Keys work but no tray icon appears: the desktop has no StatusNotifierItem host.
On GNOME that means the AppIndicator extension is not installed.

## Layout

| Path | Contents |
|---|---|
| `src/hid.rs` | hidraw discovery, reader threads, bit decoding |
| `src/mpris.rs` | player selection and control |
| `src/audio.rs` | volume, mute and sink selection via `wpctl` or `pactl` |
| `src/tray.rs` | StatusNotifierItem menu |
| `packaging/` | systemd unit, autostart, WirePlumber drop-in, `.deb` script |

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
