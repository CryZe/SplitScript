//! Linear-memory regions shared by generated runtime helpers and static data.
//!
//! Scratch roles are packed into two alias banks according to their execution
//! lifetimes. Immutable strings and parsed signatures start on the next Wasm
//! page after those banks, so neither unusually large readable records nor
//! long signatures can silently overlap static data.

pub(super) const WASM_PAGE_SIZE: u64 = 65_536;
use crate::intrinsic_registry::MAX_NATIVE_STRING_BYTES;

const SETTINGS_STRING_CAPACITY: u32 = 16_384;
const C_STRING_CAPACITY: u32 = 8_192;
const MANAGED_UTF16_CAPACITY: u32 = 4_096;
const MANAGED_UTF8_CAPACITY: u32 = 4_096;
const SIGNATURE_SCAN_WINDOW: u32 = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ScratchRequirements {
    pub abi_read_capacity: u32,
    pub maximum_signature_len: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScratchAliasClass {
    /// Mutually exclusive synchronous/phase-local input and output buffers.
    Primary,
    /// Data that must coexist with a primary buffer during one operation.
    Companion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ScratchRegion {
    alias_class: ScratchAliasClass,
    start: i32,
    capacity: i32,
}

impl ScratchRegion {
    const fn new(alias_class: ScratchAliasClass, start: i32, capacity: i32) -> Self {
        Self {
            alias_class,
            start,
            capacity,
        }
    }

    pub(super) const fn start(self) -> i32 {
        self.start
    }

    pub(super) const fn capacity(self) -> i32 {
        self.capacity
    }

    pub(super) const fn alias_class(self) -> ScratchAliasClass {
        self.alias_class
    }

    pub(super) const fn at(self, offset: i32) -> i32 {
        assert!(offset >= 0 && offset <= self.capacity);
        self.start + offset
    }

    /// Returns the host-write destination after proving its maximum size fits
    /// this named scratch role. A runtime length may be smaller, but cannot be
    /// allowed to exceed `maximum_size` before the host call.
    pub(super) fn destination(self, maximum_size: u32) -> i32 {
        assert!(
            u32::try_from(self.capacity).is_ok_and(|capacity| maximum_size <= capacity),
            "a {maximum_size}-byte host write exceeds the {}-byte scratch region",
            self.capacity
        );
        self.start
    }

    const fn end(self) -> i32 {
        self.start + self.capacity
    }
}

/// Shared output storage for synchronous `process_read` ABI calls.
///
/// Generated Wasm is single-threaded, and every consumer materializes the
/// loaded scalar/record or helper local before another read can occur. Nested
/// helpers may therefore reuse this region, but no emitter may retain a view
/// into it across a call. Keeping this role distinct from general scratch
/// regions makes that aliasing contract explicit at every call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AbiReadScratch(ScratchRegion);

impl AbiReadScratch {
    const fn new(alias_class: ScratchAliasClass, start: i32, capacity: i32) -> Self {
        Self(ScratchRegion::new(alias_class, start, capacity))
    }

    pub(super) const fn start(self) -> i32 {
        self.0.start()
    }

    pub(super) const fn capacity(self) -> i32 {
        self.0.capacity()
    }

    /// Returns the ABI destination after proving the complete read fits.
    pub(super) fn destination(self, size: u32) -> i32 {
        assert!(
            u32::try_from(self.capacity()).is_ok_and(|capacity| size <= capacity),
            "a {size}-byte process read exceeds the {}-byte ABI read region",
            self.capacity()
        );
        self.start()
    }

    pub(super) fn at(self, offset: u32) -> i32 {
        self.0
            .at(i32::try_from(offset).expect("ABI read offset must fit a wasm32 signed immediate"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RuntimeScratch {
    pub abi_read: AbiReadScratch,
    pub settings_length: ScratchRegion,
    pub settings_string: ScratchRegion,
    pub scan: ScratchRegion,
    pub c_string: ScratchRegion,
    pub native_utf8: ScratchRegion,
    pub managed_utf16: ScratchRegion,
    pub managed_utf8: ScratchRegion,
    /// Unbounded host-call staging starts after all immutable data. Helpers
    /// grow memory before writing, so long log messages cannot overwrite data.
    pub host_strings_start: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LinearMemoryLayout {
    scratch: RuntimeScratch,
    static_data_start: u32,
    static_data_end: u64,
    minimum_pages: u64,
}

impl LinearMemoryLayout {
    pub(super) fn plan(static_data_len: usize, requirements: ScratchRequirements) -> Self {
        let static_data_len = u64::try_from(static_data_len)
            .expect("static data length must fit WebAssembly linear memory");

        // Bank 0 roles never need to coexist: settings refresh, signature
        // scanning, C-string comparison, managed UTF-16 input, and ordinary
        // ABI reads execute synchronously in distinct phases. Bank 1 holds the
        // data that must coexist with bank 0: settings length and managed UTF-8
        // output respectively.
        let scan_capacity = SIGNATURE_SCAN_WINDOW
            .checked_add(requirements.maximum_signature_len.saturating_sub(1))
            .expect("signature scan scratch must fit wasm32");
        let bank_0_capacity = requirements
            .abi_read_capacity
            .max(SETTINGS_STRING_CAPACITY)
            .max(scan_capacity)
            .max(C_STRING_CAPACITY)
            .max(MAX_NATIVE_STRING_BYTES)
            .max(MANAGED_UTF16_CAPACITY);
        let bank_1_start = align_up(u64::from(bank_0_capacity), 8);
        let bank_1_capacity = MANAGED_UTF8_CAPACITY.max(4);
        let scratch_end = bank_1_start
            .checked_add(u64::from(bank_1_capacity))
            .expect("runtime scratch must fit wasm32");
        let static_data_start = align_up(scratch_end.max(1), WASM_PAGE_SIZE);
        assert!(
            static_data_start < 1u64 << 32,
            "runtime scratch exceeds the wasm32 address space"
        );
        let static_data_start = static_data_start as u32;
        let static_data_end = u64::from(static_data_start)
            .checked_add(static_data_len)
            .expect("static data must fit WebAssembly linear memory");
        let minimum_pages = static_data_end.max(WASM_PAGE_SIZE).div_ceil(WASM_PAGE_SIZE);
        assert!(
            minimum_pages <= 65_536,
            "generated static data exceeds the wasm32 address space"
        );
        let host_strings_address = minimum_pages
            .checked_mul(WASM_PAGE_SIZE)
            .expect("host string staging must fit wasm32");
        assert!(
            host_strings_address < 1u64 << 32,
            "host string staging exceeds the wasm32 address space"
        );
        let host_strings_start = host_strings_address as u32 as i32;
        let bank_0_capacity = i32::try_from(bank_0_capacity)
            .expect("one runtime scratch bank must fit wasm32 signed arithmetic");
        let bank_1_start = i32::try_from(bank_1_start)
            .expect("runtime scratch addresses must fit wasm32 signed arithmetic");
        let scratch = RuntimeScratch {
            abi_read: AbiReadScratch::new(
                ScratchAliasClass::Primary,
                0,
                i32::try_from(requirements.abi_read_capacity)
                    .expect("ABI read scratch must fit wasm32 signed arithmetic"),
            ),
            settings_length: ScratchRegion::new(ScratchAliasClass::Companion, bank_1_start, 4),
            settings_string: ScratchRegion::new(
                ScratchAliasClass::Primary,
                0,
                SETTINGS_STRING_CAPACITY as i32,
            ),
            scan: ScratchRegion::new(
                ScratchAliasClass::Primary,
                0,
                i32::try_from(scan_capacity)
                    .expect("signature scan scratch must fit wasm32 signed arithmetic"),
            ),
            c_string: ScratchRegion::new(ScratchAliasClass::Primary, 0, C_STRING_CAPACITY as i32),
            native_utf8: ScratchRegion::new(
                ScratchAliasClass::Primary,
                0,
                MAX_NATIVE_STRING_BYTES as i32,
            ),
            managed_utf16: ScratchRegion::new(
                ScratchAliasClass::Primary,
                0,
                MANAGED_UTF16_CAPACITY as i32,
            ),
            managed_utf8: ScratchRegion::new(
                ScratchAliasClass::Companion,
                bank_1_start,
                MANAGED_UTF8_CAPACITY as i32,
            ),
            host_strings_start,
        };
        assert_eq!(scratch.abi_read.start() % 8, 0);
        assert_eq!(scratch.abi_read.0.alias_class(), ScratchAliasClass::Primary);
        for region in [
            scratch.settings_string,
            scratch.scan,
            scratch.c_string,
            scratch.native_utf8,
            scratch.managed_utf16,
        ] {
            assert_eq!(region.alias_class(), ScratchAliasClass::Primary);
            assert_eq!(region.start(), 0);
        }
        for region in [scratch.settings_length, scratch.managed_utf8] {
            assert_eq!(region.alias_class(), ScratchAliasClass::Companion);
            assert_eq!(region.start(), bank_1_start);
        }
        assert!(scratch.abi_read.capacity() <= bank_0_capacity);
        assert!(scratch.settings_string.end() <= bank_0_capacity);
        assert!(scratch.scan.end() <= bank_0_capacity);
        assert!(scratch.c_string.end() <= bank_0_capacity);
        assert!(scratch.native_utf8.end() <= bank_0_capacity);
        assert!(scratch.managed_utf16.end() <= bank_0_capacity);
        assert_eq!(
            scratch.settings_length.start(),
            scratch.managed_utf8.start()
        );
        assert!(scratch.managed_utf8.end() as u64 <= u64::from(static_data_start));
        assert!(host_strings_address >= static_data_end);
        Self {
            scratch,
            static_data_start,
            static_data_end,
            minimum_pages,
        }
    }

    pub(super) fn scratch(self) -> RuntimeScratch {
        self.scratch
    }

    pub(super) fn minimum_pages(self) -> u64 {
        self.minimum_pages
    }

    pub(super) fn static_data_start(self) -> u32 {
        self.static_data_start
    }

    pub(super) fn static_data_end(self) -> u64 {
        self.static_data_end
    }
}

const fn align_up(value: u64, alignment: u64) -> u64 {
    assert!(alignment.is_power_of_two());
    value
        .checked_add(alignment - 1)
        .expect("aligned linear-memory address must fit")
        & !(alignment - 1)
}

#[cfg(test)]
mod tests {
    use super::{LinearMemoryLayout, ScratchRequirements, WASM_PAGE_SIZE};

    #[test]
    fn reserves_runtime_scratch_and_sizes_memory_for_static_data() {
        let requirements = ScratchRequirements {
            abi_read_capacity: 16,
            maximum_signature_len: 20,
        };
        let empty = LinearMemoryLayout::plan(0, requirements);
        assert_eq!(empty.static_data_start(), WASM_PAGE_SIZE as u32);
        assert_eq!(empty.static_data_end(), WASM_PAGE_SIZE);
        assert_eq!(empty.minimum_pages(), 1);
        assert_eq!(empty.scratch().abi_read.start(), 0);
        assert_eq!(empty.scratch().abi_read.capacity(), 16);
        assert_eq!(empty.scratch().settings_string.start(), 0);
        assert_eq!(
            empty.scratch().settings_length.start(),
            empty.scratch().managed_utf8.start()
        );
        assert_eq!(empty.scratch().host_strings_start, WASM_PAGE_SIZE as i32);

        let one_byte = LinearMemoryLayout::plan(1, requirements);
        assert_eq!(one_byte.static_data_end(), WASM_PAGE_SIZE + 1);
        assert_eq!(one_byte.minimum_pages(), 2);
        assert_eq!(
            one_byte.scratch().host_strings_start,
            2 * WASM_PAGE_SIZE as i32
        );

        let large = LinearMemoryLayout::plan(WASM_PAGE_SIZE as usize + 1, requirements);
        assert_eq!(large.minimum_pages(), 3);

        let large_read = LinearMemoryLayout::plan(
            0,
            ScratchRequirements {
                abi_read_capacity: 100_000,
                maximum_signature_len: 20,
            },
        );
        assert_eq!(large_read.scratch().abi_read.capacity(), 100_000);
        assert_eq!(large_read.static_data_start(), 2 * WASM_PAGE_SIZE as u32);
        assert_eq!(large_read.minimum_pages(), 2);

        let long_signature = LinearMemoryLayout::plan(
            0,
            ScratchRequirements {
                abi_read_capacity: 16,
                maximum_signature_len: 70_000,
            },
        );
        assert!(long_signature.scratch().scan.capacity() >= 74_095);
        assert!(long_signature.static_data_start() >= 2 * WASM_PAGE_SIZE as u32);
    }

    #[test]
    fn every_process_read_uses_a_named_destination_and_named_load_base() {
        for (name, source) in [
            ("expression", include_str!("expression.rs")),
            ("async", include_str!("async_state.rs")),
            ("state", include_str!("script_functions.rs")),
            ("process helper", include_str!("runtime_helpers/process.rs")),
            ("Unity helper", include_str!("runtime_helpers/unity.rs")),
            ("GBA helper", include_str!("runtime_helpers/gba.rs")),
        ] {
            let lines = source.lines().collect::<Vec<_>>();
            for (index, line) in lines.iter().enumerate() {
                if !line.contains("AbiImportId::ProcessRead") {
                    continue;
                }
                let start = index.saturating_sub(12);
                let prefix = lines[start..index].join("\n");
                assert!(
                    [
                        "abi_read",
                        "scan_start",
                        "c_string_start",
                        "native_utf8_start",
                        "utf16_start",
                    ]
                    .iter()
                    .any(|name| prefix.contains(name)),
                    "{name} has a process read without a named scratch destination near line {}",
                    index + 1
                );
            }

            for (index, pair) in lines.windows(2).enumerate() {
                assert!(
                    !(pair[0].contains("Instruction::I32Const(0)")
                        && ["I32Load", "I64Load", "F32Load", "F64Load"]
                            .iter()
                            .any(|load| pair[1].contains(load))),
                    "{name} loads from an anonymous address-zero scratch base near line {}",
                    index + 1
                );
            }
        }
    }
}
