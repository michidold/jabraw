# Where the setting definitions come from

Jabraw reads the settings in the GNP config group and shows what comes back.
What a value *means* is not in the protocol — the device answers `02` and says
nothing about what that stands for. Two Jabra services describe it. One is
readable, one is not.

## The device model files — readable, no authentication

Base: `https://cdn.cloud.jabra.com/models/v/16`

The index names every product with the numbers the next request needs:

    /product-group-bundles/bundles.json

Each product carries `productName`, its variants with `vendorId`, `productId`
and `variantType`, and its firmware releases. With those:

    /vendors/2830/products/{pid}/variants/{variant}
        /firmware-versions/{firmware}
        /device-models/Jabra%20SDK%20V4
        /schema-versions/1.10.0.json

Plain JSON. Every setting looks like this:

```json
{
  "settingId": "SOUND_MODE",
  "type": "Enum",
  "sdkProperties": ["soundMode"],
  "possibleValues": [{"key": "bass"}, {"key": "treble"}, {"key": "normal"}]
}
```

`sdkProperties` is the same vocabulary `SETTINGS` in `src/config.rs` uses, which
makes it the join between the two.

The hardware this was developed against:

| Device             | pid             | variant | firmware | settings |
|--------------------|-----------------|---------|----------|----------|
| Link 380a          | 9416 (`0x24c8`) | `04-0B` | 1.16.0   | 3        |
| Evolve2 65 Mono    | 9395 (`0x24b3`) | `01-64` | 2.9.2    | 22       |

## What they answer, and what they do not

They give the type and the names of the possible values. They do **not** give
the byte that stands for each name. `voicePrompts` is off, tones or voice, the
headset answers `02`, and which of the three that is stays open.

This is what `Kind` in `src/config.rs` rests on: the files establish that a
setting is *not* a switch, which is enough to stop calling `soundMode` 0 "off".
Naming the value needs the byte, and the byte is not in here — it comes from
the section below.

## The other service — answers, but not readably

    https://devicecapabilities.jabra.com/v4/DeviceConfiguration/{pid}/gnpdefinition
    https://devicecapabilities.jabra.com/v4/DeviceConfiguration/{pid}/form
    https://devicecapabilities.jabra.com/v4/DeviceConfiguration/{pid}/localizedtext

Named in Jabra's own SDK header (`Common.h`), and they answer without
authentication — but as `application/octet-stream` with an encrypted payload.
The GNP definition is exactly what would close the gap above.

Jabraw does not try to decrypt it. Reading a specification to build an
interoperable program is expressly permitted; taking the key out of a
proprietary binary to get at protected content is a different act, and not one
this project performs. That the protection is deliberate rather than incidental
is easy to check: `/v4/Product/list` on the same service is plain JSON.

## Closing the gap by measurement

What is left is the method that produced the battery percentage and the
charging bit, both documented in `src/gnp.rs`: read the value, change the
setting in a tool that can write it, read it again. The model files make this
cheap, because they say how many values to expect and what they are called —
the experiment only has to find out which byte is which.

## Value names, from prior art

[jabridge](https://github.com/Watchdog0x/jLink) (Apache-2.0) closes the gap the
other way: it keeps raw byte and value name side by side in a table established
against hardware, in `cmd/jabridge/choice_settings.go`. Apache-2.0 permits use
in a GPL-3 work, so the mappings for seven settings are taken over here rather
than measured again — `intellitoneLevel`, `soundMode`, `muteReminderInterval`,
`hsVoicePrompts`, `sidetoneLevel`, `inactivityInterval` and `callAcceptedSound`.

Only those seven, and the reason is mechanical: jabridge addresses several
settings with a prefix byte in the request or merges the value into a bit mask,
and a plain read like ours then returns something else than the value alone.
Where its definition needs neither, our single byte and its raw value are the
same thing. The rest of its table stays where it is until our read matches it.

What was taken are the byte-to-name mappings, written out in this project's own
form and translated; no code was copied. `debian/copyright` records it.

One correction it brought: the model files list `soundMode` as bass, treble and
normal, and reading an order into that list would have made 0 mean bass. It is
Normal. The order in the model files is not a value assignment.

Its earlier version wrapped Jabra's proprietary Linux library instead, which is
the other way to get labels: let the vendor's runtime decrypt them.

## A caution

These are Jabra's endpoints, not a specification anybody promised to keep
stable. Nothing from them is mirrored into this repository. What was learned
from them is written down here in our own words, and the device remains the
final authority.
