//! Repository-wide verification orchestration.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

struct RuntimeFixture {
    source: &'static str,
    output: &'static str,
    profile: &'static str,
    harness: &'static str,
    extra_arguments: &'static [&'static str],
}

const RUNTIME_FIXTURES: &[RuntimeFixture] = &[
    RuntimeFixture {
        source: "examples/hello_lunistice.split",
        output: "hello_lunistice.wasm",
        profile: "release",
        harness: "tests/runtime_smoke.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/abzu.split",
        output: "abzu.wasm",
        profile: "release",
        harness: "tests/abzu_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/abzu.split",
        output: "abzu.wasm",
        profile: "release",
        harness: "tests/abzu_runtime.mjs",
        extra_arguments: &["epic"],
    },
    RuntimeFixture {
        source: "examples/abzu.split",
        output: "abzu.wasm",
        profile: "release",
        harness: "tests/abzu_runtime.mjs",
        extra_arguments: &["unknown"],
    },
    RuntimeFixture {
        source: "examples/alan_wake.split",
        output: "alan_wake.wasm",
        profile: "release",
        harness: "tests/alan_wake_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/alan_wake.split",
        output: "alan_wake.wasm",
        profile: "release",
        harness: "tests/alan_wake_runtime.mjs",
        extra_arguments: &["gog"],
    },
    RuntimeFixture {
        source: "examples/alan_wake.split",
        output: "alan_wake.wasm",
        profile: "release",
        harness: "tests/alan_wake_runtime.mjs",
        extra_arguments: &["epic"],
    },
    RuntimeFixture {
        source: "examples/alan_wake.split",
        output: "alan_wake.wasm",
        profile: "release",
        harness: "tests/alan_wake_runtime.mjs",
        extra_arguments: &["unknown"],
    },
    RuntimeFixture {
        source: "examples/lunistice.split",
        output: "lunistice.debug.wasm",
        profile: "debug",
        harness: "tests/lunistice_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/lunistice.split",
        output: "lunistice.debug.wasm",
        profile: "debug",
        harness: "tests/lunistice_runtime.mjs",
        extra_arguments: &["--dlc"],
    },
    RuntimeFixture {
        source: "examples/lso_desktop_settings.split",
        output: "settings.wasm",
        profile: "release",
        harness: "tests/settings_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/minish_cap.split",
        output: "minish_cap.wasm",
        profile: "release",
        harness: "tests/minish_cap_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/minish_cap.split",
        output: "minish_cap.wasm",
        profile: "release",
        harness: "tests/minish_cap_runtime.mjs",
        extra_arguments: &["vba"],
    },
    RuntimeFixture {
        source: "examples/cancellation.split",
        output: "cancellation.wasm",
        profile: "release",
        harness: "tests/cancellation_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/action_defaults.split",
        output: "action_defaults.wasm",
        profile: "release",
        harness: "tests/action_defaults_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/async_loop.split",
        output: "async_loop.wasm",
        profile: "release",
        harness: "tests/async_loop_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/fallible_process_operations.split",
        output: "fallible_process_operations.wasm",
        profile: "release",
        harness: "tests/fallible_process_operations_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/for_loop.split",
        output: "for_loop.wasm",
        profile: "release",
        harness: "tests/for_loop_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/process_results.split",
        output: "process_results.wasm",
        profile: "release",
        harness: "tests/process_results_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/postfix_calls.split",
        output: "postfix_calls.wasm",
        profile: "release",
        harness: "tests/postfix_calls_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/while_loop.split",
        output: "while_loop.wasm",
        profile: "release",
        harness: "tests/while_loop_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/wrapper_equality.split",
        output: "wrapper_equality.wasm",
        profile: "release",
        harness: "tests/wrapper_equality_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/wrapper_fallbacks.split",
        output: "wrapper_fallbacks.wasm",
        profile: "release",
        harness: "tests/wrapper_fallbacks_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/wrapper_match.split",
        output: "wrapper_match.wasm",
        profile: "release",
        harness: "tests/wrapper_match_runtime.mjs",
        extra_arguments: &[],
    },
];

fn main() -> ExitCode {
    let mut arguments = env::args().skip(1);
    match (arguments.next().as_deref(), arguments.next()) {
        (Some("check"), None) => match check() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("verification failed: {error}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("usage: cargo xtask check");
            ExitCode::FAILURE
        }
    }
}

fn check() -> Result<(), String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    run(&root, "cargo", &["fmt", "--all", "--", "--check"])?;
    run(
        &root,
        "cargo",
        &["clippy", "--all-targets", "--", "-D", "warnings"],
    )?;
    run(
        &root,
        "cargo",
        &[
            "test",
            "--package",
            "splitscript-syntax",
            "--package",
            "splitscript-stdlib-loader",
        ],
    )?;
    // Do not ask Cargo to build the currently running xtask as a test harness:
    // Windows cannot replace its locked executable. These targets are the
    // complete repository test surface owned by the product.
    run(
        &root,
        "cargo",
        &[
            "test",
            "--lib",
            "--bin",
            "splitc",
            "--bin",
            "splitls",
            "--test",
            "compiler",
            "--examples",
        ],
    )?;
    run(&root, "cargo", &["build", "--bin", "splitc"])?;
    run(
        &root,
        "cargo",
        &[
            "build",
            "--release",
            "--target",
            "wasm32-unknown-unknown",
            "--package",
            "splitscript-vscode-wasm",
        ],
    )?;

    let extension = root.join("editors/vscode");
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    run(&extension, npm, &["run", "check"])?;
    run(&extension, npm, &["test"])?;
    run(&extension, npm, &["run", "clean"])?;
    run(&extension, npm, &["run", "compile:ts"])?;
    run(&extension, npm, &["run", "bundle:web"])?;
    let embedded_compiler =
        root.join("target/wasm32-unknown-unknown/release/splitscript_vscode_wasm.wasm");
    let packaged_compiler = extension.join("dist/splitscript_vscode_wasm.wasm");
    fs::copy(&embedded_compiler, &packaged_compiler).map_err(|error| {
        format!(
            "could not package {} as {}: {error}",
            embedded_compiler.display(),
            packaged_compiler.display()
        )
    })?;
    run(
        &root,
        "wasm-tools",
        &["validate", path_text(&embedded_compiler)?],
    )?;
    run(
        &root,
        "node",
        &[
            "tests/embedded_compiler_runtime.mjs",
            path_text(&embedded_compiler)?,
        ],
    )?;
    let embedded_worker = extension.join("dist/embeddedCompilerNodeWorker.js");
    run(
        &root,
        "node",
        &[
            "tests/embedded_compiler_worker_runtime.mjs",
            path_text(&embedded_worker)?,
            path_text(&packaged_compiler)?,
        ],
    )?;
    run(
        &root,
        "node",
        &[
            "tests/web_worker_bundles_runtime.mjs",
            path_text(&extension.join("dist/web/extension.js"))?,
            path_text(&extension.join("dist/web/embeddedCompilerWorker.js"))?,
            path_text(&extension.join("dist/web/embeddedLanguageServerWorker.js"))?,
            path_text(&packaged_compiler)?,
        ],
    )?;
    run(&extension, npm, &["run", "test:web-host"])?;
    let embedded_language_server_worker =
        extension.join("dist/embeddedLanguageServerNodeWorker.js");
    run(
        &root,
        "node",
        &[
            "tests/embedded_language_server_worker_runtime.mjs",
            path_text(&embedded_language_server_worker)?,
            path_text(&packaged_compiler)?,
        ],
    )?;

    let outputs = root.join("target/verify");
    fs::create_dir_all(&outputs)
        .map_err(|error| format!("could not create {}: {error}", outputs.display()))?;
    let compiler = root.join(if cfg!(windows) {
        "target/debug/splitc.exe"
    } else {
        "target/debug/splitc"
    });
    for fixture in RUNTIME_FIXTURES {
        compile_once(
            &root,
            &compiler,
            fixture.source,
            &outputs.join(fixture.output),
            fixture.profile,
        )?;
    }
    let lunistice_release = outputs.join("lunistice.release.wasm");
    compile_once(
        &root,
        &compiler,
        "examples/lunistice.split",
        &lunistice_release,
        "release",
    )?;
    let debug = outputs.join("debug_profile.debug.wasm");
    let release = outputs.join("debug_profile.release.wasm");
    compile_once(
        &root,
        &compiler,
        "tests/debug_profile.split",
        &debug,
        "debug",
    )?;
    compile_once(
        &root,
        &compiler,
        "tests/debug_profile.split",
        &release,
        "release",
    )?;

    let mut validated = RUNTIME_FIXTURES
        .iter()
        .map(|fixture| outputs.join(fixture.output))
        .collect::<Vec<_>>();
    validated.extend([lunistice_release, debug.clone(), release.clone()]);
    validated.sort();
    validated.dedup();
    for module in &validated {
        run(
            &root,
            "wasm-tools",
            &["validate", "--features", "all", path_text(module)?],
        )?;
    }

    for fixture in RUNTIME_FIXTURES {
        let module = outputs.join(fixture.output);
        let mut arguments = vec![fixture.harness, path_text(&module)?];
        arguments.extend_from_slice(fixture.extra_arguments);
        run(&root, "node", &arguments)?;
    }
    run(
        &root,
        "node",
        &[
            "tests/debug_profile_runtime.mjs",
            path_text(&debug)?,
            path_text(&release)?,
        ],
    )?;
    println!("repository verification passed");
    Ok(())
}

fn compile_once(
    root: &Path,
    compiler: &Path,
    source: &str,
    output: &Path,
    profile: &str,
) -> Result<(), String> {
    run(
        root,
        path_text(compiler)?,
        &[source, "-o", path_text(output)?, "--profile", profile],
    )
}

fn path_text(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| format!("path is not valid UTF-8: {}", path.display()))
}

fn run(directory: &Path, command: &str, arguments: &[&str]) -> Result<(), String> {
    println!("> {command} {}", arguments.join(" "));
    let status = Command::new(command)
        .args(arguments)
        .current_dir(directory)
        .status()
        .map_err(|error| format!("could not launch `{command}`: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{command}` exited with {status}"))
    }
}
