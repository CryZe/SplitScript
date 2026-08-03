use super::{
    Attribute, AttributeArgument, CallableOwnerDeclaration, Declaration, Documentation,
    EnumDeclaration, Error, Example, FieldDeclaration, FunctionDeclaration, Library, Parameter,
    StateProviderDeclaration, StructDeclaration, Type, TypeParameter, VariantDeclaration,
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

impl Parser<'_> {
    fn library(&mut self) -> Result<Library, Error> {
        let mut declarations = Vec::new();
        while !self.at(&TokenKind::Eof) {
            let documentation = self.documentation()?;
            let attributes = self.attributes()?;
            if self.eat_ident("struct") {
                declarations.push(Declaration::Struct(
                    self.struct_declaration(documentation, attributes)?,
                ));
            } else if self.eat_ident("enum") {
                declarations.push(Declaration::Enum(
                    self.enum_declaration(documentation, attributes)?,
                ));
            } else if self.eat_ident("intrinsic") {
                if !self.eat_ident("type") {
                    return Err(self.error("expected `type` after `intrinsic`"));
                }
                declarations.push(Declaration::IntrinsicType(
                    self.struct_declaration(documentation, attributes)?,
                ));
            } else if self.eat_ident("root") {
                declarations.push(Declaration::Root(self.callable_owner_declaration(
                    "root".to_owned(),
                    documentation,
                    attributes,
                    true,
                )?));
            } else if self.eat_ident("namespace") {
                let name = self.path("expected a namespace path")?;
                declarations.push(Declaration::Namespace(self.callable_owner_declaration(
                    name,
                    documentation,
                    attributes,
                    true,
                )?));
            } else if self.eat_ident("capability") {
                let name = self.ident("expected a capability name")?;
                declarations.push(Declaration::Capability(self.callable_owner_declaration(
                    name,
                    documentation,
                    attributes,
                    false,
                )?));
            } else if self.eat_ident("typeConstructor") {
                let name = self.ident("expected a type-constructor name")?;
                declarations.push(Declaration::TypeConstructor(
                    self.callable_owner_declaration(name, documentation, attributes, false)?,
                ));
            } else if self.eat_ident("extend") {
                let name = self.ident("expected a core type after `extend`")?;
                declarations.push(Declaration::CoreExtension(
                    self.callable_owner_declaration(name, documentation, attributes, false)?,
                ));
            } else if self.eat_ident("stateProvider") {
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
        while !self.at(&TokenKind::RBrace) {
            let TokenKind::String(process) = self.current().kind.clone() else {
                return Err(self.error("expected a quoted process name"));
            };
            self.bump();
            processes.push(process);
            if !self.eat(&TokenKind::Comma) && !self.at(&TokenKind::RBrace) {
                return Err(self.error("expected `,` between process names"));
            }
        }
        self.bump();
        Ok(StateProviderDeclaration {
            name,
            value_name,
            processes,
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
    ) -> Result<CallableOwnerDeclaration, Error> {
        let type_parameters = self.type_parameters()?;
        self.expect(TokenKind::LBrace, "expected `{` after the declaration name")?;
        let mut functions = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                return Err(self.error("expected `}` after the declaration"));
            }
            let documentation = self.documentation()?;
            let attributes = self.attributes()?;
            if !self.eat_ident("fn") {
                return Err(self.error("expected a function declaration"));
            }
            functions.push(self.function_declaration(
                documentation,
                attributes,
                functions_are_static,
            )?);
        }
        self.bump();
        Ok(CallableOwnerDeclaration {
            name,
            type_parameters,
            documentation,
            attributes,
            functions,
        })
    }

    fn enum_declaration(
        &mut self,
        documentation: Documentation,
        attributes: Vec<Attribute>,
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
            documentation,
            attributes,
            variants,
        })
    }

    fn struct_declaration(
        &mut self,
        documentation: Documentation,
        attributes: Vec<Attribute>,
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
                if private {
                    return Err(self.error("standard-library functions cannot be `private`"));
                }
                functions.push(self.function_declaration(
                    member_documentation,
                    member_attributes,
                    is_static,
                )?);
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
        let result = self.ty()?;
        let body = if self.eat(&TokenKind::Semicolon) {
            None
        } else if self.at(&TokenKind::LBrace) {
            Some(self.function_body_source()?)
        } else {
            return Err(self.error("expected `;` or a function body after the return type"));
        };
        Ok(FunctionDeclaration {
            name,
            type_parameters,
            parameters,
            result,
            is_static,
            documentation,
            attributes,
            body,
        })
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
            }
            self.expect(TokenKind::Gt, "expected `>` after type parameters")?;
        }
        Ok(type_parameters)
    }

    fn ty(&mut self) -> Result<Type, Error> {
        let mut ty = if self.eat(&TokenKind::LBracket) {
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
                    } else {
                        break;
                    }
                }
                self.expect(TokenKind::Gt, "expected `>` after type arguments")?;
                Type::Application {
                    constructor: name,
                    arguments,
                }
            } else {
                Type::Name(name)
            }
        };
        if self.eat(&TokenKind::Question) {
            ty = Type::Option(Box::new(ty));
        } else if self.eat(&TokenKind::Bang) {
            ty = Type::Result(Box::new(ty));
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
        examples.push(Example {
            title,
            source: remaining[fence_start + 1..fence_start + 1 + relative_end].join("\n"),
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
        assert_eq!(duration.documentation.details, "Used for game time.");
        assert_eq!(duration.fields[0].documentation.summary, "Whole seconds.");
        assert_eq!(duration.functions[0].parameters[0].ty.to_string(), "f32");
        assert_eq!(
            duration.functions[0].documentation.examples[0].source,
            "return Duration.fromSeconds(seconds)"
        );
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
typeConstructor Array<T> {}
extend address {}
/// GBA emulators.
@processType(GbaEmulator)
@attachment(GbaAttach)
@directRead(GbaEmulatorRead)
stateProvider GBA as gba { "mGBA.exe", "mGBA" }
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
    }
}
