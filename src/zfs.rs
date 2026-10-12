//! ZFS health from `zpool status -LP` and `zfs get keystatus`.

use std::process::Command;

/// What the strip shows, from least to most severe. The worst pool wins.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    #[default]
    Ok,
    Scrub,
    /// ONLINE, but READ/WRITE/CKSUM errors or data errors were counted.
    Errors,
    Resilver,
    Degraded,
    /// An encrypted dataset whose key is not loaded.
    Locked,
    /// A pool FAULTED, UNAVAIL or SUSPENDED.
    Fault,
}

#[derive(Debug, Default, PartialEq)]
pub struct Health {
    pub level: Level,
    /// Devices (partitions as listed, e.g. `sda1`) whose own state is unhealthy.
    pub bad_devices: Vec<String>,
}

fn healthy(state: &str) -> bool {
    // AVAIL and INUSE are spare states.
    matches!(state, "ONLINE" | "AVAIL" | "INUSE")
}

#[derive(Default)]
struct Pool {
    state: String,
    scrub: bool,
    resilver: bool,
    errors: bool,
}

impl Pool {
    fn level(&self) -> Level {
        match self.state.as_str() {
            "" => Level::Ok,
            "ONLINE" | "DEGRADED" if self.resilver => Level::Resilver,
            "DEGRADED" => Level::Degraded,
            "ONLINE" if self.errors => Level::Errors,
            "ONLINE" if self.scrub => Level::Scrub,
            "ONLINE" => Level::Ok,
            _ => Level::Fault,
        }
    }
}

pub fn parse(status: &str) -> Health {
    let mut h = Health::default();
    let mut pool: Option<Pool> = None;
    let mut in_config = false;
    for line in status.lines() {
        let t = line.trim();
        if t.starts_with("pool:") {
            if let Some(p) = pool.take() {
                h.level = h.level.max(p.level());
            }
            pool = Some(Pool::default());
            in_config = false;
            continue;
        }
        let Some(p) = pool.as_mut() else { continue };
        if let Some(s) = t.strip_prefix("state:") {
            p.state = s.trim().to_string();
        } else if let Some(s) = t.strip_prefix("scan:") {
            p.scrub = s.contains("scrub in progress");
            p.resilver = s.contains("resilver in progress");
        } else if t.starts_with("config:") {
            in_config = true;
        } else if let Some(e) = t.strip_prefix("errors:") {
            in_config = false;
            p.errors |= e.trim() != "No known data errors";
        } else if in_config {
            let f: Vec<&str> = t.split_ascii_whitespace().collect();
            if f.len() >= 5 && f[0] != "NAME" && f[2..5].iter().any(|c| *c != "0") {
                p.errors = true;
            }
            if let (Some(dev), Some(state)) = (f.first().and_then(|n| n.strip_prefix("/dev/")), f.get(1)) {
                if !healthy(state) {
                    h.bad_devices.push(dev.to_string());
                }
            }
        }
    }
    if let Some(p) = pool {
        h.level = h.level.max(p.level());
    }
    h
}

/// True if any encrypted dataset has its key unloaded (`zfs get -H -o value keystatus`).
pub fn any_locked(keystatus: &str) -> bool {
    keystatus.lines().any(|l| l.trim() == "unavailable")
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `None` if `zpool` could not be run. The key check is skipped where `zfs get
/// keystatus` fails, such as on ZFS without encryption support.
pub fn query() -> Option<Health> {
    let mut h = parse(&run("zpool", &["status", "-LP"])?);
    if run("zfs", &["get", "-H", "-o", "value", "keystatus"]).is_some_and(|k| any_locked(&k)) {
        h.level = h.level.max(Level::Locked);
    }
    Some(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONLINE: &str = "  pool: hddpool
 state: ONLINE
  scan: scrub repaired 0B in 01:02:03 with 0 errors on Sun Oct 11 01:26:04 2026
config:

\tNAME           STATE     READ WRITE CKSUM
\thddpool        ONLINE       0     0     0
\t  mirror-0     ONLINE       0     0     0
\t    /dev/sda1  ONLINE       0     0     0
\t    /dev/sdb1  ONLINE       0     0     0

errors: No known data errors

  pool: vmspool
 state: ONLINE
config:

\tNAME                STATE     READ WRITE CKSUM
\tvmspool             ONLINE       0     0     0
\t  mirror-0          ONLINE       0     0     0
\t    /dev/nvme0n1p1  ONLINE       0     0     0
\t    /dev/nvme1n1p1  ONLINE       0     0     0
\tspares
\t  /dev/sdc1         AVAIL

errors: No known data errors
";

    #[test]
    fn all_online() {
        assert_eq!(parse(ONLINE), Health::default());
    }

    #[test]
    fn scrub_in_progress() {
        let s = ONLINE.replacen("scrub repaired 0B in 01:02:03", "scrub in progress since Mon Oct 12 01:00:00 2026", 1);
        assert_eq!(parse(&s).level, Level::Scrub);
    }

    #[test]
    fn counted_errors_on_online_pool() {
        let s = ONLINE.replacen("/dev/sdb1  ONLINE       0     0     0", "/dev/sdb1  ONLINE       0     0     3", 1);
        assert_eq!(parse(&s).level, Level::Errors);
        let s = ONLINE.replacen("No known data errors", "1 data errors, use '-v' for a list", 1);
        assert_eq!(parse(&s).level, Level::Errors);
    }

    #[test]
    fn degraded_marks_only_the_failed_disk() {
        let s = ONLINE
            .replacen("state: ONLINE", "state: DEGRADED", 1)
            .replacen("/dev/sdb1  ONLINE       0     0     0", "/dev/sdb1  FAULTED      0     0     0  too many errors", 1);
        let h = parse(&s);
        assert_eq!(h.level, Level::Degraded);
        assert_eq!(h.bad_devices, vec!["sdb1"]);
    }

    #[test]
    fn resilver_outranks_degraded_on_the_same_pool() {
        let s = ONLINE
            .replacen("state: ONLINE", "state: DEGRADED", 1)
            .replacen("scrub repaired 0B in 01:02:03 with 0 errors on Sun Oct 11 01:26:04 2026", "resilver in progress since Mon Oct 12 01:00:00 2026", 1);
        assert_eq!(parse(&s).level, Level::Resilver);
    }

    #[test]
    fn worst_pool_wins() {
        let s = ONLINE.replacen("state: ONLINE", "state: DEGRADED", 1).replace("vmspool\n state: ONLINE", "vmspool\n state: SUSPENDED");
        assert_eq!(parse(&s).level, Level::Fault);
        let s = ONLINE.replace("vmspool\n state: ONLINE", "vmspool\n state: UNAVAIL");
        assert_eq!(parse(&s).level, Level::Fault);
    }

    #[test]
    fn empty_output() {
        assert_eq!(parse("no pools available\n"), Health::default());
    }

    #[test]
    fn locked_keys() {
        assert!(!any_locked("-\navailable\n-\n"));
        assert!(any_locked("-\navailable\nunavailable\n"));
    }
}
