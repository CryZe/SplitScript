//! Closed identities for compiler-provided standard-library operations.
//!
//! Unlike ordinary library declarations, these identities are part of the
//! compiler trust boundary. Privileged SplitScript source may bind a public
//! declaration to one of them, but cannot create a new intrinsic, redefine its
//! effects, or select its lowering and host dependencies.

macro_rules! trusted_intrinsics {
    ($consumer:ident) => {
        $consumer! {
            Print,
            TimerSetVariable,
            RuntimeSetTickRate,
            SettingsEnabled,
            InstantNow,
            NextTick,
            NumericAdd,
            NumericSubtract,
            NumericMin,
            NumericMax,
            FloatAbs,
            FloatFloor,
            FloatCeil,
            FloatRound,
            ArrayLength,
            ArraySet,
            SetNew,
            SetLength,
            SetContains,
            SetInsert,
            SetRemove,
            SetClear,
            AddressAdd,
            ProcessName,
            ProcessPath,
            ProcessMainModule,
            ProcessClosed,
            ProcessModule,
            ProcessFindMemoryRange,
            ProcessRead,
            ProcessFollow,
            ProcessScan,
            ProcessScanMemory,
            ProcessScanMemoryAny,
            ProcessReadRelative32,
            ProcessReadUtf8,
            ProcessReadUtf16Le,
            ProcessReadManagedString,
            TimerState,
            TimerCurrentSplitIndex,
            TimerSegmentWasSplit,
            TimerSkipSplit,
            TimerUndoSplit,
            TimerPauseGameTime,
            TimerResumeGameTime,
            RuntimeOperatingSystem,
            RuntimeArchitecture,
            StringLength,
            StringContains,
            StringStartsWith,
            StringEndsWith,
            StringEqualsIgnoreAsciiCase,
            StringToAsciiLowerCase,
            StringReplaceAll,
            StringSplit,
            StringSlice,
            StringConcat,
            ModuleScan,
            ModuleScanAny,
            ModulePath,
            UnityModuleImage,
            UnityImageClass,
            UnityClassField,
            UnityClassFieldAny,
            UnityClassStaticTable,
            UnityClassStaticInstance,
            GbaEmulatorRead,
        }
    };
}

macro_rules! define_intrinsic_ids {
    ($($id:ident),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum IntrinsicId {
            $($id),*
        }

        impl IntrinsicId {
            pub const ALL: &'static [Self] = &[$(Self::$id),*];

            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$id => stringify!($id)),*
                }
            }

            pub fn named(name: &str) -> Option<Self> {
                match name {
                    $(stringify!($id) => Some(Self::$id),)*
                    _ => None,
                }
            }
        }
    };
}

trusted_intrinsics!(define_intrinsic_ids);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_intrinsic_names_round_trip() {
        for id in IntrinsicId::ALL {
            assert_eq!(IntrinsicId::named(id.name()), Some(*id));
        }
        assert_eq!(IntrinsicId::named("UserDefinedHostEscape"), None);
    }
}
