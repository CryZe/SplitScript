use super::{
    AssociatedTypeDeclaration, Attribute, AttributeArgument, CallableOwnerDeclaration, Declaration,
    Documentation, EnumDeclaration, Error, Example, FieldDeclaration, FunctionDeclaration, Library,
    Parameter, StateProviderDeclaration, StateProviderSelectorDeclaration, StructDeclaration, Type,
    TypeConstructorSyntax, TypeParameter, VariantDeclaration,
};
use crate::{SyntaxMode, Token, TokenCursor, TokenKind, lex, parser::parse_integer};

pub fn parse(source: &str) -> Result<Library, Vec<Error>> {
    let tokens = lex(source, SyntaxMode::StandardLibrary).map_err(|error| {
        vec![Error {
            message: error.message,
            start: error.span.start,
            end: error.span.end,
        }]
    })?;
    Parser {
        source,
        cursor: TokenCursor::new(tokens),
    }
    .library()
    .map_err(|error| vec![error])
}

struct Parser<'a> {
    source: &'a str,
    cursor: TokenCursor,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AssociatedTypesMode {
    Forbidden,
    Requirements,
    Definitions,
}

impl Parser<'_> {
    fn library(&mut self) -> Result<Library, Error> {
        let mut declarations = Vec::new();
        while !self.at(&TokenKind::Eof) {
            let documentation = self.documentation()?;
            let attributes = self.attributes()?;
            let private = self.eat_ident("private");
            if self.eat_ident("struct") {
                declarations.push(Declaration::Struct(self.struct_declaration(
                    documentation,
                    attributes,
                    private,
                )?));
            } else if self.eat_ident("enum") {
                declarations.push(Declaration::Enum(self.enum_declaration(
                    documentation,
                    attributes,
                    private,
                )?));
            } else if self.eat_ident("intrinsic") {
                if !self.eat_ident("type") {
                    return Err(self.error("expected `type` after `intrinsic`"));
                }
                declarations.push(Declaration::IntrinsicType(self.struct_declaration(
                    documentation,
                    attributes,
                    private,
                )?));
            } else if self.eat_ident("root") {
                if private {
                    return Err(self.error("`private` can only modify a type declaration"));
                }
                declarations.push(Declaration::Root(self.callable_owner_declaration(
                    "root".to_owned(),
                    documentation,
                    attributes,
                    true,
                    false,
                    AssociatedTypesMode::Forbidden,
                )?));
            } else if self.eat_ident("namespace") {
                if private {
                    return Err(self.error("`private` can only modify a type declaration"));
                }
                let name = self.path("expected a namespace path")?;
                declarations.push(Declaration::Namespace(self.callable_owner_declaration(
                    name,
                    documentation,
                    attributes,
                    true,
                    false,
                    AssociatedTypesMode::Forbidden,
                )?));
            } else if self.eat_ident("capability") {
                if private {
                    return Err(self.error("`private` can only modify a type declaration"));
                }
                let name = self.ident("expected a capability name")?;
                declarations.push(Declaration::Capability(self.callable_owner_declaration(
                    name,
                    documentation,
                    attributes,
                    false,
                    false,
                    AssociatedTypesMode::Requirements,
                )?));
            } else if self.eat_ident("typeConstructor") {
                if private {
                    return Err(self.error("`private` can only modify a type declaration"));
                }
                let (name, syntax, type_parameters) = self.type_constructor_head()?;
                let mut declaration = self.callable_owner_declaration_with_parameters(
                    name,
                    type_parameters,
                    documentation,
                    attributes,
                    false,
                    true,
                    AssociatedTypesMode::Definitions,
                )?;
                declaration.type_constructor_syntax = Some(syntax);
                declarations.push(Declaration::TypeConstructor(declaration));
            } else if self.eat_ident("extend") {
                if private {
                    return Err(self.error("`private` can only modify a type declaration"));
                }
                let name = self.ident("expected a core type after `extend`")?;
                declarations.push(Declaration::CoreExtension(
                    self.callable_owner_declaration(
                        name,
                        documentation,
                        attributes,
                        false,
                        false,
                        AssociatedTypesMode::Forbidden,
                    )?,
                ));
            } else if self.eat_ident("stateProvider") {
                if private {
                    return Err(self.error("`private` can only modify a type declaration"));
                }
                declarations.push(Declaration::StateProvider(
                    self.state_provider_declaration(documentation, attributes)?,
                ));
            } else {
                return Err(self.error("expected a standard-library declaration"));
            }
        }
        Ok(Library { declarations })
    }

    fn state_provider_declaration(
        &mut self,
        documentation: Documentation,
        attributes: Vec<Attribute>,
    ) -> Result<StateProviderDeclaration, Error> {
        let name = self.ident("expected a state-provider name")?;
        if !self.eat_ident("as") {
            return Err(self.error("expected `as` and the provider value name"));
        }
        let value_name = self.ident("expected a provider value name")?;
        self.expect(
            TokenKind::LBrace,
            "expected `{` before the provider process names",
        )?;
        let mut processes = Vec::new();
        let mut selectors = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            let documentation = self.documentation()?;
            if self.eat_ident("selector") {
                let name = self.ident("expected a state-provider selector name")?;
                self.expect(
                    TokenKind::LParen,
                    "expected `(` after the state-provider selector name",
                )?;
                let mut parameters = Vec::new();
                while !self.at(&TokenKind::RParen) {
                    let name = self.ident("expected a selector parameter name")?;
                    self.expect(TokenKind::Colon, "expected `:` after the parameter name")?;
                    let ty = self.ty()?;
                    parameters.push(Parameter {
                        name,
                        ty,
                        documentation: Documentation::default(),
                        attributes: Vec::new(),
                    });
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(
                    TokenKind::RParen,
                    "expected `)` after the selector parameters",
                )?;
                selectors.push(StateProviderSelectorDeclaration {
                    name,
                    parameters,
                    documentation,
                });
            } else {
                if !documentation.summary.is_empty() || !documentation.details.is_empty() {
                    return Err(self
                        .error("documentation inside a state provider must describe a selector"));
                }
                let TokenKind::String(process) = self.current().kind.clone() else {
                    return Err(self.error("expected a quoted process name or `selector`"));
                };
                self.bump();
                processes.push(process);
            }
            if !self.eat(&TokenKind::Comma) && !self.at(&TokenKind::RBrace) {
                return Err(self.error("expected `,` between state-provider entries"));
            }
        }
        self.bump();
        Ok(StateProviderDeclaration {
            name,
            value_name,
            processes,
            selectors,
            documentation,
            attributes,
        })
    }

    fn callable_owner_declaration(
        &mut self,
        name: String,
        documentation: Documentation,
        attributes: Vec<Attribute>,
        functions_are_static: bool,
        fields_allowed: bool,
        associated_types: AssociatedTypesMode,
    ) -> Result<CallableOwnerDeclaration, Error> {
        let type_parameters = self.type_parameters()?;
        self.callable_owner_declaration_with_parameters(
            name,
            type_parameters,
            documentation,
            attributes,
            functions_are_static,
            fields_allowed,
            associated_types,
        )
    }

    fn callable_owner_declaration_with_parameters(
        &mut self,
        name: String,
        type_parameters: Vec<TypeParameter>,
        documentation: Documentation,
        attributes: Vec<Attribute>,
        functions_are_static: bool,
        fields_allowed: bool,
        associated_types_mode: AssociatedTypesMode,
    ) -> Result<CallableOwnerDeclaration, Error> {
        self.expect(TokenKind::LBrace, "expected `{` after the declaration name")?;
        let mut fields = Vec::new();
        let mut associated_types = Vec::new();
        let mut functions = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                return Err(self.error("expected `}` after the declaration"));
            }
            let documentation = self.documentation()?;
            let attributes = self.attributes()?;
            let private = self.eat_ident("private");
            let explicitly_static = self.eat_ident("static");
            if self.eat_ident("type") {
                if associated_types_mode == AssociatedTypesMode::Forbidden {
                    return Err(self.error(
                        "associated types are only valid in capabilities and type constructors",
                    ));
                }
                if private || explicitly_static {
                    return Err(self.error("an associated type cannot be `private` or `static`"));
                }
                let name = self.ident("expected an associated type name")?;
                let mut constraints = Vec::new();
                if self.eat(&TokenKind::Colon) {
                    loop {
                        constraints.push(self.ident("expected a capability constraint")?);
                        if !self.eat(&TokenKind::Plus) {
                            break;
                        }
                    }
                }
                let value = if self.eat(&TokenKind::Assign) {
                    Some(self.ty()?)
                } else {
                    None
                };
                match associated_types_mode {
                    AssociatedTypesMode::Requirements if value.is_some() => {
                        return Err(self.error("a capability associated type is a requirement and cannot define a value"));
                    }
                    AssociatedTypesMode::Definitions if value.is_none() => {
                        return Err(self.error(
                            "a type-constructor associated type must define a value after `=`",
                        ));
                    }
                    _ => {}
                }
                self.expect(
                    TokenKind::Semicolon,
                    "expected `;` after the associated type",
                )?;
                associated_types.push(AssociatedTypeDeclaration {
                    name,
                    constraints,
                    value,
                    documentation,
                });
            } else if self.eat_ident("fn") {
                functions.push(self.function_declaration(
                    documentation,
                    attributes,
                    private,
                    functions_are_static || explicitly_static,
                )?);
            } else if self.eat_ident("const") {
                if private || explicitly_static {
                    return Err(
                        self.error("an associated constant cannot be `private` or `static`")
                    );
                }
                functions.push(self.constant_declaration(documentation, attributes)?);
            } else if fields_allowed {
                if explicitly_static {
                    return Err(self.error("expected `fn` after `static`"));
                }
                fields.push(self.field_declaration(documentation, attributes, private)?);
            } else {
                return Err(self.error(if explicitly_static {
                    "expected `fn` after `static`"
                } else {
                    "expected a function declaration"
                }));
            }
        }
        self.bump();
        Ok(CallableOwnerDeclaration {
            name,
            type_constructor_syntax: None,
            type_parameters,
            documentation,
            attributes,
            fields,
            associated_types,
            functions,
        })
    }

    fn type_constructor_head(
        &mut self,
    ) -> Result<(String, TypeConstructorSyntax, Vec<TypeParameter>), Error> {
        if self.eat(&TokenKind::LBracket) {
            let parameter = self.ident("expected a type parameter in the array type form")?;
            self.expect(
                TokenKind::RBracket,
                "expected `]` after the array type parameter",
            )?;
            return Ok((
                "Array".to_owned(),
                TypeConstructorSyntax::Array,
                vec![TypeParameter {
                    name: parameter,
                    constraints: Vec::new(),
                }],
            ));
        }

        let name = self.ident("expected a type-constructor form")?;
        if self.eat(&TokenKind::Question) {
            return Ok((
                "Option".to_owned(),
                TypeConstructorSyntax::Optional,
                vec![TypeParameter {
                    name,
                    constraints: Vec::new(),
                }],
            ));
        }
        if self.eat(&TokenKind::Bang) {
            return Ok((
                "Result".to_owned(),
                TypeConstructorSyntax::Fallible,
                vec![TypeParameter {
                    name,
                    constraints: Vec::new(),
                }],
            ));
        }
        if self.at(&TokenKind::DotDotLt) || self.at(&TokenKind::DotDotEq) {
            let inclusive = self.at(&TokenKind::DotDotEq);
            self.bump();
            let upper = self.ident("expected the range type parameter after the operator")?;
            if upper != name {
                return Err(self.error(
                    "both sides of a structural range type must use the same type parameter",
                ));
            }
            return Ok((
                if inclusive {
                    "InclusiveRange".to_owned()
                } else {
                    "ExclusiveRange".to_owned()
                },
                if inclusive {
                    TypeConstructorSyntax::InclusiveRange
                } else {
                    TypeConstructorSyntax::ExclusiveRange
                },
                vec![TypeParameter {
                    name,
                    constraints: vec!["Integer".to_owned()],
                }],
            ));
        }

        let type_parameters = self.type_parameters()?;
        Ok((name, TypeConstructorSyntax::Named, type_parameters))
    }

    fn enum_declaration(
        &mut self,
        documentation: Documentation,
        attributes: Vec<Attribute>,
        private: bool,
    ) -> Result<EnumDeclaration, Error> {
        let name = self.ident("expected an enum name")?;
        self.expect(TokenKind::LBrace, "expected `{` after the enum name")?;
        let mut variants = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                return Err(self.error("expected `}` after the enum declaration"));
            }
            let documentation = self.documentation()?;
            let attributes = self.attributes()?;
            let name = self.ident("expected an enum variant")?;
            if !self.at(&TokenKind::RBrace) {
                self.expect_separator("expected `,` or `;` after the enum variant")?;
            }
            variants.push(VariantDeclaration {
                name,
                documentation,
                attributes,
            });
        }
        self.bump();
        Ok(EnumDeclaration {
            name,
            private,
            documentation,
            attributes,
            variants,
        })
    }

    fn struct_declaration(
        &mut self,
        documentation: Documentation,
        attributes: Vec<Attribute>,
        private: bool,
    ) -> Result<StructDeclaration, Error> {
        let name = self.ident("expected a struct name")?;
        self.expect(TokenKind::LBrace, "expected `{` after the struct name")?;
        let mut fields = Vec::new();
        let mut functions = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                return Err(self.error("expected `}` after the struct declaration"));
            }
            let member_documentation = self.documentation()?;
            let member_attributes = self.attributes()?;
            let private = self.eat_ident("private");
            let is_static = self.eat_ident("static");
            if self.eat_ident("fn") {
                functions.push(self.function_declaration(
                    member_documentation,
                    member_attributes,
                    private,
                    is_static,
                )?);
            } else if self.eat_ident("const") {
                if private || is_static {
                    return Err(
                        self.error("an associated constant cannot be `private` or `static`")
                    );
                }
                functions.push(self.constant_declaration(member_documentation, member_attributes)?);
            } else {
                if is_static {
                    return Err(self.error("expected `fn` after `static`"));
                }
                fields.push(self.field_declaration(
                    member_documentation,
                    member_attributes,
                    private,
                )?);
            }
        }
        self.bump();
        Ok(StructDeclaration {
            name,
            private,
            documentation,
            attributes,
            fields,
            functions,
        })
    }

    fn field_declaration(
        &mut self,
        documentation: Documentation,
        attributes: Vec<Attribute>,
        private: bool,
    ) -> Result<FieldDeclaration, Error> {
        let name = self.ident("expected a field or function declaration")?;
        self.expect(TokenKind::Colon, "expected `:` after the field name")?;
        let ty = self.ty()?;
        self.expect_separator("expected `,` or `;` after the field declaration")?;
        Ok(FieldDeclaration {
            name,
            ty,
            private,
            documentation,
            attributes,
        })
    }

    fn function_declaration(
        &mut self,
        documentation: Documentation,
        attributes: Vec<Attribute>,
        private: bool,
        is_static: bool,
    ) -> Result<FunctionDeclaration, Error> {
        let name = self.ident("expected a function name")?;
        let type_parameters = self.type_parameters()?;
        self.expect(TokenKind::LParen, "expected `(` after the function name")?;
        let mut parameters = Vec::new();
        while !self.at(&TokenKind::RParen) {
            let parameter_documentation = self.documentation()?;
            let parameter_attributes = self.attributes()?;
            let name = self.ident("expected a parameter name")?;
            self.expect(TokenKind::Colon, "expected `:` after the parameter name")?;
            let ty = self.ty()?;
            parameters.push(Parameter {
                name,
                ty,
                documentation: parameter_documentation,
                attributes: parameter_attributes,
            });
            if self.eat(&TokenKind::Comma) {
                continue;
            }
            if !self.at(&TokenKind::RParen) {
                return Err(self.error("expected `,` between parameters"));
            }
        }
        self.bump();
        self.expect(TokenKind::Minus, "expected `->` and a return type")?;
        self.expect(TokenKind::Gt, "expected `>` in the return arrow `->`")?;
        let result_is_async = self.eat_ident("async");
        let result = self.ty()?;
        let where_constraints = self.where_constraints()?;
        let body = if self.eat(&TokenKind::Semicolon) {
            None
        } else if self.at(&TokenKind::LBrace) {
            Some(self.function_body_source()?)
        } else {
            return Err(self.error("expected `;` or a function body after the return type"));
        };
        Ok(FunctionDeclaration {
            name,
            is_constant: false,
            private,
            type_parameters,
            where_constraints,
            parameters,
            result_is_async,
            result,
            is_static,
            documentation,
            attributes,
            body,
        })
    }

    fn constant_declaration(
        &mut self,
        documentation: Documentation,
        attributes: Vec<Attribute>,
    ) -> Result<FunctionDeclaration, Error> {
        let name = self.ident("expected an associated constant name")?;
        self.expect(TokenKind::Colon, "expected `:` after the constant name")?;
        let result = self.ty()?;
        self.expect(TokenKind::Assign, "expected `=` before the constant value")?;
        let start = self.current().span.start;
        let mut parentheses = 0_u32;
        let mut brackets = 0_u32;
        let mut braces = 0_u32;
        loop {
            let token = self.current().clone();
            match token.kind {
                TokenKind::LParen => parentheses += 1,
                TokenKind::RParen => parentheses = parentheses.saturating_sub(1),
                TokenKind::LBracket => brackets += 1,
                TokenKind::RBracket => brackets = brackets.saturating_sub(1),
                TokenKind::LBrace => braces += 1,
                TokenKind::RBrace => braces = braces.saturating_sub(1),
                TokenKind::Semicolon if parentheses == 0 && brackets == 0 && braces == 0 => {
                    let value = self.source[start..token.span.start].trim();
                    if value.is_empty() {
                        return Err(self.error("expected a value after `=`"));
                    }
                    self.bump();
                    return Ok(FunctionDeclaration {
                        name,
                        is_constant: true,
                        private: false,
                        type_parameters: Vec::new(),
                        where_constraints: Vec::new(),
                        parameters: Vec::new(),
                        result_is_async: false,
                        result,
                        is_static: true,
                        documentation,
                        attributes,
                        body: Some(format!("{{ return {value} }}")),
                    });
                }
                TokenKind::Eof => {
                    return Err(self.error("expected `;` after the associated constant"));
                }
                _ => {}
            }
            self.bump();
        }
    }

    /// Captures a body with the ordinary language's exact source spelling.
    /// The ordinary program parser remains the sole parser for statements and
    /// expressions; this privileged parser only finds the balanced boundary.
    fn function_body_source(&mut self) -> Result<String, Error> {
        let start = self.current().span.start;
        let mut depth = 0_u32;
        loop {
            let token = self.current().clone();
            match token.kind {
                TokenKind::LBrace => depth += 1,
                TokenKind::RBrace => {
                    depth -= 1;
                    self.bump();
                    if depth == 0 {
                        return Ok(self.source[start..token.span.end].to_owned());
                    }
                    continue;
                }
                TokenKind::Eof => return Err(self.error("unterminated function body")),
                _ => {}
            }
            self.bump();
        }
    }

    fn type_parameters(&mut self) -> Result<Vec<TypeParameter>, Error> {
        let mut type_parameters = Vec::new();
        if self.eat(&TokenKind::Lt) {
            loop {
                let name = self.ident("expected a type-parameter name")?;
                let mut constraints = Vec::new();
                if self.eat(&TokenKind::Colon) {
                    loop {
                        constraints.push(self.ident("expected a type-parameter constraint")?);
                        if !self.eat(&TokenKind::Plus) {
                            break;
                        }
                    }
                }
                type_parameters.push(TypeParameter { name, constraints });
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                if self.at_generic_close() {
                    break;
                }
            }
            self.expect_generic_close("expected `>` after type parameters")?;
        }
        Ok(type_parameters)
    }

    fn where_constraints(&mut self) -> Result<Vec<TypeParameter>, Error> {
        let mut parameters = Vec::new();
        if !self.eat_ident("where") {
            return Ok(parameters);
        }
        loop {
            let name = self.ident("expected a type-parameter name after `where`")?;
            self.expect(
                TokenKind::Colon,
                "expected `:` after the constrained type parameter",
            )?;
            let mut constraints = Vec::new();
            loop {
                constraints.push(self.ident("expected a capability constraint")?);
                if !self.eat(&TokenKind::Plus) {
                    break;
                }
            }
            parameters.push(TypeParameter { name, constraints });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
            if self.at(&TokenKind::Semicolon) || self.at(&TokenKind::LBrace) {
                break;
            }
        }
        Ok(parameters)
    }

    fn ty(&mut self) -> Result<Type, Error> {
        let mut ty = if self.eat(&TokenKind::LParen) {
            let mut parameters = Vec::new();
            while !self.at(&TokenKind::RParen) {
                parameters.push(self.ty()?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                if self.at(&TokenKind::RParen) {
                    break;
                }
            }
            self.expect(
                TokenKind::RParen,
                "expected `)` after callable parameter types",
            )?;
            self.expect(TokenKind::Minus, "expected `->` after callable parameters")?;
            self.expect(TokenKind::Gt, "expected `>` in the callable arrow `->`")?;
            Type::Callable {
                parameters,
                result: Box::new(self.ty()?),
            }
        } else if self.eat(&TokenKind::LBracket) {
            let element = self.ty()?;
            let length = if self.eat(&TokenKind::Semicolon) {
                let token = self.current().clone();
                let TokenKind::Int(text) = &token.kind else {
                    return Err(self.error("expected a fixed array length after `;`"));
                };
                let (value, suffix) = parse_integer(text).map_err(|message| self.error(message))?;
                if suffix.is_some_and(|ty| !ty.is_integer()) {
                    return Err(self.error("a fixed array length must be an integer"));
                }
                let value = u32::try_from(value)
                    .map_err(|_| self.error("a fixed array length must fit in `u32`"))?;
                self.bump();
                Some(value)
            } else {
                None
            };
            self.expect(
                TokenKind::RBracket,
                "expected `]` after the array element type",
            )?;
            match length {
                Some(length) => Type::FixedArray {
                    element: Box::new(element),
                    length,
                },
                None => Type::Array(Box::new(element)),
            }
        } else {
            let name = self.ident("expected a type")?;
            if self.eat(&TokenKind::Lt) {
                let mut arguments = Vec::new();
                loop {
                    arguments.push(self.ty()?);
                    if self.eat(&TokenKind::Comma) {
                        if self.at_generic_close() {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                self.expect_generic_close("expected `>` after type arguments")?;
                Type::Application {
                    constructor: name,
                    arguments,
                }
            } else {
                Type::Name(name)
            }
        };
        if self.at(&TokenKind::DotDotLt) || self.at(&TokenKind::DotDotEq) {
            let inclusive = self.at(&TokenKind::DotDotEq);
            self.bump();
            let upper = self.ty()?;
            if upper != ty {
                return Err(self.error("both sides of a range type must name the same bound type"));
            }
            ty = if inclusive {
                Type::InclusiveRange(Box::new(ty))
            } else {
                Type::ExclusiveRange(Box::new(ty))
            };
        }
        let mut previous_suffix = None;
        loop {
            let is_option = if self.eat(&TokenKind::Question) {
                true
            } else if self.eat(&TokenKind::Bang) {
                false
            } else {
                break;
            };
            if previous_suffix == Some(is_option) {
                let spelling = if is_option { "?" } else { "!" };
                return Err(self.error(format!(
                    "a type cannot have two adjacent `{spelling}` wrappers"
                )));
            }
            ty = if is_option {
                Type::Option(Box::new(ty))
            } else {
                Type::Result(Box::new(ty))
            };
            previous_suffix = Some(is_option);
        }
        Ok(ty)
    }

    fn attributes(&mut self) -> Result<Vec<Attribute>, Error> {
        let mut attributes = Vec::new();
        while self.eat(&TokenKind::At) {
            let name = self.ident("expected an attribute name after `@`")?;
            let mut arguments = Vec::new();
            if self.eat(&TokenKind::LParen) {
                while !self.at(&TokenKind::RParen) {
                    let argument = match self.current().kind.clone() {
                        TokenKind::Ident(value) => {
                            self.bump();
                            AttributeArgument::Name(value)
                        }
                        TokenKind::String(value) => {
                            self.bump();
                            AttributeArgument::String(value)
                        }
                        _ => return Err(self.error("expected an attribute argument")),
                    };
                    arguments.push(argument);
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(TokenKind::RParen, "expected `)` after attribute arguments")?;
            }
            attributes.push(Attribute { name, arguments });
        }
        Ok(attributes)
    }

    fn documentation(&mut self) -> Result<Documentation, Error> {
        let lines = self.cursor.take_doc_comments();
        parse_documentation(&lines).map_err(|message| self.error(message))
    }

    fn expect_separator(&mut self, message: &str) -> Result<(), Error> {
        if self.eat(&TokenKind::Comma) || self.eat(&TokenKind::Semicolon) {
            Ok(())
        } else {
            Err(self.error(message))
        }
    }

    fn ident(&mut self, message: &str) -> Result<String, Error> {
        if let TokenKind::Ident(value) = self.current().kind.clone() {
            self.bump();
            Ok(value)
        } else {
            Err(self.error(message))
        }
    }

    fn path(&mut self, message: &str) -> Result<String, Error> {
        let mut path = self.ident(message)?;
        while self.eat(&TokenKind::Dot) {
            path.push('.');
            path.push_str(&self.ident("expected a name after `.`")?);
        }
        Ok(path)
    }

    fn eat_ident(&mut self, expected: &str) -> bool {
        if matches!(&self.current().kind, TokenKind::Ident(value) if value == expected) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, expected: TokenKind, message: &str) -> Result<(), Error> {
        if self.eat(&expected) {
            Ok(())
        } else {
            Err(self.error(message))
        }
    }

    fn expect_generic_close(&mut self, message: &str) -> Result<(), Error> {
        if self.cursor.eat_leading_gt().is_some() {
            Ok(())
        } else {
            Err(self.error(message))
        }
    }

    fn at_generic_close(&self) -> bool {
        matches!(
            self.current().kind,
            TokenKind::Gt | TokenKind::Ge | TokenKind::Shr | TokenKind::ShrAssign
        )
    }

    fn eat(&mut self, expected: &TokenKind) -> bool {
        if self.at(expected) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn at(&self, expected: &TokenKind) -> bool {
        self.cursor.at_variant(expected)
    }

    fn bump(&mut self) {
        self.cursor.bump();
    }

    fn current(&self) -> &Token {
        self.cursor.current()
    }

    fn error(&self, message: impl Into<String>) -> Error {
        Error {
            message: message.into(),
            start: self.current().span.start,
            end: self.current().span.end,
        }
    }
}

fn parse_documentation(lines: &[String]) -> Result<Documentation, &'static str> {
    let mut sections = lines.split(|line| line == "# Example");
    let prose = sections.next().unwrap_or_default();
    let mut paragraphs = documentation_paragraphs(prose).into_iter();
    let summary = paragraphs.next().unwrap_or_default();
    let details = paragraphs.collect::<Vec<_>>().join("\n\n");
    let mut examples = Vec::new();
    for example in sections {
        let mut lines = example.iter().skip_while(|line| line.is_empty());
        let title = lines.next().cloned().unwrap_or_default();
        let remaining = lines.cloned().collect::<Vec<_>>();
        let Some(fence_start) = remaining
            .iter()
            .position(|line| line.starts_with("```splitscript"))
        else {
            return Err("a documentation example must contain a `splitscript` code fence");
        };
        let fence = &remaining[fence_start];
        let state_provider = if fence == "```splitscript" {
            None
        } else if let Some(provider) = fence.strip_prefix("```splitscript state ")
            && !provider.is_empty()
            && provider
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            Some(provider.to_owned())
        } else {
            return Err(
                "a documentation example fence must be `splitscript` or `splitscript state Provider`",
            );
        };
        let Some(relative_end) = remaining[fence_start + 1..]
            .iter()
            .position(|line| line == "```")
        else {
            return Err("a documentation example has an unterminated code fence");
        };
        let source_lines = &remaining[fence_start + 1..fence_start + 1 + relative_end];
        let has_hidden_lines = source_lines.iter().any(|line| line.starts_with("# "));
        let source = source_lines
            .iter()
            .filter(|line| !line.starts_with("# "))
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let validation_source = has_hidden_lines.then(|| {
            source_lines
                .iter()
                .map(|line| line.strip_prefix("# ").unwrap_or(line).to_owned())
                .collect::<Vec<_>>()
                .join("\n")
        });
        examples.push(Example {
            title,
            source,
            validation_source,
            state_provider,
        });
    }
    Ok(Documentation {
        summary,
        details,
        examples,
    })
}

fn documentation_paragraphs(lines: &[String]) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let mut paragraph = String::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            if !paragraph.is_empty() {
                paragraphs.push(std::mem::take(&mut paragraph));
            }
        } else {
            if !paragraph.is_empty() {
                paragraph.push(' ');
            }
            paragraph.push_str(line);
        }
    }
    if !paragraph.is_empty() {
        paragraphs.push(paragraph);
    }
    paragraphs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_privileged_structs_without_granting_semantics() {
        let source = r#"
/// Represents time.
///
/// Used for game time.
@representation(gcStruct)
struct Duration {
    /// Whole seconds.
    private seconds: i64,

    /// Constructs a duration.
    ///
    /// # Example
    ///
    /// Build a duration
    ///
    /// ```splitscript
    /// return Duration.fromSeconds(seconds)
    /// ```
    @intrinsic(DurationFromSeconds)
    static fn fromSeconds(
        /// Floating-point seconds.
        seconds: f32,
    ) -> Duration;
}
"#;
        let library = parse(source).expect("source should parse");
        let Declaration::Struct(duration) = &library.declarations[0] else {
            panic!("expected a struct")
        };
        assert_eq!(duration.name, "Duration");
        assert!(!duration.private);
        assert_eq!(duration.documentation.details, "Used for game time.");
        assert_eq!(duration.fields[0].documentation.summary, "Whole seconds.");
        assert_eq!(duration.functions[0].parameters[0].ty.to_string(), "f32");
        assert_eq!(
            duration.functions[0].documentation.examples[0].source,
            "return Duration.fromSeconds(seconds)"
        );
    }

    #[test]
    fn parses_library_private_nominal_types() {
        let library = parse(
            r#"
private struct Layout {
    private offset: u64,
}

private enum Backend {
    Native,
}
"#,
        )
        .expect("private type declarations should parse in privileged source");

        let Declaration::Struct(layout) = &library.declarations[0] else {
            panic!("expected a private struct")
        };
        assert!(layout.private);
        let Declaration::Enum(backend) = &library.declarations[1] else {
            panic!("expected a private enum")
        };
        assert!(backend.private);
    }

    #[test]
    fn joins_wrapped_doc_lines_but_preserves_explicit_paragraphs() {
        let source = r#"
/// Represents a precise span
/// of time.
///
/// The first detail paragraph wraps
/// across source lines.
///
/// The second detail paragraph remains separate.
struct Duration {}
"#;
        let library = parse(source).expect("source should parse");
        let Declaration::Struct(duration) = &library.declarations[0] else {
            panic!("expected a struct")
        };
        assert_eq!(
            duration.documentation.summary,
            "Represents a precise span of time."
        );
        assert_eq!(
            duration.documentation.details,
            "The first detail paragraph wraps across source lines.\n\nThe second detail paragraph remains separate."
        );
    }

    #[test]
    fn parses_state_provider_example_context() {
        let source = r#"
/// GBA emulators.
///
/// Provides GBA-addressed memory access.
///
/// # Example
///
/// Use the attached emulator
///
/// ```splitscript state GBA
/// let emulator: GbaEmulator = gba
/// ```
namespace gba {}
"#;
        let library = parse(source).expect("provider-qualified example should parse");
        let Declaration::Namespace(gba) = &library.declarations[0] else {
            panic!("expected a namespace")
        };
        assert_eq!(
            gba.documentation.examples[0].state_provider.as_deref(),
            Some("GBA")
        );
    }

    #[test]
    fn hidden_example_lines_only_participate_in_validation() {
        let source = r#"
/// Reads a value.
///
/// # Example
///
/// Read health
///
/// ```splitscript
/// # state "game.exe" {}
/// # onAttach {
/// # let player: address = 0x1000
/// let health = process.read<i32>(player)
/// # print(health)
/// # }
/// ```
namespace process {}
"#;
        let library = parse(source).expect("hidden example context should parse");
        let Declaration::Namespace(process) = &library.declarations[0] else {
            panic!("expected a namespace")
        };
        let example = &process.documentation.examples[0];
        assert_eq!(example.source, "let health = process.read<i32>(player)");
        assert_eq!(
            example.validation_source.as_deref(),
            Some(
                "state \"game.exe\" {}\nonAttach {\nlet player: address = 0x1000\nlet health = process.read<i32>(player)\nprint(health)\n}"
            )
        );
    }

    #[test]
    fn preserves_visible_and_hidden_example_indentation() {
        let source = r#"
/// Reads a value.
///
/// # Example
///
/// Read health
///
/// ```splitscript
/// # onAttach {
/// if ready {
///     print("ready")
/// }
/// #     print("validated")
/// # }
/// ```
namespace process {}
"#;
        let library = parse(source).expect("indented example should parse");
        let Declaration::Namespace(process) = &library.declarations[0] else {
            panic!("expected a namespace")
        };
        let example = &process.documentation.examples[0];
        assert_eq!(example.source, "if ready {\n    print(\"ready\")\n}");
        assert_eq!(
            example.validation_source.as_deref(),
            Some("onAttach {\nif ready {\n    print(\"ready\")\n}\n    print(\"validated\")\n}")
        );
    }

    #[test]
    fn preserves_exact_array_lengths_in_privileged_types() {
        let source = r#"
/// Fixed memory block.
struct Block {
    /// Four bytes.
    bytes: [u8; 4],

    /// Keeps exactly two values.
    static fn pair(values: [u64; 2]) -> [u64; 2] {
        return values
    }
}
"#;
        let library = parse(source).expect("fixed arrays should parse");
        let Declaration::Struct(block) = &library.declarations[0] else {
            panic!("expected a struct")
        };
        assert_eq!(block.fields[0].ty.to_string(), "[u8; 4]");
        assert_eq!(block.functions[0].parameters[0].ty.to_string(), "[u64; 2]");
        assert_eq!(block.functions[0].result.to_string(), "[u64; 2]");
    }

    #[test]
    fn parses_documented_enums() {
        let source = r#"
/// Current timer state.
@representation(enum)
enum TimerState {
    /// The timer has not started.
    NotRunning,
    /// The timer is running.
    Running,
}
"#;
        let library = parse(source).expect("source should parse");
        let Declaration::Enum(timer_state) = &library.declarations[0] else {
            panic!("expected an enum")
        };
        assert_eq!(timer_state.name, "TimerState");
        assert_eq!(
            timer_state.variants[1].documentation.summary,
            "The timer is running."
        );
    }

    #[test]
    fn captures_source_bodies_for_the_ordinary_parser() {
        let source = r#"
/// Time helpers.
struct Duration {
    /// Converts frames.
    ///
    /// Uses another library primitive.
    ///
    /// # Example
    ///
    /// Convert frames
    ///
    /// ```splitscript
    /// return Duration.fromFrames(120, 60)
    /// ```
    static fn fromFrames(frames: i64, fps: i64) -> Duration {
        let seconds = frames / fps
        return Duration.fromParts(seconds, 0)
    }
}
"#;
        let library = parse(source).expect("source should parse");
        let Declaration::Struct(duration) = &library.declarations[0] else {
            panic!("expected a struct")
        };
        assert_eq!(
            duration.functions[0].body.as_deref(),
            Some(
                "{\n        let seconds = frames / fps\n        return Duration.fromParts(seconds, 0)\n    }"
            )
        );
    }

    #[test]
    fn parses_intrinsic_types_generic_methods_and_parameter_rules() {
        let source = r#"
/// Compiler-provided value.
@representation(scalar, i64)
@valueUsage()
intrinsic type Signature {}

/// Process module.
@representation(gcStruct, nullable)
@valueUsage(localVariable)
struct Module {
    /// Reads a value.
    @intrinsic(ProcessRead)
    fn read<T: MemoryReadable>(
        /// The signature.
        @literal(signature)
        signature: Signature,
    ) -> T!;
}
"#;
        let library = parse(source).expect("source should parse");
        let Declaration::IntrinsicType(signature) = &library.declarations[0] else {
            panic!("expected an intrinsic type")
        };
        assert_eq!(signature.name, "Signature");
        let Declaration::Struct(module) = &library.declarations[1] else {
            panic!("expected a struct")
        };
        let read = &module.functions[0];
        assert!(!read.is_static);
        assert_eq!(read.type_parameters[0].name, "T");
        assert_eq!(read.type_parameters[0].constraints, ["MemoryReadable"]);
        assert_eq!(read.parameters[0].attributes[0].name, "literal");
        assert_eq!(read.result.to_string(), "T!");
    }

    #[test]
    fn accepts_trailing_commas_in_privileged_generic_lists() {
        let source = r#"
/// Generic container.
typeConstructor Box<T,> {
    /// Wraps a value.
    fn wrap<U,>(value: U,) -> Box<U,>;
}
"#;
        let library = parse(source).expect("trailing commas should parse");
        let Declaration::TypeConstructor(box_type) = &library.declarations[0] else {
            panic!("expected a type constructor")
        };
        assert_eq!(box_type.type_parameters[0].name, "T");
        assert_eq!(box_type.functions[0].type_parameters[0].name, "U");
        assert_eq!(box_type.functions[0].parameters[0].name, "value");
        assert_eq!(box_type.functions[0].result.to_string(), "Box<U>");
    }

    #[test]
    fn parses_adjacent_nested_generic_closers_in_privileged_source() {
        let source = r#"
/// Generic container.
typeConstructor Box<T> {
    /// Nests a value.
    fn nested<U>(value: U) -> Box<Box<U>>;
}
"#;
        let library = parse(source).expect("nested generic closers should parse without spaces");
        let Declaration::TypeConstructor(box_type) = &library.declarations[0] else {
            panic!("expected a type constructor")
        };
        assert_eq!(box_type.functions[0].result.to_string(), "Box<Box<U>>");
    }

    #[test]
    fn parses_mixed_optional_and_fallible_wrappers_in_privileged_source() {
        let source = r#"
/// Generic container.
typeConstructor Box<T> {
    /// Changes the wrapper order.
    fn transpose(value: T!?) -> T?!;
}
"#;
        let library = parse(source).expect("mixed type wrappers should compose");
        let Declaration::TypeConstructor(box_type) = &library.declarations[0] else {
            panic!("expected a type constructor")
        };
        assert_eq!(box_type.functions[0].parameters[0].ty.to_string(), "T!?");
        assert_eq!(box_type.functions[0].result.to_string(), "T?!");
    }

    #[test]
    fn parses_async_callable_results_in_privileged_source() {
        let source = r#"
root {
    @intrinsic(NextTick)
    private fn nextTick() -> async None;
}
"#;
        let library = parse(source).expect("async callable result should parse");
        let Declaration::Root(root) = &library.declarations[0] else {
            panic!("expected the root namespace")
        };
        let function = &root.functions[0];
        assert!(function.result_is_async);
        assert_eq!(function.result.to_string(), "None");
    }

    #[test]
    fn parses_source_defined_associated_constants() {
        let source = r#"
extend f32 {
    /// The canonical not-a-number value.
    const NaN: f32 = f32.fromBits(0x7fc00000);
}
"#;
        let library = parse(source).expect("associated constants should parse");
        let Declaration::CoreExtension(extension) = &library.declarations[0] else {
            panic!("expected a core extension")
        };
        let constant = &extension.functions[0];
        assert!(constant.is_constant);
        assert_eq!(constant.name, "NaN");
        assert_eq!(constant.result.to_string(), "f32");
        assert_eq!(
            constant.body.as_deref(),
            Some("{ return f32.fromBits(0x7fc00000) }")
        );
    }

    #[test]
    fn parses_structural_type_constructor_forms_without_public_names() {
        let library = parse(
            r#"
/// Arrays.
typeConstructor [T] {}
/// Optional values.
typeConstructor T? {}
/// Fallible values.
typeConstructor T! {}
"#,
        )
        .expect("structural type forms should parse");
        let expected = [
            ("Array", TypeConstructorSyntax::Array),
            ("Option", TypeConstructorSyntax::Optional),
            ("Result", TypeConstructorSyntax::Fallible),
        ];
        for (declaration, (catalog_name, syntax)) in library.declarations.iter().zip(expected) {
            let Declaration::TypeConstructor(constructor) = declaration else {
                panic!("expected a type constructor")
            };
            assert_eq!(constructor.name, catalog_name);
            assert_eq!(constructor.type_constructor_syntax, Some(syntax));
            assert_eq!(constructor.type_parameters[0].name, "T");
        }
    }

    #[test]
    fn parses_fields_on_structural_type_constructors() {
        let library = parse(
            r#"
/// Exclusive ranges.
typeConstructor T..<T {
    /// The lower endpoint.
    start: T,
    /// The upper endpoint.
    end: T,
}
"#,
        )
        .expect("structural type constructors should own generic fields");
        let Declaration::TypeConstructor(range) = &library.declarations[0] else {
            panic!("expected a type constructor")
        };
        assert_eq!(range.fields.len(), 2);
        assert_eq!(range.fields[0].name, "start");
        assert_eq!(range.fields[0].ty.to_string(), "T");
        assert_eq!(range.fields[1].name, "end");
        assert_eq!(range.fields[1].ty.to_string(), "T");
    }

    #[test]
    fn parses_constraints_on_inherited_callable_type_parameters() {
        let source = r#"
/// Arrays.
typeConstructor [T] {
    /// Finds a value.
    fn contains(value: T) -> bool where T: Equatable + Display, {
        return false
    }
}
"#;
        let library = parse(source).expect("where constraints should parse");
        let Declaration::TypeConstructor(array) = &library.declarations[0] else {
            panic!("expected a type constructor")
        };
        let constraints = &array.functions[0].where_constraints;
        assert_eq!(constraints.len(), 1);
        assert_eq!(constraints[0].name, "T");
        assert_eq!(constraints[0].constraints, ["Equatable", "Display"]);
    }

    #[test]
    fn parses_every_callable_owner_and_state_providers() {
        let source = r#"
root {
    /// Logs text.
    @intrinsic(Print)
    fn print(message: String) -> None;
}
/// Reads memory.
namespace process.read {}
/// Numeric values.
@behavior(declared)
capability Numeric<T> {}
/// Arrays.
typeConstructor [T] {}
extend address {}
/// GBA emulators.
@processType(GbaEmulator)
@attachment(identity)
@directRead(GbaEmulatorRead)
stateProvider GBA as gba { "mGBA.exe", "mGBA" }
/// Unity engines.
@processType(Process)
@processes(source)
@attachment(identity)
@directRead(ProcessRead)
stateProvider Unity as process {
    /// Selects IL2CPP metadata explicitly.
    ///
    /// Skips runtime-version auto-detection.
    selector il2cpp(version: u32),
    /// Selects Mono metadata explicitly.
    ///
    /// Skips runtime-version auto-detection.
    selector mono(version: MonoVersion),
}
"#;
        let library = parse(source).expect("source should parse");
        assert!(matches!(library.declarations[0], Declaration::Root(_)));
        let Declaration::Namespace(namespace) = &library.declarations[1] else {
            panic!("expected a namespace")
        };
        assert_eq!(namespace.name, "process.read");
        assert!(matches!(
            library.declarations[2],
            Declaration::Capability(_)
        ));
        assert!(matches!(
            library.declarations[3],
            Declaration::TypeConstructor(_)
        ));
        assert!(matches!(
            library.declarations[4],
            Declaration::CoreExtension(_)
        ));
        let Declaration::StateProvider(provider) = &library.declarations[5] else {
            panic!("expected a state provider")
        };
        assert_eq!(provider.value_name, "gba");
        assert_eq!(provider.processes, ["mGBA.exe", "mGBA"]);
        let Declaration::StateProvider(provider) = &library.declarations[6] else {
            panic!("expected the Unity state provider")
        };
        assert!(provider.processes.is_empty());
        assert_eq!(provider.selectors.len(), 2);
        assert_eq!(provider.selectors[0].name, "il2cpp");
        assert_eq!(provider.selectors[0].parameters[0].name, "version");
        assert_eq!(provider.selectors[0].parameters[0].ty.to_string(), "u32");
        assert_eq!(provider.selectors[1].name, "mono");
        assert_eq!(
            provider.selectors[1].parameters[0].ty.to_string(),
            "MonoVersion"
        );
    }
}
