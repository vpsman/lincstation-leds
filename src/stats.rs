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

/// Completed I/O operations (reads + writes + discards + flushes) from the
/// contents of `/sys/block/<disk>/stat`, or `None` if it does not parse.
pub fn disk_ios(stat: &str) -> Option<u64> {
    // Fields: 0 reads, 4 writes, 11 discards, 15 flushes (the last two since Linux 4.18 and 5.5).
    let f: Vec<u64> = stat.split_ascii_whitespace().map(|v| v.parse().ok()).collect::<Option<_>>()?;
    let at = |i: usize| f.get(i).copied().unwrap_or(0);
    (f.len() >= 11).then(|| at(0).wrapping_add(at(4)).wrapping_add(at(11)).wrapping_add(at(15)))
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

    #[test]
    fn stat_full() {
        // reads 1000 + writes 2000 + discards 30 + flushes 40
        let stat = "    1000       10    80000      500     2000       20   160000      900        0     1200     1400       30        0     2400        5       40        7\n";
        assert_eq!(disk_ios(stat), Some(3070));
    }

    #[test]
    fn stat_old_kernel_without_discards() {
        assert_eq!(disk_ios("11 0 88 3 22 0 176 4 0 6 7\n"), Some(33));
    }

    #[test]
    fn stat_field_positions() {
        assert_eq!(disk_ios("1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17"), Some(1 + 5 + 12 + 16));
    }

    #[test]
    fn stat_garbage() {
        assert_eq!(disk_ios(""), None);
        assert_eq!(disk_ios("1 2 3"), None);
        assert_eq!(disk_ios("1 2 x 4 5 6 7 8 9 10 11"), None);
    }
}
