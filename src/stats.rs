//! Kernel counters: `/proc/diskstats` and `/sys/class/net/<if>/statistics`.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

/// Difference between two readings of a monotonically increasing counter.
/// A counter that went backwards is assumed to have wrapped: at 32 bits if the
/// previous value fit in 32 bits, otherwise at 64 bits.
pub fn delta(prev: u64, cur: u64) -> u64 {
    if cur >= prev {
        cur - prev
    } else if prev <= u32::MAX as u64 {
        (u32::MAX as u64 - prev) + cur + 1
    } else {
        (u64::MAX - prev) + cur + 1
    }
}

/// Completed I/O operations (reads + writes + discards + flushes) for `name`,
/// or `None` if the disk is not listed.
pub fn disk_ios(diskstats: &str, name: &str) -> Option<u64> {
    for line in diskstats.lines() {
        let mut f = line.split_ascii_whitespace();
        if f.nth(2) != Some(name) {
            continue;
        }
        // After the name: 0 reads, 4 writes, 11 discards, 15 flushes.
        let rest: Vec<u64> = f.map(|v| v.parse().unwrap_or(0)).collect();
        let at = |i: usize| rest.get(i).copied().unwrap_or(0);
        return Some(at(0).wrapping_add(at(4)).wrapping_add(at(11)).wrapping_add(at(15)));
    }
    None
}

/// A file that is re-read from the start on every sample, reusing one buffer.
pub struct Rereader {
    file: File,
    pub buf: String,
}

impl Rereader {
    pub fn open(path: &str) -> io::Result<Rereader> {
        Ok(Rereader { file: File::open(path)?, buf: String::with_capacity(4096) })
    }

    pub fn read(&mut self) -> io::Result<&str> {
        self.buf.clear();
        self.file.seek(SeekFrom::Start(0))?;
        self.file.read_to_string(&mut self.buf)?;
        Ok(&self.buf)
    }

    pub fn read_u64(&mut self) -> io::Result<u64> {
        let s = self.read()?.trim();
        s.parse().map_err(|_| io::Error::new(io::ErrorKind::InvalidData, format!("not a number: {s:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_normal() {
        assert_eq!(delta(100, 250), 150);
        assert_eq!(delta(7, 7), 0);
    }

    #[test]
    fn delta_wraps_32_bit() {
        assert_eq!(delta(u32::MAX as u64 - 9, 5), 15);
    }

    #[test]
    fn delta_wraps_64_bit() {
        assert_eq!(delta(u64::MAX - 1, 3), 5);
    }

    const DISKSTATS: &str = "\
 259       0 nvme0n1 1000 10 80000 500 2000 20 160000 900 0 1200 1400 30 0 2400 5 40 7
 259       1 nvme0n1p1 900 10 72000 450 1900 20 150000 850 0 1100 1300 0 0 0 0 0 0
   8       0 sda 11 0 88 3 22 0 176 4 0 6 7
   8      16 sdb 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19
";

    #[test]
    fn diskstats_full_line() {
        // reads 1000 + writes 2000 + discards 30 + flushes 40
        assert_eq!(disk_ios(DISKSTATS, "nvme0n1"), Some(3070));
    }

    #[test]
    fn diskstats_old_kernel_without_discards() {
        assert_eq!(disk_ios(DISKSTATS, "sda"), Some(33));
    }

    #[test]
    fn diskstats_exact_name_only() {
        assert_eq!(disk_ios(DISKSTATS, "nvme0n1p1"), Some(2800));
        assert_eq!(disk_ios(DISKSTATS, "nvme0"), None);
        assert_eq!(disk_ios(DISKSTATS, "sdc"), None);
    }

    #[test]
    fn diskstats_field_positions() {
        assert_eq!(disk_ios(DISKSTATS, "sdb"), Some(1 + 5 + 12 + 16));
    }
}
