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
    pub strip_ok: Strip,
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
            strip_ok: Strip { mode: panel::MODE_SOLID, brightness: 12, rgb: [160, 0, 255] },
            strip_fault: Strip { mode: panel::MODE_BREATH, brightness: 160, rgb: [255, 0, 0] },
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

/// "R G B BRIGHTNESS".
fn strip(s: &str, mode: u8) -> Result<Strip, String> {
    let v: Vec<u8> = s.split_ascii_whitespace().map(byte).collect::<Result<_, _>>()?;
    match v[..] {
        [r, g, b, brightness] => Ok(Strip { mode, brightness, rgb: [r, g, b] }),
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
        assert_eq!(c.strip_ok, Config::default().strip_ok);
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
             interval_ms = 1\n",
        )
        .unwrap();
        assert_eq!(c.slots, vec![(Led::Nvme3, "0000:00:1c.6".to_string())]);
        assert_eq!(c.net_iface.as_deref(), Some("enp1s0"));
        assert_eq!(c.power, Colour { white: true, red: true });
        assert_eq!(c.disk_activity, c.power);
        assert_eq!(c.strip_ok, Strip { mode: panel::MODE_SOLID, brightness: 16, rgb: [0x40, 0, 0xff] });
        assert_eq!(c.interval_ms, 20);
    }

    #[test]
    fn errors_name_the_line() {
        assert_eq!(parse("\nnetwork = x\n").unwrap_err(), "line 2: unknown key: network");
        assert!(parse("strip_ok = 1 2 3\n").unwrap_err().contains("R G B"));
        assert!(parse("power = purple\n").unwrap_err().contains("expected white"));
        assert!(parse("strip_ok = 1 2 3 256\n").unwrap_err().contains("0-255"));
    }
}
