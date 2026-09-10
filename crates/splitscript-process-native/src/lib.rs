//! Native process boundary for the desktop SplitScript debugger.
//!
//! The process behavior mirrors `livesplit-auto-splitting`: process discovery
//! is cached, modules and mapped ranges are refreshed at most once per second,
//! and OS/process handles never cross into WebAssembly.

#![allow(clippy::unnecessary_cast)]

use std::{
    collections::HashMap,
    io,
    sync::{Mutex, MutexGuard, OnceLock},
    time::{Duration, Instant},
};

use napi::{Error, Result, Status, bindgen_prelude::Buffer};
use napi_derive::napi;
use proc_maps::{MapRange, Pid};
use read_process_memory::{CopyAddress, ProcessHandle};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System, UpdateKind};

// Adapted from livesplit-core commit 46126e76 (Fix Wine Module Size Reporting).
#[cfg(any(target_os = "linux", test))]
mod process_linux;

#[cfg(target_os = "linux")]
fn platform_module_size(process: &Process, address: u64, mapped_size: u64) -> u64 {
    process_linux::module_size(process, address, mapped_size)
}

#[cfg(not(target_os = "linux"))]
const fn platform_module_size(_: &Process, _: u64, mapped_size: u64) -> u64 {
    mapped_size
}

struct Process {
    handle: ProcessHandle,
    pid: Pid,
    path: Option<Box<str>>,
    memory_ranges: Vec<MapRange>,
    next_memory_range_check: Instant,
    next_open_check: Instant,
}

impl Process {
    fn attach(pid: u32, path: Option<Box<str>>) -> io::Result<Self> {
        let native_pid = pid as Pid;
        let handle = native_pid.try_into().map_err(process_attach_error)?;
        let now = Instant::now();
        Ok(Self {
            handle,
            pid: native_pid,
            path,
            memory_ranges: Vec::new(),
            next_memory_range_check: now,
            next_open_check: now + Duration::from_secs(1),
        })
    }

    fn pid(&self) -> u32 {
        self.pid as u32
    }

    fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    fn read(&self, address: u64, length: usize) -> io::Result<Vec<u8>> {
        let address = usize::try_from(address).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "address does not fit this host",
            )
        })?;
        let mut bytes = vec![0; length];
        self.handle.copy_address(address, &mut bytes)?;
        Ok(bytes)
    }

    fn is_open(&mut self, process_list: &mut ProcessList) -> bool {
        let now = Instant::now();
        if now >= self.next_open_check {
            process_list.refresh_pid(self.pid());
            self.next_open_check = now + Duration::from_secs(1);
        }
        process_list.is_open(self.pid())
    }

    fn module_address(&mut self, module: &str) -> io::Result<Option<u64>> {
        self.refresh_memory_ranges()?;
        Ok(self
            .memory_ranges
            .iter()
            .find(|range| range.filename().is_some_and(|path| path.ends_with(module)))
            .map(|range| range.start() as u64))
    }

    fn module_size(&mut self, module: &str) -> io::Result<Option<u64>> {
        self.refresh_memory_ranges()?;
        let mut ranges = self
            .memory_ranges
            .iter()
            .filter(|range| range.filename().is_some_and(|path| path.ends_with(module)));
        let Some(first_range) = ranges.next() else {
            return Ok(None);
        };
        let address = first_range.start() as u64;
        let mapped_size =
            first_range.size() as u64 + ranges.map(|range| range.size() as u64).sum::<u64>();
        Ok(Some(platform_module_size(self, address, mapped_size)))
    }

    fn module_path(&mut self, module: &str) -> io::Result<Option<String>> {
        self.refresh_memory_ranges()?;
        Ok(self
            .memory_ranges
            .iter()
            .find_map(|range| range.filename().filter(|path| path.ends_with(module)))
            .map(|path| path.to_string_lossy().into_owned()))
    }

    fn memory_range_count(&mut self) -> io::Result<usize> {
        self.refresh_memory_ranges()?;
        Ok(self.memory_ranges.len())
    }

    fn memory_range(&mut self, index: usize) -> io::Result<Option<(u64, u64, u64)>> {
        self.refresh_memory_ranges()?;
        Ok(self.memory_ranges.get(index).map(|range| {
            let mut flags = 1;
            if range.is_read() {
                flags |= 1 << 1;
            }
            if range.is_write() {
                flags |= 1 << 2;
            }
            if range.is_exec() {
                flags |= 1 << 3;
            }
            if range.filename().is_some() {
                flags |= 1 << 4;
            }
            (range.start() as u64, range.size() as u64, flags)
        }))
    }

    fn refresh_memory_ranges(&mut self) -> io::Result<()> {
        let now = Instant::now();
        if now >= self.next_memory_range_check {
            self.memory_ranges = proc_maps::get_process_maps(self.pid)?;
            self.next_memory_range_check = now + Duration::from_secs(1);
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn process_attach_error(_: io::Error) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "macOS task_for_pid denied access; process memory attachment requires debugger authorization and a target that permits inspection",
    )
}

#[cfg(not(target_os = "macos"))]
const fn process_attach_error(error: io::Error) -> io::Error {
    error
}

struct ProcessList {
    system: System,
    next_check: Instant,
}

impl ProcessList {
    fn new() -> Self {
        Self {
            system: System::new_with_specifics(
                RefreshKind::nothing().with_processes(multiple_processes()),
            ),
            next_check: Instant::now() + Duration::from_secs(1),
        }
    }

    fn refresh(&mut self) {
        let now = Instant::now();
        if now >= self.next_check {
            self.system.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                multiple_processes(),
            );
            self.next_check = now + Duration::from_secs(1);
        }
    }

    fn refresh_pid(&mut self, pid: u32) {
        let pid = sysinfo::Pid::from_u32(pid);
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            single_process(),
        );
    }

    fn info(&self, pid: u32) -> Option<(u64, Option<Box<str>>)> {
        self.system
            .process(sysinfo::Pid::from_u32(pid))
            .map(|process| {
                (
                    process.start_time(),
                    process
                        .exe()
                        .map(|path| path.to_string_lossy().into_owned().into_boxed_str()),
                )
            })
    }

    fn pids_by_name(&self, name: &str) -> Vec<u32> {
        let expected = name.as_bytes();
        #[cfg(target_os = "linux")]
        let expected = &expected[..expected.len().min(15)];
        self.system
            .processes()
            .values()
            .filter(|process| process.name().as_encoded_bytes() == expected)
            .map(|process| process.pid().as_u32())
            .collect()
    }

    fn is_open(&self, pid: u32) -> bool {
        self.system.process(sysinfo::Pid::from_u32(pid)).is_some()
    }
}

struct ProcessTable {
    next_handle: u32,
    processes: HashMap<u32, Process>,
    list: ProcessList,
}

impl Default for ProcessTable {
    fn default() -> Self {
        Self {
            next_handle: 1,
            processes: HashMap::new(),
            list: ProcessList::new(),
        }
    }
}

impl ProcessTable {
    fn insert(&mut self, process: Process) -> Result<u32> {
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

    fn process(&self, handle: u32) -> Result<&Process> {
        self.processes
            .get(&handle)
            .ok_or_else(|| unknown_handle(handle))
    }

    fn process_mut(&mut self, handle: u32) -> Result<&mut Process> {
        self.processes
            .get_mut(&handle)
            .ok_or_else(|| unknown_handle(handle))
    }
}

fn multiple_processes() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet)
}

fn single_process() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
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

fn unknown_handle(handle: u32) -> Error {
    Error::new(
        Status::InvalidArg,
        format!("unknown process handle {handle}"),
    )
}

fn native_error(context: &str, error: io::Error) -> Error {
    Error::new(Status::GenericFailure, format!("{context}: {error}"))
}

#[napi(js_name = "listProcessesByName")]
pub fn list_processes_by_name(name: String) -> Result<Vec<u32>> {
    let mut table = table()?;
    table.list.refresh();
    Ok(table.list.pids_by_name(&name))
}

#[napi(js_name = "attachByName")]
pub fn attach_by_name(name: String) -> Result<u32> {
    let mut table = table()?;
    table.list.refresh();
    let selected = table
        .list
        .pids_by_name(&name)
        .into_iter()
        .filter_map(|pid| table.list.info(pid).map(|(start, path)| (start, pid, path)))
        .max_by_key(|(start, pid, _)| (*start, *pid))
        .ok_or_else(|| {
            Error::new(
                Status::GenericFailure,
                format!("process `{name}` not found"),
            )
        })?;
    let process = Process::attach(selected.1, selected.2)
        .map_err(|error| native_error("could not attach to process", error))?;
    table.insert(process)
}

#[napi(js_name = "attachByPid")]
pub fn attach_by_pid(pid: u32) -> Result<u32> {
    let mut table = table()?;
    table.list.refresh_pid(pid);
    let (_, path) = table
        .list
        .info(pid)
        .ok_or_else(|| Error::new(Status::GenericFailure, format!("process {pid} not found")))?;
    let process = Process::attach(pid, path)
        .map_err(|error| native_error(&format!("could not attach to process {pid}"), error))?;
    table.insert(process)
}

#[napi(js_name = "detach")]
pub fn detach(handle: u32) -> Result<bool> {
    Ok(table()?.processes.remove(&handle).is_some())
}

#[napi(js_name = "processId")]
pub fn process_id(handle: u32) -> Result<u32> {
    Ok(table()?.process(handle)?.pid())
}

#[napi(js_name = "processPath")]
pub fn process_path(handle: u32) -> Result<Option<String>> {
    Ok(table()?.process(handle)?.path().map(str::to_owned))
}

#[napi(js_name = "isOpen")]
pub fn is_open(handle: u32) -> Result<bool> {
    let mut table = table()?;
    let ProcessTable {
        processes, list, ..
    } = &mut *table;
    let process = processes
        .get_mut(&handle)
        .ok_or_else(|| unknown_handle(handle))?;
    Ok(process.is_open(list))
}

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
    table()?
        .process(handle)?
        .read(address, length)
        .map(Buffer::from)
        .map_err(|error| native_error("could not read process memory", error))
}

#[napi(js_name = "moduleAddress")]
pub fn module_address(handle: u32, module: String) -> Result<Option<String>> {
    table()?
        .process_mut(handle)?
        .module_address(&module)
        .map(|value| value.map(|value| value.to_string()))
        .map_err(|error| native_error("could not list process modules", error))
}

#[napi(js_name = "moduleSize")]
pub fn module_size(handle: u32, module: String) -> Result<Option<String>> {
    table()?
        .process_mut(handle)?
        .module_size(&module)
        .map(|value| value.map(|value| value.to_string()))
        .map_err(|error| native_error("could not list process modules", error))
}

#[napi(js_name = "modulePath")]
pub fn module_path(handle: u32, module: String) -> Result<Option<String>> {
    table()?
        .process_mut(handle)?
        .module_path(&module)
        .map_err(|error| native_error("could not list process modules", error))
}

#[napi(js_name = "memoryRangeCount")]
pub fn memory_range_count(handle: u32) -> Result<u32> {
    let count = table()?
        .process_mut(handle)?
        .memory_range_count()
        .map_err(|error| native_error("could not list process memory ranges", error))?;
    Ok(u32::try_from(count).unwrap_or(u32::MAX))
}

fn memory_range(handle: u32, index: u32) -> Result<Option<(u64, u64, u64)>> {
    table()?
        .process_mut(handle)?
        .memory_range(index as usize)
        .map_err(|error| native_error("could not list process memory ranges", error))
}

#[napi(js_name = "memoryRangeAddress")]
pub fn memory_range_address(handle: u32, index: u32) -> Result<Option<String>> {
    Ok(memory_range(handle, index)?.map(|range| range.0.to_string()))
}

#[napi(js_name = "memoryRangeSize")]
pub fn memory_range_size(handle: u32, index: u32) -> Result<Option<String>> {
    Ok(memory_range(handle, index)?.map(|range| range.1.to_string()))
}

#[napi(js_name = "memoryRangeFlags")]
pub fn memory_range_flags(handle: u32, index: u32) -> Result<Option<String>> {
    Ok(memory_range(handle, index)?.map(|range| range.2.to_string()))
}
