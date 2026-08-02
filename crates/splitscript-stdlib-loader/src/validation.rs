//! Validation for the privileged source model before Rust catalog emission.
//!
//! This is deliberately source-facing validation. The compiler separately
//! validates the generated graph against its closed intrinsic/runtime trust
//! registries; the loader only guarantees that generation is total and that
//! every source-level reference is structurally meaningful.

use std::collections::{HashMap, HashSet};

use splitscript_syntax::PrimitiveType;

use crate::{
    Attribute, AttributeArgument, CallableOwnerDeclaration, Declaration, Documentation, Error,
    FunctionDeclaration, Library, StructDeclaration, Type, TypeParameter,
};

pub(crate) fn validate(library: &Library) -> Vec<Error> {
    let mut validator = Validator::new(library);
    validator.validate();
    validator.errors
}

struct Validator<'a> {
    library: &'a Library,
    types: HashSet<&'a str>,
    capabilities: HashSet<&'a str>,
    constructors: HashMap<&'a str, usize>,
    generated_items: HashSet<String>,
    example_owners: HashMap<String, String>,
    errors: Vec<Error>,
}

impl<'a> Validator<'a> {
    fn new(library: &'a Library) -> Self {
        let types = library
            .declarations
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::Struct(value) | Declaration::IntrinsicType(value) => {
                    Some(value.name.as_str())
                }
                Declaration::Enum(value) => Some(value.name.as_str()),
                _ => None,
            })
            .collect();
        let capabilities = library
            .declarations
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::Capability(value) => Some(value.name.as_str()),
                _ => None,
            })
            .collect();
        let constructors = library
            .declarations
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::TypeConstructor(value) => {
                    Some((value.name.as_str(), value.type_parameters.len()))
                }
                _ => None,
            })
            .collect();
        let generated_items = library
            .declarations
            .iter()
            .flat_map(declaration_items)
            .collect();
        Self {
            library,
            types,
            capabilities,
            constructors,
            generated_items,
            example_owners: HashMap::new(),
            errors: Vec::new(),
        }
    }

    fn validate(&mut self) {
        let mut names = HashSet::new();
        for declaration in &self.library.declarations {
            let name = declaration_name(declaration);
            if !names.insert(name) {
                self.error(format!("standard-library declaration `{name}` is repeated"));
            }
            match declaration {
                Declaration::Struct(value) | Declaration::IntrinsicType(value) => {
                    self.validate_struct(value)
                }
                Declaration::Enum(value) => {
                    let public = !has_attribute(&value.attributes, "testOnly");
                    self.validate_documentation(&value.name, &value.documentation, false, public);
                    self.validate_attributes(
                        &value.name,
                        &value.attributes,
                        &["representation", "valueUsage", "capabilities", "testOnly"],
                    );
                    self.validate_representation(&value.name, &value.attributes, "enum");
                    self.validate_value_usage(&value.name, &value.attributes);
                    self.validate_capabilities(&value.name, &value.attributes);
                    let mut variants = HashSet::new();
                    for variant in &value.variants {
                        if !variants.insert(variant.name.as_str()) {
                            self.error(format!(
                                "enum `{}` repeats variant `{}`",
                                value.name, variant.name
                            ));
                        }
                        self.validate_documentation(
                            &format!("{}.{}", value.name, variant.name),
                            &variant.documentation,
                            false,
                            public,
                        );
                        self.validate_attributes(
                            &format!("{}.{}", value.name, variant.name),
                            &variant.attributes,
                            &[],
                        );
                    }
                }
                Declaration::Root(value) => {
                    self.validate_attributes("root", &value.attributes, &[]);
                    self.validate_functions("root", &value.functions, &[]);
                }
                Declaration::Namespace(value) => self.validate_owner(value, &[], true),
                Declaration::Capability(value) => {
                    self.validate_attributes(&value.name, &value.attributes, &["behavior"]);
                    self.require_name_attribute(
                        &value.name,
                        &value.attributes,
                        "behavior",
                        |name| {
                            matches!(
                                name,
                                "declared" | "structuralEquality" | "structuralMemoryLayout"
                            )
                        },
                    );
                    if value.type_parameters.len() != 1 {
                        self.error(format!(
                            "capability `{}` must declare exactly one type parameter",
                            value.name
                        ));
                    }
                    self.validate_owner(value, &value.type_parameters, true);
                }
                Declaration::TypeConstructor(value) => {
                    self.validate_attributes(&value.name, &value.attributes, &["mustUse"]);
                    self.validate_must_use(&value.name, &value.attributes);
                    self.validate_owner(value, &value.type_parameters, true);
                }
                Declaration::CoreExtension(value) => {
                    self.validate_attributes(&value.name, &value.attributes, &[]);
                    if PrimitiveType::parse(&value.name).is_none() {
                        self.error(format!(
                            "core extension `{}` does not name a primitive type",
                            value.name
                        ));
                    }
                    self.validate_functions(&value.name, &value.functions, &[]);
                }
                Declaration::StateProvider(value) => self.validate_provider(value),
            }
        }
        self.validate_capability_hierarchy();
    }

    fn validate_capability_hierarchy(&mut self) {
        let hierarchy = self
            .library
            .declarations
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::Capability(capability) => Some((
                    capability.name.as_str(),
                    capability
                        .type_parameters
                        .first()
                        .map(|parameter| parameter.constraints.as_slice())
                        .unwrap_or_default(),
                )),
                _ => None,
            })
            .collect::<HashMap<_, _>>();
        let mut completed = HashSet::new();
        for capability in hierarchy.keys().copied() {
            let mut active = HashSet::new();
            if capability_hierarchy_has_cycle(capability, &hierarchy, &mut active, &mut completed) {
                self.error(format!(
                    "capability hierarchy contains a cycle through `{capability}`"
                ));
                break;
            }
        }
    }

    fn validate_struct(&mut self, value: &StructDeclaration) {
        let public = !has_attribute(&value.attributes, "testOnly");
        self.validate_documentation(&value.name, &value.documentation, false, public);
        self.validate_attributes(
            &value.name,
            &value.attributes,
            &["representation", "valueUsage", "capabilities", "testOnly"],
        );
        self.validate_representation(&value.name, &value.attributes, "");
        self.validate_value_usage(&value.name, &value.attributes);
        self.validate_capabilities(&value.name, &value.attributes);
        let mut fields = HashSet::new();
        for field in &value.fields {
            let owner = format!("{}.{}", value.name, field.name);
            if !fields.insert(field.name.as_str()) {
                self.error(format!(
                    "struct `{}` repeats field `{}`",
                    value.name, field.name
                ));
            }
            let public_field = public && !field.private;
            self.validate_documentation(&owner, &field.documentation, false, public_field);
            self.validate_attributes(&owner, &field.attributes, &[]);
            self.validate_type(&owner, &field.ty, &[]);
        }
        let owner = CallableOwnerDeclaration {
            name: value.name.clone(),
            type_parameters: Vec::new(),
            documentation: value.documentation.clone(),
            attributes: value.attributes.clone(),
            functions: value.functions.clone(),
        };
        if owner
            .functions
            .iter()
            .filter(|function| has_attribute(&function.attributes, "display"))
            .count()
            > 1
        {
            self.error(format!(
                "standard-library type `{}` has multiple display implementations",
                owner.name
            ));
        }
        self.validate_functions(&owner.name, &owner.functions, &[]);
    }

    fn validate_owner(
        &mut self,
        owner: &CallableOwnerDeclaration,
        inherited: &[TypeParameter],
        public: bool,
    ) {
        self.validate_documentation(&owner.name, &owner.documentation, false, public);
        self.validate_type_parameters(&owner.name, &owner.type_parameters);
        self.validate_functions(&owner.name, &owner.functions, inherited);
    }

    fn validate_functions(
        &mut self,
        owner: &str,
        functions: &[FunctionDeclaration],
        inherited: &[TypeParameter],
    ) {
        let mut names = HashSet::new();
        let mut operator_bindings = HashSet::new();
        for function in functions {
            let qualified = if owner == "root" {
                function.name.clone()
            } else {
                format!("{owner}.{}", function.name)
            };
            if !names.insert(function.name.as_str()) {
                self.error(format!("`{owner}` repeats function `{}`", function.name));
            }
            self.validate_documentation(&qualified, &function.documentation, true, true);
            self.validate_attributes(
                &qualified,
                &function.attributes,
                &["intrinsic", "display", "mustUse", "operator"],
            );
            self.validate_must_use(&qualified, &function.attributes);
            self.validate_operator(owner, &qualified, function);
            if let Some([AttributeArgument::Name(operator)]) = function
                .attributes
                .iter()
                .find(|attribute| attribute.name == "operator")
                .map(|attribute| attribute.arguments.as_slice())
                && matches!(
                    operator.as_str(),
                    "add"
                        | "subtract"
                        | "lessThan"
                        | "lessThanOrEqual"
                        | "greaterThan"
                        | "greaterThanOrEqual"
                )
                && !operator_bindings.insert(operator.as_str())
            {
                self.error(format!(
                    "`{owner}` declares more than one `{operator}` operator implementation"
                ));
            }
            if has_attribute(&function.attributes, "mustUse")
                && function.result == Type::Name("void".to_owned())
            {
                self.error(format!(
                    "`{qualified}` cannot be `@mustUse` because it returns `void`"
                ));
            }
            if has_attribute(&function.attributes, "display") {
                if function
                    .attributes
                    .iter()
                    .find(|attribute| attribute.name == "display")
                    .is_some_and(|attribute| !attribute.arguments.is_empty())
                {
                    self.error(format!(
                        "`{qualified}` marker attribute `@display` does not accept arguments"
                    ));
                }
                let owner_is_standard_type = self.types.contains(owner);
                let owner_has_display = self.library.declarations.iter().any(|declaration| {
                    matches!(
                        declaration,
                        Declaration::Struct(value) | Declaration::IntrinsicType(value)
                            if value.name == owner
                                && value.attributes.iter().any(|attribute| {
                                    attribute.name == "capabilities"
                                        && attribute.arguments.iter().any(|argument| {
                                            matches!(
                                                argument,
                                                AttributeArgument::Name(capability)
                                                    if capability == "Display"
                                            )
                                        })
                                })
                    )
                });
                if !owner_is_standard_type {
                    self.error(format!(
                        "`{qualified}` uses `@display` outside a standard-library type"
                    ));
                }
                if function.is_static || !function.parameters.is_empty() {
                    self.error(format!(
                        "`{qualified}` display implementation must be a parameterless method"
                    ));
                }
                if function.result != Type::Name("String".to_owned()) {
                    self.error(format!(
                        "`{qualified}` display implementation must return `String`"
                    ));
                }
                if !owner_has_display {
                    self.error(format!(
                        "`{qualified}` provides a display implementation but `{owner}` does not declare `Display`"
                    ));
                }
                if function.body.is_none() {
                    self.error(format!(
                        "`{qualified}` display implementation must have a source body"
                    ));
                }
            }
            let intrinsic = self
                .optional_name_attribute(&qualified, &function.attributes, "intrinsic")
                .is_some();
            match (intrinsic, function.body.is_some()) {
                (true, true) => self.error(format!(
                    "`{qualified}` cannot have both an intrinsic binding and a source body"
                )),
                (false, false) => self.error(format!(
                    "`{qualified}` must have either an intrinsic binding or a source body"
                )),
                _ => {}
            }
            self.validate_type_parameters(&qualified, &function.type_parameters);
            let parameters = if function.type_parameters.is_empty() {
                inherited
            } else {
                &function.type_parameters
            };
            let mut parameter_names = HashSet::new();
            for parameter in &function.parameters {
                let parameter_owner = format!("{qualified}.{}", parameter.name);
                if !parameter_names.insert(parameter.name.as_str()) {
                    self.error(format!(
                        "`{qualified}` repeats parameter `{}`",
                        parameter.name
                    ));
                }
                self.validate_documentation(
                    &parameter_owner,
                    &parameter.documentation,
                    false,
                    false,
                );
                self.validate_attributes(&parameter_owner, &parameter.attributes, &["literal"]);
                if let Some(rule) =
                    self.optional_name_attribute(&parameter_owner, &parameter.attributes, "literal")
                    && !matches!(rule, "string" | "signature")
                {
                    self.error(format!(
                        "`{parameter_owner}` has unknown literal rule `{rule}`"
                    ));
                }
                self.validate_type(&parameter_owner, &parameter.ty, parameters);
            }
            self.validate_type(&format!("{qualified} result"), &function.result, parameters);
        }
    }

    fn validate_operator(&mut self, owner: &str, qualified: &str, function: &FunctionDeclaration) {
        let Some(operator) = function
            .attributes
            .iter()
            .find(|attribute| attribute.name == "operator")
        else {
            return;
        };
        let valid_name = matches!(
            operator.arguments.as_slice(),
            [AttributeArgument::Name(name)] if matches!(
                name.as_str(),
                "add"
                    | "subtract"
                    | "lessThan"
                    | "lessThanOrEqual"
                    | "greaterThan"
                    | "greaterThanOrEqual"
            )
        );
        if !valid_name {
            self.error(format!(
                "`{qualified}` attribute `@operator` expects a supported binary operator name"
            ));
        }
        let owner_supports_methods = self.types.contains(owner)
            || self.capabilities.contains(owner)
            || PrimitiveType::parse(owner).is_some();
        if !owner_supports_methods || function.is_static || function.parameters.len() != 1 {
            self.error(format!(
                "`{qualified}` operator implementation must be a method with exactly one parameter"
            ));
        }
    }

    fn validate_provider(&mut self, value: &crate::StateProviderDeclaration) {
        self.validate_documentation(&value.name, &value.documentation, true, true);
        self.validate_attributes(
            &value.name,
            &value.attributes,
            &["processType", "processes", "attachment", "directRead"],
        );
        if let Some(name) =
            self.optional_name_attribute(&value.name, &value.attributes, "processType")
        {
            if !self.types.contains(name) {
                self.error(format!(
                    "`{}` has invalid `@processType({name})`",
                    value.name
                ));
            }
        } else {
            self.error(format!("`{}` is missing `@processType(...)`", value.name));
        }
        self.require_name_attribute(&value.name, &value.attributes, "attachment", |_| true);
        let process_mode =
            self.optional_name_attribute(&value.name, &value.attributes, "processes");
        let source_processes = process_mode == Some("source");
        if let Some(mode) = process_mode
            && mode != "source"
        {
            self.error(format!(
                "state provider `{}` has unknown process source `{mode}`; expected `source`",
                value.name
            ));
        }
        if let Some(name) =
            self.optional_name_attribute(&value.name, &value.attributes, "directRead")
        {
            if !self.generated_items.contains(name) {
                self.error(format!(
                    "`{}` has invalid `@directRead({name})`",
                    value.name
                ));
            }
        } else {
            self.error(format!("`{}` is missing `@directRead(...)`", value.name));
        }
        if value.processes.is_empty() && !source_processes {
            self.error(format!(
                "state provider `{}` declares no processes",
                value.name
            ));
        }
        if source_processes && !value.processes.is_empty() {
            self.error(format!(
                "state provider `{}` uses `@processes(source)` and cannot also declare process names",
                value.name
            ));
        }
    }

    fn validate_type_parameters(&mut self, owner: &str, parameters: &[TypeParameter]) {
        let mut names = HashSet::new();
        for parameter in parameters {
            if !names.insert(parameter.name.as_str()) {
                self.error(format!(
                    "`{owner}` repeats type parameter `{}`",
                    parameter.name
                ));
            }
            let mut constraints = HashSet::new();
            for constraint in &parameter.constraints {
                if !constraints.insert(constraint.as_str()) {
                    self.error(format!(
                        "`{owner}` repeats capability constraint `{constraint}`"
                    ));
                }
                if !self.capabilities.contains(constraint.as_str()) {
                    self.error(format!(
                        "`{owner}` references unknown capability `{constraint}`"
                    ));
                }
            }
        }
    }

    fn validate_type(&mut self, owner: &str, ty: &Type, parameters: &[TypeParameter]) {
        match ty {
            Type::Name(name)
                if PrimitiveType::parse(name).is_some()
                    || self.types.contains(name.as_str())
                    || parameters.iter().any(|parameter| parameter.name == *name) => {}
            Type::Name(name) => self.error(format!("`{owner}` references unknown type `{name}`")),
            Type::Array(element) => {
                self.require_constructor(owner, "Array", 1);
                self.validate_type(owner, element, parameters);
            }
            Type::FixedArray { element, .. } => {
                self.require_constructor(owner, "Array", 1);
                self.validate_type(owner, element, parameters);
            }
            Type::Option(value) => {
                self.require_constructor(owner, "Option", 1);
                self.validate_type(owner, value, parameters);
            }
            Type::Result(value) => {
                self.require_constructor(owner, "Result", 1);
                self.validate_type(owner, value, parameters);
            }
            Type::Application {
                constructor,
                arguments,
            } => {
                self.require_constructor(owner, constructor, arguments.len());
                for argument in arguments {
                    self.validate_type(owner, argument, parameters);
                }
            }
        }
    }

    fn require_constructor(&mut self, owner: &str, name: &str, arity: usize) {
        match self.constructors.get(name).copied() {
            Some(expected) if expected == arity => {}
            Some(expected) => self.error(format!(
                "`{owner}` applies `{name}` to {arity} types, but it expects {expected}"
            )),
            None => self.error(format!(
                "`{owner}` references unknown type constructor `{name}`"
            )),
        }
    }

    fn validate_representation(&mut self, owner: &str, attributes: &[Attribute], kind: &str) {
        let values = self.attribute_names(owner, attributes, "representation", true);
        let valid = match values.as_slice() {
            [name, storage] if name == "scalar" => PrimitiveType::parse(storage).is_some(),
            [name, element, rest @ ..] if name == "gcArray" => {
                PrimitiveType::parse(element).is_some()
                    && rest
                        .iter()
                        .all(|value| matches!(value.as_str(), "mutable" | "nullable"))
            }
            [name, rest @ ..] if name == "gcStruct" || name == "enum" => {
                rest.iter().all(|value| value == "nullable")
            }
            _ => false,
        };
        if !valid || (!kind.is_empty() && values.first().map(String::as_str) != Some(kind)) {
            self.error(format!("`{owner}` has an invalid runtime representation"));
        }
    }

    fn validate_value_usage(&mut self, owner: &str, attributes: &[Attribute]) {
        let values = self.attribute_names(owner, attributes, "valueUsage", true);
        if values.iter().any(|value| {
            !matches!(
                value.as_str(),
                "recordField" | "enumPayload" | "stateField" | "localVariable" | "globalVariable"
            )
        }) {
            self.error(format!("`{owner}` has an invalid value-usage rule"));
        }
    }

    fn validate_capabilities(&mut self, owner: &str, attributes: &[Attribute]) {
        for capability in self.attribute_names(owner, attributes, "capabilities", false) {
            if !self.capabilities.contains(capability.as_str()) {
                self.error(format!(
                    "`{owner}` references unknown capability `{capability}`"
                ));
            }
        }
    }

    fn validate_documentation(
        &mut self,
        owner: &str,
        docs: &Documentation,
        require_details: bool,
        require_example: bool,
    ) {
        if docs.summary.trim().is_empty() || (require_details && docs.details.trim().is_empty()) {
            self.error(format!("`{owner}` has incomplete documentation"));
        }
        if require_example && docs.examples.len() != 1 {
            self.error(format!(
                "`{owner}` must have exactly one focused documentation example"
            ));
        } else if !require_example && docs.examples.len() > 1 {
            self.error(format!(
                "`{owner}` must not have more than one focused documentation example"
            ));
        }
        for value in &docs.examples {
            if value.title.trim().is_empty() || value.source.trim().is_empty() {
                self.error(format!("`{owner}` has an incomplete documentation example"));
            }
            if let Some(provider) = &value.state_provider
                && !self.library.declarations.iter().any(|declaration| {
                    matches!(
                        declaration,
                        Declaration::StateProvider(candidate) if candidate.name == *provider
                    )
                })
            {
                self.error(format!(
                    "`{owner}` documentation example references unknown state provider `{provider}`"
                ));
            }
            if let Some(previous) = self
                .example_owners
                .insert(value.source.clone(), owner.to_owned())
            {
                self.error(format!(
                    "`{owner}` reuses the visible documentation example from `{previous}`"
                ));
            }
        }
    }

    fn validate_must_use(&mut self, owner: &str, attributes: &[Attribute]) {
        let Some(attribute) = attributes
            .iter()
            .find(|attribute| attribute.name == "mustUse")
        else {
            return;
        };
        match attribute.arguments.as_slice() {
            [AttributeArgument::String(reason)] if !reason.trim().is_empty() => {}
            _ => self.error(format!(
                "`{owner}` attribute `@mustUse` expects one non-empty string argument"
            )),
        }
    }

    fn validate_attributes(&mut self, owner: &str, attributes: &[Attribute], known: &[&str]) {
        let mut names = HashSet::new();
        for attribute in attributes {
            if !names.insert(attribute.name.as_str()) {
                self.error(format!("`{owner}` repeats attribute `@{}`", attribute.name));
            }
            if !known.contains(&attribute.name.as_str()) {
                self.error(format!(
                    "`{owner}` uses unknown privileged attribute `@{}`",
                    attribute.name
                ));
            }
        }
    }

    fn require_name_attribute(
        &mut self,
        owner: &str,
        attributes: &[Attribute],
        name: &str,
        valid: impl FnOnce(&str) -> bool,
    ) {
        let Some(value) = self.optional_name_attribute(owner, attributes, name) else {
            self.error(format!("`{owner}` is missing `@{name}(...)`"));
            return;
        };
        if !valid(value) {
            self.error(format!("`{owner}` has invalid `@{name}({value})`"));
        }
    }

    fn optional_name_attribute<'b>(
        &mut self,
        owner: &str,
        attributes: &'b [Attribute],
        name: &str,
    ) -> Option<&'b str> {
        let attribute = attributes.iter().find(|attribute| attribute.name == name)?;
        match attribute.arguments.as_slice() {
            [AttributeArgument::Name(value)] => Some(value),
            _ => {
                self.error(format!(
                    "`{owner}` attribute `@{name}` expects one name argument"
                ));
                None
            }
        }
    }

    fn attribute_names(
        &mut self,
        owner: &str,
        attributes: &[Attribute],
        name: &str,
        required: bool,
    ) -> Vec<String> {
        let Some(attribute) = attributes.iter().find(|attribute| attribute.name == name) else {
            if required {
                self.error(format!("`{owner}` is missing `@{name}(...)`"));
            }
            return Vec::new();
        };
        let mut values = Vec::new();
        for argument in &attribute.arguments {
            match argument {
                AttributeArgument::Name(value) => values.push(value.clone()),
                AttributeArgument::String(_) => self.error(format!(
                    "`{owner}` attribute `@{name}` expects name arguments"
                )),
            }
        }
        values
    }

    fn error(&mut self, message: String) {
        self.errors.push(Error {
            message,
            start: 0,
            end: 0,
        });
    }
}

fn declaration_name(declaration: &Declaration) -> &str {
    match declaration {
        Declaration::Struct(value) | Declaration::IntrinsicType(value) => &value.name,
        Declaration::Enum(value) => &value.name,
        Declaration::Root(value)
        | Declaration::Namespace(value)
        | Declaration::Capability(value)
        | Declaration::TypeConstructor(value)
        | Declaration::CoreExtension(value) => &value.name,
        Declaration::StateProvider(value) => &value.name,
    }
}

fn has_attribute(attributes: &[Attribute], name: &str) -> bool {
    attributes.iter().any(|attribute| attribute.name == name)
}

fn capability_hierarchy_has_cycle<'a>(
    capability: &'a str,
    hierarchy: &HashMap<&'a str, &'a [String]>,
    active: &mut HashSet<&'a str>,
    completed: &mut HashSet<&'a str>,
) -> bool {
    if completed.contains(capability) {
        return false;
    }
    if !active.insert(capability) {
        return true;
    }
    let cyclic = hierarchy
        .get(capability)
        .into_iter()
        .flat_map(|supers| supers.iter())
        .filter(|super_capability| hierarchy.contains_key(super_capability.as_str()))
        .any(|super_capability| {
            capability_hierarchy_has_cycle(super_capability, hierarchy, active, completed)
        });
    active.remove(capability);
    completed.insert(capability);
    cyclic
}

fn declaration_items(declaration: &Declaration) -> Vec<String> {
    let (prefix, functions) = match declaration {
        Declaration::Root(value) => (String::new(), value.functions.as_slice()),
        Declaration::Namespace(value)
        | Declaration::Capability(value)
        | Declaration::TypeConstructor(value)
        | Declaration::CoreExtension(value) => (id_path(&value.name), value.functions.as_slice()),
        Declaration::Struct(value) | Declaration::IntrinsicType(value) => {
            (id(&value.name), value.functions.as_slice())
        }
        Declaration::Enum(_) | Declaration::StateProvider(_) => return Vec::new(),
    };
    functions
        .iter()
        .map(|function| format!("{prefix}{}", id(&function.name)))
        .collect()
}

fn id_path(path: &str) -> String {
    path.split('.').map(id).collect()
}

fn id(name: &str) -> String {
    if name
        .chars()
        .all(|character| !character.is_ascii_lowercase())
    {
        let mut characters = name.chars();
        let Some(first) = characters.next() else {
            return String::new();
        };
        let mut result = first.to_ascii_uppercase().to_string();
        result.extend(characters.map(|character| character.to_ascii_lowercase()));
        result
    } else {
        let mut characters = name.chars();
        let Some(first) = characters.next() else {
            return String::new();
        };
        let mut result = first.to_ascii_uppercase().to_string();
        let mut after_digit = false;
        for character in characters {
            if after_digit && character.is_ascii_alphabetic() {
                result.push(character.to_ascii_uppercase());
                after_digit = false;
            } else {
                result.push(character);
                after_digit = character.is_ascii_digit();
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use crate::{generate_catalog, parse};

    #[test]
    fn invalid_type_references_are_reported_before_generation() {
        let source = r#"
/// Arrays.
typeConstructor Array<T> {}
/// A value.
@representation(gcStruct)
@valueUsage(localVariable)
struct Value {
    /// Missing.
    field: Missing,
}
"#;
        let errors = generate_catalog(&parse(source).unwrap()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("unknown type `Missing`")),
            "{errors:#?}"
        );
    }

    #[test]
    fn missing_callable_examples_are_structured_errors_not_panics() {
        let source = r#"
root {
    /// Prints.
    @intrinsic(Print)
    fn print() -> void;
}
"#;
        let errors = generate_catalog(&parse(source).unwrap()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("exactly one focused")),
            "{errors:#?}"
        );
    }

    #[test]
    fn public_declarations_require_focused_examples() {
        let source = r#"
/// A documented value.
///
/// Represents a public standard-library value.
@representation(gcStruct)
@valueUsage(localVariable)
struct Value {
    /// A public field.
    ///
    /// Stores the value.
    field: u32,
}
"#;
        let errors = generate_catalog(&parse(source).unwrap()).unwrap_err();
        assert!(errors.iter().any(|error| {
            error
                .message
                .contains("`Value` must have exactly one focused")
        }));
        assert!(errors.iter().any(|error| {
            error
                .message
                .contains("`Value.field` must have exactly one focused")
        }));
    }

    #[test]
    fn example_context_must_name_a_declared_state_provider() {
        let source = r#"
/// Values.
///
/// Provides documented values.
///
/// # Example
///
/// Read a value
///
/// ```splitscript state Missing
/// let value = 1
/// ```
namespace values {}
"#;
        let errors = generate_catalog(&parse(source).unwrap()).unwrap_err();
        assert!(errors.iter().any(|error| {
            error
                .message
                .contains("references unknown state provider `Missing`")
        }));
    }

    #[test]
    fn visible_examples_must_be_distinct() {
        let source = r#"
/// First namespace.
///
/// Provides the first API.
///
/// # Example
///
/// Use it
///
/// ```splitscript
/// let value = 1
/// ```
namespace first {}

/// Second namespace.
///
/// Provides the second API.
///
/// # Example
///
/// Use it too
///
/// ```splitscript
/// let value = 1
/// ```
namespace second {}
"#;
        let errors = generate_catalog(&parse(source).unwrap()).unwrap_err();
        assert!(errors.iter().any(|error| {
            error
                .message
                .contains("reuses the visible documentation example")
        }));
    }

    #[test]
    fn must_use_requires_a_reason_and_a_returned_value() {
        let malformed = r#"
/// Optional values.
@mustUse("")
typeConstructor Option<T> {}
"#;
        let errors = generate_catalog(&parse(malformed).unwrap()).unwrap_err();
        assert!(errors.iter().any(|error| {
            error
                .message
                .contains("expects one non-empty string argument")
        }));

        let void_callable = r#"
root {
    /// Prints a message.
    ///
    /// Writes diagnostic output.
    ///
    /// # Example
    ///
    /// Print
    ///
    /// ```splitscript
    /// print()
    /// ```
    @mustUse("Observe this value.")
    @intrinsic(Print)
    fn print() -> void;
}
"#;
        let errors = generate_catalog(&parse(void_callable).unwrap()).unwrap_err();
        assert!(errors.iter().any(|error| {
            error
                .message
                .contains("cannot be `@mustUse` because it returns `void`")
        }));
    }

    #[test]
    fn callable_implementations_are_exclusive_and_required() {
        let both = r#"
root {
    /// Prints.
    ///
    /// Prints a message.
    ///
    /// # Example
    ///
    /// Print
    ///
    /// ```splitscript
    /// print()
    /// ```
    @intrinsic(Print)
    fn print() -> void {}
}
"#;
        let errors = generate_catalog(&parse(both).unwrap()).unwrap_err();
        assert!(errors.iter().any(|error| error.message.contains("both")));

        let neither = both
            .replace("    @intrinsic(Print)\n", "")
            .replace(" {}", ";");
        let errors = generate_catalog(&parse(&neither).unwrap()).unwrap_err();
        assert!(errors.iter().any(|error| error.message.contains("either")));
    }

    #[test]
    fn generic_body_accepts_constructed_parameter_shapes() {
        let source = r#"
/// Arrays.
///
/// # Example
///
/// Store values
///
/// ```splitscript
/// let values: [u32] = []
/// ```
typeConstructor Array<T> {}

/// Values.
///
/// # Example
///
/// Use values
///
/// ```splitscript
/// let copiedValues: [u32] = []
/// ```
@behavior(declared)
capability Values<T> {
    /// Copies values.
    ///
    /// Returns the input values.
    ///
    /// # Example
    ///
    /// Copy values
    ///
    /// ```splitscript
    /// let copied = values.copy()
    /// ```
    fn copy(
        /// The values to copy.
        values: [T],
    ) -> [T] {
        return values
    }
}
"#;
        let generated = generate_catalog(&parse(source).unwrap()).unwrap();
        assert!(generated.contains("Implementation::LibraryBody"));
        assert!(generated.contains("TypeRef::Application"));
    }

    #[test]
    fn capability_hierarchy_rejects_cycles() {
        let source = r#"
/// First capability.
@behavior(declared)
capability First<T: Second> {}
/// Second capability.
@behavior(declared)
capability Second<T: First> {}
"#;
        let errors = generate_catalog(&parse(source).unwrap()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("hierarchy contains a cycle")),
            "{errors:#?}"
        );
    }
}
