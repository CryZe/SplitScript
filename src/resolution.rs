//! Declaration collection and nominal-name resolution.
//!
//! Parsing retains nominal spellings. This module validates declarations and
//! rewrites name-shaped enum syntax into stable source/catalog identities
//! before type checking.

use std::collections::HashMap;

use crate::{
    Diagnostic,
    ast::{
        EnumReference, EnumTypeId, ExprKind, MatchPattern, Program, SettingKind, Span, TypeNameId,
        TypeRef,
    },
    stdlib::{StandardLibrary, StdlibTypeKind},
    visit::{self, Folder},
};

/// Validates nominal declarations after syntax construction. Keeping these
/// diagnostics out of token collection is the first enforceable boundary
/// between parsing and resolution.
pub(crate) fn validate_declarations(
    program: &Program,
    standard_library: &StandardLibrary,
) -> Vec<Diagnostic> {
    let core_names = standard_library
        .core_types()
        .iter()
        .map(|ty| ty.name)
        .chain(["Address"])
        .collect::<std::collections::HashSet<_>>();
    let mut declared = HashMap::<&str, Span>::new();
    let mut diagnostics = Vec::new();

    for (kind, name, span) in program
        .records
        .iter()
        .map(|decl| ("record", decl.name.as_str(), decl.name_span))
        .chain(
            program
                .enums
                .iter()
                .map(|decl| ("enum", decl.name.as_str(), decl.name_span)),
        )
    {
        if let Some(first) = declared.insert(name, span) {
            diagnostics.push(
                Diagnostic::type_error(format!("duplicate named type `{name}`"), span)
                    .with_secondary_label(first, "the first declaration is here"),
            );
        }
        if core_names.contains(name) {
            diagnostics.push(Diagnostic::type_error(
                format!("`{name}` is a core type and cannot be redeclared as a {kind}"),
                span,
            ));
        }
        if let Some(ty) = standard_library.type_by_name(name) {
            let standard_kind = if ty.kind == StdlibTypeKind::Enum {
                "enum"
            } else {
                "type"
            };
            diagnostics.push(Diagnostic::type_error(
                format!(
                    "`{name}` is a standard-library {standard_kind} and cannot be redeclared as a {kind}"
                ),
                span,
            ));
        }
    }

    for (name, span) in program.type_names.iter().zip(&program.type_name_spans) {
        if standard_library.type_by_name(name).is_none()
            && !program.records.iter().any(|record| record.name == *name)
            && !program
                .enums
                .iter()
                .any(|enumeration| enumeration.name == *name)
        {
            diagnostics.push(Diagnostic::type_error(
                format!("unknown type `{name}`"),
                *span,
            ));
        }
    }

    diagnostics
}

/// Resolves syntax whose grammatical shape is independent of its nominal
/// meaning. This is intentionally run by `lower`, never by the parser.
pub(crate) fn resolve_program(
    program: &mut Program,
    standard_library: &StandardLibrary,
) -> Vec<Diagnostic> {
    let mut provider_diagnostics = Vec::new();
    if let Some(state) = &mut program.state
        && let Some(reference) = &mut state.provider
    {
        if let Some(provider) = standard_library.state_provider_by_name(&reference.name) {
            reference.resolved = Some(provider.id);
            state.processes = provider
                .processes
                .iter()
                .map(|process| (*process).to_owned())
                .collect();
        } else {
            provider_diagnostics.push(Diagnostic::type_error(
                format!("unknown state provider `{}`", reference.name),
                reference.span,
            ));
        }
    }

    let mut enums = program
        .enums
        .iter()
        .map(|enumeration| (enumeration.name.clone(), EnumTypeId::Source(enumeration.id)))
        .collect::<HashMap<_, _>>();
    for ty in standard_library.types() {
        if ty.kind == StdlibTypeKind::Enum {
            enums
                .entry(ty.name.to_owned())
                .or_insert(EnumTypeId::Standard(ty.id));
        }
    }

    let type_names = program
        .type_names
        .iter()
        .enumerate()
        .filter_map(|(index, name)| {
            let resolved = if let Some(record) =
                program.records.iter().find(|item| item.name == *name)
            {
                TypeRef::Record(record.id)
            } else if let Some(enumeration) = program.enums.iter().find(|item| item.name == *name) {
                TypeRef::Enum(enumeration.id)
            } else {
                TypeRef::Standard(standard_library.type_by_name(name)?.id)
            };
            Some((TypeNameId::from_index(index as u32), resolved))
        })
        .collect::<HashMap<_, _>>();

    let mut resolver = EnumResolver {
        enums: &enums,
        type_names: &type_names,
        diagnostics: Vec::new(),
    };
    for setting in &mut program.settings {
        if let SettingKind::Choice { enumeration, .. } = &mut setting.kind {
            resolver.resolve_reference(enumeration, true);
        }
    }
    resolver.fold_program(program);
    provider_diagnostics.extend(resolver.diagnostics);
    provider_diagnostics
}

struct EnumResolver<'a> {
    enums: &'a HashMap<String, EnumTypeId>,
    type_names: &'a HashMap<TypeNameId, TypeRef>,
    diagnostics: Vec<Diagnostic>,
}

impl EnumResolver<'_> {
    fn resolve_reference(&mut self, reference: &mut EnumReference, source_only: bool) {
        let EnumReference::Named { name, span } = reference else {
            return;
        };
        let Some(enumeration) = self.enums.get(name).copied() else {
            self.diagnostics.push(Diagnostic::type_error(
                format!("unknown enum `{name}`"),
                *span,
            ));
            return;
        };
        if source_only && matches!(enumeration, EnumTypeId::Standard(_)) {
            self.diagnostics.push(Diagnostic::type_error(
                format!("choice settings require a source enum, found `{name}`"),
                *span,
            ));
            return;
        }
        *reference = EnumReference::Resolved(enumeration);
    }
}

impl Folder for EnumResolver<'_> {
    fn fold_expr(&mut self, expression: &mut crate::ast::Expr) {
        visit::walk_expr_mut(self, expression);
        let replacement = match &mut expression.kind {
            ExprKind::Path(path) => {
                let [enum_name, variant] = path.as_slice() else {
                    return;
                };
                self.enums
                    .get(enum_name)
                    .copied()
                    .map(|enumeration| ExprKind::Enum {
                        enumeration: EnumReference::Resolved(enumeration),
                        variant: variant.clone(),
                        payload: None,
                    })
            }
            ExprKind::Call { callee, args, .. } => {
                let [enum_name, variant] = callee.as_slice() else {
                    return;
                };
                self.enums.get(enum_name).copied().map(|enumeration| {
                    if args.len() > 1 {
                        self.diagnostics.push(Diagnostic::type_error(
                            "enum constructors accept at most one payload",
                            expression.span,
                        ));
                    }
                    ExprKind::Enum {
                        enumeration: EnumReference::Resolved(enumeration),
                        variant: variant.clone(),
                        payload: args.drain(..).next().map(Box::new),
                    }
                })
            }
            ExprKind::Enum { enumeration, .. } => {
                self.resolve_reference(enumeration, false);
                None
            }
            _ => None,
        };
        if let Some(replacement) = replacement {
            expression.kind = replacement;
        }
    }

    fn fold_pattern(&mut self, pattern: &mut MatchPattern) {
        if let MatchPattern::Enum { enumeration, .. } = pattern {
            self.resolve_reference(enumeration, false);
        }
        visit::walk_pattern_mut(self, pattern);
    }

    fn fold_type_ref(&mut self, ty: &mut TypeRef) {
        if let TypeRef::Named(name) = ty
            && let Some(resolved) = self.type_names.get(name)
        {
            *ty = *resolved;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{ast::TypeRef, lower, parse};

    #[test]
    fn resolves_nominal_cast_targets_after_parsing() {
        let source = r#"
            state "game.exe" {}
            whileAttached { print(42 as String) }
        "#;
        let lowered = lower(parse(source).unwrap());
        let cast = lowered
            .syntax()
            .actions
            .iter()
            .flat_map(|action| &action.body.statements)
            .find_map(|statement| {
                let crate::ast::Stmt::Expression(expression) = statement else {
                    return None;
                };
                let crate::ast::ExprKind::Call { args, .. } = &expression.kind else {
                    return None;
                };
                let crate::ast::ExprKind::Cast { target, .. } = &args.first()?.kind else {
                    return None;
                };
                Some(*target)
            })
            .expect("the print argument should contain the cast");
        assert_eq!(cast, TypeRef::Standard(crate::stdlib::StdlibTypeId::String));
    }
}
