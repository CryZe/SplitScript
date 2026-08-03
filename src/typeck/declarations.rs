//! Declaration identities and signatures visible while checking bodies.
//!
//! This is deliberately separate from lexical scopes and transient checking
//! modes. It is the first concrete product extracted from the former flat
//! `Checker` state and will become the output of declaration collection.

use std::collections::{HashMap, HashSet};

use crate::{
    ast::{EnumDecl, EnumVariantId, FunctionId, RecordDecl, ValueId},
    inference::Type,
};

#[derive(Clone, Copy)]
pub(super) struct Binding {
    pub(super) id: Option<ValueId>,
    pub(super) ty: Type,
    pub(super) mutable: bool,
    pub(super) debug_only: bool,
}

#[derive(Clone)]
pub(super) struct FunctionSignature {
    pub(super) id: FunctionId,
    pub(super) params: Vec<Type>,
    pub(super) result: Type,
    /// Value accepted by `return` inside the function body. For an async
    /// signature this is the `T` inside the call result's `async T`.
    pub(super) completion: Type,
    /// Unbound inference roots generalized after this function's dependency
    /// component has been solved. Monomorphic functions leave this empty.
    pub(super) generalized: Vec<u32>,
}

pub(super) struct InstantiatedFunctionSignature {
    pub(super) id: FunctionId,
    pub(super) params: Vec<Type>,
    pub(super) result: Type,
    pub(super) type_arguments: Vec<Type>,
}

impl FunctionSignature {
    pub(super) fn monomorphic_call(&self) -> InstantiatedFunctionSignature {
        InstantiatedFunctionSignature {
            id: self.id,
            params: self.params.clone(),
            result: self.result,
            type_arguments: Vec::new(),
        }
    }

    pub(super) fn instantiate(
        &self,
        inference: &mut crate::inference::InferenceContext,
    ) -> InstantiatedFunctionSignature {
        let mut substitutions = HashMap::new();
        let params = self
            .params
            .iter()
            .map(|ty| inference.instantiate_type(*ty, &self.generalized, &mut substitutions))
            .collect();
        let result = inference.instantiate_type(self.result, &self.generalized, &mut substitutions);
        let type_arguments = self
            .generalized
            .iter()
            .map(|variable| {
                substitutions
                    .get(variable)
                    .copied()
                    .expect("every generalized signature variable occurs in its signature")
            })
            .collect();
        InstantiatedFunctionSignature {
            id: self.id,
            params,
            result,
            type_arguments,
        }
    }
}

impl DeclarationEnvironment {
    pub(super) fn set_function_generics(&mut self, function: FunctionId, generalized: Vec<u32>) {
        self.function_signatures
            .get_mut(&function)
            .expect("collected functions have canonical signatures")
            .generalized = generalized.clone();
        for signature in self.functions.values_mut().chain(self.methods.values_mut()) {
            if signature.id == function {
                signature.generalized = generalized.clone();
            }
        }
    }
}

pub(super) struct DeclarationEnvironment {
    /// Fields available on every named layout with one compatible type. For
    /// ordinary state declarations this contains every field.
    pub(super) state_fields: HashMap<String, (ValueId, Type)>,
    /// Every concrete state-field declaration, including declarations in
    /// later named layouts that project into a common field.
    pub(super) state_fields_by_id: HashMap<ValueId, Type>,
    /// Concrete fields available after refining `layout` to a variant.
    pub(super) layout_state_fields: HashMap<EnumVariantId, HashMap<String, (ValueId, Type)>>,
    /// Concrete declarations mapped to their physical snapshot field. Common
    /// declarations from later layouts map to the first layout's identity.
    pub(super) state_storage_fields: HashMap<ValueId, ValueId>,
    pub(super) settings: HashMap<String, (ValueId, Type)>,
    pub(super) globals: HashMap<String, Binding>,
    pub(super) functions: HashMap<String, FunctionSignature>,
    pub(super) methods: HashMap<(Type, String), FunctionSignature>,
    pub(super) function_signatures: HashMap<FunctionId, FunctionSignature>,
    pub(super) debug_functions: HashSet<FunctionId>,
    pub(super) records: Vec<RecordDecl>,
    pub(super) enums: Vec<EnumDecl>,
}

impl DeclarationEnvironment {
    pub(super) fn new(
        records: Vec<RecordDecl>,
        enums: Vec<EnumDecl>,
        debug_functions: HashSet<FunctionId>,
    ) -> Self {
        Self {
            state_fields: HashMap::new(),
            state_fields_by_id: HashMap::new(),
            layout_state_fields: HashMap::new(),
            state_storage_fields: HashMap::new(),
            settings: HashMap::new(),
            globals: HashMap::new(),
            functions: HashMap::new(),
            methods: HashMap::new(),
            function_signatures: HashMap::new(),
            debug_functions,
            records,
            enums,
        }
    }
}
