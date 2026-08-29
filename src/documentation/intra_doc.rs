//! Rustdoc-style links from documentation prose into the generated reference.

use crate::{
    language::{LanguageCatalog, LanguageItemKind},
    stdlib::{FieldVisibility, ItemKind, StandardLibrary, StdlibSymbolId, TypeConstructorSyntax},
};

use super::reference::{core_type_uri, language_item_uri, relative_document_link, symbol_uri};

/// Resolves ``[`symbol`]`` and ``[`label`](symbol)`` occurrences and preserves
/// unresolved occurrences so a catalog validation error can identify the
/// author's original spelling.
pub(super) fn render_links(source: &str, current_uri: &str, library: &StandardLibrary) -> String {
    rewrite_links(source, |label, explicit_target| {
        let target_name = explicit_target.unwrap_or(label);
        target_uri(target_name, library).map(|target| {
            format!(
                "[`{label}`]({})",
                relative_document_link(current_uri, &target)
            )
        })
    })
}

/// Hovers and completion details do not own reference-page-relative navigation.
/// Keep intra-doc markup readable there by reducing it to an ordinary code span.
pub(crate) fn strip_links(source: &str) -> String {
    rewrite_links(source, |label, _| Some(format!("`{label}`")))
}

pub(super) fn unresolved_links(source: &str) -> Vec<&str> {
    let mut unresolved = Vec::new();
    let mut rest = source;
    while let Some(start) = rest.find("[`") {
        let candidate = &rest[start + 2..];
        let Some(end) = candidate.find("`]") else {
            break;
        };
        let after = &candidate[end + 2..];
        let label = &candidate[..end];
        if !rest[..start].ends_with('\\') {
            if let Some((target, _)) = explicit_symbol_target(after) {
                unresolved.push(target);
            } else if !after.starts_with('(') {
                unresolved.push(label);
            }
        }
        rest = after;
    }
    unresolved
}

pub(super) fn resolvable_plain_code_spans<'a>(
    source: &'a str,
    library: &StandardLibrary,
) -> Vec<&'a str> {
    let mut spans = Vec::new();
    let mut fenced = false;
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced || line.contains("<pre class=\"hljs splitscript-code\">") {
            continue;
        }

        let mut rest = line;
        while let Some(start) = rest.find('`') {
            let candidate = &rest[start + 1..];
            let Some(end) = candidate.find('`') else {
                break;
            };
            let label = &candidate[..end];
            let parameter_name = rest[..start].trim_start().starts_with("- ")
                && candidate[end + 1..].starts_with(':');
            if !parameter_name
                && !rest[..start].ends_with('[')
                && !rest[..start].ends_with('\\')
                && target_uri(label, library).is_some()
            {
                spans.push(label);
            }
            rest = &candidate[end + 1..];
        }
    }
    spans
}

fn rewrite_links(
    source: &str,
    mut replacement: impl FnMut(&str, Option<&str>) -> Option<String>,
) -> String {
    let mut output = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(start) = rest.find("[`") {
        output.push_str(&rest[..start]);
        let candidate = &rest[start + 2..];
        let Some(end) = candidate.find("`]") else {
            output.push_str(&rest[start..]);
            return output;
        };
        let label = &candidate[..end];
        let label_end = start + 2 + end + 2;
        let after_label = &rest[label_end..];
        let explicit_target = explicit_symbol_target(after_label);
        let consumed = label_end + explicit_target.map_or(0, |(_, length)| length);
        if label.is_empty() || rest[..start].ends_with('\\') {
            output.push_str(&rest[start..consumed]);
        } else if let Some(replacement) =
            replacement(label, explicit_target.map(|(target, _)| target))
        {
            output.push_str(&replacement);
        } else {
            output.push_str(&rest[start..consumed]);
        }
        rest = &rest[consumed..];
    }
    output.push_str(rest);
    output
}

/// Returns a Rustdoc-style explicit symbol target while leaving ordinary
/// Markdown destinations such as paths, anchors, and URLs untouched.
fn explicit_symbol_target(after_label: &str) -> Option<(&str, usize)> {
    let target = after_label.strip_prefix('(')?;
    let end = target.find(')')?;
    let target = &target[..end];
    if target.is_empty()
        || target.contains('/')
        || target.contains('\\')
        || target.contains('#')
        || target.contains(':')
        || target.ends_with(".md")
    {
        return None;
    }
    Some((target, end + 2))
}

fn target_uri(label: &str, library: &StandardLibrary) -> Option<String> {
    let label = label.strip_suffix("()").unwrap_or(label);

    if let Some((disambiguator, name)) = label.split_once('@') {
        return disambiguated_target_uri(disambiguator, name, library);
    }

    if let Some(item) = LanguageCatalog::new().item_for_source_token(label) {
        return Some(language_item_uri(item.id));
    }
    if let Some(ty) = library.core_types().iter().find(|ty| ty.name == label) {
        return Some(core_type_uri(ty.id, library));
    }
    if let Some(ty) = library.type_by_name(label) {
        return Some(symbol_uri(StdlibSymbolId::Type(ty.id), library));
    }
    if let Some(capability) = library
        .capabilities()
        .iter()
        .find(|capability| capability.name == label)
    {
        return Some(symbol_uri(
            StdlibSymbolId::Capability(capability.id),
            library,
        ));
    }
    if let Some(provider) = library.state_provider_by_name(label) {
        return Some(symbol_uri(
            StdlibSymbolId::StateProvider(provider.id),
            library,
        ));
    }
    if let Some(namespace) = library.namespaces().iter().find(|namespace| {
        namespace.path.join(".") == label
            || (namespace.name == label
                && library
                    .namespaces()
                    .iter()
                    .filter(|candidate| candidate.name == label)
                    .count()
                    == 1)
    }) {
        return Some(symbol_uri(StdlibSymbolId::Namespace(namespace.id), library));
    }
    if let Some(constructor) = library.type_constructors().iter().find(|constructor| {
        library.render_type_constructor(constructor.id) == label
            || (constructor.syntax == TypeConstructorSyntax::Named && constructor.name == label)
    }) {
        return Some(symbol_uri(
            StdlibSymbolId::TypeConstructor(constructor.id),
            library,
        ));
    }
    if let Some(item) = library.item_by_name(label) {
        return Some(symbol_uri(StdlibSymbolId::Item(item.id), library));
    }

    let mut symbols = library
        .fields()
        .iter()
        .filter(|field| {
            field.visibility == FieldVisibility::Public
                && format!("{}.{}", library.render_field_owner(field.owner), field.name) == label
        })
        .map(|field| StdlibSymbolId::Field(field.id))
        .chain(library.public_variants().filter_map(|variant| {
            (format!("{}.{}", library.type_decl(variant.owner).name, variant.name) == label)
                .then_some(StdlibSymbolId::Variant(variant.id))
        }))
        .chain(
            library
                .items()
                .filter(|item| item.name == label)
                .map(|item| StdlibSymbolId::Item(item.id)),
        );
    let symbol = symbols.next()?;
    symbols
        .next()
        .is_none()
        .then(|| symbol_uri(symbol, library))
}

fn disambiguated_target_uri(
    disambiguator: &str,
    name: &str,
    library: &StandardLibrary,
) -> Option<String> {
    let language = LanguageCatalog::new();
    match disambiguator {
        "keyword" => language
            .item_for_source_token(name)
            .filter(|item| item.kind == LanguageItemKind::Keyword)
            .map(|item| language_item_uri(item.id)),
        "syntax" => language
            .item_by_name(name)
            .filter(|item| {
                matches!(
                    item.kind,
                    LanguageItemKind::Declaration
                        | LanguageItemKind::Syntax
                        | LanguageItemKind::SnapshotRoot
                        | LanguageItemKind::Action(_)
                )
            })
            .map(|item| language_item_uri(item.id)),
        "type" => language
            .item_for_source_token(name)
            .filter(|item| matches!(item.kind, LanguageItemKind::BuiltinType(_)))
            .map(|item| language_item_uri(item.id))
            .or_else(|| {
                library
                    .core_types()
                    .iter()
                    .find(|ty| ty.name == name)
                    .map(|ty| core_type_uri(ty.id, library))
            })
            .or_else(|| {
                library
                    .type_by_name(name)
                    .map(|ty| symbol_uri(StdlibSymbolId::Type(ty.id), library))
            })
            .or_else(|| {
                library
                    .type_constructors()
                    .iter()
                    .find(|constructor| {
                        library.render_type_constructor(constructor.id) == name
                            || (constructor.syntax == TypeConstructorSyntax::Named
                                && constructor.name == name)
                    })
                    .map(|constructor| {
                        symbol_uri(StdlibSymbolId::TypeConstructor(constructor.id), library)
                    })
            }),
        "fn" | "method" | "operator" => library.item_by_name(name).and_then(|item| {
            let matches = match disambiguator {
                "fn" => matches!(item.kind, ItemKind::Function),
                "method" => matches!(item.kind, ItemKind::Method { .. }),
                "operator" => item.binary_operator.is_some() || item.unary_operator.is_some(),
                _ => unreachable!(),
            };
            matches.then(|| symbol_uri(StdlibSymbolId::Item(item.id), library))
        }),
        "field" => library
            .fields()
            .iter()
            .find(|field| {
                field.visibility == FieldVisibility::Public
                    && format!("{}.{}", library.render_field_owner(field.owner), field.name) == name
            })
            .map(|field| symbol_uri(StdlibSymbolId::Field(field.id), library)),
        "variant" => library
            .public_variants()
            .find(|variant| {
                format!("{}.{}", library.type_decl(variant.owner).name, variant.name) == name
            })
            .map(|variant| symbol_uri(StdlibSymbolId::Variant(variant.id), library)),
        "capability" => library
            .capabilities()
            .iter()
            .find(|capability| capability.name == name)
            .map(|capability| symbol_uri(StdlibSymbolId::Capability(capability.id), library)),
        "namespace" => library
            .namespaces()
            .iter()
            .find(|namespace| namespace.path.join(".") == name)
            .map(|namespace| symbol_uri(StdlibSymbolId::Namespace(namespace.id), library)),
        "provider" => library
            .state_provider_by_name(name)
            .map(|provider| symbol_uri(StdlibSymbolId::StateProvider(provider.id), library)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_language_and_standard_library_links_exactly() {
        let library = StandardLibrary::new();
        let rendered = render_links(
            "Use [`await`], [`Process.read`], and [`Duration`].",
            "/language/async.md",
            &library,
        );
        assert!(rendered.contains("[`await`](await.md)"), "{rendered}");
        assert!(
            rendered.contains("[`Process.read`](../stdlib/types/Process/methods/read.md)"),
            "{rendered}"
        );
        assert!(
            rendered.contains("[`Duration`](../stdlib/types/Duration/index.md)"),
            "{rendered}"
        );
    }

    #[test]
    fn resolves_explicit_targets_without_changing_the_visible_label() {
        let rendered = render_links(
            "The [`*`](Numeric.multiply) operator, [`read`](method@Process.read) method, [`wait`](keyword@await) keyword, and [`pointer base`](syntax@at) syntax.",
            "/language/operators.md",
            &StandardLibrary::new(),
        );
        assert!(
            rendered.contains("[`*`](../stdlib/capabilities/Numeric/operators/multiply.md)"),
            "{rendered}"
        );
        assert!(
            rendered.contains("[`read`](../stdlib/types/Process/methods/read.md)"),
            "{rendered}"
        );
        assert!(rendered.contains("[`wait`](await.md)"), "{rendered}");
        assert!(rendered.contains("[`pointer base`](at.md)"), "{rendered}");
    }

    #[test]
    fn leaves_ambiguous_or_unknown_links_visible() {
        let rendered = render_links(
            "[`get`], [`DoesNotExist`], and [`Option`]",
            "/index.md",
            &StandardLibrary::new(),
        );
        assert_eq!(rendered, "[`get`], [`DoesNotExist`], and [`Option`]");
    }

    #[test]
    fn strips_reference_only_navigation_for_compact_markdown() {
        assert_eq!(
            strip_links("Use [`await`] and [`*`](operator@Numeric.multiply) here."),
            "Use `await` and `*` here."
        );
    }

    #[test]
    fn distinguishes_resolved_markdown_links_from_unresolved_intra_doc_links() {
        assert_eq!(
            unresolved_links(
                "[`await`](await.md), [`operator`](operator@Missing.multiply), then [`missing`]."
            ),
            ["operator@Missing.multiply", "missing"]
        );
    }

    #[test]
    fn preserves_ordinary_markdown_links_with_code_labels() {
        let source = "See [`guide`](../guide.md) and [`site`](https://example.com).";
        assert_eq!(
            render_links(source, "/language/operators.md", &StandardLibrary::new()),
            source
        );
    }

    #[test]
    fn finds_only_plain_code_spans_with_exact_documentation_targets() {
        let source = "Use `await`, [`retry`](retry.md), and `not a symbol`.\n```splitscript\nawait task\n```";
        assert_eq!(
            resolvable_plain_code_spans(source, &StandardLibrary::new()),
            ["await"]
        );
    }
}
