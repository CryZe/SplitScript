//! WebAssembly host-import declaration and stable function-index assignment.

use wasm_encoder::{EntityType, ImportSection, TypeSection, ValType};

use crate::abi::{AbiCatalog, AbiImportId, AbiType};

use super::{dependencies::BackendDependencies, function_types::FunctionTypes};

pub(super) struct EncodedImports {
    pub section: ImportSection,
    pub abi: Abi,
    pub function_count: u32,
}

/// Concrete Wasm function indices for the declarative host ABI.
pub(super) struct Abi {
    functions: [Option<u32>; AbiImportId::COUNT],
}

impl Abi {
    pub fn function(&self, id: AbiImportId) -> u32 {
        self.functions[id.index()].expect("emitted code requires a planned host import")
    }

    pub(super) fn debug_names(&self) -> impl Iterator<Item = (u32, &'static str)> + '_ {
        AbiImportId::ALL.iter().filter_map(|id| {
            self.functions[id.index()].map(|index| {
                let declaration = AbiCatalog::new().import(*id);
                (index, declaration.name)
            })
        })
    }
}

pub(super) fn encode(
    types: &mut TypeSection,
    signatures: &mut FunctionTypes,
    dependencies: &BackendDependencies,
) -> EncodedImports {
    let mut section = ImportSection::new();
    let mut function_count = 0;
    let catalog = AbiCatalog::new();
    let mut functions = [None; AbiImportId::COUNT];

    for declaration in catalog.imports() {
        if !dependencies
            .host_imports()
            .any(|required| required == declaration.id)
        {
            continue;
        }
        let ty = signatures.intern(
            types,
            declaration
                .parameters
                .iter()
                .map(|parameter| val_type(parameter.ty))
                .collect(),
            declaration
                .results
                .iter()
                .map(|result| val_type(result.ty))
                .collect(),
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
    use wasm_encoder::TypeSection;

    use crate::{
        abi::{AbiCatalog, AbiImportId},
        codegen::dependencies::BackendDependencies,
    };

    use super::{FunctionTypes, encode};

    #[test]
    fn catalog_order_preserves_function_indices_while_sharing_signatures() {
        let mut types = TypeSection::new();
        let first_type = 41;
        let dependencies = BackendDependencies::with_host_imports(
            AbiCatalog::new().imports().map(|import| import.id),
        );
        let mut signatures = FunctionTypes::new(first_type);
        let encoded = encode(&mut types, &mut signatures, &dependencies);

        assert_eq!(encoded.section.len(), AbiImportId::COUNT as u32);
        assert!(types.len() < AbiImportId::COUNT as u32);
        assert_eq!(encoded.function_count, AbiImportId::COUNT as u32);
        assert_eq!(encoded.abi.function(AbiImportId::TimerGetState), 0);
        assert_eq!(
            encoded.abi.function(AbiImportId::SettingValueGetString),
            AbiImportId::COUNT as u32 - 1
        );
    }
}
