//! WebAssembly host-import declaration and stable function-index assignment.

use wasm_encoder::{EntityType, ImportSection, TypeSection, ValType};

use crate::abi::{AbiCatalog, AbiImportId, AbiType};

use super::dependencies::BackendDependencies;

pub(super) struct EncodedImports {
    pub section: ImportSection,
    pub abi: Abi,
    pub function_count: u32,
    pub next_type_index: u32,
}

/// Concrete Wasm function indices for the declarative host ABI.
pub(super) struct Abi {
    functions: [Option<u32>; AbiImportId::COUNT],
}

impl Abi {
    pub fn function(&self, id: AbiImportId) -> u32 {
        self.functions[id.index()].expect("emitted code requires a planned host import")
    }
}

pub(super) fn encode(
    types: &mut TypeSection,
    first_type_index: u32,
    dependencies: &BackendDependencies,
) -> EncodedImports {
    let mut section = ImportSection::new();
    let mut function_count = 0;
    let mut next_type_index = first_type_index;
    let catalog = AbiCatalog::new();
    let mut functions = [None; AbiImportId::COUNT];

    for declaration in catalog.imports() {
        if !dependencies
            .host_imports()
            .any(|required| required == declaration.id)
        {
            continue;
        }
        let ty = next_type_index;
        next_type_index += 1;
        types.ty().function(
            declaration
                .parameters
                .iter()
                .map(|parameter| val_type(parameter.ty)),
            declaration.results.iter().map(|result| val_type(result.ty)),
        );
        section.import(
            declaration.module,
            declaration.name,
            EntityType::Function(ty),
        );
        functions[declaration.id.index()] = Some(function_count);
        function_count += 1;
    }

    EncodedImports {
        section,
        abi: Abi { functions },
        function_count,
        next_type_index,
    }
}

fn val_type(ty: AbiType) -> ValType {
    match ty {
        AbiType::I32 => ValType::I32,
        AbiType::I64 => ValType::I64,
        AbiType::F64 => ValType::F64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_order_assigns_contiguous_type_and_function_indices() {
        let mut types = TypeSection::new();
        let first_type = 41;
        let dependencies = BackendDependencies::with_host_imports(
            AbiCatalog::new().imports().map(|import| import.id),
        );
        let encoded = encode(&mut types, first_type, &dependencies);

        assert_eq!(encoded.section.len(), AbiImportId::COUNT as u32);
        assert_eq!(types.len(), AbiImportId::COUNT as u32);
        assert_eq!(encoded.function_count, AbiImportId::COUNT as u32);
        assert_eq!(
            encoded.next_type_index,
            first_type + AbiImportId::COUNT as u32
        );
        assert_eq!(encoded.abi.function(AbiImportId::TimerGetState), 0);
        assert_eq!(
            encoded.abi.function(AbiImportId::SettingValueGetString),
            AbiImportId::COUNT as u32 - 1
        );
    }
}
