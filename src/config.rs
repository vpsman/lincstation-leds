//! `/etc/lincstation-leds.conf`: `key = value` lines, `#` starts a comment.

use crate::panel::{self, Colour, Led, Strip};

#[derive(Debug, PartialEq)]
pub struct Config {
    /// Disk LED and the sysfs path components that identify its slot.
    pub slots: Vec<(Led, String)>,
    /// Interface for the network LED; `None` picks the physical NIC.
    pub net_iface: Option<String>,
    /// Traffic below this many bytes per second does not flicker the network LED.
    pub net_threshold: u64,
    pub interval_ms: u64,
    pub zfs_interval_s: u64,
    pub power: Colour,
    /// What a healthy disk LED blinks to on I/O; it is steady white when idle.
    pub disk_activity: Colour,
    /// Strip per pool health level; see `zfs::Level`.
    pub strip_ok: Strip,
    pub strip_scrub: Strip,
    pub strip_errors: Strip,
    pub strip_resilver: Strip,
    pub strip_degraded: Strip,
    pub strip_fault: Strip,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            slots: Vec::new(),
            net_iface: None,
            net_threshold: 16384,
            interval_ms: 100,
            zfs_interval_s: 10,
            power: panel::WHITE,
            disk_activity: panel::OFF_C,
            strip_ok: strip_of(panel::MODE_SOLID, PURPLE, 12),
            strip_scrub: strip_of(panel::MODE_BREATH, PURPLE, 40),
            strip_errors: strip_of(panel::MODE_SOLID, AMBER, 40),
            strip_resilver: Strip { rgb2: PURPLE, ..strip_of(panel::MODE_LOOP, AMBER, 80) },
            strip_degraded: strip_of(panel::MODE_BREATH, AMBER, 160),
            strip_fault: strip_of(panel::MODE_BREATH, [255, 0, 0], 160),
        }
    }
}

fn num(s: &str) -> Result<u64, String> {
    let r = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u64::from_str_radix(h, 16),
        None => s.parse(),
    };
    r.map_err(|_| format!("not a number: {s}"))
}

fn byte(s: &str) -> Result<u8, String> {
    u8::try_from(num(s)?).map_err(|_| format!("out of range 0-255: {s}"))
}

/// A status LED colour. On the N2 red looks orange and white+red looks pink.
fn colour(s: &str) -> Result<Colour, String> {
    match s {
        "white" => Ok(panel::WHITE),
        "red" | "orange" => Ok(panel::RED),
        "white+red" | "pink" => Ok(Colour { white: true, red: true }),
        "off" => Ok(panel::OFF_C),
        _ => Err(format!("expected white, orange, pink or off: {s}")),
    }
}

const PURPLE: [u8; 3] = [160, 0, 255];
const AMBER: [u8; 3] = [255, 80, 0];

const fn strip_of(mode: u8, rgb: [u8; 3], brightness: u8) -> Strip {
    Strip { mode, brightness, rgb, rgb2: [0; 3] }
}

/// "R G B BRIGHTNESS", or "R G B R G B BRIGHTNESS" for loop mode.
fn strip(s: &str, mode: u8) -> Result<Strip, String> {
    let v: Vec<u8> = s.split_ascii_whitespace().map(byte).collect::<Result<_, _>>()?;
    match (mode, &v[..]) {
        (panel::MODE_LOOP, &[r, g, b, r2, g2, b2, brightness]) => {
            Ok(Strip { rgb2: [r2, g2, b2], ..strip_of(mode, [r, g, b], brightness) })
        }
        (panel::MODE_LOOP, _) => Err(format!("expected R G B R G B BRIGHTNESS: {s}")),
        (_, &[r, g, b, brightness]) => Ok(strip_of(mode, [r, g, b], brightness)),
        _ => Err(format!("expected R G B BRIGHTNESS: {s}")),
    }
}

pub fn parse(text: &str) -> Result<Config, String> {
    let mut c = Config::default();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let err = |e: String| format!("line {}: {e}", n + 1);
        let (key, value) = line.split_once('=').ok_or_else(|| err("expected key = value".into()))?;
        let (key, value) = (key.trim(), value.trim());
        match key {
            "net_iface" => c.net_iface = (value != "auto").then(|| value.to_string()),
            "net_threshold" => c.net_threshold = num(value).map_err(err)?,
            "interval_ms" => c.interval_ms = num(value).map_err(err)?.clamp(20, 5000),
            "zfs_interval_s" => c.zfs_interval_s = num(value).map_err(err)?.max(1),
            "power" => c.power = colour(value).map_err(err)?,
            "disk_activity" => c.disk_activity = colour(value).map_err(err)?,
            "strip_ok" => c.strip_ok = strip(value, panel::MODE_SOLID).map_err(err)?,
            "strip_scrub" => c.strip_scrub = strip(value, panel::MODE_BREATH).map_err(err)?,
            "strip_errors" => c.strip_errors = strip(value, panel::MODE_SOLID).map_err(err)?,
            "strip_resilver" => c.strip_resilver = strip(value, panel::MODE_LOOP).map_err(err)?,
            "strip_degraded" => c.strip_degraded = strip(value, panel::MODE_BREATH).map_err(err)?,
            "strip_fault" => c.strip_fault = strip(value, panel::MODE_BREATH).map_err(err)?,
            _ => match Led::from_name(key) {
                Some(led) if Led::DISKS.contains(&led) => c.slots.push((led, value.to_string())),
                _ => return Err(err(format!("unknown key: {key}"))),
            },
        }
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_config_parses() {
        let c = parse(include_str!("../debian/lincstation-leds.conf")).unwrap();
        assert_eq!(c.slots[0], (Led::Hdd0, "0000:00:17.0/ata1".to_string()));
        assert_eq!(c.slots[1], (Led::Hdd1, "0000:00:17.0/ata2".to_string()));
        assert_eq!(c.slots[2], (Led::Nvme1, "0000:00:1c.3".to_string()));
        assert_eq!(c.net_iface, None);
        let d = Config::default();
        assert_eq!(
            (c.strip_ok, c.strip_scrub, c.strip_errors, c.strip_resilver, c.strip_degraded, c.strip_fault),
            (d.strip_ok, d.strip_scrub, d.strip_errors, d.strip_resilver, d.strip_degraded, d.strip_fault)
        );
        assert_eq!(c.disk_activity, panel::OFF_C);
    }

    #[test]
    fn values() {
        let c = parse(
            "nvme3 = 0000:00:1c.6  # slot 3\n\
             net_iface = enp1s0\n\
             power = pink\n\
             disk_activity = white+red\n\
             strip_ok = 0x40 0 0xff 16\n\
             strip_resilver = 1 2 3 4 5 6 7\n\
             interval_ms = 1\n",
        )
        .unwrap();
        assert_eq!(c.slots, vec![(Led::Nvme3, "0000:00:1c.6".to_string())]);
        assert_eq!(c.net_iface.as_deref(), Some("enp1s0"));
        assert_eq!(c.power, Colour { white: true, red: true });
        assert_eq!(c.disk_activity, c.power);
        assert_eq!(c.strip_ok, strip_of(panel::MODE_SOLID, [0x40, 0, 0xff], 16));
        assert_eq!(c.strip_resilver, Strip { mode: panel::MODE_LOOP, brightness: 7, rgb: [1, 2, 3], rgb2: [4, 5, 6] });
        assert_eq!(c.interval_ms, 20);
    }

    #[test]
    fn errors_name_the_line() {
        assert_eq!(parse("\nnetwork = x\n").unwrap_err(), "line 2: unknown key: network");
        assert!(parse("strip_ok = 1 2 3\n").unwrap_err().contains("R G B"));
        assert!(parse("power = purple\n").unwrap_err().contains("expected white"));
        assert!(parse("strip_resilver = 1 2 3 4\n").unwrap_err().contains("R G B R G B"));
        assert!(parse("strip_ok = 1 2 3 256\n").unwrap_err().contains("0-255"));
    }
}
