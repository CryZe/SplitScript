//! Declaration identities and signatures visible while checking bodies.
//!
//! This is deliberately separate from lexical scopes and transient checking
//! modes. It is the first concrete product extracted from the former flat
//! `Checker` state and will become the output of declaration collection.

use std::collections::{HashMap, HashSet};

use crate::{
    ast::{EnumDecl, FunctionId, RecordDecl, ValueId},
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
}

pub(super) struct DeclarationEnvironment {
    pub(super) state_fields: HashMap<String, (ValueId, Type)>,
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
