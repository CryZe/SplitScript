//! Declaration identities and signatures visible while checking bodies.
//!
//! This is deliberately separate from lexical scopes and transient checking
//! modes. It is the first concrete product extracted from the former flat
//! `Checker` state and will become the output of declaration collection.

use std::collections::{HashMap, HashSet};

use crate::{
    ast::{
        EnumDecl, EnumVariantId, FunctionId, ManagedFieldId, RecordDecl, RecordFieldId, Span,
        ValueId,
    },
    inference::Type,
};

/// One fact established about the attachment-wide `layout` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct LayoutConstraint {
    pub(super) dimension: RecordFieldId,
    pub(super) variant: EnumVariantId,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RuntimeSettingKind {
    Bool,
    Choice,
    File,
    Title,
}

#[derive(Clone)]
pub(super) struct RuntimeSettingDeclaration {
    pub(super) source_name: Option<String>,
    pub(super) kind: RuntimeSettingKind,
    pub(super) span: Span,
}

#[derive(Clone, Copy)]
pub(super) struct Binding {
    pub(super) id: Option<ValueId>,
    pub(super) ty: Type,
    pub(super) mutable: bool,
    pub(super) debug_only: bool,
    pub(super) declaration_span: Option<Span>,
}

#[derive(Clone)]
pub(super) struct FunctionSignature {
    pub(super) id: FunctionId,
    pub(super) params: Vec<Type>,
    pub(super) parameter_declarations: Vec<FunctionParameterDeclaration>,
    pub(super) result: Type,
    /// Value accepted by `return` inside the function body. For an async
    /// signature this is the `T` inside the call result's `async T`.
    pub(super) completion: Type,
    /// Unbound inference roots generalized after this function's dependency
    /// component has been solved. Monomorphic functions leave this empty.
    pub(super) generalized: Vec<u32>,
    /// Associated outputs inferred from generic parameters used in the body,
    /// such as the item yielded by an `Iterable` parameter.
    pub(super) associated_projections: Vec<crate::inference::AssociatedProjection>,
}

#[derive(Clone)]
pub(super) struct FunctionParameterDeclaration {
    pub(super) name: String,
    pub(super) span: Span,
}

pub(super) struct InstantiatedFunctionSignature {
    pub(super) id: FunctionId,
    pub(super) params: Vec<Type>,
    pub(super) parameter_declarations: Vec<FunctionParameterDeclaration>,
    pub(super) result: Type,
    pub(super) type_arguments: Vec<Type>,
}

impl FunctionSignature {
    pub(super) fn monomorphic_call(&self) -> InstantiatedFunctionSignature {
        InstantiatedFunctionSignature {
            id: self.id,
            params: self.params.clone(),
            parameter_declarations: self.parameter_declarations.clone(),
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
        for projection in &self.associated_projections {
            let receiver = substitutions
                .get(&projection.receiver)
                .copied()
                .unwrap_or_else(|| {
                    inference.instantiate_type(
                        Type::Variable(projection.receiver),
                        &self.generalized,
                        &mut substitutions,
                    )
                });
            let projected =
                inference.associated_type(receiver, projection.capability, projection.name);
            // `T.Item` need not itself remain generic. A body can constrain it
            // to a concrete type while leaving `T` generic over all matching
            // iterable shapes. Instantiate the original output either way,
            // then preserve the associated-type equality at this call site.
            let expected = inference.instantiate_type(
                Type::Variable(projection.output),
                &self.generalized,
                &mut substitutions,
            );
            inference
                .unify_deferred(expected, projected)
                .expect("validated generic associated projections remain compatible");
        }
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
            parameter_declarations: self.parameter_declarations.clone(),
            result,
            type_arguments,
        }
    }
}

impl DeclarationEnvironment {
    pub(super) fn set_function_generics(
        &mut self,
        function: FunctionId,
        generalized: Vec<u32>,
        associated_projections: Vec<crate::inference::AssociatedProjection>,
    ) {
        self.function_signatures
            .get_mut(&function)
            .expect("collected functions have canonical signatures")
            .generalized = generalized.clone();
        for signature in self.functions.values_mut().chain(self.methods.values_mut()) {
            if signature.id == function {
                signature.generalized = generalized.clone();
                signature.associated_projections = associated_projections.clone();
            }
        }
        self.function_signatures
            .get_mut(&function)
            .expect("collected functions have canonical signatures")
            .associated_projections = associated_projections;
    }
}

pub(super) struct DeclarationEnvironment {
    /// Fields available on every named layout with one compatible type. For
    /// ordinary state declarations this contains every field.
    pub(super) state_fields: HashMap<String, (ValueId, Type)>,
    /// Every concrete state-field declaration, including declarations in
    /// later named layouts that project into a common field.
    pub(super) state_fields_by_id: HashMap<ValueId, Type>,
    pub(super) state_field_spans: HashMap<ValueId, crate::ast::Span>,
    /// Concrete fields available after refining `layout` to a variant.
    pub(super) layout_state_fields: HashMap<EnumVariantId, HashMap<String, (ValueId, Type)>>,
    /// State declarations guarded by attachment-wide layout facts.
    pub(super) conditional_state_fields:
        HashMap<String, Vec<(ValueId, Type, Vec<LayoutConstraint>)>>,
    /// Canonical layout facts guarding each conditionally bound managed field.
    pub(super) conditional_managed_fields: HashMap<ManagedFieldId, Vec<LayoutConstraint>>,
    /// Concrete declarations mapped to their physical snapshot field. Common
    /// declarations from later layouts map to the first layout's identity.
    pub(super) state_storage_fields: HashMap<ValueId, ValueId>,
    pub(super) settings: HashMap<String, (ValueId, Type)>,
    pub(super) settings_by_runtime_key: HashMap<String, RuntimeSettingDeclaration>,
    /// Top-level declarations without an initializer. Their values exist
    /// only after `onAttach` establishes the selected attachment layout.
    pub(super) attachment_globals: HashSet<ValueId>,
    pub(super) globals: HashMap<String, Binding>,
    pub(super) functions: HashMap<String, FunctionSignature>,
    pub(super) methods: HashMap<(Type, String), FunctionSignature>,
    pub(super) function_signatures: HashMap<FunctionId, FunctionSignature>,
    pub(super) debug_functions: HashSet<FunctionId>,
    pub(super) records: Vec<RecordDecl>,
    pub(super) enums: Vec<EnumDecl>,
    pub(super) managed_classes: Vec<crate::ast::ManagedClassDecl>,
}

impl DeclarationEnvironment {
    pub(super) fn new(
        records: Vec<RecordDecl>,
        enums: Vec<EnumDecl>,
        managed_classes: Vec<crate::ast::ManagedClassDecl>,
        debug_functions: HashSet<FunctionId>,
    ) -> Self {
        Self {
            state_fields: HashMap::new(),
            state_fields_by_id: HashMap::new(),
            state_field_spans: HashMap::new(),
            layout_state_fields: HashMap::new(),
            conditional_state_fields: HashMap::new(),
            conditional_managed_fields: HashMap::new(),
            state_storage_fields: HashMap::new(),
            settings: HashMap::new(),
            settings_by_runtime_key: HashMap::new(),
            attachment_globals: HashSet::new(),
            globals: HashMap::new(),
            functions: HashMap::new(),
            methods: HashMap::new(),
            function_signatures: HashMap::new(),
            debug_functions,
            records,
            enums,
            managed_classes,
        }
    }
}
