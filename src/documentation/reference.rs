//! Navigable standard-library reference pages over compiler-owned catalogs.

use crate::{
    catalog::Documentation,
    stdlib::{
        FieldVisibility, ItemKind, StandardLibrary, StdlibOwner, StdlibSymbolId, StdlibTypeKind,
    },
};

use super::StandardLibraryDocumentation;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentationIndexEntry {
    /// Stable virtual-document path, such as `/stdlib/types/Duration.md`.
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
                    kind: match item.kind {
                        ItemKind::Function => "function",
                        ItemKind::Method { .. } => "method",
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

        self.library
            .namespaces()
            .iter()
            .find(|value| symbol_uri(StdlibSymbolId::Namespace(value.id), &self.library) == uri)
            .map(|value| {
                self.declaration_page(
                    uri,
                    value.path.join("."),
                    "namespace",
                    None,
                    &value.documentation,
                    self.owned_items(StdlibOwner::Namespace(value.id)),
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
                        let mut members = value
                            .super_capabilities
                            .iter()
                            .map(|id| StdlibSymbolId::Capability(*id))
                            .collect::<Vec<_>>();
                        members.extend(self.owned_items(StdlibOwner::Capability(value.id)));
                        self.declaration_page(
                            uri,
                            value.name.to_owned(),
                            "capability",
                            Some(format!("capability {}", value.name)),
                            &value.documentation,
                            members,
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
                            uri,
                            value.name.to_owned(),
                            "type constructor",
                            Some(render_type_constructor(value, &self.library)),
                            &value.documentation,
                            self.owned_items(StdlibOwner::TypeConstructor(value.id)),
                        )
                    })
            })
            .or_else(|| {
                self.library
                    .types()
                    .iter()
                    .find(|value| symbol_uri(StdlibSymbolId::Type(value.id), &self.library) == uri)
                    .map(|value| {
                        let mut members = value
                            .capabilities
                            .iter()
                            .map(|id| StdlibSymbolId::Capability(*id))
                            .collect::<Vec<_>>();
                        members.extend(
                            self.library
                                .public_fields(value.id)
                                .map(|field| StdlibSymbolId::Field(field.id)),
                        );
                        members.extend(
                            self.library
                                .variants_of(value.id)
                                .map(|variant| StdlibSymbolId::Variant(variant.id)),
                        );
                        members.extend(self.owned_items(StdlibOwner::Type(value.id)));
                        self.declaration_page(
                            uri,
                            value.name.to_owned(),
                            match value.kind {
                                StdlibTypeKind::Intrinsic => "type",
                                StdlibTypeKind::Struct => "record",
                                StdlibTypeKind::Enum => "enum",
                            },
                            Some(render_type_declaration(value)),
                            &value.documentation,
                            members,
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
                            uri,
                            format!("{}.{}", owner.name, value.name),
                            "field",
                            Some(format!(
                                "{}.{}: {}",
                                owner.name,
                                value.name,
                                self.library.render_type(value.ty)
                            )),
                            &value.documentation,
                            vec![StdlibSymbolId::Type(value.owner)],
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
                            uri,
                            format!("{}.{}", owner.name, value.name),
                            "enum variant",
                            Some(format!("{}.{}", owner.name, value.name)),
                            &value.documentation,
                            vec![StdlibSymbolId::Type(value.owner)],
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
                            uri,
                            value.name.to_owned(),
                            "state provider",
                            Some(format!("state {}", value.name)),
                            &value.documentation,
                            vec![StdlibSymbolId::Type(value.process_type)],
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
                            "# {}\n\n_{}_\n\n{}",
                            value.qualified_name,
                            match value.kind {
                                ItemKind::Function => "Function",
                                ItemKind::Method { .. } => "Method",
                            },
                            documentation.hover_markdown()
                        );
                        append_related(&mut markdown, &self.library, value.documentation.related);
                        DocumentationPage {
                            uri: uri.to_owned(),
                            title: value.qualified_name.to_owned(),
                            markdown,
                        }
                    })
            })
    }

    fn owned_items(&self, owner: StdlibOwner) -> Vec<StdlibSymbolId> {
        self.library
            .items()
            .iter()
            .filter(|item| item.owner == owner)
            .map(|item| StdlibSymbolId::Item(item.id))
            .collect()
    }

    fn declaration_page(
        &self,
        uri: &str,
        title: String,
        kind: &str,
        signature: Option<String>,
        documentation: &Documentation<StdlibSymbolId>,
        mut members: Vec<StdlibSymbolId>,
    ) -> DocumentationPage {
        let mut markdown = format!("# {title}\n\n_{kind}_");
        if let Some(signature) = signature {
            markdown.push_str(&format!("\n\n```splitscript\n{signature}\n```"));
        }
        append_documentation(&mut markdown, documentation);
        members.sort_by_key(|symbol| symbol_label(*symbol, &self.library));
        members.dedup();
        if !members.is_empty() {
            markdown.push_str("\n\n## Members\n");
            for member in members {
                append_symbol_link(&mut markdown, member, &self.library);
            }
        }
        append_related(&mut markdown, &self.library, documentation.related);
        DocumentationPage {
            uri: uri.to_owned(),
            title,
            markdown,
        }
    }

    fn index_page(&self) -> DocumentationPage {
        let entries = self.index();
        let mut markdown = String::from(
            "# SplitScript standard library\n\n\
             This reference is generated from the same compiler-owned catalog used by type checking, completion, and hover.\n",
        );
        let mut groups = std::collections::BTreeMap::<&str, Vec<&DocumentationIndexEntry>>::new();
        for entry in &entries {
            groups.entry(entry.kind).or_default().push(entry);
        }
        for (kind, entries) in groups {
            markdown.push_str(&format!("\n## {}\n", plural_heading(kind)));
            for entry in entries {
                markdown.push_str(&format!(
                    "\n- [{}](splitscript-docs:{}) — {}",
                    entry.title, entry.uri, entry.summary
                ));
            }
        }
        DocumentationPage {
            uri: "/index.md".to_owned(),
            title: "SplitScript standard library".to_owned(),
            markdown,
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

fn append_related(markdown: &mut String, library: &StandardLibrary, related: &[StdlibSymbolId]) {
    if related.is_empty() {
        return;
    }
    markdown.push_str("\n\n## Related\n");
    for symbol in related {
        append_symbol_link(markdown, *symbol, library);
    }
}

fn append_symbol_link(markdown: &mut String, symbol: StdlibSymbolId, library: &StandardLibrary) {
    markdown.push_str(&format!(
        "\n- [{}](splitscript-docs:{})",
        symbol_label(symbol, library),
        symbol_uri(symbol, library)
    ));
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

fn symbol_uri(symbol: StdlibSymbolId, library: &StandardLibrary) -> String {
    let (category, label) = match symbol {
        StdlibSymbolId::StateProvider(id) => (
            "state-providers",
            library.state_provider(id).name.to_owned(),
        ),
        StdlibSymbolId::Namespace(id) => ("namespaces", library.namespace(id).path.join(".")),
        StdlibSymbolId::Capability(id) => ("capabilities", library.capability(id).name.to_owned()),
        StdlibSymbolId::TypeConstructor(id) => (
            "type-constructors",
            library.type_constructor(id).name.to_owned(),
        ),
        StdlibSymbolId::Type(id) => ("types", library.type_decl(id).name.to_owned()),
        StdlibSymbolId::Field(id) => {
            let field = library.field(id);
            (
                "fields",
                format!("{}.{}", library.type_decl(field.owner).name, field.name),
            )
        }
        StdlibSymbolId::Variant(id) => {
            let variant = library.variant(id);
            (
                "variants",
                format!("{}.{}", library.type_decl(variant.owner).name, variant.name),
            )
        }
        StdlibSymbolId::Item(id) => ("items", library.item(id).qualified_name.to_owned()),
    };
    format!("/stdlib/{category}/{label}.md")
}

fn plural_heading(kind: &str) -> &str {
    match kind {
        "capability" => "Capabilities",
        "enum" => "Enums",
        "enum variant" => "Enum variants",
        "field" => "Fields",
        "function" => "Functions",
        "method" => "Methods",
        "namespace" => "Namespaces",
        "record" => "Records",
        "state provider" => "State providers",
        "type" => "Types",
        "type constructor" => "Type constructors",
        _ => "Symbols",
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
        ] {
            assert!(index.iter().any(|entry| entry.kind == kind), "{kind}");
        }

        let duration = index
            .iter()
            .find(|entry| entry.title == "Duration")
            .expect("Duration is indexed");
        assert_eq!(duration.uri, "/stdlib/types/Duration.md");
        let page = reference.page(&duration.uri).expect("Duration has a page");
        assert!(page.markdown.starts_with("# Duration\n"));
        assert!(page.markdown.contains("Duration.fromSeconds"));
        assert!(page.markdown.contains("splitscript-docs:"));
    }

    #[test]
    fn reference_root_is_generated_from_the_search_index() {
        let reference = DocumentationReference::default();
        let page = reference.page("/index.md").expect("root page exists");
        assert_eq!(page.title, "SplitScript standard library");
        assert!(page.markdown.contains("# SplitScript standard library"));
        assert!(page.markdown.contains("Duration.fromSeconds"));
        assert!(reference.page("/missing.md").is_none());
    }
}
