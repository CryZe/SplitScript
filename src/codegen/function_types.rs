//! Exact ordinary function signatures appended after the GC recursive group.

use std::collections::{HashMap, hash_map::Entry};

use wasm_encoder::{TypeSection, ValType};

pub(super) struct FunctionTypes {
    indices: HashMap<(Vec<ValType>, Vec<ValType>), u32>,
    next: u32,
}

impl FunctionTypes {
    pub fn new(first_index: u32) -> Self {
        Self {
            indices: HashMap::new(),
            next: first_index,
        }
    }

    pub fn intern(
        &mut self,
        section: &mut TypeSection,
        params: Vec<ValType>,
        results: Vec<ValType>,
    ) -> u32 {
        match self.indices.entry((params, results)) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let index = self.next;
                self.next += 1;
                let (params, results) = entry.key();
                section
                    .ty()
                    .function(params.iter().copied(), results.iter().copied());
                entry.insert(index);
                index
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FunctionTypes;
    use wasm_encoder::{HeapType, RefType, TypeSection, ValType};

    #[test]
    fn exact_signatures_preserve_reference_identity_and_nullability() {
        let mut types = TypeSection::new();
        let mut signatures = FunctionTypes::new(40);
        let reference = |index, nullable| {
            ValType::Ref(RefType {
                nullable,
                heap_type: HeapType::Concrete(index),
            })
        };
        assert_eq!(
            signatures.intern(&mut types, vec![reference(3, true)], vec![]),
            40
        );
        assert_eq!(
            signatures.intern(&mut types, vec![reference(3, false)], vec![]),
            41
        );
        assert_eq!(
            signatures.intern(&mut types, vec![reference(4, true)], vec![]),
            42
        );
        assert_eq!(
            signatures.intern(&mut types, vec![], vec![reference(3, true)]),
            43
        );
        assert_eq!(
            signatures.intern(&mut types, vec![reference(3, true)], vec![]),
            40
        );
        assert_eq!(types.len(), 4);
    }
}
