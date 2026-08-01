//! Stable compiler identities generated from privileged SplitScript source.
//!
//! The bundled source owns ordinary symbol identity and declaration order.
//! Trusted intrinsic identities remain independently Rust-owned.

pub use super::intrinsics::IntrinsicId;

macro_rules! catalog_id {
    (
        $name:ident, $discriminant:ident {
            $($(#[$attribute:meta])* $variant:ident),* $(,)?
        }
    ) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(u32);

        #[repr(u32)]
        #[allow(clippy::enum_variant_names)]
        enum $discriminant {
            $($(#[$attribute])* $variant),*
        }

        #[allow(non_upper_case_globals)]
        impl $name {
            $(
                $(#[$attribute])*
                pub const $variant: Self = Self($discriminant::$variant as u32);
            )*

            pub const fn as_u32(self) -> u32 {
                self.0
            }

            /// Allocates an identity supplied by the trusted catalog loader.
            #[allow(dead_code)]
            pub(crate) const fn from_u32(value: u32) -> Self {
                Self(value)
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match *self {
                    $($(#[$attribute])* Self::$variant => {
                        formatter.write_str(stringify!($variant))
                    },)*
                    Self(value) => formatter.debug_tuple(stringify!($name)).field(&value).finish(),
                }
            }
        }
    };
}

include!(concat!(env!("OUT_DIR"), "/stdlib_ids.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_id_space_accepts_loaded_declarations_beyond_well_known_constants() {
        let loaded = StdlibTypeId::from_u32(u32::MAX);
        assert_ne!(loaded, StdlibTypeId::String);
        assert_eq!(loaded.as_u32(), u32::MAX);
        assert_eq!(format!("{loaded:?}"), "StdlibTypeId(4294967295)");
        assert_eq!(format!("{:?}", StdlibTypeId::String), "String");
    }
}
