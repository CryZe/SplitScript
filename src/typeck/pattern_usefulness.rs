//! Recursive pattern usefulness, reachability, and exhaustiveness.

use crate::{inference::Type, semantic::ResolvedEnumVariantId, stdlib::StdlibTypeId};

use super::Checker;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IntegerInterval {
    start: i128,
    end: i128,
}

impl IntegerInterval {
    fn intersect(self, other: Self) -> Option<Self> {
        let interval = Self {
            start: self.start.max(other.start),
            end: self.end.min(other.end),
        };
        (interval.start <= interval.end).then_some(interval)
    }

    fn contains(self, other: Self) -> bool {
        self.start <= other.start && other.end <= self.end
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum PatternCoverage {
    Irrefutable,
    Enum {
        variant: ResolvedEnumVariantId,
        name: String,
        payload: Box<Self>,
    },
    Struct {
        structure: crate::ast::StructId,
        name: String,
        fields: Vec<(crate::ast::StructFieldId, String, Self)>,
    },
    Bool(bool),
    Char(char),
    String(String),
    Int {
        value: u64,
        negative: bool,
    },
    IntRange {
        start: i128,
        end: i128,
        kind: crate::ast::RangeKind,
    },
    FileVersion([u16; 4]),
    OptionNone,
    OptionSome(Box<Self>),
    IteratorEnd,
    IteratorItem(Box<Self>),
    ResultSuccess(Box<Self>),
    ResultError(Box<Self>),
    Array(Vec<Self>),
    Alternation(Vec<Self>),
    Invalid(crate::ast::PatternId),
}

impl PatternCoverage {
    pub(super) fn is_irrefutable(&self) -> bool {
        matches!(self, Self::Irrefutable)
    }

    pub(super) fn display(&self) -> String {
        match self {
            Self::Irrefutable => "_".to_owned(),
            Self::Enum { name, payload, .. } if payload.is_irrefutable() => name.clone(),
            Self::Enum { name, payload, .. } => format!("{name}({})", payload.display()),
            Self::Struct { name, fields, .. } => format!(
                "{name} {{ {} }}",
                fields
                    .iter()
                    .map(|(_, field, pattern)| format!("{field}: {}", pattern.display()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Bool(value) => value.to_string(),
            Self::Char(value) => format!("{value:?}"),
            Self::String(value) => format!("{value:?}"),
            Self::Int { value, negative } => {
                format!("{}{value}", if *negative { "-" } else { "" })
            }
            Self::IntRange { start, end, kind } => {
                format!("{start}{}{end}", kind.operator())
            }
            Self::FileVersion(parts) => {
                format!("v\"{}.{}.{}.{}\"", parts[0], parts[1], parts[2], parts[3])
            }
            Self::OptionNone => "None".to_owned(),
            Self::OptionSome(payload) if payload.is_irrefutable() => "Some(value)".to_owned(),
            Self::OptionSome(payload) => format!("Some({})", payload.display()),
            Self::IteratorEnd => "End".to_owned(),
            Self::IteratorItem(payload) if payload.is_irrefutable() => "Item(value)".to_owned(),
            Self::IteratorItem(payload) => format!("Item({})", payload.display()),
            Self::ResultSuccess(payload) if payload.is_irrefutable() => "Ok(value)".to_owned(),
            Self::ResultSuccess(payload) => format!("Ok({})", payload.display()),
            Self::ResultError(payload) if payload.is_irrefutable() => "Err(error)".to_owned(),
            Self::ResultError(payload) => format!("Err({})", payload.display()),
            Self::Array(elements) => format!(
                "[{}]",
                elements
                    .iter()
                    .map(Self::display)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Alternation(alternatives) => alternatives
                .iter()
                .map(Self::display)
                .collect::<Vec<_>>()
                .join(" | "),
            Self::Invalid(_) => "<invalid pattern>".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum PatternConstructor {
    Enum {
        variant: ResolvedEnumVariantId,
        name: String,
    },
    Struct {
        structure: crate::ast::StructId,
        name: String,
    },
    Bool(bool),
    Char(char),
    String(String),
    Int {
        value: u64,
        negative: bool,
    },
    FileVersion([u16; 4]),
    OptionNone,
    OptionSome,
    IteratorEnd,
    IteratorItem,
    ResultSuccess,
    ResultError,
    Array(usize),
}

impl PatternConstructor {
    fn of(pattern: &PatternCoverage) -> Option<Self> {
        match pattern {
            PatternCoverage::Enum { variant, name, .. } => Some(Self::Enum {
                variant: *variant,
                name: name.clone(),
            }),
            PatternCoverage::Struct {
                structure, name, ..
            } => Some(Self::Struct {
                structure: *structure,
                name: name.clone(),
            }),
            PatternCoverage::Bool(value) => Some(Self::Bool(*value)),
            PatternCoverage::Char(value) => Some(Self::Char(*value)),
            PatternCoverage::String(value) => Some(Self::String(value.clone())),
            PatternCoverage::Int { value, negative } => Some(Self::Int {
                value: *value,
                negative: *negative,
            }),
            PatternCoverage::FileVersion(parts) => Some(Self::FileVersion(*parts)),
            PatternCoverage::OptionNone => Some(Self::OptionNone),
            PatternCoverage::OptionSome(_) => Some(Self::OptionSome),
            PatternCoverage::IteratorEnd => Some(Self::IteratorEnd),
            PatternCoverage::IteratorItem(_) => Some(Self::IteratorItem),
            PatternCoverage::ResultSuccess(_) => Some(Self::ResultSuccess),
            PatternCoverage::ResultError(_) => Some(Self::ResultError),
            PatternCoverage::Array(elements) => Some(Self::Array(elements.len())),
            PatternCoverage::Irrefutable
            | PatternCoverage::IntRange { .. }
            | PatternCoverage::Alternation(_)
            | PatternCoverage::Invalid(_) => None,
        }
    }
}

impl Checker {
    fn pattern_constructors(&mut self, ty: Type) -> Option<Vec<PatternConstructor>> {
        let ty = self.shallow_type(ty);
        if self.is_never_type(ty) {
            return Some(Vec::new());
        }
        if ty == self.core_type(crate::stdlib::CoreTypeId::Bool) {
            return Some(vec![
                PatternConstructor::Bool(false),
                PatternConstructor::Bool(true),
            ]);
        }
        match ty {
            Type::Option(_) => Some(vec![
                PatternConstructor::OptionNone,
                PatternConstructor::OptionSome,
            ]),
            Type::Result(_) => Some(vec![
                PatternConstructor::ResultSuccess,
                PatternConstructor::ResultError,
            ]),
            Type::Application(step)
                if self.inference.application_constructor(step)
                    == crate::stdlib::StdlibTypeConstructorId::IteratorStep =>
            {
                Some(vec![
                    PatternConstructor::IteratorItem,
                    PatternConstructor::IteratorEnd,
                ])
            }
            Type::Array(array) => self
                .inference
                .array_length(array)
                .map(|length| vec![PatternConstructor::Array(length as usize)]),
            Type::Known(_) if self.source_struct_id(ty).is_some() => {
                let structure = self.source_struct_id(ty).unwrap();
                let declaration = self
                    .declarations
                    .structs
                    .iter()
                    .find(|declaration| declaration.id == structure)?;
                Some(vec![PatternConstructor::Struct {
                    structure,
                    name: declaration.name.clone(),
                }])
            }
            Type::Known(_) => self.enum_info_for_type(ty).map(|(_, enumeration)| {
                enumeration
                    .variants
                    .into_iter()
                    .map(|variant| PatternConstructor::Enum {
                        variant: variant.id,
                        name: variant.name,
                    })
                    .collect()
            }),
            _ => None,
        }
    }

    fn pattern_constructor_argument_types(
        &mut self,
        constructor: &PatternConstructor,
        ty: Type,
    ) -> Vec<Type> {
        let ty = self.shallow_type(ty);
        match constructor {
            PatternConstructor::Enum { variant, .. } => self
                .enum_info_for_type(ty)
                .and_then(|(_, enumeration)| {
                    enumeration
                        .variants
                        .into_iter()
                        .find(|candidate| candidate.id == *variant)
                })
                .and_then(|variant| variant.payload)
                .into_iter()
                .collect(),
            PatternConstructor::Struct { structure, .. } => self
                .declarations
                .structs
                .iter()
                .find(|declaration| declaration.id == *structure)
                .map(|declaration| {
                    declaration
                        .fields
                        .iter()
                        .map(|field| self.syntax_type(field.ty))
                        .collect()
                })
                .unwrap_or_default(),
            PatternConstructor::OptionSome => match ty {
                Type::Option(option) => vec![self.inference.option_value(option)],
                _ => Vec::new(),
            },
            PatternConstructor::IteratorItem => match ty {
                Type::Application(step) => self
                    .inference
                    .application_arguments(step)
                    .first()
                    .copied()
                    .into_iter()
                    .collect(),
                _ => Vec::new(),
            },
            PatternConstructor::ResultSuccess => match ty {
                Type::Result(result) => vec![self.inference.result_value(result)],
                _ => Vec::new(),
            },
            PatternConstructor::ResultError => {
                vec![self.standard_type(StdlibTypeId::String)]
            }
            PatternConstructor::Array(length) => match ty {
                Type::Array(array) => vec![self.inference.array_element(array); *length],
                _ => Vec::new(),
            },
            PatternConstructor::Bool(_)
            | PatternConstructor::Char(_)
            | PatternConstructor::String(_)
            | PatternConstructor::Int { .. }
            | PatternConstructor::FileVersion(_)
            | PatternConstructor::OptionNone
            | PatternConstructor::IteratorEnd => Vec::new(),
        }
    }

    fn pattern_constructor_arguments(
        &self,
        pattern: &PatternCoverage,
        constructor: &PatternConstructor,
        arity: usize,
    ) -> Vec<PatternCoverage> {
        let mut arguments = match (pattern, constructor) {
            (PatternCoverage::Irrefutable, _) => vec![PatternCoverage::Irrefutable; arity],
            (PatternCoverage::Enum { payload, .. }, PatternConstructor::Enum { .. }) => (arity
                != 0)
                .then(|| (**payload).clone())
                .into_iter()
                .collect(),
            (
                PatternCoverage::Struct { fields, .. },
                PatternConstructor::Struct { structure, .. },
            ) => self
                .declarations
                .structs
                .iter()
                .find(|declaration| declaration.id == *structure)
                .map(|declaration| {
                    declaration
                        .fields
                        .iter()
                        .map(|field| {
                            fields
                                .iter()
                                .find(|(candidate, _, _)| *candidate == field.id)
                                .map(|(_, _, pattern)| pattern.clone())
                                .unwrap_or(PatternCoverage::Irrefutable)
                        })
                        .collect()
                })
                .unwrap_or_default(),
            (PatternCoverage::OptionSome(payload), PatternConstructor::OptionSome)
            | (PatternCoverage::IteratorItem(payload), PatternConstructor::IteratorItem)
            | (PatternCoverage::ResultSuccess(payload), PatternConstructor::ResultSuccess)
            | (PatternCoverage::ResultError(payload), PatternConstructor::ResultError) => {
                vec![(**payload).clone()]
            }
            (PatternCoverage::Array(elements), PatternConstructor::Array(_)) => elements.clone(),
            _ => Vec::new(),
        };
        arguments.resize(arity, PatternCoverage::Irrefutable);
        arguments.truncate(arity);
        arguments
    }

    fn expand_pattern_alternatives(pattern: &PatternCoverage, output: &mut Vec<PatternCoverage>) {
        if let PatternCoverage::Alternation(alternatives) = pattern {
            for alternative in alternatives {
                Self::expand_pattern_alternatives(alternative, output);
            }
        } else {
            output.push(pattern.clone());
        }
    }

    fn specialize_pattern_matrix(
        &self,
        matrix: &[Vec<PatternCoverage>],
        constructor: &PatternConstructor,
        arity: usize,
    ) -> Vec<Vec<PatternCoverage>> {
        let mut specialized = Vec::new();
        for row in matrix {
            let Some((head, tail)) = row.split_first() else {
                continue;
            };
            let mut alternatives = Vec::new();
            Self::expand_pattern_alternatives(head, &mut alternatives);
            for head in alternatives {
                if matches!(head, PatternCoverage::Invalid(_)) {
                    continue;
                }
                if head.is_irrefutable()
                    || PatternConstructor::of(&head).as_ref() == Some(constructor)
                {
                    let mut row = self.pattern_constructor_arguments(&head, constructor, arity);
                    row.extend_from_slice(tail);
                    specialized.push(row);
                }
            }
        }
        specialized
    }

    fn default_pattern_matrix(&self, matrix: &[Vec<PatternCoverage>]) -> Vec<Vec<PatternCoverage>> {
        let mut default = Vec::new();
        for row in matrix {
            let Some((head, tail)) = row.split_first() else {
                continue;
            };
            let mut alternatives = Vec::new();
            Self::expand_pattern_alternatives(head, &mut alternatives);
            for head in alternatives {
                if head.is_irrefutable() {
                    default.push(tail.to_vec());
                }
            }
        }
        default
    }

    fn pattern_from_constructor(
        &self,
        constructor: &PatternConstructor,
        arguments: Vec<PatternCoverage>,
    ) -> PatternCoverage {
        match constructor {
            PatternConstructor::Enum { variant, name } => PatternCoverage::Enum {
                variant: *variant,
                name: name.clone(),
                payload: Box::new(
                    arguments
                        .into_iter()
                        .next()
                        .unwrap_or(PatternCoverage::Irrefutable),
                ),
            },
            PatternConstructor::Struct { structure, name } => {
                let fields = self
                    .declarations
                    .structs
                    .iter()
                    .find(|declaration| declaration.id == *structure)
                    .map(|declaration| {
                        declaration
                            .fields
                            .iter()
                            .zip(arguments)
                            .map(|(field, pattern)| (field.id, field.name.clone(), pattern))
                            .collect()
                    })
                    .unwrap_or_default();
                PatternCoverage::Struct {
                    structure: *structure,
                    name: name.clone(),
                    fields,
                }
            }
            PatternConstructor::Bool(value) => PatternCoverage::Bool(*value),
            PatternConstructor::Char(value) => PatternCoverage::Char(*value),
            PatternConstructor::String(value) => PatternCoverage::String(value.clone()),
            PatternConstructor::Int { value, negative } => PatternCoverage::Int {
                value: *value,
                negative: *negative,
            },
            PatternConstructor::FileVersion(parts) => PatternCoverage::FileVersion(*parts),
            PatternConstructor::OptionNone => PatternCoverage::OptionNone,
            PatternConstructor::OptionSome => PatternCoverage::OptionSome(Box::new(
                arguments
                    .into_iter()
                    .next()
                    .unwrap_or(PatternCoverage::Irrefutable),
            )),
            PatternConstructor::IteratorEnd => PatternCoverage::IteratorEnd,
            PatternConstructor::IteratorItem => PatternCoverage::IteratorItem(Box::new(
                arguments
                    .into_iter()
                    .next()
                    .unwrap_or(PatternCoverage::Irrefutable),
            )),
            PatternConstructor::ResultSuccess => PatternCoverage::ResultSuccess(Box::new(
                arguments
                    .into_iter()
                    .next()
                    .unwrap_or(PatternCoverage::Irrefutable),
            )),
            PatternConstructor::ResultError => PatternCoverage::ResultError(Box::new(
                arguments
                    .into_iter()
                    .next()
                    .unwrap_or(PatternCoverage::Irrefutable),
            )),
            PatternConstructor::Array(_) => PatternCoverage::Array(arguments),
        }
    }

    fn integer_pattern_domain(&mut self, ty: Type) -> Option<IntegerInterval> {
        let ty = self.shallow_type(ty);
        let bounds = [
            (
                crate::stdlib::CoreTypeId::I8,
                i8::MIN as i128,
                i8::MAX as i128,
            ),
            (crate::stdlib::CoreTypeId::U8, 0, u8::MAX as i128),
            (
                crate::stdlib::CoreTypeId::I16,
                i16::MIN as i128,
                i16::MAX as i128,
            ),
            (crate::stdlib::CoreTypeId::U16, 0, u16::MAX as i128),
            (
                crate::stdlib::CoreTypeId::I32,
                i32::MIN as i128,
                i32::MAX as i128,
            ),
            (crate::stdlib::CoreTypeId::U32, 0, u32::MAX as i128),
            (
                crate::stdlib::CoreTypeId::I64,
                i64::MIN as i128,
                i64::MAX as i128,
            ),
            (crate::stdlib::CoreTypeId::U64, 0, u64::MAX as i128),
            (crate::stdlib::CoreTypeId::Address, 0, u64::MAX as i128),
        ];
        for (core, start, end) in bounds {
            if ty == self.core_type(core) {
                return Some(IntegerInterval { start, end });
            }
        }
        self.inference.is_integer(ty).then_some(IntegerInterval {
            start: -(u64::MAX as i128),
            end: u64::MAX as i128,
        })
    }

    fn integer_interval(
        &self,
        pattern: &PatternCoverage,
        domain: IntegerInterval,
    ) -> Option<IntegerInterval> {
        let interval = match pattern {
            PatternCoverage::Irrefutable => domain,
            PatternCoverage::Int { value, negative } => {
                let value = if *negative && *value != 0 {
                    -i128::from(*value)
                } else {
                    i128::from(*value)
                };
                IntegerInterval {
                    start: value,
                    end: value,
                }
            }
            PatternCoverage::IntRange { start, end, kind } => IntegerInterval {
                start: *start,
                end: match kind {
                    crate::ast::RangeKind::Exclusive => end - 1,
                    crate::ast::RangeKind::Inclusive => *end,
                },
            },
            _ => return None,
        };
        interval.intersect(domain)
    }

    fn integer_interval_pattern(interval: IntegerInterval) -> PatternCoverage {
        if interval.start == interval.end {
            let negative = interval.start < 0;
            let value = if negative {
                (-interval.start) as u64
            } else {
                interval.start as u64
            };
            PatternCoverage::Int { value, negative }
        } else {
            PatternCoverage::IntRange {
                start: interval.start,
                end: interval.end,
                kind: crate::ast::RangeKind::Inclusive,
            }
        }
    }

    fn specialize_integer_matrix(
        &self,
        matrix: &[Vec<PatternCoverage>],
        cell: IntegerInterval,
        domain: IntegerInterval,
    ) -> Vec<Vec<PatternCoverage>> {
        let mut specialized = Vec::new();
        for row in matrix {
            let Some((head, tail)) = row.split_first() else {
                continue;
            };
            let mut alternatives = Vec::new();
            Self::expand_pattern_alternatives(head, &mut alternatives);
            if alternatives.into_iter().any(|alternative| {
                self.integer_interval(&alternative, domain)
                    .is_some_and(|interval| interval.contains(cell))
            }) {
                specialized.push(tail.to_vec());
            }
        }
        specialized
    }

    fn useful_integer_witness(
        &mut self,
        matrix: &[Vec<PatternCoverage>],
        candidate: IntegerInterval,
        tail: &[PatternCoverage],
        remaining_types: &[Type],
        domain: IntegerInterval,
    ) -> Option<Vec<PatternCoverage>> {
        let mut boundaries = vec![candidate.start, candidate.end + 1];
        for row in matrix {
            let Some(head) = row.first() else {
                continue;
            };
            let mut alternatives = Vec::new();
            Self::expand_pattern_alternatives(head, &mut alternatives);
            for alternative in alternatives {
                if let Some(interval) = self
                    .integer_interval(&alternative, domain)
                    .and_then(|interval| interval.intersect(candidate))
                {
                    boundaries.push(interval.start);
                    boundaries.push(interval.end + 1);
                }
            }
        }
        boundaries.sort_unstable();
        boundaries.dedup();

        for pair in boundaries.windows(2) {
            let cell = IntegerInterval {
                start: pair[0],
                end: pair[1] - 1,
            };
            if cell.start > cell.end {
                continue;
            }
            let specialized = self.specialize_integer_matrix(matrix, cell, domain);
            if let Some(mut witness) =
                self.useful_pattern_witness(&specialized, tail, remaining_types)
            {
                witness.insert(0, Self::integer_interval_pattern(cell));
                return Some(witness);
            }
        }
        None
    }

    fn useful_pattern_witness(
        &mut self,
        matrix: &[Vec<PatternCoverage>],
        candidate: &[PatternCoverage],
        types: &[Type],
    ) -> Option<Vec<PatternCoverage>> {
        let Some((head, tail)) = candidate.split_first() else {
            return matrix.is_empty().then(Vec::new);
        };
        let (&ty, remaining_types) = types.split_first()?;
        if let PatternCoverage::Alternation(alternatives) = head {
            for alternative in alternatives {
                let mut branch = vec![alternative.clone()];
                branch.extend_from_slice(tail);
                if let Some(witness) = self.useful_pattern_witness(matrix, &branch, types) {
                    return Some(witness);
                }
            }
            return None;
        }
        if matches!(head, PatternCoverage::Invalid(_)) {
            return Some(candidate.to_vec());
        }

        if let Some(domain) = self.integer_pattern_domain(ty)
            && let Some(interval) = self.integer_interval(head, domain)
        {
            return self.useful_integer_witness(matrix, interval, tail, remaining_types, domain);
        }

        if let Some(constructor) = PatternConstructor::of(head) {
            let argument_types = self.pattern_constructor_argument_types(&constructor, ty);
            let arity = argument_types.len();
            let specialized = self.specialize_pattern_matrix(matrix, &constructor, arity);
            let mut specialized_candidate =
                self.pattern_constructor_arguments(head, &constructor, arity);
            specialized_candidate.extend_from_slice(tail);
            let mut specialized_types = argument_types;
            specialized_types.extend_from_slice(remaining_types);
            let mut witness = self.useful_pattern_witness(
                &specialized,
                &specialized_candidate,
                &specialized_types,
            )?;
            let tail = witness.split_off(arity);
            let pattern = self.pattern_from_constructor(&constructor, witness);
            let mut result = vec![pattern];
            result.extend(tail);
            return Some(result);
        }

        if let Some(constructors) = self.pattern_constructors(ty) {
            for constructor in constructors {
                let argument_types = self.pattern_constructor_argument_types(&constructor, ty);
                let arity = argument_types.len();
                let specialized = self.specialize_pattern_matrix(matrix, &constructor, arity);
                let mut specialized_candidate = vec![PatternCoverage::Irrefutable; arity];
                specialized_candidate.extend_from_slice(tail);
                let mut specialized_types = argument_types;
                specialized_types.extend_from_slice(remaining_types);
                if let Some(mut witness) = self.useful_pattern_witness(
                    &specialized,
                    &specialized_candidate,
                    &specialized_types,
                ) {
                    let tail = witness.split_off(arity);
                    let pattern = self.pattern_from_constructor(&constructor, witness);
                    let mut result = vec![pattern];
                    result.extend(tail);
                    return Some(result);
                }
            }
            None
        } else {
            let default = self.default_pattern_matrix(matrix);
            let mut witness = self.useful_pattern_witness(&default, tail, remaining_types)?;
            witness.insert(0, PatternCoverage::Irrefutable);
            Some(witness)
        }
    }

    pub(super) fn pattern_is_useful(
        &mut self,
        previous: &[PatternCoverage],
        candidate: &PatternCoverage,
        ty: Type,
    ) -> bool {
        let matrix = previous
            .iter()
            .cloned()
            .map(|pattern| vec![pattern])
            .collect::<Vec<_>>();
        self.useful_pattern_witness(&matrix, std::slice::from_ref(candidate), &[ty])
            .is_some()
    }

    pub(super) fn missing_patterns(
        &mut self,
        previous: &[PatternCoverage],
        ty: Type,
    ) -> Vec<PatternCoverage> {
        let matrix = previous
            .iter()
            .cloned()
            .map(|pattern| vec![pattern])
            .collect::<Vec<_>>();
        let ty = self.shallow_type(ty);
        if let Some(constructors) = self.pattern_constructors(ty) {
            constructors
                .into_iter()
                .filter_map(|constructor| {
                    let arity = self
                        .pattern_constructor_argument_types(&constructor, ty)
                        .len();
                    let candidate = self.pattern_from_constructor(
                        &constructor,
                        vec![PatternCoverage::Irrefutable; arity],
                    );
                    self.useful_pattern_witness(&matrix, std::slice::from_ref(&candidate), &[ty])
                        .and_then(|mut witness| (!witness.is_empty()).then(|| witness.remove(0)))
                })
                .collect()
        } else {
            self.useful_pattern_witness(&matrix, &[PatternCoverage::Irrefutable], &[ty])
                .and_then(|mut witness| (!witness.is_empty()).then(|| witness.remove(0)))
                .into_iter()
                .collect()
        }
    }
}
