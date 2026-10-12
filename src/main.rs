//! LED daemon for the LincStation N2 front panel.
//!
//! Disk LEDs are steady white while healthy and blink to `disk_activity` (off by
//! default) on I/O. A disk that ZFS reports as faulted, removed or unavailable
//! turns steady red (orange on this panel). The network LED is steady white while
//! the link is up, flickers on traffic and is red when the link is down. The power
//! LED shows the configured colour. The RGB strip shows the worst pool's state:
//! ok, scrub, errors, resilver, degraded or fault (which includes a locked key).
//!
//! On SIGTERM or SIGINT the daemon leaves the panel in a defined state: disk and
//! network LEDs off, power LED white, strip off, no LED blinking.

mod config;
mod disks;
mod panel;
mod smbus;
mod stats;
mod zfs;

use panel::{Frame, Led, Panel};
use stats::Rereader;
use std::collections::HashSet;
use std::fs;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

const CONFIG: &str = "/etc/lincstation-leds.conf";
const DISCOVERY_ATTEMPTS: u32 = 30;
const FULL_REFRESH: Duration = Duration::from_secs(60);

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

fn stopping() -> bool {
    STOP.load(Ordering::Relaxed)
}

/// The physical NIC: has a `device` link and is not wireless. Bridge ports first.
fn find_nic() -> Option<String> {
    let mut nics: Vec<(bool, String)> = fs::read_dir("/sys/class/net")
        .ok()?
        .filter_map(|e| {
            let p = e.ok()?.path();
            if !p.join("device").exists() || p.join("wireless").exists() {
                return None;
            }
            Some((!p.join("master").exists(), p.file_name()?.to_string_lossy().into_owned()))
        })
        .collect();
    nics.sort();
    nics.into_iter().next().map(|(_, n)| n)
}

struct Disk {
    name: String,
    led: Led,
    prev: Option<u64>,
    /// Showing the activity colour in this sample.
    blink: bool,
}

fn map_disks(cfg: &config::Config, old: &[Disk], announce: bool) -> Vec<Disk> {
    let mut out = Vec::new();
    for (name, path) in disks::scan() {
        let led = disks::led_for_path(&path, &cfg.slots);
        if announce {
            match led {
                Some(l) => eprintln!("disk {name} -> {} ({path})", l.name()),
                None => eprintln!("disk {name} unmapped ({path})"),
            }
        }
        let Some(led) = led else { continue };
        let prev = old.iter().find(|d| d.name == name && d.led == led).and_then(|d| d.prev);
        out.push(Disk { name, led, prev, blink: false });
    }
    out
}

struct Net {
    rx: Rereader,
    tx: Rereader,
    state: Rereader,
    prev: Option<u64>,
    lit: bool,
}

impl Net {
    fn open(iface: &str) -> std::io::Result<Net> {
        let base = format!("/sys/class/net/{iface}");
        Ok(Net {
            rx: Rereader::open(&format!("{base}/statistics/rx_bytes"))?,
            tx: Rereader::open(&format!("{base}/statistics/tx_bytes"))?,
            state: Rereader::open(&format!("{base}/operstate"))?,
            prev: None,
            lit: false,
        })
    }

    fn colour(&mut self, elapsed: Duration, threshold: u64) -> panel::Colour {
        if self.state.read().map(|s| s.trim() != "up").unwrap_or(true) {
            self.prev = None;
            return panel::RED;
        }
        let (Ok(rx), Ok(tx)) = (self.rx.read_u64(), self.tx.read_u64()) else { return panel::RED };
        let total = rx.wrapping_add(tx);
        let bytes = self.prev.map_or(0, |p| stats::delta(p, total));
        self.prev = Some(total);
        let per_sec = bytes as u128 * 1000 / elapsed.as_millis().max(1);
        if per_sec >= threshold.max(1) as u128 {
            self.lit = !self.lit;
        } else {
            self.lit = true;
        }
        if self.lit { panel::WHITE } else { panel::OFF_C }
    }
}

fn main() -> ExitCode {
    let cfg = match fs::read_to_string(CONFIG) {
        Ok(text) => match config::parse(&text) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{CONFIG}: {e}");
                return ExitCode::FAILURE;
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => config::Config::default(),
        Err(e) => {
            eprintln!("{CONFIG}: {e}");
            return ExitCode::FAILURE;
        }
    };

    for sig in [libc::SIGTERM, libc::SIGINT] {
        // SAFETY: the handler only stores to an atomic.
        unsafe { libc::signal(sig, on_signal as *const () as libc::sighandler_t) };
    }

    // The bus number can change between boots, and i2c-i801 may still be loading.
    let mut attempt = 0;
    let dev = loop {
        if let Some(d) = smbus::discover(panel::ADDR) {
            break d;
        }
        attempt += 1;
        if attempt >= DISCOVERY_ATTEMPTS || stopping() {
            eprintln!("no device at 0x26 on any /dev/i2c-* (is i2c-dev loaded?)");
            return ExitCode::FAILURE;
        }
        sleep(Duration::from_secs(1));
    };
    let mut panel = Panel::new(dev);
    eprintln!("LED controller 0x26 on /dev/i2c-{}", panel.bus());

    let iface = cfg.net_iface.clone().or_else(find_nic);
    let mut net = match iface.as_deref().map(Net::open) {
        Some(Ok(n)) => {
            eprintln!("network LED tracks {}", iface.as_deref().unwrap_or_default());
            Some(n)
        }
        Some(Err(e)) => {
            eprintln!("network LED off: {}: {e}", iface.as_deref().unwrap_or_default());
            None
        }
        None => {
            eprintln!("network LED off: no physical NIC found");
            None
        }
    };

    let mut diskstats = match Rereader::open("/proc/diskstats") {
        Ok(r) => r,
        Err(e) => {
            eprintln!("/proc/diskstats: {e}");
            return ExitCode::FAILURE;
        }
    };

    let interval = Duration::from_millis(cfg.interval_ms);
    let zfs_interval = Duration::from_secs(cfg.zfs_interval_s);
    let mut disks = map_disks(&cfg, &[], true);
    let mut bad_disks: HashSet<String> = HashSet::new();
    let mut health: Option<zfs::Health> = None;
    let mut level = zfs::Level::Ok;
    let mut zfs_ok = true;
    let mut last_zfs: Option<Instant> = None;
    let mut last_force: Option<Instant> = None;
    let mut io_failing = false;
    let mut last_tick = Instant::now();
    let mut next_tick = Instant::now();

    while !stopping() {
        let now = Instant::now();
        let elapsed = now - last_tick;
        last_tick = now;

        if last_zfs.is_none_or(|t| now - t >= zfs_interval) {
            last_zfs = Some(now);
            disks = map_disks(&cfg, &disks, false);
            match zfs::query() {
                Some(h) => {
                    if !zfs_ok {
                        eprintln!("zpool status works again");
                    }
                    zfs_ok = true;
                    level = h.level;
                    bad_disks = h.bad_devices.iter().map(|d| disks::parent_disk(d)).collect();
                    if health.as_ref() != Some(&h) {
                        eprintln!("zfs: {:?}, unhealthy devices {:?}", h.level, h.bad_devices);
                        health = Some(h);
                    }
                }
                None => {
                    if zfs_ok {
                        eprintln!("zpool or zfs failed; pool health unknown");
                    }
                    zfs_ok = false;
                }
            }
        }

        let mut frame = Frame::default();
        frame.set(Led::Power, cfg.power);
        let table = diskstats.read().unwrap_or("");
        for d in &mut disks {
            let ios = stats::disk_ios(table, &d.name);
            let active = matches!((d.prev, ios), (Some(p), Some(c)) if stats::delta(p, c) > 0);
            d.prev = ios;
            d.blink = active && !d.blink;
            let colour = if bad_disks.contains(&d.name) {
                panel::RED
            } else if d.blink {
                cfg.disk_activity
            } else {
                panel::WHITE
            };
            frame.set(d.led, colour);
        }
        if let Some(n) = net.as_mut() {
            frame.set(Led::Network, n.colour(elapsed, cfg.net_threshold));
        }
        let strip = match level {
            zfs::Level::Ok => cfg.strip_ok,
            zfs::Level::Scrub => cfg.strip_scrub,
            zfs::Level::Errors => cfg.strip_errors,
            zfs::Level::Resilver => cfg.strip_resilver,
            zfs::Level::Degraded => cfg.strip_degraded,
            zfs::Level::Fault => cfg.strip_fault,
        };

        // Re-assert every register now and then, and after a failed write.
        let result = if io_failing || last_force.is_none_or(|t| now - t >= FULL_REFRESH) {
            last_force = Some(now);
            panel.force(&frame, strip)
        } else {
            panel.show(&frame, strip)
        };
        match result {
            Err(e) if !io_failing => {
                eprintln!("i2c write failed: {e}");
                io_failing = true;
            }
            Ok(()) if io_failing => {
                eprintln!("i2c writes work again");
                io_failing = false;
            }
            _ => {}
        }

        next_tick += interval;
        let now = Instant::now();
        if next_tick > now {
            sleep(next_tick - now);
        } else {
            next_tick = now;
        }
    }

    let mut frame = Frame::default();
    frame.set(Led::Power, panel::WHITE);
    match panel.force(&frame, panel::STRIP_OFF) {
        Ok(()) => {
            eprintln!("stopped: power white, other LEDs and strip off");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("stopping: i2c write failed: {e}");
            ExitCode::FAILURE
        }
    }
}
