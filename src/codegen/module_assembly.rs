use std::borrow::Cow;

use wasm_encoder::{
    CodeSection, CustomSection, ExportKind, ExportSection, FunctionSection, GlobalSection,
    ImportSection, MemorySection, MemoryType, Module, NameSection, TypeSection,
};

use super::data_plan::StaticData;

pub(super) struct Sections {
    pub types: TypeSection,
    pub imports: ImportSection,
    pub functions: FunctionSection,
    pub globals: GlobalSection,
    pub codes: CodeSection,
}

pub(super) fn finish(
    sections: Sections,
    data: &StaticData,
    start_function: u32,
    update_function: u32,
    debug_names: Option<&NameSection>,
) -> Vec<u8> {
    let Sections {
        types,
        imports,
        functions,
        globals,
        codes,
    } = sections;
    let mut memories = MemorySection::new();
    memories.memory(MemoryType {
        minimum: data.layout().minimum_pages(),
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    exports.export("_start", ExportKind::Func, start_function);
    exports.export("update", ExportKind::Func, update_function);
    let data = data.encode();

    let mut module = Module::new();
    module.section(&types);
    module.section(&imports);
    module.section(&functions);
    module.section(&memories);
    module.section(&globals);
    module.section(&exports);
    module.section(&codes);
    module.section(&data);
    if let Some(debug_names) = debug_names {
        module.section(debug_names);
    }
    module.section(&CustomSection {
        name: Cow::Borrowed("splitscript"),
        data: Cow::Owned(crate::build_identity::module_metadata()),
    });
    module.finish()
}
