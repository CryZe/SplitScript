use std::{
    env,
    ffi::OsString,
    fs,
    io::IsTerminal as _,
    path::{Path, PathBuf},
    process::{self, ExitCode},
    thread,
    time::Duration,
};

use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use codespan_reporting::term::termcolor::{ColorChoice, StandardStream};

mod cli_diagnostics;
mod cli_documentation;

const WATCH_INTERVAL: Duration = Duration::from_millis(150);

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Compile {
        input: PathBuf,
        output: PathBuf,
        profile: splitscript::BuildProfile,
        warnings: splitscript::WarningPolicy,
    },
    Watch {
        input: PathBuf,
        output: PathBuf,
        profile: splitscript::BuildProfile,
        warnings: splitscript::WarningPolicy,
    },
    Format {
        input: PathBuf,
        check: bool,
    },
    Documentation {
        topic: Option<String>,
    },
}

/// Compile, watch, format, and explore SplitScript autosplitters.
#[derive(Debug, Parser)]
#[command(
    name = "splitc",
    bin_name = "splitc",
    version = splitscript::COMPILER_VERSION_TEXT,
    args_conflicts_with_subcommands = true
)]
struct Cli {
    /// Source file to compile. The output defaults to the same path with a
    /// `.wasm` extension.
    #[arg(value_name = "INPUT.split")]
    input: Option<PathBuf>,

    #[command(flatten)]
    build: BuildArgs,

    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Compile immediately, then rebuild whenever the source changes.
    #[command(
        after_help = "A failed build leaves the last successful output untouched. Press Ctrl+C to stop watching."
    )]
    Watch(CompileArgs),
    /// Format a source file in place.
    Fmt(FormatArgs),
    /// Render compiler-owned language, migration, and standard-library documentation.
    #[command(
        name = "docs",
        after_help = "QUERY may be an exact symbol such as Process.read, a migration identity printed by a diagnostic, a foreign spelling, or ordinary search terms. Exact matches open directly; broader queries show ranked results. With no query, renders the reference index."
    )]
    Documentation(DocumentationArgs),
}

#[derive(Debug, Args)]
struct CompileArgs {
    /// Source file to compile.
    #[arg(value_name = "INPUT.split")]
    input: PathBuf,

    #[command(flatten)]
    build: BuildArgs,
}

#[derive(Debug, Args)]
struct FormatArgs {
    /// Source file to format.
    #[arg(value_name = "INPUT.split")]
    input: PathBuf,

    /// Verify canonical formatting without writing the file.
    #[arg(long)]
    check: bool,
}

#[derive(Debug, Args)]
struct DocumentationArgs {
    /// Exact documentation topic or search terms.
    #[arg(value_name = "QUERY", num_args = 0..)]
    query: Vec<String>,
}

#[derive(Debug, Args)]
struct BuildArgs {
    /// Output path. Defaults to INPUT.wasm.
    #[arg(short = 'o', long, value_name = "OUTPUT.wasm")]
    output: Option<PathBuf>,

    /// Select debug information and debug-only code, or an optimized release
    /// module.
    #[arg(long, value_enum, default_value_t)]
    profile: CliProfile,

    #[command(flatten)]
    warnings: WarningArgs,
}

#[derive(Debug, Default, Clone, Copy, ValueEnum)]
enum CliProfile {
    #[default]
    Debug,
    Release,
}

impl From<CliProfile> for splitscript::BuildProfile {
    fn from(profile: CliProfile) -> Self {
        match profile {
            CliProfile::Debug => Self::Debug,
            CliProfile::Release => Self::Release,
        }
    }
}

#[derive(Debug, Args)]
struct WarningArgs {
    /// Suppress a warning code, or every warning with `warnings`.
    #[arg(long, value_name = "CODE|warnings", value_parser = parse_warning_selector)]
    allow: Vec<WarningSelector>,

    /// Emit a warning code, or every warning with `warnings`.
    #[arg(long, value_name = "CODE|warnings", value_parser = parse_warning_selector)]
    warn: Vec<WarningSelector>,

    /// Treat a warning code, or every warning with `warnings`, as an error.
    #[arg(long, value_name = "CODE|warnings", value_parser = parse_warning_selector)]
    deny: Vec<WarningSelector>,
}

#[derive(Debug, Clone, Copy)]
enum WarningSelector {
    All,
    Code(splitscript::DiagnosticCode),
}

fn parse_warning_selector(value: &str) -> Result<WarningSelector, String> {
    if value.eq_ignore_ascii_case("warnings") {
        return Ok(WarningSelector::All);
    }
    let code = value
        .parse::<splitscript::DiagnosticCode>()
        .map_err(|_| format!("`{value}` is not a warning code such as SS1002 or `warnings`"))?;
    splitscript::WarningPolicy::default()
        .level(code)
        .is_some()
        .then_some(WarningSelector::Code(code))
        .ok_or_else(|| format!("`{value}` is not a configurable warning code"))
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Command, clap::Error> {
    let matches = Cli::command().try_get_matches_from(args)?;
    let cli = Cli::from_arg_matches(&matches)?;
    match cli.command {
        Some(CliCommand::Watch(arguments)) => {
            let subcommand = matches
                .subcommand_matches("watch")
                .expect("Clap should retain matches for the parsed watch command");
            Ok(compile_command(
                arguments.input,
                arguments.build,
                subcommand,
                true,
            ))
        }
        Some(CliCommand::Fmt(arguments)) => Ok(Command::Format {
            input: arguments.input,
            check: arguments.check,
        }),
        Some(CliCommand::Documentation(arguments)) => Ok(Command::Documentation {
            topic: (!arguments.query.is_empty()).then(|| arguments.query.join(" ")),
        }),
        None => {
            let Some(input) = cli.input else {
                return Err(Cli::command().error(
                    clap::error::ErrorKind::MissingRequiredArgument,
                    "the required <INPUT.split> argument was not provided",
                ));
            };
            Ok(compile_command(input, cli.build, &matches, false))
        }
    }
}

fn compile_command(
    input: PathBuf,
    arguments: BuildArgs,
    matches: &clap::ArgMatches,
    watch: bool,
) -> Command {
    let output = arguments
        .output
        .unwrap_or_else(|| input.with_extension("wasm"));
    let profile = arguments.profile.into();
    let warnings = warning_policy(&arguments.warnings, matches);
    if watch {
        Command::Watch {
            input,
            output,
            profile,
            warnings,
        }
    } else {
        Command::Compile {
            input,
            output,
            profile,
            warnings,
        }
    }
}

fn warning_policy(
    arguments: &WarningArgs,
    matches: &clap::ArgMatches,
) -> splitscript::WarningPolicy {
    let mut directives = Vec::new();
    for (name, level, selectors) in [
        ("allow", splitscript::WarningLevel::Allow, &arguments.allow),
        ("warn", splitscript::WarningLevel::Warn, &arguments.warn),
        ("deny", splitscript::WarningLevel::Deny, &arguments.deny),
    ] {
        if let Some(indices) = matches.indices_of(name) {
            directives.extend(
                indices
                    .zip(selectors)
                    .map(|(index, selector)| (index, level, *selector)),
            );
        }
    }
    directives.sort_unstable_by_key(|(index, _, _)| *index);

    let mut policy = splitscript::WarningPolicy::default();
    for (_, level, selector) in directives {
        match selector {
            WarningSelector::All => policy.set_all(level),
            WarningSelector::Code(code) => {
                assert!(
                    policy.set(code, level),
                    "Clap accepted an unknown warning code"
                );
            }
        }
    }
    policy
}

fn main() -> ExitCode {
    let command = match parse_args(env::args_os()) {
        Ok(command) => command,
        Err(error) => {
            let exit_code = error.exit_code();
            if let Err(print_error) = error.print() {
                eprintln!("splitc: could not print command-line error: {print_error}");
            }
            return ExitCode::from(exit_code as u8);
        }
    };

    match command {
        Command::Compile {
            input,
            output,
            profile,
            warnings,
        } => match fs::read(&input) {
            Ok(source)
                if compile_source(
                    &input,
                    &output,
                    &source,
                    splitscript::CompilerOptions { profile, warnings },
                ) =>
            {
                ExitCode::SUCCESS
            }
            Ok(_) => ExitCode::FAILURE,
            Err(error) => {
                eprintln!("{}: {error}", input.display());
                ExitCode::FAILURE
            }
        },
        Command::Watch {
            input,
            output,
            profile,
            warnings,
        } => watch(
            &input,
            &output,
            splitscript::CompilerOptions { profile, warnings },
        ),
        Command::Format { input, check } => {
            if format_file(&input, check) {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Command::Documentation { topic } => render_documentation(topic.as_deref()),
    }
}

fn render_documentation(topic: Option<&str>) -> ExitCode {
    let reference = splitscript::DocumentationReference::for_terminal();
    let requested = topic.unwrap_or("");
    let markdown = if let Some(page) = reference.topic(requested) {
        page.markdown
    } else {
        let results = reference.search(requested);
        if results.is_empty() {
            eprintln!("splitc: no documentation matches `{requested}`");
            return ExitCode::FAILURE;
        }
        cli_documentation::search_results_markdown(requested, &results)
    };
    let width = if std::io::stdout().is_terminal() {
        textwrap::termwidth()
    } else {
        80
    };
    let writer = StandardStream::stdout(ColorChoice::Auto);
    if let Err(error) = cli_documentation::emit(&mut writer.lock(), &markdown, width) {
        eprintln!("splitc: could not render documentation: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn format_file(input: &Path, check: bool) -> bool {
    let source = match fs::read_to_string(input) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("{}: {error}", input.display());
            return false;
        }
    };
    let formatted = match splitscript::format_source(&source) {
        Ok(formatted) => formatted,
        Err(errors) => {
            emit_diagnostics(input, &source, &errors);
            return false;
        }
    };
    if formatted == source {
        return true;
    }
    if check {
        eprintln!("{} is not formatted", input.display());
        return false;
    }
    if let Err(error) = fs::write(input, formatted) {
        eprintln!("{}: {error}", input.display());
        return false;
    }
    println!("formatted {}", input.display());
    true
}

fn watch(input: &Path, output: &Path, options: splitscript::CompilerOptions) -> ExitCode {
    println!(
        "watching {} -> {} [{:?}] (press Ctrl+C to stop)",
        input.display(),
        output.display(),
        options.profile
    );

    // Compare contents rather than timestamps. Editors commonly save by
    // replacing a file and some filesystems have coarse timestamp precision;
    // content snapshots handle both without platform-specific watcher APIs.
    let mut previous_source: Option<Vec<u8>> = None;
    let mut read_failed = false;
    loop {
        match fs::read(input) {
            Ok(source) => {
                if read_failed || previous_source.as_deref() != Some(source.as_slice()) {
                    read_failed = false;
                    previous_source = Some(source.clone());
                    compile_source(input, output, &source, options);
                }
            }
            Err(error) => {
                if !read_failed {
                    eprintln!(
                        "{}: {error}; waiting for the file to become readable",
                        input.display()
                    );
                }
                read_failed = true;
                previous_source = None;
            }
        }
        thread::sleep(WATCH_INTERVAL);
    }
}

fn compile_source(
    input: &Path,
    output: &Path,
    source: &[u8],
    options: splitscript::CompilerOptions,
) -> bool {
    let source = match std::str::from_utf8(source) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("{}: source is not valid UTF-8: {error}", input.display());
            return false;
        }
    };
    let source_path = std::path::absolute(input).unwrap_or_else(|_| input.to_path_buf());
    let source_name = source_path.to_string_lossy();
    let (wasm, diagnostics) = match splitscript::compile_named_with_context_and_options_diagnostics(
        splitscript::CompilerContext::default(),
        source_name.as_ref(),
        source,
        options,
    ) {
        Ok(output) => output,
        Err(errors) => {
            emit_diagnostics(input, source, &errors);
            return false;
        }
    };
    emit_diagnostics(input, source, &diagnostics);

    if let Err(error) = replace_output(output, &wasm) {
        eprintln!("{}: {error}", output.display());
        return false;
    }

    println!("compiled {} -> {}", input.display(), output.display());
    true
}

fn emit_diagnostics(input: &Path, source: &str, diagnostics: &[splitscript::Diagnostic]) {
    let writer = StandardStream::stderr(ColorChoice::Auto);
    if let Err(error) = cli_diagnostics::emit(
        &mut writer.lock(),
        &input.display().to_string(),
        source,
        diagnostics,
    ) {
        eprintln!("splitc: could not render diagnostics: {error}");
    }
}

/// Writes beside the destination and renames only once the complete Wasm is
/// ready. Consumers watching the output never observe a partially written
/// module, and a failed compilation never touches the last successful build.
fn replace_output(output: &Path, wasm: &[u8]) -> std::io::Result<()> {
    let mut temporary_name = output
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("output.wasm"))
        .to_os_string();
    temporary_name.push(format!(".tmp-{}", process::id()));
    let temporary = output.with_file_name(temporary_name);

    fs::write(&temporary, wasm)?;
    if let Err(error) = fs::rename(&temporary, output) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_compile_and_watch_commands() {
        assert_eq!(
            parse_args(["splitc".into(), "game.split".into()]).unwrap(),
            Command::Compile {
                input: "game.split".into(),
                output: "game.wasm".into(),
                profile: splitscript::BuildProfile::Debug,
                warnings: splitscript::WarningPolicy::default(),
            }
        );
        assert_eq!(
            parse_args(["splitc".into(), "fmt".into(), "game.split".into()]).unwrap(),
            Command::Format {
                input: "game.split".into(),
                check: false,
            }
        );
        assert_eq!(
            parse_args([
                "splitc".into(),
                "fmt".into(),
                "game.split".into(),
                "--check".into(),
            ])
            .unwrap(),
            Command::Format {
                input: "game.split".into(),
                check: true,
            }
        );
        assert_eq!(
            parse_args([
                "splitc".into(),
                "watch".into(),
                "game.split".into(),
                "-o".into(),
                "build/game.wasm".into(),
                "--profile".into(),
                "release".into(),
            ])
            .unwrap(),
            Command::Watch {
                input: "game.split".into(),
                output: "build/game.wasm".into(),
                profile: splitscript::BuildProfile::Release,
                warnings: splitscript::WarningPolicy::default(),
            }
        );
        assert_eq!(
            parse_args(["splitc".into(), "docs".into()]).unwrap(),
            Command::Documentation { topic: None }
        );
        assert_eq!(
            parse_args([
                "splitc".into(),
                "docs".into(),
                "asl.lifecycle.update".into(),
            ])
            .unwrap(),
            Command::Documentation {
                topic: Some("asl.lifecycle.update".to_owned()),
            }
        );
        assert_eq!(
            parse_args([
                "splitc".into(),
                "docs".into(),
                "multiple".into(),
                "processes".into(),
            ])
            .unwrap(),
            Command::Documentation {
                topic: Some("multiple processes".to_owned()),
            }
        );

        let command = parse_args([
            "splitc".into(),
            "game.split".into(),
            "--deny".into(),
            "warnings".into(),
            "--allow".into(),
            "ss1002".into(),
        ])
        .unwrap();
        let Command::Compile { warnings, .. } = command else {
            panic!("expected a compile command");
        };
        assert_eq!(
            warnings.level(splitscript::DiagnosticCode::MustUse),
            Some(splitscript::WarningLevel::Deny)
        );
        assert_eq!(
            warnings.level(splitscript::DiagnosticCode::UnusedBinding),
            Some(splitscript::WarningLevel::Allow)
        );
    }

    #[test]
    fn clap_handles_help_and_version_in_conventional_positions() {
        for arguments in [
            vec!["splitc", "--help"],
            vec!["splitc", "-h"],
            vec!["splitc", "help", "watch"],
            vec!["splitc", "watch", "--help"],
            vec!["splitc", "fmt", "game.split", "--help"],
            vec!["splitc", "docs", "--help"],
        ] {
            assert_eq!(
                parse_args(arguments.into_iter().map(OsString::from))
                    .unwrap_err()
                    .kind(),
                clap::error::ErrorKind::DisplayHelp
            );
        }
        for argument in ["--version", "-V"] {
            assert_eq!(
                parse_args(["splitc".into(), argument.into()])
                    .unwrap_err()
                    .kind(),
                clap::error::ErrorKind::DisplayVersion
            );
        }
    }

    #[test]
    fn rejects_incomplete_or_unknown_arguments() {
        assert!(parse_args(["splitc".into()]).is_err());
        assert!(parse_args(["splitc".into(), "--help".into(), "extra".into()]).is_err());
        assert!(parse_args(["splitc".into(), "--version".into(), "extra".into()]).is_err());
        assert!(parse_args(["splitc".into(), "help".into(), "unknown".into()]).is_err());
        assert!(parse_args(["splitc".into(), "watch".into()]).is_err());
        assert!(parse_args(["splitc".into(), "fmt".into()]).is_err());
        assert!(
            parse_args([
                "splitc".into(),
                "fmt".into(),
                "game.split".into(),
                "--write".into()
            ])
            .is_err()
        );
        assert!(
            parse_args([
                "splitc".into(),
                "fmt".into(),
                "game.split".into(),
                "--check".into(),
                "--check".into(),
            ])
            .is_err()
        );
        assert!(parse_args(["splitc".into(), "game.split".into(), "--wat".into()]).is_err());
        assert!(parse_args(["splitc".into(), "game.split".into(), "-o".into()]).is_err());
        assert!(parse_args(["splitc".into(), "game.split".into(), "--profile".into()]).is_err());
        assert!(
            parse_args([
                "splitc".into(),
                "game.split".into(),
                "--profile".into(),
                "fast".into(),
            ])
            .is_err()
        );
        assert!(parse_args(["splitc".into(), "game.split".into(), "--deny".into()]).is_err());
        assert!(
            parse_args([
                "splitc".into(),
                "game.split".into(),
                "--deny".into(),
                "SS0003".into()
            ])
            .is_err()
        );
        assert!(
            parse_args([
                "splitc".into(),
                "game.split".into(),
                "--allow".into(),
                "SS9999".into()
            ])
            .is_err()
        );
    }

    #[test]
    fn format_command_checks_and_rewrites_a_source_file() {
        let directory =
            std::env::temp_dir().join(format!("splitscript-format-command-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let input = directory.join("game.split");
        let unformatted = "state \"game.exe\"{}\nwhileAttached{print(\"ready\")}";
        fs::write(&input, unformatted).unwrap();

        assert!(!format_file(&input, true));
        assert_eq!(fs::read_to_string(&input).unwrap(), unformatted);
        assert!(format_file(&input, false));
        let formatted = fs::read_to_string(&input).unwrap();
        assert_eq!(
            formatted,
            "state \"game.exe\" {}\nwhileAttached {\n    print(\"ready\")\n}\n"
        );
        assert!(format_file(&input, true));

        fs::remove_file(input).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
