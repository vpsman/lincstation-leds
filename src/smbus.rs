//! Minimal SMBus access through the Linux i2c-dev ioctls.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::io::AsRawFd;

const I2C_SLAVE: libc::c_ulong = 0x0703;
const I2C_FUNCS: libc::c_ulong = 0x0705;
const I2C_SMBUS: libc::c_ulong = 0x0720;

const I2C_SMBUS_WRITE: u8 = 0;
const I2C_SMBUS_QUICK: u32 = 0;
const I2C_SMBUS_BYTE_DATA: u32 = 2;

const I2C_FUNC_SMBUS_QUICK: libc::c_ulong = 0x0001_0000;
const I2C_FUNC_SMBUS_WRITE_BYTE_DATA: libc::c_ulong = 0x0010_0000;

/// `union i2c_smbus_data`: the largest member is `block[I2C_SMBUS_BLOCK_MAX + 2]`.
#[repr(C)]
struct SmbusData {
    block: [u8; 34],
}

/// `struct i2c_smbus_ioctl_data`.
#[repr(C)]
struct SmbusIoctl {
    read_write: u8,
    command: u8,
    size: u32,
    data: *mut SmbusData,
}

pub struct Device {
    file: File,
    pub bus: u32,
}

impl Device {
    fn open(bus: u32, addr: u16) -> io::Result<Device> {
        let file = OpenOptions::new().read(true).write(true).open(format!("/dev/i2c-{bus}"))?;
        // SAFETY: I2C_SLAVE takes the address by value.
        if unsafe { libc::ioctl(file.as_raw_fd(), I2C_SLAVE as _, addr as libc::c_ulong) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Device { file, bus })
    }

    fn funcs(&self) -> io::Result<libc::c_ulong> {
        let mut funcs: libc::c_ulong = 0;
        // SAFETY: I2C_FUNCS writes one unsigned long to the pointer.
        if unsafe { libc::ioctl(self.file.as_raw_fd(), I2C_FUNCS as _, &mut funcs) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(funcs)
    }

    fn smbus(&self, command: u8, size: u32, data: Option<&mut SmbusData>) -> io::Result<()> {
        let mut args = SmbusIoctl {
            read_write: I2C_SMBUS_WRITE,
            command,
            size,
            data: data.map_or(std::ptr::null_mut(), |d| d as *mut _),
        };
        // SAFETY: args and the data it points to outlive the call.
        if unsafe { libc::ioctl(self.file.as_raw_fd(), I2C_SMBUS as _, &mut args) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// The same probe `i2cdetect` uses for this address range: an SMBus quick write.
    fn probe(&self) -> bool {
        self.smbus(0, I2C_SMBUS_QUICK, None).is_ok()
    }

    pub fn write_byte(&self, reg: u8, value: u8) -> io::Result<()> {
        let mut data = SmbusData { block: [0; 34] };
        data.block[0] = value;
        self.smbus(reg, I2C_SMBUS_BYTE_DATA, Some(&mut data))
    }
}

/// Finds the bus with a device answering at `addr`. SMBus-class adapters are probed
/// first, so on the N2 the display (i915, DP AUX) buses are never touched.
pub fn discover(addr: u16) -> Option<Device> {
    let mut buses: Vec<(bool, u32)> = fs::read_dir("/sys/class/i2c-dev")
        .ok()?
        .filter_map(|e| {
            let name = e.ok()?.file_name().into_string().ok()?;
            let bus: u32 = name.strip_prefix("i2c-")?.parse().ok()?;
            let label = fs::read_to_string(format!("/sys/class/i2c-dev/{name}/name")).unwrap_or_default();
            Some((!label.starts_with("SMBus"), bus))
        })
        .collect();
    buses.sort();
    for (_, bus) in buses {
        let Ok(dev) = Device::open(bus, addr) else { continue };
        let need = I2C_FUNC_SMBUS_QUICK | I2C_FUNC_SMBUS_WRITE_BYTE_DATA;
        if dev.funcs().is_ok_and(|f| f & need == need) && dev.probe() {
            return Some(dev);
        }
    }
    None
}
