//! Native process-memory boundary for the desktop SplitScript debugger.
//!
//! The process behavior is intentionally kept close to the implementation in
//! `livesplit-auto-splitting`. The public boundary uses opaque handles so no OS
//! handles or pointers are ever exposed to a webview or WebAssembly guest.

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard, OnceLock},
};

use napi::{Error, Result, Status, bindgen_prelude::Buffer};
use napi_derive::napi;

#[cfg(windows)]
mod platform {
    use std::{ffi::c_void, io, ptr};

    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::{
            Diagnostics::Debug::ReadProcessMemory,
            Threading::{
                OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
                QueryFullProcessImageNameW,
            },
        },
    };

    pub struct Process {
        handle: HANDLE,
        pid: u32,
    }

    // A process HANDLE can be used from any thread. Access to Process instances
    // is additionally serialized by the outer handle table.
    unsafe impl Send for Process {}

    impl Process {
        pub fn attach(pid: u32) -> io::Result<Self> {
            let handle =
                unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, 0, pid) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            Ok(Self { handle, pid })
        }

        pub fn pid(&self) -> u32 {
            self.pid
        }

        pub fn read(&self, address: u64, length: usize) -> io::Result<Vec<u8>> {
            let address = usize::try_from(address).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "address does not fit this host",
                )
            })?;
            let mut bytes = vec![0; length];
            let mut bytes_read = 0;
            let succeeded = unsafe {
                ReadProcessMemory(
                    self.handle,
                    address as *const c_void,
                    bytes.as_mut_ptr().cast(),
                    length,
                    &mut bytes_read,
                )
            };
            if succeeded == 0 {
                return Err(io::Error::last_os_error());
            }
            if bytes_read != length {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("read {bytes_read} of {length} requested bytes"),
                ));
            }
            Ok(bytes)
        }

        pub fn path(&self) -> io::Result<String> {
            // Windows documents 32,767 UTF-16 code units as the maximum
            // extended-length path. The API updates `length` to the used size.
            let mut buffer = vec![0_u16; 32_768];
            let mut length = u32::try_from(buffer.len()).unwrap();
            let succeeded = unsafe {
                QueryFullProcessImageNameW(self.handle, 0, buffer.as_mut_ptr(), &mut length)
            };
            if succeeded == 0 {
                return Err(io::Error::last_os_error());
            }
            buffer.truncate(length as usize);
            String::from_utf16(&buffer)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
        }
    }

    impl Drop for Process {
        fn drop(&mut self) {
            if !self.handle.is_null() {
                unsafe {
                    CloseHandle(self.handle);
                }
                self.handle = ptr::null_mut();
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use std::io;

    pub struct Process;

    impl Process {
        pub fn attach(_pid: u32) -> io::Result<Self> {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "the process debugger prototype currently supports Windows only",
            ))
        }

        pub fn pid(&self) -> u32 {
            0
        }

        pub fn read(&self, _address: u64, _length: usize) -> io::Result<Vec<u8>> {
            unreachable!("unsupported hosts cannot create Process values")
        }

        pub fn path(&self) -> io::Result<String> {
            unreachable!("unsupported hosts cannot create Process values")
        }
    }
}

struct ProcessTable {
    next_handle: u32,
    processes: HashMap<u32, platform::Process>,
}

impl Default for ProcessTable {
    fn default() -> Self {
        Self {
            next_handle: 1,
            processes: HashMap::new(),
        }
    }
}

impl ProcessTable {
    fn insert(&mut self, process: platform::Process) -> Result<u32> {
        for _ in 0..u32::MAX {
            let handle = self.next_handle;
            self.next_handle = self.next_handle.wrapping_add(1).max(1);
            if let std::collections::hash_map::Entry::Vacant(entry) = self.processes.entry(handle) {
                entry.insert(process);
                return Ok(handle);
            }
        }
        Err(Error::new(
            Status::GenericFailure,
            "native process handle table is exhausted",
        ))
    }
}

fn table() -> Result<MutexGuard<'static, ProcessTable>> {
    static TABLE: OnceLock<Mutex<ProcessTable>> = OnceLock::new();
    TABLE
        .get_or_init(|| Mutex::new(ProcessTable::default()))
        .lock()
        .map_err(|_| {
            Error::new(
                Status::GenericFailure,
                "native process handle table is poisoned",
            )
        })
}

fn native_error(context: &str, error: std::io::Error) -> Error {
    Error::new(Status::GenericFailure, format!("{context}: {error}"))
}

#[napi(js_name = "attachByPid")]
pub fn attach_by_pid(pid: u32) -> Result<u32> {
    let process = platform::Process::attach(pid)
        .map_err(|error| native_error(&format!("could not attach to process {pid}"), error))?;
    table()?.insert(process)
}

#[napi(js_name = "detach")]
pub fn detach(handle: u32) -> Result<bool> {
    Ok(table()?.processes.remove(&handle).is_some())
}

#[napi(js_name = "processId")]
pub fn process_id(handle: u32) -> Result<u32> {
    table()?
        .processes
        .get(&handle)
        .map(platform::Process::pid)
        .ok_or_else(|| {
            Error::new(
                Status::InvalidArg,
                format!("unknown process handle {handle}"),
            )
        })
}

#[napi(js_name = "processPath")]
pub fn process_path(handle: u32) -> Result<String> {
    let table = table()?;
    let process = table.processes.get(&handle).ok_or_else(|| {
        Error::new(
            Status::InvalidArg,
            format!("unknown process handle {handle}"),
        )
    })?;
    process
        .path()
        .map_err(|error| native_error("could not query process path", error))
}

/// Reads from an attached process. The address is decimal text so the N-API
/// surface cannot silently truncate a 64-bit address through a JavaScript
/// `number`; the TypeScript ASR host will pass its WebAssembly `BigInt` as text.
#[napi(js_name = "readProcessMemory")]
pub fn read_process_memory(handle: u32, address: String, length: u32) -> Result<Buffer> {
    let address = address.parse::<u64>().map_err(|_| {
        Error::new(
            Status::InvalidArg,
            format!("invalid unsigned 64-bit address `{address}`"),
        )
    })?;
    let length = usize::try_from(length)
        .map_err(|_| Error::new(Status::InvalidArg, "read length does not fit this host"))?;
    let table = table()?;
    let process = table.processes.get(&handle).ok_or_else(|| {
        Error::new(
            Status::InvalidArg,
            format!("unknown process handle {handle}"),
        )
    })?;
    process
        .read(address, length)
        .map(Buffer::from)
        .map_err(|error| native_error("could not read process memory", error))
}
