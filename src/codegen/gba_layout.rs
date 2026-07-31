//! Static names and signatures used by GBA emulator discovery.
//!
//! The runtime helper consumes these entries through the ordinary compiler
//! string and signature pools, keeping embedded data planning deterministic.

pub(super) const PROCESS_NAMES: &[&str] = &[
    "visualboyadvance-m.exe",
    "VisualBoyAdvance.exe",
    "mGBA.exe",
    "mGBA",
    "NO$GBA.EXE",
    "retroarch.exe",
    "EmuHawk.exe",
    "mednafen.exe",
];

pub(super) const RETROARCH_CORES: &[&str] = &[
    "vbam_libretro.dll",
    "mednafen_gba_libretro.dll",
    "vba_next_libretro.dll",
    "mgba_libretro.dll",
    "gpsp_libretro.dll",
];

pub(super) const EMUHAWK_CORE: &str = "mgba.dll";

pub(super) const VBA_X64_EWRAM: &str = "48 8B 05 ?? ?? ?? ?? 81 E3 FF FF 03 00";
pub(super) const VBA_X64_IWRAM: &str = "48 8B 05 ?? ?? ?? ?? 81 E3 FF 7F 00 00";
pub(super) const SHARED_X64_EWRAM: &str = "48 8B 05 ?? ?? ?? ?? 81 E1 FF FF 03 00";
pub(super) const SHARED_X64_IWRAM: &str = "48 8B 05 ?? ?? ?? ?? 81 E1 FF 7F 00 00";
pub(super) const SHARED_X86_EWRAM: &str = "A1 ?? ?? ?? ?? 81 ?? FF FF 03 00";
pub(super) const SHARED_X86_IWRAM: &str = "A1 ?? ?? ?? ?? 81 ?? FF 7F 00 00";
pub(super) const VBA_X86_OLD_EWRAM: &str = "81 E6 FF FF 03 00 8B 15 ?? ?? ?? ??";
pub(super) const VBA_RUNNING: &str = "83 3D ?? ?? ?? ?? 00 74 ?? 80 3D ?? ?? ?? ?? 00 75 ?? 66";
pub(super) const VBA_X64_RUNNING_FALLBACK: &str = "48 8B 15 ?? ?? ?? ?? 31 C0 8B 12 85 D2 74 ?? 48";
pub(super) const VBA_X86_RUNNING_FALLBACK: &str = "8B 15 ?? ?? ?? ?? 31 C0 85 D2 74 ?? 0F";
pub(super) const VBA_X86_OLD_RUNNING: &str = "8B 0D ?? ?? ?? ?? 85 C9 74 ?? 8A";
pub(super) const NOCASH_BASE: &str = "FF 35 ?? ?? ?? ?? 55";
pub(super) const GPSP_BASE_X64: &str = "48 8B 15 ?? ?? ?? ?? 8B 42 40";
pub(super) const GPSP_BASE_X86: &str = "A3 ?? ?? ?? ?? F7 C5 02 00 00 00";
pub(super) const GPSP_EWRAM: &str = "25 FF FF 03 00 88 94 03";
pub(super) const GPSP_IWRAM: &str = "25 FE 7F 00 00 66 89 94 03";

pub(super) const SIGNATURES: &[&str] = &[
    VBA_X64_EWRAM,
    VBA_X64_IWRAM,
    SHARED_X64_EWRAM,
    SHARED_X64_IWRAM,
    SHARED_X86_EWRAM,
    SHARED_X86_IWRAM,
    VBA_X86_OLD_EWRAM,
    VBA_RUNNING,
    VBA_X64_RUNNING_FALLBACK,
    VBA_X86_RUNNING_FALLBACK,
    VBA_X86_OLD_RUNNING,
    NOCASH_BASE,
    GPSP_BASE_X64,
    GPSP_BASE_X86,
    GPSP_EWRAM,
    GPSP_IWRAM,
];
