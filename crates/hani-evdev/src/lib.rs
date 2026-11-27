// SPDX-License-Identifier: AGPL-3.0-only

//! The little of Linux's evdev and uinput interfaces HaniSculpt uses, over
//! `libc`.
//!
//! The tablet and SpaceMouse readers (`hani-app`'s `tablet.rs`,
//! `spacemouse.rs` and `input_watch.rs`) open an event device, ask what it
//! has (its name, its keys, its absolute and relative axes, an axis's range)
//! and read its events. Their tests, and the `poke` example, make a virtual
//! device through `/dev/uinput` and write events to it. That is all of it.
//!
//! It replaced the `evdev` crate on 2026-11-27 (`docs/DEPENDENCIES.md`,
//! proposal 5), which brought `nix` and `bitvec` with four crates of its own
//! for this. The names are evdev's own (`Device`, `KeyCode::BTN_TOOL_PEN`,
//! `AttributeSet`, `VirtualDevice::builder()`), so the readers changed only
//! the line that imports them.
//!
//! # The ioctls
//!
//! Numbered by the generic `_IOC` encoding: direction in the top two bits,
//! then the size, the type letter and the number. That is the encoding on
//! x86-64, AArch64 and RISC-V, which are the Linux targets this application
//! is built for; PowerPC, MIPS and SPARC lay the bits out differently and are
//! refused at compile time rather than sent the wrong numbers.
//!
//! On anything but Linux this crate is empty.

#![cfg(target_os = "linux")]

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "riscv64")))]
compile_error!(
    "hani-evdev numbers its ioctls by the generic _IOC layout, which this target does not use"
);

use std::ffi::CStr;
use std::fs::{File, OpenOptions};
use std::io;
use std::marker::PhantomData;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

const IOC_NONE: u64 = 0;
const IOC_WRITE: u64 = 1;
const IOC_READ: u64 = 2;

const fn ioc(direction: u64, kind: u8, number: u8, size: usize) -> u64 {
    (direction << 30) | ((size as u64) << 16) | ((kind as u64) << 8) | number as u64
}

fn eviocgname(length: usize) -> u64 {
    ioc(IOC_READ, b'E', 0x06, length)
}

fn eviocgbit(event_type: u16, length: usize) -> u64 {
    ioc(IOC_READ, b'E', 0x20 + event_type as u8, length)
}

fn eviocgabs(axis: u16) -> u64 {
    ioc(IOC_READ, b'E', 0x40 + axis as u8, std::mem::size_of::<libc::input_absinfo>())
}

const UI_DEV_CREATE: u64 = ioc(IOC_NONE, b'U', 1, 0);
const UI_DEV_DESTROY: u64 = ioc(IOC_NONE, b'U', 2, 0);
const UI_DEV_SETUP: u64 = ioc(IOC_WRITE, b'U', 3, std::mem::size_of::<libc::uinput_setup>());
const UI_ABS_SETUP: u64 = ioc(IOC_WRITE, b'U', 4, std::mem::size_of::<libc::uinput_abs_setup>());
const UI_SET_EVBIT: u64 = ioc(IOC_WRITE, b'U', 100, std::mem::size_of::<libc::c_int>());
const UI_SET_KEYBIT: u64 = ioc(IOC_WRITE, b'U', 101, std::mem::size_of::<libc::c_int>());
const UI_SET_RELBIT: u64 = ioc(IOC_WRITE, b'U', 102, std::mem::size_of::<libc::c_int>());
const UI_SET_ABSBIT: u64 = ioc(IOC_WRITE, b'U', 103, std::mem::size_of::<libc::c_int>());

fn ui_get_sysname(length: usize) -> u64 {
    ioc(IOC_READ, b'U', 44, length)
}

/// One ioctl on `file`, its result or the error it set.
///
/// # Safety
///
/// `argument` must be what `request` reads or writes, of the size the request
/// encodes.
unsafe fn ioctl(file: &File, request: u64, argument: *mut libc::c_void) -> io::Result<libc::c_int> {
    // SAFETY: the caller's contract; the descriptor is open for as long as
    // `file` is borrowed.
    let result = unsafe { libc::ioctl(file.as_raw_fd(), request as libc::Ioctl, argument) };
    if result < 0 { Err(io::Error::last_os_error()) } else { Ok(result) }
}

/// The highest key code, `KEY_MAX`.
const KEY_MAX: u16 = 0x2ff;
/// The highest absolute axis, `ABS_MAX`.
const ABS_MAX: u16 = 0x3f;
/// The highest relative axis, `REL_MAX`.
const REL_MAX: u16 = 0x0f;

/// A kind of event: `EV_KEY`, `EV_ABS` and the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EventType(pub u16);

impl EventType {
    pub const SYNCHRONIZATION: EventType = EventType(0x00);
    pub const KEY: EventType = EventType(0x01);
    pub const RELATIVE: EventType = EventType(0x02);
    pub const ABSOLUTE: EventType = EventType(0x03);
}

/// A key or button code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeyCode(pub u16);

#[rustfmt::skip]
impl KeyCode {
    pub const fn new(code: u16) -> KeyCode { KeyCode(code) }
    pub const KEY_1: KeyCode = KeyCode(2);
    pub const KEY_2: KeyCode = KeyCode(3);
    pub const KEY_3: KeyCode = KeyCode(4);
    pub const KEY_4: KeyCode = KeyCode(5);
    pub const KEY_5: KeyCode = KeyCode(6);
    pub const KEY_6: KeyCode = KeyCode(7);
    pub const KEY_7: KeyCode = KeyCode(8);
    pub const KEY_8: KeyCode = KeyCode(9);
    pub const KEY_9: KeyCode = KeyCode(10);
    pub const KEY_0: KeyCode = KeyCode(11);
    pub const KEY_MINUS: KeyCode = KeyCode(12);
    pub const KEY_Q: KeyCode = KeyCode(16);
    pub const KEY_W: KeyCode = KeyCode(17);
    pub const KEY_E: KeyCode = KeyCode(18);
    pub const KEY_R: KeyCode = KeyCode(19);
    pub const KEY_T: KeyCode = KeyCode(20);
    pub const KEY_Y: KeyCode = KeyCode(21);
    pub const KEY_U: KeyCode = KeyCode(22);
    pub const KEY_I: KeyCode = KeyCode(23);
    pub const KEY_O: KeyCode = KeyCode(24);
    pub const KEY_P: KeyCode = KeyCode(25);
    pub const KEY_ENTER: KeyCode = KeyCode(28);
    pub const KEY_A: KeyCode = KeyCode(30);
    pub const KEY_S: KeyCode = KeyCode(31);
    pub const KEY_D: KeyCode = KeyCode(32);
    pub const KEY_F: KeyCode = KeyCode(33);
    pub const KEY_G: KeyCode = KeyCode(34);
    pub const KEY_H: KeyCode = KeyCode(35);
    pub const KEY_J: KeyCode = KeyCode(36);
    pub const KEY_K: KeyCode = KeyCode(37);
    pub const KEY_L: KeyCode = KeyCode(38);
    pub const KEY_SEMICOLON: KeyCode = KeyCode(39);
    pub const KEY_LEFTSHIFT: KeyCode = KeyCode(42);
    pub const KEY_Z: KeyCode = KeyCode(44);
    pub const KEY_X: KeyCode = KeyCode(45);
    pub const KEY_C: KeyCode = KeyCode(46);
    pub const KEY_V: KeyCode = KeyCode(47);
    pub const KEY_B: KeyCode = KeyCode(48);
    pub const KEY_N: KeyCode = KeyCode(49);
    pub const KEY_M: KeyCode = KeyCode(50);
    pub const KEY_COMMA: KeyCode = KeyCode(51);
    pub const KEY_DOT: KeyCode = KeyCode(52);
    pub const KEY_SLASH: KeyCode = KeyCode(53);
    pub const KEY_SPACE: KeyCode = KeyCode(57);
    pub const BTN_0: KeyCode = KeyCode(0x100);
    pub const BTN_LEFT: KeyCode = KeyCode(0x110);
    pub const BTN_RIGHT: KeyCode = KeyCode(0x111);
    pub const BTN_TOOL_PEN: KeyCode = KeyCode(0x140);
    pub const BTN_TOOL_RUBBER: KeyCode = KeyCode(0x141);
    pub const BTN_TOUCH: KeyCode = KeyCode(0x14a);
}

/// An absolute axis code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AbsoluteAxisCode(pub u16);

impl AbsoluteAxisCode {
    pub const ABS_X: AbsoluteAxisCode = AbsoluteAxisCode(0x00);
    pub const ABS_Y: AbsoluteAxisCode = AbsoluteAxisCode(0x01);
    pub const ABS_PRESSURE: AbsoluteAxisCode = AbsoluteAxisCode(0x18);
    pub const ABS_TILT_X: AbsoluteAxisCode = AbsoluteAxisCode(0x1a);
    pub const ABS_TILT_Y: AbsoluteAxisCode = AbsoluteAxisCode(0x1b);
}

/// A relative axis code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RelativeAxisCode(pub u16);

impl RelativeAxisCode {
    pub const REL_X: RelativeAxisCode = RelativeAxisCode(0x00);
    pub const REL_Y: RelativeAxisCode = RelativeAxisCode(0x01);
    pub const REL_Z: RelativeAxisCode = RelativeAxisCode(0x02);
    pub const REL_RX: RelativeAxisCode = RelativeAxisCode(0x03);
    pub const REL_RY: RelativeAxisCode = RelativeAxisCode(0x04);
    pub const REL_RZ: RelativeAxisCode = RelativeAxisCode(0x05);
}

/// A code that can be kept in an [`AttributeSet`].
pub trait Code: Copy {
    fn from_code(code: u16) -> Self;
    fn code(self) -> u16;
}

macro_rules! code {
    ($type:ty) => {
        impl Code for $type {
            fn from_code(code: u16) -> Self {
                Self(code)
            }
            fn code(self) -> u16 {
                self.0
            }
        }
    };
}
code!(KeyCode);
code!(AbsoluteAxisCode);
code!(RelativeAxisCode);

/// A set of codes, as the kernel reports and takes them: one bit a code.
#[derive(Clone, PartialEq, Eq)]
pub struct AttributeSet<T: Code> {
    bits: Vec<u8>,
    kind: PhantomData<T>,
}

impl<T: Code> std::fmt::Debug for AttributeSet<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter().map(Code::code)).finish()
    }
}

impl<T: Code> Default for AttributeSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Code> AttributeSet<T> {
    pub fn new() -> Self {
        Self { bits: Vec::new(), kind: PhantomData }
    }

    fn from_bits(bits: Vec<u8>) -> Self {
        Self { bits, kind: PhantomData }
    }

    pub fn insert(&mut self, code: T) {
        let code = code.code() as usize;
        if self.bits.len() <= code / 8 {
            self.bits.resize(code / 8 + 1, 0);
        }
        self.bits[code / 8] |= 1 << (code % 8);
    }

    pub fn contains(&self, code: T) -> bool {
        let code = code.code() as usize;
        self.bits.get(code / 8).is_some_and(|byte| byte & (1 << (code % 8)) != 0)
    }

    pub fn iter(&self) -> impl Iterator<Item = T> + '_ {
        self.bits.iter().enumerate().flat_map(|(at, byte)| {
            (0..8)
                .filter(move |bit| byte & (1 << bit) != 0)
                .map(move |bit| T::from_code((at * 8 + bit) as u16))
        })
    }

    fn is_empty(&self) -> bool {
        self.bits.iter().all(|byte| *byte == 0)
    }
}

impl<'a, T: Code> IntoIterator for &'a AttributeSet<T> {
    type Item = T;
    type IntoIter = Box<dyn Iterator<Item = T> + 'a>;
    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}

/// An absolute axis's range, as `EVIOCGABS` reports and `UI_ABS_SETUP` takes.
#[derive(Debug, Clone, Copy)]
pub struct AbsInfo(libc::input_absinfo);

impl AbsInfo {
    pub fn new(
        value: i32,
        minimum: i32,
        maximum: i32,
        fuzz: i32,
        flat: i32,
        resolution: i32,
    ) -> Self {
        AbsInfo(libc::input_absinfo { value, minimum, maximum, fuzz, flat, resolution })
    }
    pub fn value(&self) -> i32 {
        self.0.value
    }
    pub fn minimum(&self) -> i32 {
        self.0.minimum
    }
    pub fn maximum(&self) -> i32 {
        self.0.maximum
    }
}

/// One event: its type, its code and its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputEvent {
    kind: u16,
    code: u16,
    value: i32,
}

impl InputEvent {
    pub fn new(kind: u16, code: u16, value: i32) -> Self {
        InputEvent { kind, code, value }
    }
    pub fn event_type(&self) -> EventType {
        EventType(self.kind)
    }
    pub fn code(&self) -> u16 {
        self.code
    }
    pub fn value(&self) -> i32 {
        self.value
    }
    fn raw(&self) -> libc::input_event {
        libc::input_event {
            time: libc::timeval { tv_sec: 0, tv_usec: 0 },
            type_: self.kind,
            code: self.code,
            value: self.value,
        }
    }
}

/// An event device, opened to read: its name and what it has, read once
/// when it is opened, and its events.
pub struct Device {
    file: File,
    name: Option<String>,
    keys: AttributeSet<KeyCode>,
    absolute: AttributeSet<AbsoluteAxisCode>,
    relative: AttributeSet<RelativeAxisCode>,
    ranges: Vec<(AbsoluteAxisCode, AbsInfo)>,
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device").field("name", &self.name).finish_non_exhaustive()
    }
}

/// The bits of one event type the device has.
fn bits_of(file: &File, event_type: EventType, highest: u16) -> io::Result<Vec<u8>> {
    let mut bits = vec![0u8; highest as usize / 8 + 1];
    // SAFETY: `EVIOCGBIT` writes at most the length it encodes, which is
    // the buffer's.
    unsafe { ioctl(file, eviocgbit(event_type.0, bits.len()), bits.as_mut_ptr().cast())? };
    Ok(bits)
}

impl Device {
    /// Open an event node, `/dev/input/eventN`, and read what it has.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Device> {
        let file = OpenOptions::new().read(true).custom_flags(libc::O_CLOEXEC).open(path)?;
        let mut name = [0u8; 256];
        // SAFETY: `EVIOCGNAME` writes at most the length it encodes.
        let name = match unsafe { ioctl(&file, eviocgname(name.len()), name.as_mut_ptr().cast()) } {
            Ok(_) => CStr::from_bytes_until_nul(&name)
                .ok()
                .map(|name| name.to_string_lossy().into_owned()),
            Err(_) => None,
        };
        let keys: AttributeSet<KeyCode> =
            AttributeSet::from_bits(bits_of(&file, EventType::KEY, KEY_MAX)?);
        let absolute: AttributeSet<AbsoluteAxisCode> =
            AttributeSet::from_bits(bits_of(&file, EventType::ABSOLUTE, ABS_MAX)?);
        let relative: AttributeSet<RelativeAxisCode> =
            AttributeSet::from_bits(bits_of(&file, EventType::RELATIVE, REL_MAX)?);
        let mut ranges = Vec::new();
        for axis in absolute.iter() {
            let mut info = AbsInfo::new(0, 0, 0, 0, 0, 0);
            // SAFETY: `EVIOCGABS` writes one `input_absinfo`.
            if unsafe {
                ioctl(&file, eviocgabs(axis.0), (&mut info.0 as *mut libc::input_absinfo).cast())
            }
            .is_ok()
            {
                ranges.push((axis, info));
            }
        }
        Ok(Device { file, name, keys, absolute, relative, ranges })
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn supported_keys(&self) -> Option<&AttributeSet<KeyCode>> {
        (!self.keys.is_empty()).then_some(&self.keys)
    }

    pub fn supported_absolute_axes(&self) -> Option<&AttributeSet<AbsoluteAxisCode>> {
        (!self.absolute.is_empty()).then_some(&self.absolute)
    }

    pub fn supported_relative_axes(&self) -> Option<&AttributeSet<RelativeAxisCode>> {
        (!self.relative.is_empty()).then_some(&self.relative)
    }

    /// Each absolute axis and its range, as read when the device was opened.
    pub fn get_absinfo(
        &self,
    ) -> io::Result<impl Iterator<Item = (AbsoluteAxisCode, AbsInfo)> + '_> {
        Ok(self.ranges.iter().copied())
    }

    /// Wait for events, and those that have arrived.
    pub fn fetch_events(&mut self) -> io::Result<impl Iterator<Item = InputEvent> + '_> {
        const AT_ONCE: usize = 64;
        let size = std::mem::size_of::<libc::input_event>();
        let mut raw = [0u8; AT_ONCE * std::mem::size_of::<libc::input_event>()];
        let read = loop {
            // SAFETY: `read` writes at most the buffer's length into it.
            let read =
                unsafe { libc::read(self.file.as_raw_fd(), raw.as_mut_ptr().cast(), raw.len()) };
            if read >= 0 {
                break read as usize;
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        };
        if read == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the device went away"));
        }
        let events: Vec<InputEvent> = raw[..read - read % size]
            .chunks_exact(size)
            .map(|bytes| {
                // SAFETY: a whole `input_event` read from the kernel, copied
                // out unaligned.
                let event: libc::input_event =
                    unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast()) };
                InputEvent::new(event.type_, event.code, event.value)
            })
            .collect();
        Ok(events.into_iter())
    }
}

/// Where `evdev` keeps it, so the callers' imports read the same.
pub use uinput::UinputAbsSetup;

pub mod uinput {
    //! A virtual device, made through `/dev/uinput`, for tests and for
    //! `poke`.

    use super::*;

    /// A device being described, before it is made.
    pub struct VirtualDeviceBuilder {
        file: File,
        name: String,
        absolute: Vec<UinputAbsSetup>,
    }

    /// An absolute axis and its range, for a virtual device.
    #[derive(Debug, Clone, Copy)]
    pub struct UinputAbsSetup(libc::uinput_abs_setup);

    impl UinputAbsSetup {
        pub fn new(axis: AbsoluteAxisCode, info: AbsInfo) -> Self {
            UinputAbsSetup(libc::uinput_abs_setup { code: axis.0, absinfo: info.0 })
        }
    }

    fn set_bit(file: &File, request: u64, code: u16) -> io::Result<()> {
        // SAFETY: the `UI_SET_*BIT` requests take an int by value.
        unsafe { ioctl(file, request, libc::c_int::from(code) as usize as *mut libc::c_void)? };
        Ok(())
    }

    impl VirtualDeviceBuilder {
        pub fn name(mut self, name: &str) -> Self {
            self.name = name.to_string();
            self
        }

        pub fn with_keys(self, keys: &AttributeSet<KeyCode>) -> io::Result<Self> {
            set_bit(&self.file, UI_SET_EVBIT, EventType::KEY.0)?;
            for key in keys.iter() {
                set_bit(&self.file, UI_SET_KEYBIT, key.0)?;
            }
            Ok(self)
        }

        pub fn with_relative_axes(self, axes: &AttributeSet<RelativeAxisCode>) -> io::Result<Self> {
            set_bit(&self.file, UI_SET_EVBIT, EventType::RELATIVE.0)?;
            for axis in axes.iter() {
                set_bit(&self.file, UI_SET_RELBIT, axis.0)?;
            }
            Ok(self)
        }

        pub fn with_absolute_axis(mut self, axis: &UinputAbsSetup) -> io::Result<Self> {
            set_bit(&self.file, UI_SET_EVBIT, EventType::ABSOLUTE.0)?;
            set_bit(&self.file, UI_SET_ABSBIT, axis.0.code)?;
            self.absolute.push(*axis);
            Ok(self)
        }

        pub fn build(self) -> io::Result<VirtualDevice> {
            for axis in &self.absolute {
                let mut setup = axis.0;
                // SAFETY: `UI_ABS_SETUP` reads one `uinput_abs_setup`.
                unsafe {
                    ioctl(
                        &self.file,
                        UI_ABS_SETUP,
                        (&mut setup as *mut libc::uinput_abs_setup).cast(),
                    )?
                };
            }
            // SAFETY: zeroed is a valid `uinput_setup`: integers and a name.
            let mut setup: libc::uinput_setup = unsafe { std::mem::zeroed() };
            setup.id.bustype = 0x03; // BUS_USB
            setup.id.vendor = 0x1234;
            setup.id.product = 0x5678;
            for (slot, byte) in
                setup.name.iter_mut().zip(self.name.bytes().take(libc::UINPUT_MAX_NAME_SIZE - 1))
            {
                *slot = byte as libc::c_char;
            }
            // SAFETY: `UI_DEV_SETUP` reads one `uinput_setup`.
            unsafe {
                ioctl(&self.file, UI_DEV_SETUP, (&mut setup as *mut libc::uinput_setup).cast())?;
                ioctl(&self.file, UI_DEV_CREATE, std::ptr::null_mut())?;
            }
            Ok(VirtualDevice { file: self.file })
        }
    }

    /// A virtual input device, removed when dropped.
    pub struct VirtualDevice {
        file: File,
    }

    impl VirtualDevice {
        /// Start describing one; needs write access to `/dev/uinput`.
        pub fn builder() -> io::Result<VirtualDeviceBuilder> {
            let file = OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
                .open("/dev/uinput")?;
            Ok(VirtualDeviceBuilder { file, name: String::new(), absolute: Vec::new() })
        }

        /// Write events, and the report that ends them.
        pub fn emit(&mut self, events: &[InputEvent]) -> io::Result<()> {
            use std::io::Write;
            let report = InputEvent::new(EventType::SYNCHRONIZATION.0, 0, 0);
            let size = std::mem::size_of::<libc::input_event>();
            let mut bytes = Vec::with_capacity((events.len() + 1) * size);
            for event in events.iter().chain(std::iter::once(&report)) {
                let raw = event.raw();
                // SAFETY: a plain C struct viewed as its bytes.
                let raw = unsafe {
                    std::slice::from_raw_parts(
                        (&raw as *const libc::input_event).cast::<u8>(),
                        size,
                    )
                };
                bytes.extend_from_slice(raw);
            }
            self.file.write_all(&bytes)
        }

        /// The event nodes the kernel made for this device, waiting up to a
        /// second for them to appear.
        pub fn enumerate_dev_nodes_blocking(
            &mut self,
        ) -> io::Result<impl Iterator<Item = io::Result<PathBuf>>> {
            let mut name = [0u8; 64];
            // SAFETY: `UI_GET_SYSNAME` writes at most the length it encodes.
            unsafe { ioctl(&self.file, ui_get_sysname(name.len()), name.as_mut_ptr().cast())? };
            let sysname = CStr::from_bytes_until_nul(&name)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "an unnamed device"))?
                .to_string_lossy()
                .into_owned();
            let directory = Path::new("/sys/devices/virtual/input").join(sysname);
            let mut nodes = Vec::new();
            for _ in 0..100 {
                nodes = std::fs::read_dir(&directory)?
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .filter(|name| name.starts_with("event"))
                    .map(|name| Ok(Path::new("/dev/input").join(name)))
                    .collect();
                if !nodes.is_empty() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Ok(nodes.into_iter())
        }
    }

    impl Drop for VirtualDevice {
        fn drop(&mut self) {
            // SAFETY: `UI_DEV_DESTROY` takes no argument.
            let _ = unsafe { ioctl(&self.file, UI_DEV_DESTROY, std::ptr::null_mut()) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The request numbers, against the kernel headers' values for these
    /// targets (`linux/input.h`, `linux/uinput.h`).
    #[test]
    fn the_requests_are_numbered_as_the_kernel_numbers_them() {
        assert_eq!(eviocgname(256), 0x8100_4506);
        assert_eq!(eviocgbit(0x01, 96), 0x8060_4521);
        assert_eq!(eviocgabs(0x18), 0x8018_4558);
        assert_eq!(UI_DEV_CREATE, 0x5501);
        assert_eq!(UI_DEV_DESTROY, 0x5502);
        assert_eq!(UI_DEV_SETUP, 0x405c_5503);
        assert_eq!(UI_ABS_SETUP, 0x401c_5504);
        assert_eq!(UI_SET_EVBIT, 0x4004_5564);
        assert_eq!(UI_SET_KEYBIT, 0x4004_5565);
        assert_eq!(ui_get_sysname(64), 0x8040_552c);
        assert_eq!(std::mem::size_of::<libc::input_event>(), 24);
    }

    #[test]
    fn a_set_holds_what_was_put_in_it_and_nothing_else() {
        let mut keys = AttributeSet::<KeyCode>::new();
        assert!(keys.is_empty());
        keys.insert(KeyCode::BTN_TOOL_PEN);
        keys.insert(KeyCode::KEY_A);
        assert!(keys.contains(KeyCode::BTN_TOOL_PEN) && keys.contains(KeyCode::KEY_A));
        assert!(!keys.contains(KeyCode::BTN_TOOL_RUBBER));
        assert_eq!(keys.iter().collect::<Vec<_>>(), vec![KeyCode::KEY_A, KeyCode::BTN_TOOL_PEN]);
        let reported = AttributeSet::<KeyCode>::from_bits(vec![0, 0b0000_0100]);
        assert_eq!(reported.iter().collect::<Vec<_>>(), vec![KeyCode(10)]);
    }

    /// A real device, if this machine lets one be made: a pen through
    /// uinput, found again by its node, its name, keys and pressure range
    /// read back through `Device::open`, and an event read from it.
    #[test]
    fn a_virtual_pen_is_read_back_as_it_was_described() {
        use uinput::{UinputAbsSetup, VirtualDevice};
        let mut keys = AttributeSet::<KeyCode>::new();
        keys.insert(KeyCode::BTN_TOOL_PEN);
        let made = VirtualDevice::builder().and_then(|builder| {
            builder
                .name("hani-evdev test pen")
                .with_keys(&keys)?
                .with_absolute_axis(&UinputAbsSetup::new(
                    AbsoluteAxisCode::ABS_PRESSURE,
                    AbsInfo::new(0, 0, 4095, 0, 0, 0),
                ))?
                .build()
        });
        let Ok(mut pen) = made else {
            eprintln!("skipping: cannot make a uinput device here");
            return;
        };
        let Some(Ok(node)) = pen.enumerate_dev_nodes_blocking().unwrap().next() else {
            eprintln!("skipping: the device has no node here");
            return;
        };
        let Ok(mut device) = Device::open(&node) else {
            eprintln!("skipping: cannot read {node:?}");
            return;
        };
        assert_eq!(device.name(), Some("hani-evdev test pen"));
        assert!(device.supported_keys().is_some_and(|keys| keys.contains(KeyCode::BTN_TOOL_PEN)));
        let range = device
            .get_absinfo()
            .unwrap()
            .find(|(axis, _)| *axis == AbsoluteAxisCode::ABS_PRESSURE)
            .map(|(_, info)| (info.minimum(), info.maximum()));
        assert_eq!(range, Some((0, 4095)));
        pen.emit(&[InputEvent::new(EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_PRESSURE.0, 2000)])
            .unwrap();
        let events: Vec<InputEvent> = device.fetch_events().unwrap().collect();
        assert!(
            events.iter().any(|event| event.event_type() == EventType::ABSOLUTE
                && event.code() == AbsoluteAxisCode::ABS_PRESSURE.0
                && event.value() == 2000),
            "{events:?}"
        );
    }
}
