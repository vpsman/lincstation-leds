//! Register map of the front-panel controller at SMBus address 0x26, as confirmed
//! on pve3 on 2026-10-12. Each "on" register sets the LEDs whose bits are written
//! and each "off" register clears them; multi-bit masks work on both.

use crate::smbus::Device;
use std::io;

pub const ADDR: u16 = 0x26;

const ON: [u8; 2] = [0xA0, 0xA1];
const OFF: [u8; 2] = [0xB0, 0xB1];
const BLINK: [u8; 8] = [0x50, 0x52, 0x54, 0x56, 0x58, 0x5A, 0x5C, 0x5E];

const STRIP_MODE: u8 = 0x90;
const STRIP_BRIGHTNESS: u8 = 0x91;
const STRIP_RGB: u8 = 0x92; // 0x92 red, 0x93 green, 0x94 blue
pub const MODE_SOLID: u8 = 0;
pub const MODE_BREATH: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Led {
    Power,
    Hdd0,
    Hdd1,
    Network,
    Nvme1,
    Nvme2,
    Nvme3,
    Nvme4,
}

impl Led {
    pub const DISKS: [Led; 6] = [Led::Hdd0, Led::Hdd1, Led::Nvme1, Led::Nvme2, Led::Nvme3, Led::Nvme4];

    pub fn name(self) -> &'static str {
        match self {
            Led::Power => "power",
            Led::Hdd0 => "hdd0",
            Led::Hdd1 => "hdd1",
            Led::Network => "network",
            Led::Nvme1 => "nvme1",
            Led::Nvme2 => "nvme2",
            Led::Nvme3 => "nvme3",
            Led::Nvme4 => "nvme4",
        }
    }

    pub fn from_name(s: &str) -> Option<Led> {
        [Led::Power, Led::Network].into_iter().chain(Led::DISKS).find(|l| l.name() == s)
    }

    /// (register index, white bit). The red bit is always `white << 1`.
    fn slot(self) -> (usize, u8) {
        match self {
            Led::Power => (0, 0x01),
            Led::Hdd0 => (0, 0x04),
            Led::Hdd1 => (0, 0x10),
            Led::Network => (0, 0x40),
            Led::Nvme1 => (1, 0x01),
            Led::Nvme2 => (1, 0x04),
            Led::Nvme3 => (1, 0x10),
            Led::Nvme4 => (1, 0x40),
        }
    }
}

/// The two documented colour elements of a status LED. On pve3 red alone looks
/// orange and white plus red looks pink. Other colours are untested.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Colour {
    pub white: bool,
    pub red: bool,
}

pub const OFF_C: Colour = Colour { white: false, red: false };
pub const WHITE: Colour = Colour { white: true, red: false };
pub const RED: Colour = Colour { white: false, red: true };

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Strip {
    pub mode: u8,
    pub brightness: u8,
    pub rgb: [u8; 3],
}

pub const STRIP_OFF: Strip = Strip { mode: MODE_SOLID, brightness: 0, rgb: [0, 0, 0] };

/// Desired state of every LED, as register bitmasks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Frame {
    pub bits: [u8; 2],
}

impl Frame {
    pub fn set(&mut self, led: Led, c: Colour) {
        let (reg, white) = led.slot();
        let both = white | white << 1;
        let mut v = 0;
        if c.white {
            v |= white;
        }
        if c.red {
            v |= white << 1;
        }
        self.bits[reg] = (self.bits[reg] & !both) | v;
    }
}

/// Register writes needed to go from `old` to `new`: (register, mask).
pub fn diff(old: &Frame, new: &Frame) -> Vec<(u8, u8)> {
    let mut w = Vec::new();
    for i in 0..2 {
        let on = new.bits[i] & !old.bits[i];
        let off = old.bits[i] & !new.bits[i];
        if off != 0 {
            w.push((OFF[i], off));
        }
        if on != 0 {
            w.push((ON[i], on));
        }
    }
    w
}

pub struct Panel {
    dev: Device,
    shown: Frame,
    strip: Option<Strip>,
}

impl Panel {
    pub fn new(dev: Device) -> Panel {
        Panel { dev, shown: Frame::default(), strip: None }
    }

    pub fn bus(&self) -> u32 {
        self.dev.bus
    }

    /// Writes every register, whatever was shown before: blink off, LEDs, strip.
    pub fn force(&mut self, frame: &Frame, strip: Strip) -> io::Result<()> {
        for r in BLINK {
            self.dev.write_byte(r, 0)?;
        }
        for i in 0..2 {
            self.dev.write_byte(OFF[i], !frame.bits[i])?;
            self.dev.write_byte(ON[i], frame.bits[i])?;
        }
        self.shown = *frame;
        self.strip = None;
        self.show(frame, strip)
    }

    /// Writes only what differs from the last successful write.
    pub fn show(&mut self, frame: &Frame, strip: Strip) -> io::Result<()> {
        for (reg, mask) in diff(&self.shown, frame) {
            self.dev.write_byte(reg, mask)?;
        }
        self.shown = *frame;
        if self.strip != Some(strip) {
            self.strip = None; // stays unknown if a write below fails
            for (i, v) in strip.rgb.iter().enumerate() {
                self.dev.write_byte(STRIP_RGB + i as u8, *v)?;
            }
            self.dev.write_byte(STRIP_BRIGHTNESS, strip.brightness)?;
            self.dev.write_byte(STRIP_MODE, strip.mode)?;
            self.strip = Some(strip);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_bits() {
        let mut f = Frame::default();
        f.set(Led::Power, WHITE);
        f.set(Led::Network, RED);
        f.set(Led::Nvme4, Colour { white: true, red: true });
        assert_eq!(f.bits, [0x01 | 0x80, 0x40 | 0x80]);
        f.set(Led::Network, OFF_C);
        f.set(Led::Nvme4, RED);
        assert_eq!(f.bits, [0x01, 0x80]);
    }

    #[test]
    fn diff_writes_only_changes() {
        let mut a = Frame::default();
        a.set(Led::Hdd0, WHITE);
        let mut b = a;
        assert!(diff(&a, &b).is_empty());
        b.set(Led::Hdd0, RED);
        b.set(Led::Nvme2, WHITE);
        assert_eq!(diff(&a, &b), vec![(0xB0, 0x04), (0xA0, 0x08), (0xA1, 0x04)]);
    }

    #[test]
    fn led_names_round_trip() {
        for l in [Led::Power, Led::Network].into_iter().chain(Led::DISKS) {
            assert_eq!(Led::from_name(l.name()), Some(l));
        }
        assert_eq!(Led::from_name("nvme5"), None);
    }
}
