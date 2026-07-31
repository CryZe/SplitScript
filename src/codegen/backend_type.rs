//! Physical value categories used by the WebAssembly backend.
//!
//! These values are derived from semantic [`TypeId`](crate::types::TypeId)
//! values after type checking. They deliberately do not participate in source
//! inference or name resolution.

use crate::{
    ast::{ArrayTypeId, EnumId, OptionTypeId, RecordId, ResultTypeId},
    stdlib::{
        CoreTypeId, DeclaredTypeRef, RuntimeRepresentation, StandardLibrary, StdlibTypeId,
        with_core_types,
    },
};

macro_rules! define_backend_type {
    ($($core:ident => { $($declaration:tt)* }),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub(super) enum Type {
            $($core),*,
            Standard(StdlibTypeId),
            Record(RecordId),
            Enum(EnumId),
            Array(ArrayTypeId),
            Option(OptionTypeId),
            Result(ResultTypeId),
        }
    };
}

with_core_types!(define_backend_type);

macro_rules! backend_type_from_core {
    ($($core:ident => { $($declaration:tt)* }),* $(,)?) => {
        |core| match core {
            $(CoreTypeId::$core => Self::$core),*
        }
    };
}

impl Type {
    pub(super) fn from_core(core: CoreTypeId) -> Self {
        with_core_types!(backend_type_from_core)(core)
    }

    pub(super) fn from_standard(ty: StdlibTypeId) -> Self {
        Self::Standard(ty)
    }

    pub(super) fn from_declared(ty: DeclaredTypeRef) -> Self {
        match ty {
            DeclaredTypeRef::Core(core) => Self::from_core(core),
            DeclaredTypeRef::Standard(standard) => Self::from_standard(standard),
        }
    }

    pub(super) fn is_signed(self) -> bool {
        matches!(self, Self::I8 | Self::I16 | Self::I32 | Self::I64)
    }

    pub(super) fn is_enum(self, standard_library: &StandardLibrary) -> bool {
        matches!(self, Self::Enum(_))
            || matches!(
                self,
                Self::Standard(standard)
                    if matches!(
                        standard_library.type_decl(standard).representation,
                        RuntimeRepresentation::Enum { .. }
                    )
            )
    }
}
