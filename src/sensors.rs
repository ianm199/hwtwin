//! The OS-specific sensor seam.
//!
//! `SensorSource` is the trait every backend implements: enumerate the machine's
//! raw sensor keys, and read a set of them. Everything above this layer (the
//! normalized [`crate::model::Snapshot`], the twin UI, the analysis) is written
//! against the trait and never mentions IOKit — so adding Linux or Windows is a
//! matter of writing one more implementation, not touching the core.
//!
//! The only implementation today is [`AppleSmc`], a dependency-free client of the
//! `AppleSMC` IOKit driver on Apple Silicon and Intel Macs.

use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::{c_char, c_void};

/// A machine's raw sensors, independent of how they are physically read.
pub trait SensorSource {
    /// Every readable numeric key paired with its decode-type tag.
    fn schema(&self) -> Vec<(String, String)>;
    /// Current values for the requested keys; absent/unreadable keys are omitted.
    fn read(&self, keys: &[&str]) -> HashMap<String, f32>;
}

type KernReturn = i32;
type MachPort = u32;
type IoService = MachPort;
type IoConnect = MachPort;

const KERNEL_INDEX_SMC: u32 = 2;
const SMC_CMD_READ_BYTES: u8 = 5;
const SMC_CMD_READ_INDEX: u8 = 8;
const SMC_CMD_READ_KEYINFO: u8 = 9;

#[link(name = "IOKit", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    fn IOServiceGetMatchingService(main_port: MachPort, matching: *mut c_void) -> IoService;
    fn IOServiceOpen(
        service: IoService,
        owning_task: MachPort,
        type_: u32,
        connect: *mut IoConnect,
    ) -> KernReturn;
    fn IOServiceClose(connect: IoConnect) -> KernReturn;
    fn IOObjectRelease(object: IoService) -> KernReturn;
    fn IOConnectCallStructMethod(
        connection: IoConnect,
        selector: u32,
        input: *const c_void,
        input_cnt: usize,
        output: *mut c_void,
        output_cnt: *mut usize,
    ) -> KernReturn;
}

unsafe extern "C" {
    static mach_task_self_: MachPort;
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SmcVersion {
    major: u8,
    minor: u8,
    build: u8,
    reserved: u8,
    release: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SmcPLimitData {
    version: u16,
    length: u16,
    cpu_plimit: u32,
    gpu_plimit: u32,
    mem_plimit: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SmcKeyInfo {
    data_size: u32,
    data_type: u32,
    data_attributes: u8,
}

/// The exact 80-byte request/response packet the SMC driver expects.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SmcKeyData {
    key: u32,
    vers: SmcVersion,
    plimit: SmcPLimitData,
    key_info: SmcKeyInfo,
    result: u8,
    status: u8,
    data8: u8,
    data32: u32,
    bytes: [u8; 32],
}

const _: () = assert!(std::mem::size_of::<SmcKeyData>() == 80);

/// A low-level open connection to the `AppleSMC` user client.
struct SmcConnection {
    conn: IoConnect,
}

impl SmcConnection {
    fn open() -> Result<Self, KernReturn> {
        let name = CString::new("AppleSMC").unwrap();
        let conn = unsafe {
            let matching = IOServiceMatching(name.as_ptr());
            if matching.is_null() {
                return Err(-1);
            }
            let service = IOServiceGetMatchingService(0, matching);
            if service == 0 {
                return Err(-2);
            }
            let mut conn: IoConnect = 0;
            let kr = IOServiceOpen(service, mach_task_self_, 0, &mut conn);
            IOObjectRelease(service);
            if kr != 0 {
                return Err(kr);
            }
            conn
        };
        Ok(Self { conn })
    }

    fn call(&self, input: &SmcKeyData) -> Result<SmcKeyData, KernReturn> {
        let mut output = SmcKeyData::default();
        let mut out_size = std::mem::size_of::<SmcKeyData>();
        let kr = unsafe {
            IOConnectCallStructMethod(
                self.conn,
                KERNEL_INDEX_SMC,
                input as *const _ as *const c_void,
                std::mem::size_of::<SmcKeyData>(),
                &mut output as *mut _ as *mut c_void,
                &mut out_size,
            )
        };
        if kr != 0 {
            return Err(kr);
        }
        if output.result != 0 {
            return Err(output.result as KernReturn);
        }
        Ok(output)
    }

    fn key_count(&self) -> Result<u32, KernReturn> {
        let (info, bytes) = self.read_key(fourcc("#KEY"))?;
        Ok(decode_be_uint(&bytes, info.data_size as usize) as u32)
    }

    fn key_at_index(&self, index: u32) -> Result<u32, KernReturn> {
        let input = SmcKeyData {
            data8: SMC_CMD_READ_INDEX,
            data32: index,
            ..Default::default()
        };
        Ok(self.call(&input)?.key)
    }

    fn read_key(&self, key: u32) -> Result<(SmcKeyInfo, [u8; 32]), KernReturn> {
        let info_req = SmcKeyData {
            key,
            data8: SMC_CMD_READ_KEYINFO,
            ..Default::default()
        };
        let info = self.call(&info_req)?.key_info;
        let mut value_req = SmcKeyData {
            key,
            data8: SMC_CMD_READ_BYTES,
            ..Default::default()
        };
        value_req.key_info.data_size = info.data_size;
        Ok((info, self.call(&value_req)?.bytes))
    }

    fn read_numeric(&self, key: &str) -> Option<f32> {
        let (info, bytes) = self.read_key(fourcc(key)).ok()?;
        decode_f32(&info, &bytes)
    }
}

impl Drop for SmcConnection {
    fn drop(&mut self) {
        unsafe {
            IOServiceClose(self.conn);
        }
    }
}

/// The Apple SMC sensor backend.
pub struct AppleSmc {
    conn: SmcConnection,
}

impl AppleSmc {
    /// Opens a connection to the SMC, or returns the IOKit error code.
    pub fn open() -> Result<Self, i32> {
        Ok(Self {
            conn: SmcConnection::open()?,
        })
    }
}

impl SensorSource for AppleSmc {
    fn schema(&self) -> Vec<(String, String)> {
        let count = match self.conn.key_count() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let mut out = Vec::new();
        for index in 0..count {
            let Ok(code) = self.conn.key_at_index(index) else {
                continue;
            };
            let Ok((info, bytes)) = self.conn.read_key(code) else {
                continue;
            };
            if decode_f32(&info, &bytes).is_none() {
                continue;
            }
            let key = fourcc_to_string(code);
            if !key.bytes().all(|b| b.is_ascii_graphic()) {
                continue;
            }
            out.push((key, fourcc_to_string(info.data_type).trim_end().to_string()));
        }
        out
    }

    fn read(&self, keys: &[&str]) -> HashMap<String, f32> {
        let mut out = HashMap::with_capacity(keys.len());
        for k in keys {
            if let Some(v) = self.conn.read_numeric(k) {
                if v.is_finite() {
                    out.insert((*k).to_string(), v);
                }
            }
        }
        out
    }
}

/// Packs a four-character key into the big-endian `u32` the SMC expects.
fn fourcc(s: &str) -> u32 {
    let b = s.as_bytes();
    ((b[0] as u32) << 24) | ((b[1] as u32) << 16) | ((b[2] as u32) << 8) | (b[3] as u32)
}

/// Unpacks a big-endian FourCC `u32` into its four printable characters.
fn fourcc_to_string(code: u32) -> String {
    code.to_be_bytes().iter().map(|&b| b as char).collect()
}

/// Decodes a big-endian unsigned integer of the given byte width.
fn decode_be_uint(bytes: &[u8], size: usize) -> u64 {
    bytes[..size].iter().fold(0u64, |acc, &b| (acc << 8) | b as u64)
}

/// Decodes a numeric SMC value to `f32`, honoring per-type endianness:
/// `flt` is little-endian IEEE-754, integers and fixed-point are big-endian.
fn decode_f32(info: &SmcKeyInfo, bytes: &[u8]) -> Option<f32> {
    let size = info.data_size as usize;
    if size == 0 {
        return None;
    }
    match fourcc_to_string(info.data_type).trim_end() {
        "flt" => Some(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        "ui8" | "ui16" | "ui32" => Some(decode_be_uint(bytes, size) as f32),
        "sp78" => Some(i16::from_be_bytes([bytes[0], bytes[1]]) as f32 / 256.0),
        "fpe2" => Some(u16::from_be_bytes([bytes[0], bytes[1]]) as f32 / 4.0),
        _ => None,
    }
}
