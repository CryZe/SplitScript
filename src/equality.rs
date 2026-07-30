//! Structural equality capabilities for nominal GC values.
//!
//! Primitive equality is intrinsic. Records and enums gain equality
//! automatically when every contained field or payload is itself equatable.
//! This is shared by diagnostics, future editor queries, and Wasm helper
//! generation rather than being inferred independently in the backend.

use std::collections::{HashMap, HashSet};

use crate::{
    ast::{EnumDecl, EnumId, RecordDecl, RecordId},
    semantic::SemanticModel,
    stdlib::{StandardLibrary, StdlibCapabilityId},
    types::{BuiltinType, TypeId, TypeKind},
};

#[derive(Debug, Clone, Default)]
pub struct EqualityCapabilities {
    records: HashMap<RecordId, Result<(), String>>,
    enums: HashMap<EnumId, Result<(), String>>,
}

impl EqualityCapabilities {
    pub fn build(records: &[RecordDecl], enums: &[EnumDecl], semantics: &SemanticModel) -> Self {
        let mut capabilities = Self::default();
        for record in records {
            let result = capabilities.check_record(
                record.id,
                records,
                enums,
                semantics,
                &mut HashSet::new(),
            );
            capabilities.records.entry(record.id).or_insert(result);
        }
        for enumeration in enums {
            let result = capabilities.check_enum(
                enumeration.id,
                records,
                enums,
                semantics,
                &mut HashSet::new(),
            );
            capabilities.enums.entry(enumeration.id).or_insert(result);
        }
        capabilities
    }

    pub fn require(&self, ty: TypeId, semantics: &SemanticModel) -> Result<(), String> {
        match semantics.types().kind(ty) {
            TypeKind::Builtin(builtin) if builtin_is_equatable(*builtin) => Ok(()),
            TypeKind::Standard(standard)
                if StandardLibrary::new()
                    .type_has_capability(*standard, StdlibCapabilityId::Equatable) =>
            {
                Ok(())
            }
            TypeKind::Record(record) => self.record(*record).map_err(str::to_owned),
            TypeKind::Enum(enumeration) => self.enumeration(*enumeration).map_err(str::to_owned),
            TypeKind::Option { value, .. } => self
                .require(*value, semantics)
                .map_err(|error| format!("optional value does not support equality: {error}")),
            TypeKind::Result { value, .. } => self
                .require(*value, semantics)
                .map_err(|error| format!("result value does not support equality: {error}")),
            _ => Err("this type does not support equality".to_owned()),
        }
    }

    pub fn record(&self, record: RecordId) -> Result<(), &str> {
        self.records
            .get(&record)
            .expect("every record has an equality result")
            .as_ref()
            .copied()
            .map_err(String::as_str)
    }

    pub fn enumeration(&self, enumeration: EnumId) -> Result<(), &str> {
        self.enums
            .get(&enumeration)
            .expect("every enum has an equality result")
            .as_ref()
            .copied()
            .map_err(String::as_str)
    }

    fn check_type(
        &mut self,
        ty: TypeId,
        records: &[RecordDecl],
        enums: &[EnumDecl],
        semantics: &SemanticModel,
        visiting: &mut HashSet<TypeId>,
    ) -> Result<(), String> {
        if !visiting.insert(ty) {
            return Err("recursive values do not currently support structural equality".to_owned());
        }
        let result = match semantics.types().kind(ty) {
            TypeKind::Builtin(builtin) if builtin_is_equatable(*builtin) => Ok(()),
            TypeKind::Standard(standard)
                if StandardLibrary::new()
                    .type_has_capability(*standard, StdlibCapabilityId::Equatable) =>
            {
                Ok(())
            }
            TypeKind::Record(record) => {
                self.check_record(*record, records, enums, semantics, visiting)
            }
            TypeKind::Enum(enumeration) => {
                self.check_enum(*enumeration, records, enums, semantics, visiting)
            }
            TypeKind::Option { value, .. } => {
                self.check_type(*value, records, enums, semantics, visiting)
            }
            TypeKind::Result { value, .. } => {
                self.check_type(*value, records, enums, semantics, visiting)
            }
            _ => Err("the contained type does not support equality".to_owned()),
        };
        visiting.remove(&ty);
        result
    }

    fn check_record(
        &mut self,
        record: RecordId,
        records: &[RecordDecl],
        enums: &[EnumDecl],
        semantics: &SemanticModel,
        visiting: &mut HashSet<TypeId>,
    ) -> Result<(), String> {
        if let Some(result) = self.records.get(&record) {
            return result.clone();
        }
        let declaration = records
            .iter()
            .find(|declaration| declaration.id == record)
            .expect("record IDs refer to declarations");
        for field in &declaration.fields {
            let ty = semantics
                .record_field_type(field.id)
                .expect("checked record fields have types");
            self.check_type(ty, records, enums, semantics, visiting)
                .map_err(|error| {
                    format!(
                        "record `{}.{}` does not support equality: {error}",
                        declaration.name, field.name
                    )
                })?;
        }
        self.records.insert(record, Ok(()));
        Ok(())
    }

    fn check_enum(
        &mut self,
        enumeration: EnumId,
        records: &[RecordDecl],
        enums: &[EnumDecl],
        semantics: &SemanticModel,
        visiting: &mut HashSet<TypeId>,
    ) -> Result<(), String> {
        if let Some(result) = self.enums.get(&enumeration) {
            return result.clone();
        }
        let declaration = enums
            .iter()
            .find(|declaration| declaration.id == enumeration)
            .expect("enum IDs refer to declarations");
        for variant in &declaration.variants {
            if let Some(ty) = semantics.enum_variant_payload(variant.id) {
                self.check_type(ty, records, enums, semantics, visiting)
                    .map_err(|error| {
                        format!(
                            "enum `{}.{}` does not support equality: {error}",
                            declaration.name, variant.name
                        )
                    })?;
            }
        }
        self.enums.insert(enumeration, Ok(()));
        Ok(())
    }
}

fn builtin_is_equatable(ty: BuiltinType) -> bool {
    ty.is_numeric() || ty == BuiltinType::Bool
}
