//! Maps block devices to disk LEDs by their sysfs (PCI) path, so the result does
//! not depend on the order in which the kernel names `nvmeN` or `sdX`.

use crate::panel::Led;
use std::fs;

/// True if `key` is a whole component sequence of `path`:
/// "0000:00:17.0/ata1" matches ".../0000:00:17.0/ata1/host0/..." but not ".../ata10/...".
pub fn path_matches(path: &str, key: &str) -> bool {
    let key = key.trim_matches('/');
    !key.is_empty()
        && path.match_indices(key).any(|(i, _)| {
            path[..i].ends_with('/') && matches!(path[i + key.len()..].chars().next(), None | Some('/'))
        })
}

/// The LED whose slot key matches `path`, if any.
pub fn led_for_path(path: &str, slots: &[(Led, String)]) -> Option<Led> {
    slots.iter().find(|(_, key)| path_matches(path, key)).map(|(led, _)| *led)
}

/// Whole disks under /sys/block with their resolved device path.
pub fn scan() -> Vec<(String, String)> {
    let Ok(dir) = fs::read_dir("/sys/block") else { return Vec::new() };
    let mut out: Vec<(String, String)> = dir
        .filter_map(|e| {
            let e = e.ok()?;
            let name = e.file_name().into_string().ok()?;
            let dev = fs::canonicalize(e.path().join("device")).ok()?;
            Some((name, dev.to_string_lossy().into_owned()))
        })
        .collect();
    out.sort();
    out
}

/// Disk name for a partition or disk name (`nvme0n1p3` -> `nvme0n1`, `sda1` -> `sda`).
pub fn parent_disk(name: &str) -> String {
    let link = fs::canonicalize(format!("/sys/class/block/{name}"));
    match link {
        Ok(p) if p.join("partition").exists() => p
            .parent()
            .and_then(|q| q.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.to_string()),
        _ => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Paths as reported on pve3 on 2026-10-12.
    const NVME0: &str = "/sys/devices/pci0000:00/0000:00:1c.6/0000:03:00.0/nvme/nvme0";
    const NVME1: &str = "/sys/devices/pci0000:00/0000:00:1c.3/0000:02:00.0/nvme/nvme1";
    const SDA: &str = "/sys/devices/pci0000:00/0000:00:17.0/ata1/host0/target0:0:0/0:0:0:0";
    const SDB: &str = "/sys/devices/pci0000:00/0000:00:17.0/ata2/host1/target1:0:0/1:0:0:0";

    fn slots() -> Vec<(Led, String)> {
        vec![
            (Led::Hdd0, "0000:00:17.0/ata1".into()),
            (Led::Hdd1, "0000:00:17.0/ata2".into()),
            (Led::Nvme1, "0000:00:1c.3".into()),
            (Led::Nvme2, "0000:00:1c.6".into()),
        ]
    }

    #[test]
    fn maps_pve3_paths() {
        let s = slots();
        assert_eq!(led_for_path(SDA, &s), Some(Led::Hdd0));
        assert_eq!(led_for_path(SDB, &s), Some(Led::Hdd1));
        assert_eq!(led_for_path(NVME1, &s), Some(Led::Nvme1));
        assert_eq!(led_for_path(NVME0, &s), Some(Led::Nvme2));
    }

    #[test]
    fn whole_components_only() {
        let ata10 = "/sys/devices/pci0000:00/0000:00:17.0/ata10/host9/target9:0:0/9:0:0:0";
        assert_eq!(led_for_path(ata10, &slots()), None);
        assert!(!path_matches(NVME0, "0000:00:1c"));
        assert!(!path_matches(NVME0, "00:1c.6"));
        assert!(path_matches(NVME0, "/0000:00:1c.6/"));
    }

    #[test]
    fn empty_key_never_matches() {
        assert!(!path_matches(SDA, ""));
        assert_eq!(led_for_path("/sys/devices/virtual/block/loop0", &slots()), None);
    }
}
