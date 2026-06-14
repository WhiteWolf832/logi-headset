# AGENTS.md

Guidance for AI coding assistants (Claude Code, Copilot, Cursor, Codex, …) and human
contributors working in this repository.

## What this is

`logi-headset` is a small Linux tool for Logitech wireless (HID++) headsets: it remaps
the mute button to a media key, controls the LEDs, and warns on low battery. Two crates:

- **`daemon/`** — `logi-headset`, the background daemon. **libc only, zero dependencies.**
  Talks raw HID++ over `/dev/hidraw*` and injects keys via `/dev/uinput`. Runs as a
  systemd *user* service, without root (udev `uaccess` rule).
- **`gui/`** — `logi-headset-config`, a GTK4 config panel (gtk4-rs). Reads/writes the
  same config file and drives the service via `systemctl --user`.

## Build & run

```sh
( cd daemon && cargo build --release && cargo clippy --release )
( cd gui    && cargo build --release && cargo clippy --release )
```

Keep both **warning-free** (clippy clean) before committing. There are no unit tests —
validation is done on real hardware with the `--diagnose` / `--watch` tools below.

## Architecture notes (daemon)

- **Auto-detection, not PID-locked:** `find_headset()` keeps any Logitech (`046d`)
  exposing the G-keys feature `0x8010`. Feature *indices* (G-key, LED `0x8070`,
  battery, state `0x1f20`) are resolved at runtime, per model — never hard-coded.
- **Mute remap:** the mute button is a divertible HID++ G-key. The daemon enables
  diversion (`0x8010` fn 2), then injects the configured key on each press via uinput.
- **Battery:** read as a *voltage* from the proprietary `0x1f20` feature (fallback to
  the standard `0x1004` / `0x1000` as a percentage). Voltage → % uses **per-model
  calibration curves** (`CURVE_G533`, `CURVE_G633`, generic fallback), with the data
  points sourced from HeadsetControl. The raw voltage is always shown alongside.
- **Volatile wireless state:** on power-off the headset forgets the diversion; the
  daemon re-applies it on the wake notification and every few seconds. Some models
  (e.g. the G533) need ~1–2 min after power-on before they re-accept it — a firmware
  quirk, not a bug.

## Adding / fixing a headset model

1. `logi-headset --diagnose` (stop the service first) prints the HID++ feature list,
   resolved indices, model name and battery reading.
2. If a button isn't seen, `logi-headset --watch` shows raw HID++ reports as you press.
3. For battery accuracy, add a calibration curve in `daemon/src/main.rs`
   (`curve_for_model`) — discharge-curve data points, e.g. from HeadsetControl's
   `logitech_calibrations.hpp`.
4. When reporting a new model, paste a `--diagnose` report into a GitHub issue.

## Conventions

- Code **comments are in French** (the maintainer's language); identifiers and
  user-facing English strings are English.
- New source files carry the **GPL-3.0-or-later** header; copyright holder `WhiteWolf832`.
- The GUI is multilingual: add any new UI string to the `Strings` struct for **all six**
  languages (en, fr, de, it, es, pt).
- **No network, ever.** The daemon must never gain an `INTERNET`-style capability; it
  only touches `/dev/hidraw*` and `/dev/uinput`.
