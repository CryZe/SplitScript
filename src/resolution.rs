//! Declaration collection and nominal-name resolution.
//!
//! Parsing retains nominal spellings. This module validates declarations and
//! records stable source/catalog identities beside the immutable syntax tree
//! before type checking.

use std::collections::HashMap;

use crate::{
    Diagnostic,
    ast::{
        EnumReference, ExprId, ExprKind, MatchPattern, PatternId, Program, SettingKind, Span,
        TypeNameId, TypeRef, ValueId,
    },
    stdlib::{StandardLibrary, StdlibStateProviderId, StdlibTypeKind},
    types::{EnumTypeId, ResolvedTypeRef},
    visit::{self, Visitor},
};

/// Catalog identities resolved from one parsed program before type checking.
///
/// These facts deliberately live beside the immutable syntax tree. Syntax
/// retains the names and spans the author wrote, while later compiler stages
/// consume stable catalog identities from this table.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProgramResolutions {
    state_provider: Option<StdlibStateProviderId>,
    type_names: HashMap<TypeNameId, ResolvedTypeRef>,
    expression_enums: HashMap<ExprId, EnumTypeId>,
    pattern_enums: HashMap<PatternId, EnumTypeId>,
    setting_enums: HashMap<ValueId, EnumTypeId>,
}

impl ProgramResolutions {
    pub(crate) fn state_provider(&self) -> Option<StdlibStateProviderId> {
        self.state_provider
    }

    pub(crate) fn type_ref(&self, ty: TypeRef) -> Option<ResolvedTypeRef> {
        match ty {
            TypeRef::Named(name) => self.type_names.get(&name).copied(),
            TypeRef::Core(core) => Some(ResolvedTypeRef::Core(core)),
            TypeRef::Array(id) => Some(ResolvedTypeRef::Array(id)),
            TypeRef::Option(id) => Some(ResolvedTypeRef::Option(id)),
            TypeRef::Result(id) => Some(ResolvedTypeRef::Result(id)),
        }
    }

    pub(crate) fn expression_enum(&self, expression: ExprId) -> Option<EnumTypeId> {
        self.expression_enums.get(&expression).copied()
    }

    pub(crate) fn pattern_enum(&self, pattern: PatternId) -> Option<EnumTypeId> {
        self.pattern_enums.get(&pattern).copied()
    }

    pub(crate) fn setting_enum(&self, setting: ValueId) -> Option<EnumTypeId> {
        self.setting_enums.get(&setting).copied()
    }
}

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

    for function in &program.functions {
        if function
            .name
            .starts_with(crate::stdlib::RESERVED_FUNCTION_PREFIX)
        {
            diagnostics.push(Diagnostic::type_error(
                "function names beginning with `__splitscript_stdlib_` are reserved",
                function.span,
            ));
        }
    }

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
    program: &Program,
    standard_library: &StandardLibrary,
    resolutions: &mut ProgramResolutions,
) -> Vec<Diagnostic> {
    let mut provider_diagnostics = Vec::new();
    if let Some(state) = &program.state {
        if let Some(reference) = &state.provider {
            if let Some(provider) = standard_library.state_provider_by_name(&reference.name) {
                resolutions.state_provider = Some(provider.id);
            } else {
                provider_diagnostics.push(Diagnostic::type_error(
                    format!("unknown state provider `{}`", reference.name),
                    reference.span,
                ));
            }
        } else {
            resolutions.state_provider = standard_library
                .source_state_provider()
                .map(|provider| provider.id);
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
        .type_names()
        .filter_map(|(id, name, _)| {
            let resolved = if let Some(record) =
                program.records.iter().find(|item| item.name == *name)
            {
                ResolvedTypeRef::Record(record.id)
            } else if let Some(enumeration) = program.enums.iter().find(|item| item.name == *name) {
                ResolvedTypeRef::Enum(enumeration.id)
            } else {
                ResolvedTypeRef::Standard(standard_library.type_by_name(name)?.id)
            };
            Some((id, resolved))
        })
        .collect::<HashMap<_, _>>();
    resolutions.type_names = type_names;

    let mut resolver = EnumResolver {
        enums: &enums,
        resolutions,
        diagnostics: Vec::new(),
    };
    resolver.visit_program(program);
    provider_diagnostics.extend(resolver.diagnostics);
    provider_diagnostics
}

struct EnumResolver<'a> {
    enums: &'a HashMap<String, EnumTypeId>,
    resolutions: &'a mut ProgramResolutions,
    diagnostics: Vec<Diagnostic>,
}

impl EnumResolver<'_> {
    fn resolve_reference(
        &mut self,
        reference: &EnumReference,
        source_only: bool,
    ) -> Option<EnumTypeId> {
        let EnumReference { name, span } = reference;
        let Some(enumeration) = self.enums.get(name).copied() else {
            self.diagnostics.push(Diagnostic::type_error(
                format!("unknown enum `{name}`"),
                *span,
            ));
            return None;
        };
        if source_only && matches!(enumeration, EnumTypeId::Standard(_)) {
            self.diagnostics.push(Diagnostic::type_error(
                format!("choice settings require a source enum, found `{name}`"),
                *span,
            ));
            return None;
        }
        Some(enumeration)
    }
}

impl<'ast> Visitor<'ast> for EnumResolver<'_> {
    fn visit_setting(&mut self, setting: &'ast crate::ast::SettingDecl) {
        if let SettingKind::Choice { enumeration, .. } = &setting.kind
            && let Some(enumeration) = self.resolve_reference(enumeration, true)
        {
            self.resolutions
                .setting_enums
                .insert(setting.id, enumeration);
        }
    }

    fn visit_expr(&mut self, expression: &'ast crate::ast::Expr) {
        let enumeration = match &expression.kind {
            ExprKind::Path(path) => {
                let [enum_name, _] = path.as_slice() else {
                    visit::walk_expr(self, expression);
                    return;
                };
                self.enums.get(enum_name).copied()
            }
            ExprKind::Call { callee, args, .. } => {
                let [enum_name, _] = callee.as_slice() else {
                    visit::walk_expr(self, expression);
                    return;
                };
                self.enums.get(enum_name).copied().inspect(|_| {
                    if args.len() > 1 {
                        self.diagnostics.push(Diagnostic::type_error(
                            "enum constructors accept at most one payload",
                            expression.span,
                        ));
                    }
                })
            }
            _ => None,
        };
        if let Some(enumeration) = enumeration {
            self.resolutions
                .expression_enums
                .insert(expression.id, enumeration);
        }
        visit::walk_expr(self, expression);
    }

    fn visit_match_arm(&mut self, arm: &'ast crate::ast::MatchArm) {
        if let MatchPattern::Enum { enumeration, .. } = &arm.pattern
            && let Some(enumeration) = self.resolve_reference(enumeration, false)
        {
            self.resolutions
                .pattern_enums
                .insert(arm.pattern_id, enumeration);
        }
        visit::walk_match_arm(self, arm);
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
        let TypeRef::Named(name) = cast else {
            panic!("lowering should preserve the source-written nominal type");
        };
        assert_eq!(lowered.syntax().type_name(name), "String");
    }
}
