//! Navigable standard-library reference pages over compiler-owned catalogs.

use crate::{
    catalog::Documentation,
    language::LanguageCatalog,
    stdlib::{
        CoreTypeId, FieldVisibility, ItemKind, StandardLibrary, StdlibOwner, StdlibSymbolId,
        StdlibTypeKind,
    },
};

use super::StandardLibraryDocumentation;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentationIndexEntry {
    /// Stable virtual-document path, such as `/stdlib/types/Duration/index.md`.
    pub uri: String,
    pub title: String,
    pub kind: &'static str,
    pub summary: &'static str,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentationPage {
    pub uri: String,
    pub title: String,
    pub markdown: String,
}

struct DocumentationMemberGroup {
    title: &'static str,
    members: Vec<DocumentationMember>,
}

impl DocumentationMemberGroup {
    fn symbols(title: &'static str, members: impl IntoIterator<Item = StdlibSymbolId>) -> Self {
        Self {
            title,
            members: members
                .into_iter()
                .map(DocumentationMember::Symbol)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocumentationMember {
    Symbol(StdlibSymbolId),
    CapabilitySymbol {
        symbol: StdlibSymbolId,
        capability: crate::stdlib::StdlibCapabilityId,
    },
    CoreType(CoreTypeId),
}

/// Compiler-owned documentation reference consumed by editor integrations.
///
/// The reference deliberately returns Markdown rather than VS Code-specific
/// HTML. Editors can render it with their native Markdown UI, expose it as
/// plain read-only text, or transform the same pages for another frontend.
#[derive(Debug, Clone, Default)]
pub struct DocumentationReference {
    library: StandardLibrary,
}

impl DocumentationReference {
    pub fn index(&self) -> Vec<DocumentationIndexEntry> {
        let mut entries = Vec::new();

        entries.extend(
            self.library
                .namespaces()
                .iter()
                .map(|namespace| DocumentationIndexEntry {
                    uri: symbol_uri(StdlibSymbolId::Namespace(namespace.id), &self.library),
                    title: namespace.path.join("."),
                    kind: "namespace",
                    summary: namespace.documentation.summary,
                    signature: None,
                }),
        );
        entries.extend(self.library.core_types().iter().filter_map(|ty| {
            let language = LanguageCatalog::new().builtin_type(ty.id)?;
            Some(DocumentationIndexEntry {
                uri: core_type_uri(ty.id, &self.library),
                title: ty.name.to_owned(),
                kind: "built-in type",
                summary: language.documentation.summary,
                signature: Some(ty.name.to_owned()),
            })
        }));
        entries.extend(self.library.capabilities().iter().map(|capability| {
            DocumentationIndexEntry {
                uri: symbol_uri(StdlibSymbolId::Capability(capability.id), &self.library),
                title: capability.name.to_owned(),
                kind: "capability",
                summary: capability.documentation.summary,
                signature: Some(format!("capability {}", capability.name)),
            }
        }));
        entries.extend(self.library.type_constructors().iter().map(|constructor| {
            DocumentationIndexEntry {
                uri: symbol_uri(
                    StdlibSymbolId::TypeConstructor(constructor.id),
                    &self.library,
                ),
                title: constructor.name.to_owned(),
                kind: "type constructor",
                summary: constructor.documentation.summary,
                signature: Some(render_type_constructor(constructor, &self.library)),
            }
        }));
        entries.extend(
            self.library
                .types()
                .iter()
                .map(|ty| DocumentationIndexEntry {
                    uri: symbol_uri(StdlibSymbolId::Type(ty.id), &self.library),
                    title: ty.name.to_owned(),
                    kind: match ty.kind {
                        StdlibTypeKind::Intrinsic => "type",
                        StdlibTypeKind::Struct => "record",
                        StdlibTypeKind::Enum => "enum",
                    },
                    summary: ty.documentation.summary,
                    signature: Some(render_type_declaration(ty)),
                }),
        );
        entries.extend(
            self.library
                .fields()
                .iter()
                .filter(|field| field.visibility == FieldVisibility::Public)
                .map(|field| {
                    let owner = self.library.type_decl(field.owner);
                    DocumentationIndexEntry {
                        uri: symbol_uri(StdlibSymbolId::Field(field.id), &self.library),
                        title: format!("{}.{}", owner.name, field.name),
                        kind: "field",
                        summary: field.documentation.summary,
                        signature: Some(format!(
                            "{}.{}: {}",
                            owner.name,
                            field.name,
                            self.library.render_type(field.ty)
                        )),
                    }
                }),
        );
        entries.extend(self.library.variants().iter().map(|variant| {
            let owner = self.library.type_decl(variant.owner);
            DocumentationIndexEntry {
                uri: symbol_uri(StdlibSymbolId::Variant(variant.id), &self.library),
                title: format!("{}.{}", owner.name, variant.name),
                kind: "enum variant",
                summary: variant.documentation.summary,
                signature: Some(format!("{}.{}", owner.name, variant.name)),
            }
        }));
        entries.extend(self.library.state_providers().iter().map(|provider| {
            DocumentationIndexEntry {
                uri: symbol_uri(StdlibSymbolId::StateProvider(provider.id), &self.library),
                title: provider.name.to_owned(),
                kind: "state provider",
                summary: provider.documentation.summary,
                signature: Some(format!("state {}", provider.name)),
            }
        }));
        entries.extend(
            self.library
                .items()
                .iter()
                .map(|item| DocumentationIndexEntry {
                    uri: symbol_uri(StdlibSymbolId::Item(item.id), &self.library),
                    title: item.qualified_name.to_owned(),
                    kind: if is_operator(item) {
                        "operator"
                    } else {
                        match item.kind {
                            ItemKind::Function => "function",
                            ItemKind::Method { .. } => "method",
                        }
                    },
                    summary: item.documentation.summary,
                    signature: Some(self.library.render_signature(item.id)),
                }),
        );

        entries.sort_by(|left, right| {
            left.title
                .to_ascii_lowercase()
                .cmp(&right.title.to_ascii_lowercase())
                .then_with(|| left.kind.cmp(right.kind))
        });
        entries
    }

    pub fn page(&self, uri: &str) -> Option<DocumentationPage> {
        if uri == "/index.md" {
            return Some(self.index_page());
        }

        if let Some(ty) = self
            .library
            .core_types()
            .iter()
            .find(|ty| core_type_uri(ty.id, &self.library) == uri)
            && let Some(language) = LanguageCatalog::new().builtin_type(ty.id)
        {
            return Some(self.core_type_page(ty.id, &language.documentation));
        }

        self.library
            .namespaces()
            .iter()
            .find(|value| symbol_uri(StdlibSymbolId::Namespace(value.id), &self.library) == uri)
            .map(|value| {
                self.declaration_page(
                    StdlibSymbolId::Namespace(value.id),
                    value.path.join("."),
                    "namespace",
                    None,
                    &value.documentation,
                    vec![
                        DocumentationMemberGroup::symbols(
                            "Namespaces",
                            self.child_namespaces(value.id),
                        ),
                        DocumentationMemberGroup::symbols(
                            "Functions",
                            self.owned_non_operators(StdlibOwner::Namespace(value.id)),
                        ),
                    ],
                )
            })
            .or_else(|| {
                self.library
                    .capabilities()
                    .iter()
                    .find(|value| {
                        symbol_uri(StdlibSymbolId::Capability(value.id), &self.library) == uri
                    })
                    .map(|value| {
                        self.declaration_page(
                            StdlibSymbolId::Capability(value.id),
                            value.name.to_owned(),
                            "capability",
                            Some(format!("capability {}", value.name)),
                            &value.documentation,
                            vec![
                                DocumentationMemberGroup::symbols(
                                    "Requires",
                                    value
                                        .super_capabilities
                                        .iter()
                                        .map(|id| StdlibSymbolId::Capability(*id)),
                                ),
                                DocumentationMemberGroup {
                                    title: "Implemented by",
                                    members: self.capability_types(value.id),
                                },
                                DocumentationMemberGroup::symbols(
                                    "Operators",
                                    self.owned_operators(StdlibOwner::Capability(value.id)),
                                ),
                                DocumentationMemberGroup::symbols(
                                    "Methods",
                                    self.owned_non_operators(StdlibOwner::Capability(value.id)),
                                ),
                            ],
                        )
                    })
            })
            .or_else(|| {
                self.library
                    .type_constructors()
                    .iter()
                    .find(|value| {
                        symbol_uri(StdlibSymbolId::TypeConstructor(value.id), &self.library) == uri
                    })
                    .map(|value| {
                        self.declaration_page(
                            StdlibSymbolId::TypeConstructor(value.id),
                            value.name.to_owned(),
                            "type constructor",
                            Some(render_type_constructor(value, &self.library)),
                            &value.documentation,
                            vec![
                                DocumentationMemberGroup::symbols(
                                    "Operators",
                                    self.owned_operators(StdlibOwner::TypeConstructor(value.id)),
                                ),
                                DocumentationMemberGroup::symbols(
                                    "Methods",
                                    self.owned_non_operators(StdlibOwner::TypeConstructor(
                                        value.id,
                                    )),
                                ),
                            ],
                        )
                    })
            })
            .or_else(|| {
                self.library
                    .types()
                    .iter()
                    .find(|value| symbol_uri(StdlibSymbolId::Type(value.id), &self.library) == uri)
                    .map(|value| {
                        self.declaration_page(
                            StdlibSymbolId::Type(value.id),
                            value.name.to_owned(),
                            match value.kind {
                                StdlibTypeKind::Intrinsic => "type",
                                StdlibTypeKind::Struct => "record",
                                StdlibTypeKind::Enum => "enum",
                            },
                            Some(render_type_declaration(value)),
                            &value.documentation,
                            vec![
                                DocumentationMemberGroup::symbols(
                                    "Capabilities",
                                    value
                                        .capabilities
                                        .iter()
                                        .map(|id| StdlibSymbolId::Capability(*id)),
                                ),
                                DocumentationMemberGroup::symbols(
                                    "Fields",
                                    self.library
                                        .public_fields(value.id)
                                        .map(|field| StdlibSymbolId::Field(field.id)),
                                ),
                                DocumentationMemberGroup::symbols(
                                    "Variants",
                                    self.library
                                        .variants_of(value.id)
                                        .map(|variant| StdlibSymbolId::Variant(variant.id)),
                                ),
                                DocumentationMemberGroup {
                                    title: "Operators",
                                    members: self.available_members(
                                        StdlibOwner::Type(value.id),
                                        value.capabilities,
                                        true,
                                    ),
                                },
                                DocumentationMemberGroup {
                                    title: "Methods",
                                    members: self.available_members(
                                        StdlibOwner::Type(value.id),
                                        value.capabilities,
                                        false,
                                    ),
                                },
                            ],
                        )
                    })
            })
            .or_else(|| {
                self.library
                    .fields()
                    .iter()
                    .filter(|value| value.visibility == FieldVisibility::Public)
                    .find(|value| symbol_uri(StdlibSymbolId::Field(value.id), &self.library) == uri)
                    .map(|value| {
                        let owner = self.library.type_decl(value.owner);
                        self.declaration_page(
                            StdlibSymbolId::Field(value.id),
                            format!("{}.{}", owner.name, value.name),
                            "field",
                            Some(format!(
                                "{}.{}: {}",
                                owner.name,
                                value.name,
                                self.library.render_type(value.ty)
                            )),
                            &value.documentation,
                            Vec::new(),
                        )
                    })
            })
            .or_else(|| {
                self.library
                    .variants()
                    .iter()
                    .find(|value| {
                        symbol_uri(StdlibSymbolId::Variant(value.id), &self.library) == uri
                    })
                    .map(|value| {
                        let owner = self.library.type_decl(value.owner);
                        self.declaration_page(
                            StdlibSymbolId::Variant(value.id),
                            format!("{}.{}", owner.name, value.name),
                            "enum variant",
                            Some(format!("{}.{}", owner.name, value.name)),
                            &value.documentation,
                            Vec::new(),
                        )
                    })
            })
            .or_else(|| {
                self.library
                    .state_providers()
                    .iter()
                    .find(|value| {
                        symbol_uri(StdlibSymbolId::StateProvider(value.id), &self.library) == uri
                    })
                    .map(|value| {
                        self.declaration_page(
                            StdlibSymbolId::StateProvider(value.id),
                            value.name.to_owned(),
                            "state provider",
                            Some(format!("state {}", value.name)),
                            &value.documentation,
                            vec![DocumentationMemberGroup::symbols(
                                "Value type",
                                [StdlibSymbolId::Type(value.process_type)],
                            )],
                        )
                    })
            })
            .or_else(|| {
                self.library
                    .items()
                    .iter()
                    .find(|value| symbol_uri(StdlibSymbolId::Item(value.id), &self.library) == uri)
                    .map(|value| {
                        let documentation = StandardLibraryDocumentation::generate_with_library(
                            &self.library,
                            value.id,
                            &[],
                        );
                        let mut markdown = format!(
                            "{}\n\n# {}\n\n_{}_\n\n{}",
                            self.symbol_breadcrumb(StdlibSymbolId::Item(value.id), uri),
                            value.qualified_name,
                            if is_operator(value) {
                                "Operator"
                            } else {
                                match value.kind {
                                    ItemKind::Function => "Function",
                                    ItemKind::Method { .. } => "Method",
                                }
                            },
                            documentation.hover_markdown()
                        );
                        append_related(
                            &mut markdown,
                            uri,
                            &self.library,
                            value.documentation.related,
                        );
                        DocumentationPage {
                            uri: uri.to_owned(),
                            title: value.qualified_name.to_owned(),
                            markdown,
                        }
                    })
            })
    }

    fn owned_operators(&self, owner: StdlibOwner) -> Vec<StdlibSymbolId> {
        self.library
            .items()
            .iter()
            .filter(|item| item.owner == owner && is_operator(item))
            .map(|item| StdlibSymbolId::Item(item.id))
            .collect()
    }

    fn owned_non_operators(&self, owner: StdlibOwner) -> Vec<StdlibSymbolId> {
        self.library
            .items()
            .iter()
            .filter(|item| item.owner == owner && !is_operator(item))
            .map(|item| StdlibSymbolId::Item(item.id))
            .collect()
    }

    fn available_members(
        &self,
        direct_owner: StdlibOwner,
        capabilities: &[crate::stdlib::StdlibCapabilityId],
        operators: bool,
    ) -> Vec<DocumentationMember> {
        self.library
            .items()
            .iter()
            .filter(|item| item.owner == direct_owner && is_operator(item) == operators)
            .map(|item| DocumentationMember::Symbol(StdlibSymbolId::Item(item.id)))
            .chain(self.library.items().iter().filter_map(|item| {
                let StdlibOwner::Capability(capability) = item.owner else {
                    return None;
                };
                (is_operator(item) == operators
                    && self.library.capabilities_satisfy(capabilities, capability))
                .then_some(DocumentationMember::CapabilitySymbol {
                    symbol: StdlibSymbolId::Item(item.id),
                    capability,
                })
            }))
            .collect()
    }

    fn capability_types(
        &self,
        capability: crate::stdlib::StdlibCapabilityId,
    ) -> Vec<DocumentationMember> {
        self.library
            .core_types()
            .iter()
            .filter(|ty| {
                self.library
                    .capabilities_satisfy(ty.capabilities, capability)
            })
            .map(|ty| DocumentationMember::CoreType(ty.id))
            .chain(self.library.types().iter().filter_map(|ty| {
                self.library
                    .capabilities_satisfy(ty.capabilities, capability)
                    .then_some(DocumentationMember::Symbol(StdlibSymbolId::Type(ty.id)))
            }))
            .collect()
    }

    fn child_namespaces(&self, parent: crate::stdlib::StdlibNamespaceId) -> Vec<StdlibSymbolId> {
        let parent = self.library.namespace(parent);
        self.library
            .namespaces()
            .iter()
            .filter(|namespace| {
                namespace.path.len() == parent.path.len() + 1
                    && namespace.path.starts_with(parent.path)
            })
            .map(|namespace| StdlibSymbolId::Namespace(namespace.id))
            .collect()
    }

    fn core_type_page<Id>(
        &self,
        ty: CoreTypeId,
        documentation: &Documentation<Id>,
    ) -> DocumentationPage {
        let ty = self.library.core_type(ty);
        let uri = core_type_uri(ty.id, &self.library);
        let mut markdown = format!(
            "{}\n\n# {}\n\n_Built-in type_\n\n```splitscript\n{}\n```",
            breadcrumb(&uri, Vec::new(), ty.name),
            ty.name,
            ty.name
        );
        append_documentation(&mut markdown, documentation);
        self.append_member_groups(
            &mut markdown,
            &uri,
            vec![
                DocumentationMemberGroup::symbols(
                    "Capabilities",
                    ty.capabilities
                        .iter()
                        .map(|id| StdlibSymbolId::Capability(*id)),
                ),
                DocumentationMemberGroup {
                    title: "Operators",
                    members: self.available_members(
                        StdlibOwner::Core(ty.id),
                        ty.capabilities,
                        true,
                    ),
                },
                DocumentationMemberGroup {
                    title: "Methods",
                    members: self.available_members(
                        StdlibOwner::Core(ty.id),
                        ty.capabilities,
                        false,
                    ),
                },
            ],
        );
        DocumentationPage {
            uri,
            title: ty.name.to_owned(),
            markdown,
        }
    }

    fn declaration_page(
        &self,
        symbol: StdlibSymbolId,
        title: String,
        kind: &str,
        signature: Option<String>,
        documentation: &Documentation<StdlibSymbolId>,
        member_groups: Vec<DocumentationMemberGroup>,
    ) -> DocumentationPage {
        let uri = symbol_uri(symbol, &self.library);
        let mut markdown = format!(
            "{}\n\n# {title}\n\n_{kind}_",
            self.symbol_breadcrumb(symbol, &uri)
        );
        if let Some(signature) = signature {
            markdown.push_str(&format!("\n\n```splitscript\n{signature}\n```"));
        }
        append_documentation(&mut markdown, documentation);
        self.append_member_groups(&mut markdown, &uri, member_groups);
        append_related(&mut markdown, &uri, &self.library, documentation.related);
        DocumentationPage {
            uri,
            title,
            markdown,
        }
    }

    fn append_member_groups(
        &self,
        markdown: &mut String,
        uri: &str,
        member_groups: Vec<DocumentationMemberGroup>,
    ) {
        for mut group in member_groups {
            group
                .members
                .sort_by_key(|member| member_label(*member, &self.library));
            group.members.dedup();
            if group.members.is_empty() {
                continue;
            }
            markdown.push_str(&format!("\n\n## {}\n", group.title));
            for member in group.members {
                append_member_link(markdown, uri, member, &self.library);
            }
        }
    }

    fn index_page(&self) -> DocumentationPage {
        let entries = self.index();
        let mut markdown = String::from(
            "# SplitScript standard library\n\n\
             This reference is generated from the same compiler-owned catalog used by type checking, completion, and hover.\n",
        );
        let sections: [(&str, Vec<String>); 5] = [
            (
                "Namespaces",
                self.library
                    .namespaces()
                    .iter()
                    .filter(|namespace| namespace.path.len() == 1)
                    .map(|namespace| {
                        symbol_uri(StdlibSymbolId::Namespace(namespace.id), &self.library)
                    })
                    .collect(),
            ),
            (
                "Types",
                self.library
                    .core_types()
                    .iter()
                    .map(|ty| core_type_uri(ty.id, &self.library))
                    .chain(
                        self.library
                            .types()
                            .iter()
                            .map(|ty| symbol_uri(StdlibSymbolId::Type(ty.id), &self.library)),
                    )
                    .chain(self.library.type_constructors().iter().map(|constructor| {
                        symbol_uri(
                            StdlibSymbolId::TypeConstructor(constructor.id),
                            &self.library,
                        )
                    }))
                    .collect(),
            ),
            (
                "Capabilities",
                self.library
                    .capabilities()
                    .iter()
                    .map(|capability| {
                        symbol_uri(StdlibSymbolId::Capability(capability.id), &self.library)
                    })
                    .collect(),
            ),
            (
                "State providers",
                self.library
                    .state_providers()
                    .iter()
                    .map(|provider| {
                        symbol_uri(StdlibSymbolId::StateProvider(provider.id), &self.library)
                    })
                    .collect(),
            ),
            (
                "Functions",
                self.owned_non_operators(StdlibOwner::Root)
                    .into_iter()
                    .map(|item| symbol_uri(item, &self.library))
                    .collect(),
            ),
        ];
        for (title, uris) in sections {
            let mut section = uris
                .iter()
                .filter_map(|uri| entries.iter().find(|entry| entry.uri == *uri))
                .collect::<Vec<_>>();
            section.sort_by_key(|entry| entry.title.to_ascii_lowercase());
            if section.is_empty() {
                continue;
            }
            markdown.push_str(&format!("\n## {title}\n"));
            for entry in section {
                markdown.push_str(&format!(
                    "\n- [{}]({}) — {}",
                    entry.title,
                    relative_document_link("/index.md", &entry.uri),
                    entry.summary
                ));
            }
        }
        DocumentationPage {
            uri: "/index.md".to_owned(),
            title: "SplitScript standard library".to_owned(),
            markdown,
        }
    }

    fn symbol_breadcrumb(&self, symbol: StdlibSymbolId, uri: &str) -> String {
        match symbol {
            StdlibSymbolId::Namespace(id) => {
                let namespace = self.library.namespace(id);
                let ancestors = namespace
                    .path
                    .iter()
                    .enumerate()
                    .take(namespace.path.len().saturating_sub(1))
                    .filter_map(|(index, name)| {
                        let path = &namespace.path[..=index];
                        self.library
                            .namespaces()
                            .iter()
                            .find(|candidate| candidate.path == path)
                            .map(|ancestor| {
                                (
                                    (*name).to_owned(),
                                    symbol_uri(
                                        StdlibSymbolId::Namespace(ancestor.id),
                                        &self.library,
                                    ),
                                )
                            })
                    })
                    .collect();
                breadcrumb(uri, ancestors, namespace.name)
            }
            StdlibSymbolId::Field(id) => {
                let field = self.library.field(id);
                let owner = self.library.type_decl(field.owner);
                breadcrumb(
                    uri,
                    vec![(
                        owner.name.to_owned(),
                        symbol_uri(StdlibSymbolId::Type(owner.id), &self.library),
                    )],
                    field.name,
                )
            }
            StdlibSymbolId::Variant(id) => {
                let variant = self.library.variant(id);
                let owner = self.library.type_decl(variant.owner);
                breadcrumb(
                    uri,
                    vec![(
                        owner.name.to_owned(),
                        symbol_uri(StdlibSymbolId::Type(owner.id), &self.library),
                    )],
                    variant.name,
                )
            }
            StdlibSymbolId::Item(id) => {
                let item = self.library.item(id);
                breadcrumb(uri, self.owner_breadcrumb(item.owner), item.name)
            }
            _ => breadcrumb(uri, Vec::new(), &symbol_local_label(symbol, &self.library)),
        }
    }

    fn owner_breadcrumb(&self, owner: StdlibOwner) -> Vec<(String, String)> {
        match owner {
            StdlibOwner::Root => Vec::new(),
            StdlibOwner::Namespace(id) => {
                let namespace = self.library.namespace(id);
                namespace
                    .path
                    .iter()
                    .enumerate()
                    .filter_map(|(index, name)| {
                        let path = &namespace.path[..=index];
                        self.library
                            .namespaces()
                            .iter()
                            .find(|candidate| candidate.path == path)
                            .map(|ancestor| {
                                (
                                    (*name).to_owned(),
                                    symbol_uri(
                                        StdlibSymbolId::Namespace(ancestor.id),
                                        &self.library,
                                    ),
                                )
                            })
                    })
                    .collect()
            }
            StdlibOwner::Type(id) => vec![(
                self.library.type_decl(id).name.to_owned(),
                symbol_uri(StdlibSymbolId::Type(id), &self.library),
            )],
            StdlibOwner::Core(id) => vec![(
                self.library.core_type(id).name.to_owned(),
                core_type_uri(id, &self.library),
            )],
            StdlibOwner::Capability(id) => vec![(
                self.library.capability(id).name.to_owned(),
                symbol_uri(StdlibSymbolId::Capability(id), &self.library),
            )],
            StdlibOwner::TypeConstructor(id) => vec![(
                self.library.type_constructor(id).name.to_owned(),
                symbol_uri(StdlibSymbolId::TypeConstructor(id), &self.library),
            )],
        }
    }
}

fn render_type_constructor(
    constructor: &crate::stdlib::StdlibTypeConstructor,
    library: &StandardLibrary,
) -> String {
    if constructor.parameters.is_empty() {
        return constructor.name.to_owned();
    }
    let parameters = constructor
        .parameters
        .iter()
        .map(|parameter| {
            let capabilities = library.minimal_capabilities(parameter.constraints);
            if capabilities.is_empty() {
                parameter.name.to_owned()
            } else {
                format!(
                    "{} where {}",
                    parameter.name,
                    capabilities
                        .iter()
                        .map(|capability| library.capability(*capability).name)
                        .collect::<Vec<_>>()
                        .join(" + ")
                )
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("{}<{parameters}>", constructor.name)
}

fn render_type_declaration(ty: &crate::stdlib::StdlibType) -> String {
    match ty.kind {
        StdlibTypeKind::Intrinsic => ty.name.to_owned(),
        StdlibTypeKind::Struct => format!("record {}", ty.name),
        StdlibTypeKind::Enum => format!("enum {}", ty.name),
    }
}

fn append_documentation<Id>(markdown: &mut String, documentation: &Documentation<Id>) {
    markdown.push_str(&format!(
        "\n\n{}\n\n{}",
        documentation.summary, documentation.details
    ));
    if !documentation.examples.is_empty() {
        markdown.push_str("\n\n## Examples");
        for example in documentation.examples {
            markdown.push_str(&format!(
                "\n\n_{}_\n\n```splitscript\n{}\n```",
                example.title, example.source
            ));
        }
    }
}

fn append_related(
    markdown: &mut String,
    current_uri: &str,
    library: &StandardLibrary,
    related: &[StdlibSymbolId],
) {
    if related.is_empty() {
        return;
    }
    markdown.push_str("\n\n## Related\n");
    for symbol in related {
        append_symbol_link(markdown, current_uri, *symbol, library);
    }
}

fn append_symbol_link(
    markdown: &mut String,
    current_uri: &str,
    symbol: StdlibSymbolId,
    library: &StandardLibrary,
) {
    append_symbol_link_with_label(
        markdown,
        current_uri,
        symbol,
        symbol_label(symbol, library),
        library,
    );
}

fn append_symbol_link_with_label(
    markdown: &mut String,
    current_uri: &str,
    symbol: StdlibSymbolId,
    label: String,
    library: &StandardLibrary,
) {
    let target = symbol_uri(symbol, library);
    markdown.push_str(&format!(
        "\n- [{}]({})",
        label,
        relative_document_link(current_uri, &target)
    ));
}

fn append_member_link(
    markdown: &mut String,
    current_uri: &str,
    member: DocumentationMember,
    library: &StandardLibrary,
) {
    let (label, target, available_through) = match member {
        DocumentationMember::Symbol(symbol) => (
            symbol_local_label(symbol, library),
            symbol_uri(symbol, library),
            None,
        ),
        DocumentationMember::CapabilitySymbol { symbol, capability } => (
            symbol_local_label(symbol, library),
            symbol_uri(symbol, library),
            Some(capability),
        ),
        DocumentationMember::CoreType(ty) => (
            library.core_type(ty).name.to_owned(),
            core_type_uri(ty, library),
            None,
        ),
    };
    markdown.push_str(&format!(
        "\n- [{label}]({})",
        relative_document_link(current_uri, &target)
    ));
    if let Some(capability) = available_through {
        let capability = StdlibSymbolId::Capability(capability);
        markdown.push_str(&format!(
            " — available through [{}]({})",
            symbol_local_label(capability, library),
            relative_document_link(current_uri, &symbol_uri(capability, library))
        ));
    }
}

/// Produces an ordinary relative Markdown link so VS Code's Markdown preview
/// keeps navigation inside the current virtual-document scheme. Absolute
/// custom-scheme links are treated as external resources and are not routed
/// back through `TextDocumentContentProvider`.
fn relative_document_link(current_uri: &str, target_uri: &str) -> String {
    let mut current = current_uri
        .trim_start_matches('/')
        .split('/')
        .collect::<Vec<_>>();
    current.pop();
    let target = target_uri
        .trim_start_matches('/')
        .split('/')
        .collect::<Vec<_>>();

    let common = current
        .iter()
        .zip(&target)
        .take_while(|(left, right)| left == right)
        .count();
    let mut relative = "../".repeat(current.len() - common);
    relative.push_str(&target[common..].join("/"));
    relative
}

fn breadcrumb(uri: &str, ancestors: Vec<(String, String)>, current: &str) -> String {
    let mut markdown = format!(
        "[Standard library]({})",
        relative_document_link(uri, "/index.md")
    );
    for (label, target) in ancestors {
        markdown.push_str(&format!(
            " / [{label}]({})",
            relative_document_link(uri, &target)
        ));
    }
    markdown.push_str(&format!(" / {current}"));
    markdown
}

fn symbol_label(symbol: StdlibSymbolId, library: &StandardLibrary) -> String {
    match symbol {
        StdlibSymbolId::StateProvider(id) => library.state_provider(id).name.to_owned(),
        StdlibSymbolId::Namespace(id) => library.namespace(id).path.join("."),
        StdlibSymbolId::Capability(id) => library.capability(id).name.to_owned(),
        StdlibSymbolId::TypeConstructor(id) => library.type_constructor(id).name.to_owned(),
        StdlibSymbolId::Type(id) => library.type_decl(id).name.to_owned(),
        StdlibSymbolId::Field(id) => {
            let field = library.field(id);
            format!("{}.{}", library.type_decl(field.owner).name, field.name)
        }
        StdlibSymbolId::Variant(id) => {
            let variant = library.variant(id);
            format!("{}.{}", library.type_decl(variant.owner).name, variant.name)
        }
        StdlibSymbolId::Item(id) => library.item(id).qualified_name.to_owned(),
    }
}

fn member_label(member: DocumentationMember, library: &StandardLibrary) -> String {
    match member {
        DocumentationMember::Symbol(symbol)
        | DocumentationMember::CapabilitySymbol { symbol, .. } => symbol_label(symbol, library),
        DocumentationMember::CoreType(ty) => library.core_type(ty).name.to_owned(),
    }
}

fn symbol_local_label(symbol: StdlibSymbolId, library: &StandardLibrary) -> String {
    match symbol {
        StdlibSymbolId::StateProvider(id) => library.state_provider(id).name.to_owned(),
        StdlibSymbolId::Namespace(id) => library.namespace(id).name.to_owned(),
        StdlibSymbolId::Capability(id) => library.capability(id).name.to_owned(),
        StdlibSymbolId::TypeConstructor(id) => library.type_constructor(id).name.to_owned(),
        StdlibSymbolId::Type(id) => library.type_decl(id).name.to_owned(),
        StdlibSymbolId::Field(id) => library.field(id).name.to_owned(),
        StdlibSymbolId::Variant(id) => library.variant(id).name.to_owned(),
        StdlibSymbolId::Item(id) => {
            let item = library.item(id);
            operator_symbol(item).unwrap_or(item.name).to_owned()
        }
    }
}

fn is_operator(item: &crate::stdlib::StdlibItem) -> bool {
    item.binary_operator.is_some() || item.unary_operator.is_some()
}

fn operator_symbol(item: &crate::stdlib::StdlibItem) -> Option<&'static str> {
    item.binary_operator
        .map(crate::stdlib::StandardBinaryOperator::symbol)
        .or_else(|| {
            item.unary_operator
                .map(crate::stdlib::StandardUnaryOperator::symbol)
        })
}

fn core_type_uri(id: CoreTypeId, library: &StandardLibrary) -> String {
    format!("/stdlib/types/{}/index.md", library.core_type(id).name)
}

fn symbol_uri(symbol: StdlibSymbolId, library: &StandardLibrary) -> String {
    match symbol {
        StdlibSymbolId::StateProvider(id) => format!(
            "/stdlib/state-providers/{}.md",
            library.state_provider(id).name
        ),
        StdlibSymbolId::Namespace(id) => format!(
            "/stdlib/namespaces/{}/index.md",
            library.namespace(id).path.join("/")
        ),
        StdlibSymbolId::Capability(id) => format!(
            "/stdlib/capabilities/{}/index.md",
            library.capability(id).name
        ),
        StdlibSymbolId::TypeConstructor(id) => format!(
            "/stdlib/generic-types/{}/index.md",
            library.type_constructor(id).name
        ),
        StdlibSymbolId::Type(id) => {
            format!("/stdlib/types/{}/index.md", library.type_decl(id).name)
        }
        StdlibSymbolId::Field(id) => {
            let field = library.field(id);
            format!(
                "/stdlib/types/{}/fields/{}.md",
                library.type_decl(field.owner).name,
                field.name
            )
        }
        StdlibSymbolId::Variant(id) => {
            let variant = library.variant(id);
            format!(
                "/stdlib/types/{}/variants/{}.md",
                library.type_decl(variant.owner).name,
                variant.name
            )
        }
        StdlibSymbolId::Item(id) => {
            let item = library.item(id);
            match item.owner {
                StdlibOwner::Root => format!("/stdlib/functions/{}.md", item.name),
                StdlibOwner::Namespace(owner) => format!(
                    "/stdlib/namespaces/{}/{}.md",
                    library.namespace(owner).path.join("/"),
                    item.name
                ),
                StdlibOwner::Type(owner) => format!(
                    "/stdlib/types/{}/{}/{}.md",
                    library.type_decl(owner).name,
                    if is_operator(item) {
                        "operators"
                    } else {
                        "methods"
                    },
                    item.name
                ),
                StdlibOwner::Core(owner) => format!(
                    "/stdlib/types/{}/{}/{}.md",
                    library.core_type(owner).name,
                    if is_operator(item) {
                        "operators"
                    } else {
                        "methods"
                    },
                    item.name
                ),
                StdlibOwner::Capability(owner) => format!(
                    "/stdlib/capabilities/{}/{}/{}.md",
                    library.capability(owner).name,
                    if is_operator(item) {
                        "operators"
                    } else {
                        "methods"
                    },
                    item.name
                ),
                StdlibOwner::TypeConstructor(owner) => format!(
                    "/stdlib/generic-types/{}/{}/{}.md",
                    library.type_constructor(owner).name,
                    if is_operator(item) {
                        "operators"
                    } else {
                        "methods"
                    },
                    item.name
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_indexes_and_renders_all_standard_library_symbol_kinds() {
        let reference = DocumentationReference::default();
        let index = reference.index();
        for kind in [
            "namespace",
            "built-in type",
            "capability",
            "type constructor",
            "type",
            "record",
            "enum",
            "field",
            "enum variant",
            "state provider",
            "function",
            "method",
            "operator",
        ] {
            assert!(index.iter().any(|entry| entry.kind == kind), "{kind}");
        }

        let duration = index
            .iter()
            .find(|entry| entry.title == "Duration")
            .expect("Duration is indexed");
        assert_eq!(duration.uri, "/stdlib/types/Duration/index.md");
        let page = reference.page(&duration.uri).expect("Duration has a page");
        assert!(page.markdown.contains("\n\n# Duration\n"));
        assert!(
            page.markdown
                .starts_with("[Standard library](../../../index.md) / Duration")
        );
        assert!(
            page.markdown
                .contains("[fromSeconds](methods/fromSeconds.md)")
        );
        assert!(!page.markdown.contains("splitscript-docs:"));
    }

    #[test]
    fn reference_root_only_contains_top_level_declarations() {
        let reference = DocumentationReference::default();
        let page = reference.page("/index.md").expect("root page exists");
        assert_eq!(page.title, "SplitScript standard library");
        assert!(page.markdown.contains("# SplitScript standard library"));
        assert!(
            page.markdown
                .contains("[Duration](stdlib/types/Duration/index.md)")
        );
        assert!(!page.markdown.contains("Duration.fromSeconds"));
        assert!(!page.markdown.contains("FileVersion.major"));
        assert!(!page.markdown.contains("TimerState.Running"));
        assert!(reference.page("/missing.md").is_none());
    }

    #[test]
    fn owned_symbols_are_nested_under_their_declaring_types() {
        let reference = DocumentationReference::default();

        let method = reference
            .page("/stdlib/types/Duration/methods/fromSeconds.md")
            .expect("Duration.fromSeconds has a page");
        assert!(method.markdown.starts_with(
            "[Standard library](../../../../index.md) / [Duration](../index.md) / fromSeconds"
        ));

        let field = reference
            .page("/stdlib/types/FileVersion/fields/major.md")
            .expect("FileVersion.major has a page");
        assert!(field.markdown.starts_with(
            "[Standard library](../../../../index.md) / [FileVersion](../index.md) / major"
        ));

        let variant = reference
            .page("/stdlib/types/TimerState/variants/Running.md")
            .expect("TimerState.Running has a page");
        assert!(variant.markdown.starts_with(
            "[Standard library](../../../../index.md) / [TimerState](../index.md) / Running"
        ));
    }

    #[test]
    fn documentation_links_are_relative_to_the_current_virtual_page() {
        assert_eq!(
            relative_document_link("/index.md", "/stdlib/types/Duration/index.md"),
            "stdlib/types/Duration/index.md"
        );
        assert_eq!(
            relative_document_link(
                "/stdlib/types/Duration/index.md",
                "/stdlib/types/Duration/methods/fromSeconds.md"
            ),
            "methods/fromSeconds.md"
        );
        assert_eq!(
            relative_document_link(
                "/stdlib/types/Duration/methods/fromSeconds.md",
                "/stdlib/types/Duration/methods/fromMinutes.md"
            ),
            "fromMinutes.md"
        );
    }

    #[test]
    fn operators_are_separate_from_methods_under_their_owner() {
        let reference = DocumentationReference::default();
        let index = reference.index();
        let add = index
            .iter()
            .find(|entry| entry.title == "Numeric.add")
            .expect("Numeric.add is indexed");
        assert_eq!(add.kind, "operator");
        assert_eq!(add.uri, "/stdlib/capabilities/Numeric/operators/add.md");

        let numeric = reference
            .page("/stdlib/capabilities/Numeric/index.md")
            .expect("Numeric has a page");
        assert!(numeric.markdown.contains("## Implemented by"));
        assert!(numeric.markdown.contains("[i32](../../types/i32/index.md)"));
        assert!(
            !numeric
                .markdown
                .contains("[bool](../../types/bool/index.md)")
        );
        assert!(numeric.markdown.contains("## Operators"));
        assert!(numeric.markdown.contains("[+](operators/add.md)"));
        assert!(numeric.markdown.contains("## Methods"));
        assert!(numeric.markdown.contains("[squared](methods/squared.md)"));

        let operator = reference.page(&add.uri).expect("the + operator has a page");
        assert!(operator.markdown.starts_with(
            "[Standard library](../../../../index.md) / [Numeric](../index.md) / add"
        ));
        assert!(operator.markdown.contains("\n\n_Operator_\n\n"));

        let equatable = reference
            .page("/stdlib/capabilities/Equatable/index.md")
            .expect("Equatable has a page");
        assert!(
            equatable
                .markdown
                .contains("[FileVersion](../../types/FileVersion/index.md)")
        );
    }

    #[test]
    fn type_pages_include_members_available_through_transitive_capabilities() {
        let reference = DocumentationReference::default();
        let integer = reference
            .page("/stdlib/types/i32/index.md")
            .expect("i32 has a page");

        assert!(integer.markdown.contains(
            "[+](../../capabilities/Numeric/operators/add.md) — available through \
             [Numeric](../../capabilities/Numeric/index.md)"
        ));
        assert!(integer.markdown.contains(
            "[%](../../capabilities/Integer/operators/remainder.md) — available through \
             [Integer](../../capabilities/Integer/index.md)"
        ));
        assert!(integer.markdown.contains(
            "[abs](../../capabilities/Signed/methods/abs.md) — available through \
             [Signed](../../capabilities/Signed/index.md)"
        ));

        let duration = reference
            .page("/stdlib/types/Duration/index.md")
            .expect("Duration has a page");
        assert!(
            duration
                .markdown
                .lines()
                .any(|line| line == "- [+](operators/add.md)")
        );
        assert!(duration.markdown.contains(
            "[==](../../capabilities/Equatable/operators/equals.md) — available through \
             [Equatable](../../capabilities/Equatable/index.md)"
        ));
    }
}
