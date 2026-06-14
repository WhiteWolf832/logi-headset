# logi-headset

A lightweight Linux tool for **Logitech wireless (Lightspeed) headsets**, speaking
native **HID++** — no G HUB, no root, no cloud.

It does three things the official software does on Windows:

- **Remaps the mute button** to a media key (Play/Pause by default; single *and*
  double-click actions).
- **Controls the LEDs** (turn them off, or set a fixed color).
- **Warns when the battery is low** — a desktop notification, plus the live charge
  shown in the config panel.

Tested on the **Logitech G733** and **G533**, and built to work across the Logitech
HID++ headset family: the device and its features are **auto-detected** (by HID++
feature, not by hard-coded product ID), so other Lightspeed headsets have a good
chance of working out of the box. Headsets without RGB lighting (like the G533) are
handled gracefully — the LED settings simply have no effect. If yours doesn't work,
see [Adding your headset](#adding-your-headset).

> **After powering the headset off and on**, some models take a while before they
> re-accept the mute-button diversion. On the **G533**, the mute button works again
> about **~1 minute after switching it on** — a firmware quirk, not a bug (it can be
> longer if the battery is nearly empty, when the headset's wireless link keeps
> flapping). The daemon keeps re-applying the diversion every few seconds, so it
> **recovers on its own** — just give it a minute (or `systemctl --user restart
> logi-headset` to retry immediately). If a button is never seen at all, use `--watch`
> to check
> what (if anything) it emits, and open an issue.

## Components

| Binary | Role |
|---|---|
| `logi-headset` | the daemon (libc only, zero deps) — runs as a systemd **user** service |
| `logi-headset-config` | a small GTK4 configuration panel (multilingual: en, fr, de, it, es, pt) |

No network access, ever. The daemon talks only to `/dev/hidraw*` and `/dev/uinput`.

## Requirements

- A Logitech wireless headset exposing the HID++ G-keys feature (`0x8010`).
- **GTK4 ≥ 4.10** and **libadwaita** (for the config panel).
- Rust (to build) and access to `/dev/hidraw*` + `/dev/uinput` via the included
  udev `uaccess` rule (so it runs without root).

## Build & install

```sh
# 1. build
( cd daemon && cargo build --release )
( cd gui    && cargo build --release )

# 2. binaries
install -Dm755 daemon/target/release/logi-headset        ~/.local/bin/logi-headset
install -Dm755 gui/target/release/logi-headset-config    ~/.local/bin/logi-headset-config

# 3. udev rule (grants the logged-in user access to the device — no root at runtime)
sudo install -m644 42-logitech-hidpp-uaccess.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger

# 4. desktop entry + user service
install -Dm644 logi-headset-config.desktop  ~/.local/share/applications/logi-headset-config.desktop
install -Dm644 logi-headset.service         ~/.config/systemd/user/logi-headset.service
systemctl --user daemon-reload
systemctl --user enable --now logi-headset.service
```

Then open **“Logitech Headset — Configuration”** from your app menu.

## Configuration

The daemon reads `~/.config/logi-headset/config` (written by the GUI):

```ini
key          = playpause        # single-click action
key_double   = next             # double-click action (or 'none')
double_ms    = 1000             # double-click window (ms)
leds         = off              # keep | off | color
led_color    = a51d2d           # RRGGBB (if leds=color)
battery_warn = 15               # low-battery threshold in % (0 = no alert)
```

## Adding your headset

If your Logitech headset isn't fully detected, generate a diagnostic and open an
issue with it — that's all it takes to add support for a new model:

- In the GUI: **Status → “Analyze headset”**, then **Copy**.
- Or from a terminal (stop the service first so it frees the device):

  ```sh
  systemctl --user stop logi-headset
  logi-headset --diagnose
  systemctl --user start logi-headset
  ```

The report lists the HID++ features and resolved indices. Paste it into a new issue:
**https://github.com/WhiteWolf832/logi-headset/issues**

> The battery percentage is **estimated** from the cell voltage using a per-model
> calibration curve (Logitech doesn't publish its exact curves; the data points come
> from HeadsetControl). The raw voltage is shown alongside so it's never misleading,
> and a new model's curve can be contributed the same way. **While charging**, the
> voltage is pushed up by the charger, so the % reads high — the daemon detects this
> and shows *“en charge”* next to the reading (and suppresses the low-battery alert).

## Prior art & credits

[HeadsetControl](https://github.com/Sapd/HeadsetControl) is the reference tool for
Logitech / SteelSeries / Corsair / etc. headsets on Linux — battery, sidetone, lights,
EQ and more, across many vendors. `logi-headset` borrows its approach for reading the
G533/G733 battery voltage over HID++. The two are complementary: HeadsetControl manages
*settings* but doesn't remap buttons, whereas `logi-headset`'s focus is **remapping the
mute button** to a media key (which HeadsetControl doesn't do). If you want sidetone,
EQ, inactive-time and broad vendor support, use HeadsetControl alongside this.

## License

Copyright (C) 2026 WhiteWolf832. Released under the **GNU General Public License
v3.0 or later** — see [LICENSE](LICENSE). This program comes with ABSOLUTELY NO
WARRANTY.
