//! Stable compiler identities generated independently from authored declarations.
//!
//! This consumer intentionally depends only on the raw hierarchy in
//! `source`; declaration schema and normalized graph data consume these IDs
//! in one direction.

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

macro_rules! standard_library_ids {
    (
        root {
            items { $($root_item_id:ident => {
                intrinsic: $root_intrinsic:ident, $($root_item:tt)*
            }),* $(,)? }
        }
        state_providers {
            $($state_provider_id:ident => {
                name: $state_provider_name:literal,
                value_name: $state_provider_value_name:literal,
                processes: $state_provider_processes:expr,
                process_type: $state_provider_type:ident,
                attachment: $state_provider_attachment:ident,
                direct_read: $state_provider_direct_read:ident,
                summary: $state_provider_summary:literal,
                details: $state_provider_details:literal,
                example: {
                    title: $state_provider_example_title:literal,
                    source: $state_provider_example_source:literal,
                    validation: $state_provider_example_validation:expr $(,)?
                } $(,)?
            }),* $(,)?
        }
        capabilities {
            $($capability_id:ident => {
                name: $capability_name:literal,
                behavior: $capability_behavior:ident,
                receiver: $capability_receiver:expr,
                summary: $capability_summary:literal,
                details: $capability_details:literal,
                items { $($capability_item_id:ident => {
                    intrinsic: $capability_intrinsic:ident, $($capability_item:tt)*
                }),* $(,)? }
            }),* $(,)?
        }
        type_constructors {
            $($constructor_id:ident => {
                name: $constructor_name:literal,
                parameters: $constructor_parameters:expr,
                receiver: $constructor_receiver:expr,
                summary: $constructor_summary:literal,
                details: $constructor_details:literal,
                items { $($constructor_item_id:ident => {
                    intrinsic: $constructor_intrinsic:ident, $($constructor_item:tt)*
                }),* $(,)? }
            }),* $(,)?
        }
        core_extensions {
            $($core_id:ident => {
                name: $core_name:literal,
                items { $($core_item_id:ident => {
                    intrinsic: $core_intrinsic:ident, $($core_item:tt)*
                }),* $(,)? }
            }),* $(,)?
        }
        namespaces {
            $($namespace_id:ident => {
                name: $namespace_name:literal,
                path: $namespace_path:expr,
                qualified: $namespace_qualified:literal,
                summary: $namespace_summary:literal,
                details: $namespace_details:literal,
                items { $($namespace_item_id:ident => {
                    intrinsic: $namespace_intrinsic:ident, $($namespace_item:tt)*
                }),* $(,)? }
            }),* $(,)?
        }
        types {
            $(
                $(#[$type_attribute:meta])*
                $type_id:ident => {
                    name: $type_name:literal,
                    kind: $type_kind:ident,
                    capabilities: $type_capabilities:expr,
                    representation: $type_representation:expr,
                    value_usage: $type_value_usage:expr,
                    summary: $type_summary:literal,
                    details: $type_details:literal,
                    fields {
                        $($(#[$field_attribute:meta])*
                            $field_id:ident => {
                                name: $field_name:literal,
                                ty: $field_type:expr,
                                visibility: $field_visibility:ident,
                                docs: $field_docs:literal $(,)?
                            }
                        ),* $(,)?
                    }
                    variants {
                        $($(#[$variant_attribute:meta])*
                            $variant_id:ident => {
                                name: $variant_name:literal,
                                docs: $variant_docs:literal $(,)?
                            }
                        ),* $(,)?
                    }
                    items {
                        $($type_item_id:ident => {
                            intrinsic: $type_intrinsic:ident, $($type_item:tt)*
                        }),* $(,)?
                    }
                }
            ),* $(,)?
        }
    )
    => {
        catalog_id!(StdlibCapabilityId, StdlibCapabilityIdDiscriminant {
            $($capability_id),*
        });

        catalog_id!(StdlibStateProviderId, StdlibStateProviderIdDiscriminant {
            $($state_provider_id),*
        });

        catalog_id!(StdlibTypeConstructorId, StdlibTypeConstructorIdDiscriminant {
            $($constructor_id),*
        });

        catalog_id!(StdlibNamespaceId, StdlibNamespaceIdDiscriminant {
            $($namespace_id),*
        });

        catalog_id!(StdlibTypeId, StdlibTypeIdDiscriminant {
            $($(#[$type_attribute])* $type_id),*
        });

        catalog_id!(StdlibFieldId, StdlibFieldIdDiscriminant {
            $($($(#[$field_attribute])* $field_id,)*)*
        });

        catalog_id!(StdlibVariantId, StdlibVariantIdDiscriminant {
            $($($(#[$variant_attribute])* $variant_id,)*)*
        });

        catalog_id!(StdlibItemId, StdlibItemIdDiscriminant {
            $($root_item_id,)*
            $($($capability_item_id,)*)*
            $($($constructor_item_id,)*)*
            $($($core_item_id,)*)*
            $($($namespace_item_id,)*)*
            $($($type_item_id,)*)*
        });

        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum IntrinsicId {
            $($root_intrinsic,)*
            $($state_provider_attachment,)*
            $($($capability_intrinsic,)*)*
            $($($constructor_intrinsic,)*)*
            $($($core_intrinsic,)*)*
            $($($namespace_intrinsic,)*)*
            $($($type_intrinsic,)*)*
        }

        impl IntrinsicId {
            pub const ALL: &'static [Self] = &[
                $(Self::$root_intrinsic,)*
                $(Self::$state_provider_attachment,)*
                $($(Self::$capability_intrinsic,)*)*
                $($(Self::$constructor_intrinsic,)*)*
                $($(Self::$core_intrinsic,)*)*
                $($(Self::$namespace_intrinsic,)*)*
                $($(Self::$type_intrinsic,)*)*
            ];
        }
    };
}

super::source::with_standard_library!(standard_library_ids);

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
