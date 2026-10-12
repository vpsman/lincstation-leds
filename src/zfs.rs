//! ZFS health from `zpool status -LP`.

use std::process::Command;

#[derive(Debug, Default, PartialEq)]
pub struct Health {
    /// Pools whose state is not ONLINE.
    pub bad_pools: Vec<String>,
    /// Disk device names (partitions as listed, e.g. `sda1`) in an unhealthy pool
    /// or with an unhealthy state of their own.
    pub bad_devices: Vec<String>,
}

fn healthy(state: &str) -> bool {
    // AVAIL and INUSE are spare states.
    matches!(state, "ONLINE" | "AVAIL" | "INUSE")
}

pub fn parse(status: &str) -> Health {
    let mut h = Health::default();
    let mut pool = String::new();
    let mut pool_ok = true;
    for line in status.lines() {
        let t = line.trim();
        if let Some(p) = t.strip_prefix("pool:") {
            pool = p.trim().to_string();
            pool_ok = true;
        } else if let Some(s) = t.strip_prefix("state:") {
            pool_ok = healthy(s.trim());
            if !pool_ok {
                h.bad_pools.push(pool.clone());
            }
        } else if let Some(dev) = t.strip_prefix("/dev/") {
            let mut f = dev.split_ascii_whitespace();
            let (Some(name), state) = (f.next(), f.next().unwrap_or("")) else { continue };
            if !pool_ok || !healthy(state) {
                h.bad_devices.push(name.to_string());
            }
        }
    }
    h
}

/// `None` if `zpool` could not be run.
pub fn query() -> Option<Health> {
    let out = Command::new("zpool").args(["status", "-LP"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(parse(&String::from_utf8_lossy(&out.stdout)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONLINE: &str = "  pool: hddpool
 state: ONLINE
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

errors: No known data errors
";

    const DEGRADED: &str = "  pool: hddpool
 state: DEGRADED
status: One or more devices could not be used because the label is missing or
\tinvalid.
config:

\tNAME           STATE     READ WRITE CKSUM
\thddpool        DEGRADED     0     0     0
\t  mirror-0     DEGRADED     0     0     0
\t    /dev/sda1  ONLINE       0     0     0
\t    /dev/sdb1  FAULTED      0     0     0  corrupted data
\tspares
\t  /dev/sdc1    AVAIL

  pool: vmspool
 state: ONLINE
config:

\tNAME                STATE     READ WRITE CKSUM
\tvmspool             ONLINE       0     0     0
\t  /dev/nvme0n1p1    ONLINE       0     0     0
\t  /dev/nvme1n1p1    REMOVED      0     0     0
";

    #[test]
    fn all_online() {
        assert_eq!(parse(ONLINE), Health::default());
    }

    #[test]
    fn degraded_pool_marks_its_disks() {
        let h = parse(DEGRADED);
        assert_eq!(h.bad_pools, vec!["hddpool"]);
        // Every disk of a degraded pool, the spare included, plus a removed disk elsewhere.
        assert_eq!(h.bad_devices, vec!["sda1", "sdb1", "sdc1", "nvme1n1p1"]);
    }

    #[test]
    fn empty_output() {
        assert_eq!(parse("no pools available\n"), Health::default());
    }
}
