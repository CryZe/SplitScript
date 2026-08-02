use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{self, ExitCode},
    thread,
    time::Duration,
};

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
}

fn usage() {
    eprintln!(
        "usage: splitc <input.split> [-o <output.wasm>] [--profile debug|release] [--allow|--warn|--deny <SS100x|warnings>]..."
    );
    eprintln!(
        "       splitc watch <input.split> [-o <output.wasm>] [--profile debug|release] [--allow|--warn|--deny <SS100x|warnings>]..."
    );
    eprintln!("       splitc fmt <input.split> [--check]");
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Command, ()> {
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Err(());
    };
    if first == "fmt" {
        let Some(input) = args.next() else {
            return Err(());
        };
        let mut check = false;
        for arg in args {
            if arg != "--check" || check {
                return Err(());
            }
            check = true;
        }
        return Ok(Command::Format {
            input: PathBuf::from(input),
            check,
        });
    }
    let (watch, input) = if first == "watch" {
        let Some(input) = args.next() else {
            return Err(());
        };
        (true, PathBuf::from(input))
    } else {
        (false, PathBuf::from(first))
    };

    let mut output = input.with_extension("wasm");
    let mut profile = splitscript::BuildProfile::Debug;
    let mut profile_set = false;
    let mut warnings = splitscript::WarningPolicy::default();
    while let Some(arg) = args.next() {
        if arg == "-o" || arg == "--output" {
            let Some(path) = args.next() else {
                return Err(());
            };
            output = PathBuf::from(path);
        } else if arg == "--profile" {
            if profile_set {
                return Err(());
            }
            let Some(value) = args.next() else {
                return Err(());
            };
            profile = if value == "debug" {
                splitscript::BuildProfile::Debug
            } else if value == "release" {
                splitscript::BuildProfile::Release
            } else {
                return Err(());
            };
            profile_set = true;
        } else if arg == "--allow" || arg == "--warn" || arg == "--deny" {
            let level = if arg == "--allow" {
                splitscript::WarningLevel::Allow
            } else if arg == "--warn" {
                splitscript::WarningLevel::Warn
            } else {
                splitscript::WarningLevel::Deny
            };
            let Some(selector) = args.next() else {
                return Err(());
            };
            let Some(selector) = selector.to_str() else {
                return Err(());
            };
            if selector.eq_ignore_ascii_case("warnings") {
                warnings.set_all(level);
            } else {
                let Ok(code) = selector.parse::<splitscript::DiagnosticCode>() else {
                    return Err(());
                };
                if !warnings.set(code, level) {
                    return Err(());
                }
            }
        } else {
            return Err(());
        }
    }

    Ok(if watch {
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
    })
}

fn main() -> ExitCode {
    let command = match parse_args(env::args_os().skip(1)) {
        Ok(command) => command,
        Err(()) => {
            usage();
            return ExitCode::from(2);
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
    }
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
            for error in errors {
                eprintln!("{}", error.render(&input.display().to_string(), &source));
            }
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
    let (wasm, diagnostics) = match splitscript::compile_with_context_and_options_diagnostics(
        splitscript::CompilerContext::default(),
        source,
        options,
    ) {
        Ok(output) => output,
        Err(errors) => {
            for error in errors {
                eprintln!("{}", error.render(&input.display().to_string(), source));
            }
            return false;
        }
    };
    for diagnostic in diagnostics {
        eprintln!(
            "{}",
            diagnostic.render(&input.display().to_string(), source)
        );
    }

    if let Err(error) = replace_output(output, &wasm) {
        eprintln!("{}: {error}", output.display());
        return false;
    }

    println!("compiled {} -> {}", input.display(), output.display());
    true
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
            parse_args(["game.split".into()]),
            Ok(Command::Compile {
                input: "game.split".into(),
                output: "game.wasm".into(),
                profile: splitscript::BuildProfile::Debug,
                warnings: splitscript::WarningPolicy::default(),
            })
        );
        assert_eq!(
            parse_args(["fmt".into(), "game.split".into()]),
            Ok(Command::Format {
                input: "game.split".into(),
                check: false,
            })
        );
        assert_eq!(
            parse_args(["fmt".into(), "game.split".into(), "--check".into()]),
            Ok(Command::Format {
                input: "game.split".into(),
                check: true,
            })
        );
        assert_eq!(
            parse_args([
                "watch".into(),
                "game.split".into(),
                "-o".into(),
                "build/game.wasm".into(),
                "--profile".into(),
                "release".into(),
            ]),
            Ok(Command::Watch {
                input: "game.split".into(),
                output: "build/game.wasm".into(),
                profile: splitscript::BuildProfile::Release,
                warnings: splitscript::WarningPolicy::default(),
            })
        );

        let command = parse_args([
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
    fn rejects_incomplete_or_unknown_arguments() {
        assert!(parse_args(Vec::<OsString>::new()).is_err());
        assert!(parse_args(["watch".into()]).is_err());
        assert!(parse_args(["fmt".into()]).is_err());
        assert!(parse_args(["fmt".into(), "game.split".into(), "--write".into()]).is_err());
        assert!(
            parse_args([
                "fmt".into(),
                "game.split".into(),
                "--check".into(),
                "--check".into(),
            ])
            .is_err()
        );
        assert!(parse_args(["game.split".into(), "--wat".into()]).is_err());
        assert!(parse_args(["game.split".into(), "-o".into()]).is_err());
        assert!(parse_args(["game.split".into(), "--profile".into()]).is_err());
        assert!(parse_args(["game.split".into(), "--profile".into(), "fast".into(),]).is_err());
        assert!(parse_args(["game.split".into(), "--deny".into()]).is_err());
        assert!(parse_args(["game.split".into(), "--deny".into(), "SS0003".into()]).is_err());
        assert!(parse_args(["game.split".into(), "--allow".into(), "SS9999".into()]).is_err());
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
