# lincstation-leds

A small daemon for the front-panel LEDs and RGB strip of the LincPlus LincStation N2, for Linux systems other than Unraid. It was written for, and tested on, an N2 running Proxmox VE on Debian 13. LincPlus only ships LED software for Unraid, so on any other OS every LED blinks in unison.

> **This project was vibe coded.** Claude (Anthropic's AI model) wrote the code. A human directed it and checked the hardware behaviour by hand on a real N2. Read the code before you trust it with your hardware. It is about 1,000 lines of Rust, tests included.

## What the LEDs show

| LED | Meaning |
|---|---|
| Disk (HDD 0/1, NVMe 1–4) | Off when idle. Flickers white on I/O. Steady red while the disk's ZFS pool is not ONLINE, or the disk itself is faulted or removed. |
| Network | Steady white while the link is up. Flickers on traffic above `net_threshold` bytes per second. Red when the link is down. |
| Power | The `power` colour from the config file, white by default. |
| RGB strip | `strip_ok` (dim purple by default) while every pool is ONLINE, `strip_fault` (breathing red) otherwise. |

The documented registers drive two elements per status LED, white and red. On the test unit the red looks orange, and white plus red looks pink. Whether the controller can show any other colour is untested.

When the daemon stops (SIGTERM or SIGINT), it turns the disk and network LEDs off, sets the power LED to white and switches the strip off. A stopped service is therefore visible on the panel.

## How it works

- **Bus discovery.** At startup the daemon probes every `/dev/i2c-*` for the controller at SMBus address `0x26`, SMBus adapters first, using the same quick-write probe as `i2cdetect`. The bus number can change between boots, so it is never hardcoded.
- **Disk mapping.** Each drive is matched to its LED by its sysfs path: the SATA port for the 2.5" bays, the PCIe root port for the M.2 slots. Kernel names like `nvme0n1` can swap between boots, so the daemon doesn't rely on them.
- **Activity.** Disk activity comes from `/proc/diskstats` and network activity from `/sys/class/net/<if>/statistics`, both as counter deltas with wraparound handled. The daemon samples every 100 ms and writes to the bus only when an LED changes. It re-writes every register once a minute.
- **ZFS health.** Read from `zpool status -LP` every 10 seconds.
- **Talking to the controller.** It uses the i2c-dev SMBus ioctls directly, without calling `i2cset`. The only dependency is `libc`.

## Build and install on the NAS

You need Rust 1.82 or newer. Debian 13's `cargo` package is new enough.

```sh
apt install git cargo
git clone https://github.com/vpsman/lincstation-leds.git
cd lincstation-leds
make            # cargo build --release --locked
make test       # optional: unit tests
make install    # as root: installs, enables and starts the service
```

`make install` puts the binary in `/usr/sbin`, the unit in `/etc/systemd/system` and an `i2c-dev` entry in `/etc/modules-load.d/`. It also installs `/etc/lincstation-leds.conf` if that file doesn't exist yet. `make uninstall` removes everything except the config file.

### Or as a .deb

To get a static binary in a package you can copy to other machines:

```sh
rustup target add x86_64-unknown-linux-musl
cargo install cargo-deb
cargo deb --target x86_64-unknown-linux-musl
apt install ./target/debian/lincstation-leds_*_amd64.deb
```

The package enables and starts the service on install and stops it on removal.

## Configuration

`/etc/lincstation-leds.conf` is commented. After an edit, run `systemctl restart lincstation-leds`.

The disk LED mapping depends on which slots your drives are in. At startup the daemon logs every drive it finds and the LED it chose:

```sh
journalctl -u lincstation-leds -b | grep disk
```

For a drive marked `unmapped`, find its root port in the logged path (`/sys/devices/pci0000:00/0000:00:1c.6/...` means `0000:00:1c.6`), then add a line for the slot it sits in, for example `nvme2 = 0000:00:1c.6`. The SATA bays are preset to `ata1` for HDD 0 and `ata2` for HDD 1. The M.2 slots are preset to root ports `1c.3`, `1c.6`, `1d.0` and `1d.1` for NVMe 1 to 4, in port order, which is not yet confirmed against the physical slots.

## Register map

Every write goes to SMBus address `0x26`. This map comes from [ffalt's reverse-engineering gist](https://gist.github.com/ffalt/984aa3644a90d4230eaf5b129aaf1eeb), with corrections confirmed by testing on an N2.

| LED | On reg | Off reg | White bit | Red bit | Blink reg |
|---|---|---|---|---|---|
| Power | 0xA0 | 0xB0 | 0x01 | 0x02 | 0x50 |
| HDD 0 | 0xA0 | 0xB0 | 0x04 | 0x08 | 0x52 |
| HDD 1 | 0xA0 | 0xB0 | 0x10 | 0x20 | 0x54 |
| Network | 0xA0 | 0xB0 | 0x40 | 0x80 | 0x56 |
| NVMe 1 | 0xA1 | 0xB1 | 0x01 | 0x02 | 0x58 |
| NVMe 2 | 0xA1 | 0xB1 | 0x04 | 0x08 | 0x5A |
| NVMe 3 | 0xA1 | 0xB1 | 0x10 | 0x20 | 0x5C |
| NVMe 4 | 0xA1 | 0xB1 | 0x40 | 0x80 | 0x5E |

- The on and off registers accept multi-bit masks, for example `0xFF` to turn everything off.
- Writing 0 to every blink register stops the power-on blinking.
- Strip: `0x90` sets the mode (0 solid, 1 breath, 2 loop) and `0x91` the brightness (0 turns the strip off). `0x92`–`0x94` hold the colour in **R, G, B** order. Loop mode cycles between colour 1 at `0x95`–`0x97` and colour 2 at **`0x98`–`0x9A`**; the gist's register list wrongly gives `0x95`–`0x97` for both.

## Prior art

- [fazalmajid/lincstation_leds](https://github.com/fazalmajid/lincstation_leds) is a C daemon for the same hardware.
- [ffalt's gist](https://gist.github.com/ffalt/984aa3644a90d4230eaf5b129aaf1eeb) has the original protocol notes.

## License

MIT
