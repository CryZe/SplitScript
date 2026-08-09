use std::collections::HashSet;

use splitscript_syntax::PrimitiveType;

use crate::{
    Attribute, AttributeArgument, CallableOwnerDeclaration, Declaration, Error,
    FunctionDeclaration, Library, StructDeclaration, Type, TypeParameter,
};

pub fn generate_catalog(library: &Library) -> Result<String, Vec<Error>> {
    let generator = CatalogGenerator::new(library)?;
    Ok(generator.generate())
}

struct CatalogGenerator<'a> {
    library: &'a Library,
    type_names: HashSet<&'a str>,
    capability_names: HashSet<&'a str>,
}

impl<'a> CatalogGenerator<'a> {
    fn new(library: &'a Library) -> Result<Self, Vec<Error>> {
        let errors = crate::validation::validate(library);
        if !errors.is_empty() {
            return Err(errors);
        }
        let type_names = library
            .declarations
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::Struct(declaration) | Declaration::IntrinsicType(declaration) => {
                    Some(declaration.name.as_str())
                }
                Declaration::Enum(declaration) => Some(declaration.name.as_str()),
                _ => None,
            })
            .collect();
        let capability_names = library
            .declarations
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::Capability(declaration) => Some(declaration.name.as_str()),
                _ => None,
            })
            .collect();
        Ok(Self {
            library,
            type_names,
            capability_names,
        })
    }

    fn generate(&self) -> String {
        let mut output = String::new();
        output.push_str("pub(super) const STATE_PROVIDERS: &[StdlibStateProvider] = &[\n");
        for declaration in &self.library.declarations {
            if let Declaration::StateProvider(provider) = declaration {
                let process_type = attribute_name(&provider.attributes, "processType");
                let attachment = attribute_name(&provider.attributes, "attachment");
                let direct_read = attribute_name(&provider.attributes, "directRead");
                let source_processes =
                    optional_attribute_name(&provider.attributes, "processes") == Some("source");
                let processes = if source_processes {
                    "StateProviderProcesses::SourceState".to_owned()
                } else {
                    format!(
                        "StateProviderProcesses::Declared(&[{}])",
                        provider
                            .processes
                            .iter()
                            .map(|process| quote(process))
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                };
                let attachment = if attachment == "identity" {
                    "StateProviderAttachment::Identity".to_owned()
                } else {
                    format!("StateProviderAttachment::Callable(StdlibItemId::{attachment})")
                };
                output.push_str(&format!(
                    "StdlibStateProvider {{ id: StdlibStateProviderId::{}, name: {}, value_name: {}, processes: {}, process_type: StdlibTypeId::{}, attachment: {}, direct_read: StdlibItemId::{}, documentation: Documentation {{ summary: {}, details: {}, examples: &[Example::checked({}, {}, {})], related: &[] }} }},\n",
                    ident(&provider.name),
                    quote(&provider.name),
                    quote(&provider.value_name),
                    processes,
                    process_type,
                    attachment,
                    direct_read,
                    quote(&provider.documentation.summary),
                    quote(&provider.documentation.details),
                    quote(&provider.documentation.examples[0].title),
                    quote(&provider.documentation.examples[0].source),
                    quote(&provider.documentation.examples[0].source),
                ));
            }
        }
        output.push_str("];\n\npub(super) const CAPABILITIES: &[StdlibCapability] = &[\n");
        for declaration in &self.library.declarations {
            if let Declaration::Capability(owner) = declaration {
                let behavior = match attribute_name(&owner.attributes, "behavior") {
                    "declared" => "Declared",
                    "structuralEquality" => "StructuralEquality",
                    "structuralMemoryLayout" => "StructuralMemoryLayout",
                    other => other,
                };
                let super_capabilities = owner
                    .type_parameters
                    .first()
                    .map(|parameter| parameter.constraints.as_slice())
                    .unwrap_or_default()
                    .iter()
                    .map(|capability| format!("StdlibCapabilityId::{}", ident(capability)))
                    .collect::<Vec<_>>()
                    .join(",");
                output.push_str(&format!(
                    "StdlibCapability {{ id: StdlibCapabilityId::{}, name: {}, super_capabilities: &[{super_capabilities}], behavior: CapabilityBehavior::{behavior}, documentation: {} }},\n",
                    ident(&owner.name), quote(&owner.name),
                    self.documentation(&owner.documentation)
                ));
            }
        }
        output
            .push_str("];\n\npub(super) const TYPE_CONSTRUCTORS: &[StdlibTypeConstructor] = &[\n");
        for declaration in &self.library.declarations {
            if let Declaration::TypeConstructor(owner) = declaration {
                let id = ident(&owner.name);
                let must_use = optional_attribute_name(&owner.attributes, "mustUse")
                    .map(|reason| format!("Some({})", quote(reason)))
                    .unwrap_or_else(|| "None".to_owned());
                output.push_str(&format!(
                    "StdlibTypeConstructor {{ id: StdlibTypeConstructorId::{id}, name: {}, parameters: {}, must_use: {must_use}, documentation: {} }},\n",
                    quote(&owner.name),
                    self.type_parameters(&owner.type_parameters, owner),
                    self.documentation(&owner.documentation)
                ));
            }
        }
        output.push_str("];\n\npub(super) const NAMESPACES: &[StdlibNamespace] = &[\n");
        for declaration in &self.library.declarations {
            if let Declaration::Namespace(owner) = declaration {
                let id = path_ident(&owner.name);
                output.push_str(&format!(
                    "StdlibNamespace {{ id: StdlibNamespaceId::{id}, name: {}, path: &[{}], documentation: {} }},\n",
                    quote(owner.name.rsplit('.').next().unwrap()),
                    owner.name.split('.').map(quote).collect::<Vec<_>>().join(","),
                    self.documentation(&owner.documentation)
                ));
            }
        }
        output.push_str("];\n\npub(super) const TYPES: &[StdlibType] = &[\n");
        for declaration in &self.library.declarations {
            match declaration {
                Declaration::Struct(declaration) => {
                    self.emit_type(&mut output, declaration, "Struct")
                }
                Declaration::IntrinsicType(declaration) => {
                    self.emit_type(&mut output, declaration, "Intrinsic")
                }
                Declaration::Enum(declaration) => self.emit_enum_type(&mut output, declaration),
                _ => {}
            }
        }
        output.push_str("];\n\npub(super) const FIELDS: &[StdlibField] = &[\n");
        for declaration in &self.library.declarations {
            if let Declaration::Struct(declaration) | Declaration::IntrinsicType(declaration) =
                declaration
            {
                self.emit_fields(&mut output, declaration);
            }
        }
        output.push_str("];\n\npub(super) const VARIANTS: &[StdlibVariant] = &[\n");
        for declaration in &self.library.declarations {
            if let Declaration::Enum(declaration) = declaration {
                let owner = ident(&declaration.name);
                for variant in &declaration.variants {
                    output.push_str(&format!(
                        "StdlibVariant {{ id: StdlibVariantId::{}{}, owner: StdlibTypeId::{owner}, name: {}, documentation: {} }},\n",
                        owner, ident(&variant.name), quote(&variant.name),
                        self.documentation(&variant.documentation)
                    ));
                }
            }
        }
        output.push_str("];\n\npub(super) const ITEMS: &[StdlibItem] = &[\n");
        self.emit_all_items(&mut output);
        output.push_str("];\n");
        output
    }

    fn documentation(&self, documentation: &crate::Documentation) -> String {
        let examples = documentation
            .examples
            .iter()
            .map(|example| {
                example.state_provider.as_ref().map_or_else(
                    || {
                        format!(
                            "Example::on_attach_body({}, {})",
                            quote(&example.title),
                            quote(&example.source)
                        )
                    },
                    |provider| {
                        format!(
                            "Example::provider_on_attach_body({}, {}, {})",
                            quote(&example.title),
                            quote(&example.source),
                            quote(provider)
                        )
                    },
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "Documentation {{ summary: {}, details: {}, examples: &[{examples}], related: &[] }}",
            quote(&documentation.summary),
            quote(if documentation.details.is_empty() {
                &documentation.summary
            } else {
                &documentation.details
            })
        )
    }

    fn emit_type(&self, output: &mut String, declaration: &StructDeclaration, kind: &str) {
        let id = ident(&declaration.name);
        let display = declaration
            .functions
            .iter()
            .find(|function| has_attribute(&function.attributes, "display"))
            .map_or_else(
                || "None".to_owned(),
                |function| {
                    format!(
                        "Some(StdlibItemId::{}{})",
                        path_ident(&declaration.name),
                        ident(&function.name)
                    )
                },
            );
        if has_attribute(&declaration.attributes, "testOnly") {
            output.push_str("#[cfg(test)] ");
        }
        output.push_str(&format!(
            "StdlibType {{ id: StdlibTypeId::{id}, name: {}, kind: StdlibTypeKind::{kind}, capabilities: {}, display: {display}, representation: {}, value_usage: {}, documentation: {} }},\n",
            quote(&declaration.name), self.capabilities(&declaration.attributes),
            self.representation(&declaration.attributes), self.value_usage(&declaration.attributes),
            self.documentation(&declaration.documentation)
        ));
    }

    fn emit_enum_type(&self, output: &mut String, declaration: &crate::EnumDeclaration) {
        let id = ident(&declaration.name);
        output.push_str(&format!(
            "StdlibType {{ id: StdlibTypeId::{id}, name: {}, kind: StdlibTypeKind::Enum, capabilities: {}, display: None, representation: {}, value_usage: {}, documentation: {} }},\n",
            quote(&declaration.name), self.capabilities(&declaration.attributes),
            self.representation(&declaration.attributes), self.value_usage(&declaration.attributes),
            self.documentation(&declaration.documentation)
        ));
    }

    fn emit_fields(&self, output: &mut String, declaration: &StructDeclaration) {
        let owner = ident(&declaration.name);
        let test_only = has_attribute(&declaration.attributes, "testOnly");
        for field in &declaration.fields {
            if test_only {
                output.push_str("#[cfg(test)] ");
            }
            output.push_str(&format!(
                "StdlibField {{ id: StdlibFieldId::{}{}, owner: StdlibTypeId::{owner}, name: {}, ty: {}, visibility: FieldVisibility::{}, documentation: {} }},\n",
                owner, ident(&field.name),
                quote(&field.name),
                self.type_ref(&field.ty, &[]),
                if field.private {
                    "RuntimePrivate"
                } else {
                    "Public"
                },
                self.documentation(&field.documentation)
            ));
        }
    }

    fn emit_all_items(&self, output: &mut String) {
        for declaration in &self.library.declarations {
            match declaration {
                Declaration::Root(owner) => self.emit_functions(
                    output,
                    owner,
                    "",
                    "StdlibOwner::Root",
                    "TypeRef::Core(CoreTypeId::None)",
                    None,
                ),
                Declaration::Namespace(owner) => self.emit_functions(
                    output,
                    owner,
                    &owner.name,
                    &format!(
                        "StdlibOwner::Namespace(StdlibNamespaceId::{})",
                        path_ident(&owner.name)
                    ),
                    "TypeRef::Core(CoreTypeId::None)",
                    None,
                ),
                Declaration::Capability(owner) => self.emit_functions(
                    output,
                    owner,
                    &owner.name,
                    &format!(
                        "StdlibOwner::Capability(StdlibCapabilityId::{})",
                        ident(&owner.name)
                    ),
                    &format!(
                        "TypeRef::Parameter({})",
                        quote(
                            &owner
                                .type_parameters
                                .first()
                                .expect("validated capabilities have one type parameter")
                                .name
                        )
                    ),
                    Some(&owner.type_parameters),
                ),
                Declaration::TypeConstructor(owner) => self.emit_functions(
                    output,
                    owner,
                    &owner.name,
                    &format!(
                        "StdlibOwner::TypeConstructor(StdlibTypeConstructorId::{})",
                        ident(&owner.name)
                    ),
                    &self.application_type(&owner.name, &owner.type_parameters),
                    Some(&owner.type_parameters),
                ),
                Declaration::CoreExtension(owner) => self.emit_functions(
                    output,
                    owner,
                    &owner.name,
                    &format!("StdlibOwner::Core(CoreTypeId::{})", ident(&owner.name)),
                    &format!("TypeRef::Core(CoreTypeId::{})", ident(&owner.name)),
                    None,
                ),
                Declaration::Struct(declaration) | Declaration::IntrinsicType(declaration) => {
                    let owner = CallableOwnerDeclaration {
                        name: declaration.name.clone(),
                        type_parameters: Vec::new(),
                        documentation: declaration.documentation.clone(),
                        attributes: declaration.attributes.clone(),
                        functions: declaration.functions.clone(),
                    };
                    self.emit_functions(
                        output,
                        &owner,
                        &declaration.name,
                        &format!(
                            "StdlibOwner::Type(StdlibTypeId::{})",
                            ident(&declaration.name)
                        ),
                        &format!(
                            "TypeRef::Standard(StdlibTypeId::{})",
                            ident(&declaration.name)
                        ),
                        None,
                    );
                }
                Declaration::Enum(_) | Declaration::StateProvider(_) => {}
            }
        }
    }

    fn emit_functions(
        &self,
        output: &mut String,
        owner: &CallableOwnerDeclaration,
        prefix: &str,
        owner_expression: &str,
        receiver: &str,
        inherited: Option<&[TypeParameter]>,
    ) {
        for function in &owner.functions {
            let id_prefix = if prefix.is_empty() {
                String::new()
            } else {
                path_ident(prefix)
            };
            let id = format!("{id_prefix}{}", ident(&function.name));
            let intrinsic = optional_attribute_name(&function.attributes, "intrinsic");
            let mut type_parameters = if function.type_parameters.is_empty() {
                inherited.unwrap_or_default().to_vec()
            } else {
                function.type_parameters.clone()
            };
            for constrained in &function.where_constraints {
                let parameter = type_parameters
                    .iter_mut()
                    .find(|parameter| parameter.name == constrained.name)
                    .expect("validated where clauses reference an available type parameter");
                parameter
                    .constraints
                    .extend(constrained.constraints.clone());
            }
            let kind = if function.is_static {
                "ItemKind::Function".to_owned()
            } else {
                format!("ItemKind::Method {{ receiver: {receiver} }}")
            };
            if has_attribute(&owner.attributes, "testOnly") {
                output.push_str("#[cfg(test)] ");
            }
            self.emit_item(
                output,
                function,
                owner,
                &id,
                intrinsic,
                &type_parameters,
                owner_expression,
                prefix,
                &kind,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_item(
        &self,
        output: &mut String,
        function: &FunctionDeclaration,
        owner: &CallableOwnerDeclaration,
        id: &str,
        intrinsic: Option<&str>,
        type_parameters: &[TypeParameter],
        owner_expression: &str,
        prefix: &str,
        kind: &str,
    ) {
        let qualified_name = if prefix.is_empty() {
            function.name.clone()
        } else {
            format!("{prefix}.{}", function.name)
        };
        let implementation = if let Some(intrinsic) = intrinsic {
            format!("Implementation::Intrinsic(IntrinsicId::{intrinsic})")
        } else {
            let function_name = format!("__splitscript_stdlib_{id}");
            format!(
                "Implementation::LibraryBody {{ function_name: {}, body: {} }}",
                quote(&function_name),
                quote(
                    function
                        .body
                        .as_deref()
                        .expect("validated source-defined functions have bodies")
                ),
            )
        };
        let must_use = optional_attribute_name(&function.attributes, "mustUse")
            .map(|reason| format!("Some({})", quote(reason)))
            .unwrap_or_else(|| "None".to_owned());
        let binary_operator = optional_attribute_name(&function.attributes, "operator")
            .map(|operator| match operator {
                "add" => "Some(StandardBinaryOperator::Add)",
                "subtract" => "Some(StandardBinaryOperator::Subtract)",
                "lessThan" => "Some(StandardBinaryOperator::LessThan)",
                "lessThanOrEqual" => "Some(StandardBinaryOperator::LessThanOrEqual)",
                "greaterThan" => "Some(StandardBinaryOperator::GreaterThan)",
                "greaterThanOrEqual" => "Some(StandardBinaryOperator::GreaterThanOrEqual)",
                _ => unreachable!("validated operator binding"),
            })
            .unwrap_or("None");
        output.push_str(&format!(
                "StdlibItem {{ id: StdlibItemId::{id}, owner: {owner_expression}, name: {}, qualified_name: {}, kind: {kind}, binary_operator: {binary_operator}, signature: Signature {{ type_parameters: {}, explicit_type_parameters: {}, parameters: &[{}], result: {} }}, must_use: {must_use}, deprecation: None, documentation: Documentation {{ summary: {}, details: {}, examples: &[Example::checked({}, {}, validation_fixture(StdlibItemId::{id}))], related: &[] }}, implementation: {implementation} }},\n",
                quote(&function.name),
                quote(&qualified_name),
                self.type_parameters(type_parameters, owner),
                function.type_parameters.len(),
                function.parameters.iter().map(|parameter| self.parameter(parameter, type_parameters)).collect::<Vec<_>>().join(","),
                self.type_ref(&function.result, type_parameters),
                quote(&function.documentation.summary), quote(&function.documentation.details),
                quote(&function.documentation.examples[0].title), quote(&function.documentation.examples[0].source)
            ));
    }

    fn type_parameters(
        &self,
        parameters: &[TypeParameter],
        owner: &CallableOwnerDeclaration,
    ) -> String {
        let values = parameters.iter().map(|parameter| {
            let constraints = if self.library.declarations.iter().any(|declaration| matches!(declaration, Declaration::Capability(candidate) if candidate.name == owner.name)) {
                vec![owner.name.clone()]
            } else if !parameter.constraints.is_empty() {
                parameter.constraints.clone()
            } else {
                Vec::new()
            };
            format!(
                "TypeParameter {{ name: {}, constraints: &[{}] }}",
                quote(&parameter.name),
                constraints.iter().map(|constraint| format!("StdlibCapabilityId::{}", ident(constraint))).collect::<Vec<_>>().join(",")
            )
        }).collect::<Vec<_>>();
        format!("&[{}]", values.join(","))
    }

    fn parameter(&self, parameter: &crate::Parameter, type_parameters: &[TypeParameter]) -> String {
        let rule = optional_attribute_name(&parameter.attributes, "literal");
        let constructor = if rule.is_some() {
            "literal_parameter"
        } else {
            "parameter"
        };
        let rule = rule
            .map(|rule| match rule {
                "string" => ", ParameterRule::StringLiteral",
                "signature" => ", ParameterRule::SignatureLiteral",
                _ => "",
            })
            .unwrap_or("");
        format!(
            "{constructor}({}, {}{rule}, {})",
            quote(&parameter.name),
            self.type_ref(&parameter.ty, type_parameters),
            quote(&parameter.documentation.summary)
        )
    }

    fn application_type(&self, constructor: &str, parameters: &[TypeParameter]) -> String {
        format!(
            "TypeRef::Application {{ constructor: StdlibTypeConstructorId::{}, arguments: &[{}] }}",
            ident(constructor),
            parameters
                .iter()
                .map(|parameter| format!("TypeRef::Parameter({})", quote(&parameter.name)))
                .collect::<Vec<_>>()
                .join(",")
        )
    }

    fn type_ref(&self, ty: &Type, parameters: &[TypeParameter]) -> String {
        match ty {
            Type::Option(value) => format!(
                "TypeRef::Application {{ constructor: StdlibTypeConstructorId::Option, arguments: &[{}] }}",
                self.type_ref(value, parameters)
            ),
            Type::Result(value) => format!(
                "TypeRef::Application {{ constructor: StdlibTypeConstructorId::Result, arguments: &[{}] }}",
                self.type_ref(value, parameters)
            ),
            Type::Array(element) => format!(
                "TypeRef::Application {{ constructor: StdlibTypeConstructorId::Array, arguments: &[{}] }}",
                self.type_ref(element, parameters)
            ),
            Type::FixedArray { element, length } => format!(
                "TypeRef::FixedArray {{ element: &{}, length: {length} }}",
                self.type_ref(element, parameters)
            ),
            Type::Application {
                constructor,
                arguments,
            } => format!(
                "TypeRef::Application {{ constructor: StdlibTypeConstructorId::{}, arguments: &[{}] }}",
                ident(constructor),
                arguments
                    .iter()
                    .map(|argument| self.type_ref(argument, parameters))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Type::Name(name) if parameters.iter().any(|parameter| parameter.name == *name) => {
                format!("TypeRef::Parameter({})", quote(name))
            }
            Type::Name(name) if is_core_type(name) => {
                format!("TypeRef::Core(CoreTypeId::{})", ident(name))
            }
            Type::Name(name) if self.type_names.contains(name.as_str()) => {
                format!("TypeRef::Standard(StdlibTypeId::{})", ident(name))
            }
            _ => unreachable!("validated standard-library types resolve before generation"),
        }
    }

    fn capabilities(&self, attributes: &[Attribute]) -> String {
        let values = attribute_names(attributes, "capabilities");
        format!(
            "&[{}]",
            values
                .iter()
                .map(|capability| {
                    assert!(self.capability_names.contains(capability.as_str()));
                    format!("StdlibCapabilityId::{}", ident(capability))
                })
                .collect::<Vec<_>>()
                .join(",")
        )
    }

    fn representation(&self, attributes: &[Attribute]) -> String {
        let values = attribute_names(attributes, "representation");
        match values.as_slice() {
            [kind, storage] if kind == "scalar" => format!(
                "RuntimeRepresentation::Scalar {{ storage: CoreTypeId::{} }}",
                ident(storage)
            ),
            [kind, element, rest @ ..] if kind == "gcArray" => format!(
                "RuntimeRepresentation::GcArray {{ element: CoreTypeId::{}, mutable: {}, nullable: {} }}",
                ident(element),
                rest.iter().any(|value| value == "mutable"),
                rest.iter().any(|value| value == "nullable")
            ),
            [kind, rest @ ..] if kind == "gcStruct" => format!(
                "RuntimeRepresentation::GcStruct {{ nullable: {} }}",
                rest.iter().any(|value| value == "nullable")
            ),
            [kind, rest @ ..] if kind == "enum" => format!(
                "RuntimeRepresentation::Enum {{ nullable: {} }}",
                rest.iter().any(|value| value == "nullable")
            ),
            _ => panic!("invalid generated representation `{values:?}`"),
        }
    }

    fn value_usage(&self, attributes: &[Attribute]) -> String {
        let values = attribute_names(attributes, "valueUsage");
        let has = |name: &str| values.iter().any(|value| value == name);
        format!(
            "ValueUsage {{ record_field: {}, enum_payload: {}, state_field: {}, local_variable: {}, global_variable: {} }}",
            has("recordField"),
            has("enumPayload"),
            has("stateField"),
            has("localVariable"),
            has("globalVariable")
        )
    }
}

fn attribute_name<'a>(attributes: &'a [Attribute], name: &str) -> &'a str {
    optional_attribute_name(attributes, name)
        .unwrap_or_else(|| panic!("missing generated `@{name}` attribute"))
}

fn has_attribute(attributes: &[Attribute], name: &str) -> bool {
    attributes.iter().any(|attribute| attribute.name == name)
}

fn optional_attribute_name<'a>(attributes: &'a [Attribute], name: &str) -> Option<&'a str> {
    let attribute = attributes.iter().find(|attribute| attribute.name == name)?;
    match attribute.arguments.as_slice() {
        [AttributeArgument::Name(value) | AttributeArgument::String(value)] => Some(value),
        _ => panic!("generated `@{name}` must have exactly one argument"),
    }
}

fn attribute_names(attributes: &[Attribute], name: &str) -> Vec<String> {
    attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .map(|attribute| {
            attribute
                .arguments
                .iter()
                .map(|argument| match argument {
                    AttributeArgument::Name(value) | AttributeArgument::String(value) => {
                        value.clone()
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn quote(value: &str) -> String {
    format!("{value:?}")
}

fn is_core_type(name: &str) -> bool {
    PrimitiveType::parse(name).is_some()
}

pub fn generate_ids(library: &Library) -> Result<String, Vec<Error>> {
    let mut capabilities = Vec::new();
    let mut providers = Vec::new();
    let mut constructors = Vec::new();
    let mut namespaces = Vec::new();
    let mut types = Vec::new();
    let mut fields = Vec::new();
    let mut variants = Vec::new();
    let mut items = Vec::new();

    for declaration in &library.declarations {
        match declaration {
            Declaration::Root(owner) => {
                items.extend(owner.functions.iter().map(|function| ident(&function.name)));
            }
            Declaration::StateProvider(provider) => providers.push(ident(&provider.name)),
            Declaration::Namespace(owner) => {
                let owner_id = path_ident(&owner.name);
                namespaces.push(owner_id.clone());
                items.extend(
                    owner
                        .functions
                        .iter()
                        .map(|function| format!("{owner_id}{}", ident(&function.name))),
                );
            }
            Declaration::Capability(owner) => {
                let owner_id = ident(&owner.name);
                capabilities.push(owner_id.clone());
                items.extend(
                    owner
                        .functions
                        .iter()
                        .map(|function| format!("{owner_id}{}", ident(&function.name))),
                );
            }
            Declaration::TypeConstructor(owner) => {
                let owner_id = ident(&owner.name);
                constructors.push(owner_id.clone());
                items.extend(
                    owner
                        .functions
                        .iter()
                        .map(|function| format!("{owner_id}{}", ident(&function.name))),
                );
            }
            Declaration::CoreExtension(owner) => {
                let owner_id = ident(&owner.name);
                items.extend(
                    owner
                        .functions
                        .iter()
                        .map(|function| format!("{owner_id}{}", ident(&function.name))),
                );
            }
            Declaration::Struct(declaration) | Declaration::IntrinsicType(declaration) => {
                let owner_id = ident(&declaration.name);
                types.push(owner_id.clone());
                fields.extend(
                    declaration
                        .fields
                        .iter()
                        .map(|field| format!("{owner_id}{}", ident(&field.name))),
                );
                items.extend(
                    declaration
                        .functions
                        .iter()
                        .map(|function| format!("{owner_id}{}", ident(&function.name))),
                );
            }
            Declaration::Enum(declaration) => {
                let owner_id = ident(&declaration.name);
                types.push(owner_id.clone());
                variants.extend(
                    declaration
                        .variants
                        .iter()
                        .map(|variant| format!("{owner_id}{}", ident(&variant.name))),
                );
            }
        }
    }

    let groups = [
        ("capability", &capabilities),
        ("state provider", &providers),
        ("type constructor", &constructors),
        ("namespace", &namespaces),
        ("type", &types),
        ("field", &fields),
        ("variant", &variants),
        ("item", &items),
    ];
    let mut errors = Vec::new();
    for (description, values) in groups {
        let mut seen = HashSet::new();
        for value in values {
            if !seen.insert(value) {
                errors.push(Error {
                    message: format!("duplicate generated {description} identity `{value}`"),
                    start: 0,
                    end: 0,
                });
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    let mut output = String::new();
    emit_group(&mut output, "StdlibCapabilityId", &capabilities);
    emit_group(&mut output, "StdlibStateProviderId", &providers);
    emit_group(&mut output, "StdlibTypeConstructorId", &constructors);
    emit_group(&mut output, "StdlibNamespaceId", &namespaces);
    emit_group(&mut output, "StdlibTypeId", &types);
    emit_group(&mut output, "StdlibFieldId", &fields);
    emit_group(&mut output, "StdlibVariantId", &variants);
    emit_group(&mut output, "StdlibItemId", &items);
    Ok(output)
}

fn emit_group(output: &mut String, name: &str, values: &[String]) {
    output.push_str("catalog_id!(");
    output.push_str(name);
    output.push_str(", ");
    output.push_str(name);
    output.push_str("Discriminant {");
    for value in values {
        output.push_str(value);
        output.push(',');
    }
    output.push_str("});\n");
}

fn path_ident(path: &str) -> String {
    path.split('.').map(ident).collect()
}

fn ident(name: &str) -> String {
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
    use crate::parse;

    use super::*;

    #[test]
    fn source_paths_and_member_names_produce_existing_style_identities() {
        let source = r#"
root { @intrinsic(Print) fn print() -> None; }
namespace process.read {
    @intrinsic(ProcessReadManagedString)
    fn managedString() -> String;
}
@behavior(declared)
capability Numeric<T> { @intrinsic(NumericMin) fn min() -> T; }
@representation(gcStruct)
@valueUsage(localVariable)
struct Duration {
    seconds: i64,
    @intrinsic(DurationFromSeconds)
    static fn fromSeconds() -> Duration;
}
@representation(enum)
@valueUsage(localVariable)
enum TimerState { NotRunning }
@processType(GbaEmulator)
@attachment(identity)
@directRead(GbaEmulatorRead)
stateProvider GBA as gba { "mGBA" }
"#;
        let generated = generate_ids(&parse(source).unwrap()).unwrap();
        for identity in [
            "Print",
            "ProcessRead",
            "ProcessReadManagedString",
            "NumericMin",
            "DurationSeconds",
            "DurationFromSeconds",
            "TimerStateNotRunning",
            "Gba",
        ] {
            assert!(generated.contains(identity), "missing {identity}");
        }
    }

    #[test]
    fn source_body_generates_an_ordinary_hidden_function() {
        let source = r#"
/// Duration.
///
/// # Example
///
/// Store a duration
///
/// ```splitscript
/// let delay: Duration = Duration.fromFrames(120, 60)
/// ```
@representation(gcStruct)
@valueUsage(localVariable)
struct Duration {
    /// Constructs a duration.
    ///
    /// Delegates to a primitive.
    ///
    /// # Example
    ///
    /// Convert frames
    ///
    /// ```splitscript
    /// return Duration.fromFrames(120, 60)
    /// ```
    static fn fromFrames(
        /// Frame count.
        frames: i64,
        /// Frames per second.
        fps: i64,
    ) -> Duration {
        return Duration.fromParts(frames / fps, 0)
    }
}
"#;
        let generated = generate_catalog(&parse(source).unwrap()).unwrap();
        assert!(generated.contains("Implementation::LibraryBody"));
        assert!(generated.contains("__splitscript_stdlib_DurationFromFrames"));
        assert!(generated.contains("return Duration.fromParts(frames / fps, 0)"));
        assert!(!generated.contains("IntrinsicId::DurationFromFrames"));
    }

    #[test]
    fn standard_types_can_name_a_source_defined_display_implementation() {
        let source = r#"
/// Displayable values.
///
/// # Example
///
/// Interpolate a value
///
/// ```splitscript
/// let text = `{42}`
/// ```
@behavior(declared)
capability Display<T> {}

/// Text.
///
/// # Example
///
/// Store text
///
/// ```splitscript
/// let text: String = "value"
/// ```
@representation(gcArray, u8, mutable, nullable)
@valueUsage(localVariable)
@capabilities(Display)
intrinsic type String {}

/// A file version.
///
/// # Example
///
/// Store a version
///
/// ```splitscript
/// let version: FileVersion = fileVersion
/// ```
@representation(gcStruct, nullable)
@valueUsage(localVariable)
@capabilities(Display)
struct FileVersion {
    /// Major component.
    ///
    /// # Example
    ///
    /// Read the major component
    ///
    /// ```splitscript
    /// let major = version.major
    /// ```
    major: u16,

    /// Formats the version.
    ///
    /// Uses dotted components.
    ///
    /// # Example
    ///
    /// Display a version
    ///
    /// ```splitscript
    /// print(version)
    /// ```
    @display
    fn toString() -> String {
        return `{self.major}`
    }
}
"#;
        let generated = generate_catalog(&parse(source).unwrap()).unwrap();
        assert!(generated.contains("display: Some(StdlibItemId::FileVersionToString)"));
        assert!(generated.contains("Implementation::LibraryBody"));
        assert!(generated.contains("__splitscript_stdlib_FileVersionToString"));
    }

    #[test]
    fn non_callable_examples_survive_catalog_generation_as_focused_snippets() {
        let source = r#"
/// Timer operations.
///
/// # Example
///
/// Inspect the timer
///
/// ```splitscript
/// let state = timer.state()
/// ```
namespace timer {}

/// Optional values.
///
/// # Example
///
/// Store an optional value
///
/// ```splitscript
/// let value: u32? = 4
/// ```
typeConstructor Option<T> {}
"#;
        let generated = generate_catalog(&parse(source).unwrap()).unwrap();
        assert!(generated.contains(
            "Example::on_attach_body(\"Inspect the timer\", \"let state = timer.state()\")"
        ));
        assert!(generated.contains(
            "Example::on_attach_body(\"Store an optional value\", \"let value: u32? = 4\")"
        ));
    }

    #[test]
    fn generic_capability_body_is_preserved_as_a_typed_template() {
        let source = r#"
/// Numeric values.
///
/// # Example
///
/// Add numbers
///
/// ```splitscript
/// let total = 1 + 2
/// ```
@behavior(declared)
capability Numeric<T> {
    /// Restricts a value.
    ///
    /// Uses numeric primitives to apply both bounds.
    ///
    /// # Example
    ///
    /// Restrict a value
    ///
    /// ```splitscript
    /// let bounded = value.clamp(0, 7)
    /// ```
    fn clamp(
        /// The lower bound.
        minimum: T,
        /// The upper bound.
        maximum: T,
    ) -> T {
        let lowerBounded = self.max(minimum)
        return lowerBounded.min(maximum)
    }
}
"#;
        let generated = generate_catalog(&parse(source).unwrap()).unwrap();
        assert!(generated.contains("Implementation::LibraryBody"));
        assert!(generated.contains(
            "TypeParameter { name: \"T\", constraints: &[StdlibCapabilityId::Numeric] }"
        ));
        assert!(generated.contains("let lowerBounded = self.max(minimum)"));
        assert!(!generated.contains("IntrinsicId::NumericClamp"));
    }

    #[test]
    fn capability_constraints_generate_super_capabilities() {
        let source = r#"
/// Displayable values.
///
/// # Example
///
/// Display a value
///
/// ```splitscript
/// let text = `{1}`
/// ```
@behavior(declared)
capability Display<T> {}
/// Equatable values.
///
/// # Example
///
/// Compare values
///
/// ```splitscript
/// let equal = 1 == 1
/// ```
@behavior(structuralEquality)
capability Equatable<T> {}
/// Numeric values.
///
/// # Example
///
/// Add values
///
/// ```splitscript
/// let total = 1 + 3
/// ```
@behavior(declared)
capability Numeric<T: Equatable> {}
/// Integer values.
///
/// # Example
///
/// Shift a value
///
/// ```splitscript
/// let mask = 1 << 2
/// ```
@behavior(declared)
capability Integer<T: Numeric + Display> {}
"#;
        let generated = generate_catalog(&parse(source).unwrap()).unwrap();
        assert!(generated.contains(
            "super_capabilities: &[StdlibCapabilityId::Numeric,StdlibCapabilityId::Display]"
        ));
        assert!(generated.contains("super_capabilities: &[StdlibCapabilityId::Equatable]"));
    }

    #[test]
    fn bundled_source_generates_final_typed_catalog_arrays() {
        let source = include_str!("../../../stdlib/standard.split");
        let generated = generate_catalog(&parse(source).unwrap()).unwrap();
        let retired_invocation = ["standard_", "library!"].concat();

        for declaration in [
            "pub(super) const STATE_PROVIDERS: &[StdlibStateProvider]",
            "pub(super) const TYPES: &[StdlibType]",
            "pub(super) const ITEMS: &[StdlibItem]",
        ] {
            assert!(generated.contains(declaration), "missing `{declaration}`");
        }
        assert!(generated.contains("StateProviderProcesses::SourceState"));
        assert!(generated.contains("StateProviderAttachment::Identity"));
        assert!(generated.contains("ItemKind::Method"));
        assert!(generated.contains(
            "TypeRef::FixedArray { element: &TypeRef::Core(CoreTypeId::U16), length: 3 }"
        ));
        assert!(!generated.contains(&retired_invocation));
    }
}
