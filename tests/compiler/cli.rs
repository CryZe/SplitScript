use std::{fs, process::Command, time::SystemTime};

fn splitc(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_splitc"))
        .args(arguments)
        .output()
        .expect("splitc should start")
}

#[test]
fn help_is_successful_and_uses_stdout() {
    for arguments in [
        &["--help"][..],
        &["-h"][..],
        &["help"][..],
        &["help", "watch"][..],
        &["watch", "--help"][..],
        &["fmt", "--help"][..],
        &["docs", "--help"][..],
    ] {
        let output = splitc(arguments);
        assert!(output.status.success(), "arguments: {arguments:?}");
        assert!(output.stderr.is_empty(), "arguments: {arguments:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("Usage:"), "arguments: {arguments:?}");
    }
}

#[test]
fn documentation_topics_render_without_a_source_checkout() {
    for (arguments, expected) in [
        (&["docs"][..], "SplitScript reference"),
        (
            &["docs", "asl.lifecycle.update"][..],
            "update lifecycle block",
        ),
        (&["docs", "Process.read"][..], "Process.read"),
    ] {
        let output = splitc(arguments);
        assert!(output.status.success(), "arguments: {arguments:?}");
        assert!(output.stderr.is_empty(), "arguments: {arguments:?}");
        assert!(
            String::from_utf8(output.stdout).unwrap().contains(expected),
            "arguments: {arguments:?}",
        );
    }

    let search = splitc(&["docs", "Process.r"]);
    assert!(search.status.success());
    assert!(search.stderr.is_empty());
    let stdout = String::from_utf8(search.stdout).unwrap();
    assert!(stdout.contains("Documentation results for Process.r"));
    assert!(stdout.contains("Process.read"));

    let forced_color = Command::new(env!("CARGO_BIN_EXE_splitc"))
        .args(["docs", "Process.r"])
        .env("FORCE_COLOR", "2")
        .output()
        .expect("splitc should start with forced color");
    assert!(forced_color.status.success());
    assert!(forced_color.stderr.is_empty());
    let stdout = String::from_utf8(forced_color.stdout).unwrap();
    assert!(
        !stdout.contains('\u{1b}'),
        "redirected output must be plain text"
    );
    assert!(stdout.contains("Documentation results for Process.r"));
    assert!(stdout.contains("Process.read"));
}

#[test]
fn version_is_successful_and_stable() {
    for argument in ["--version", "-V"] {
        let output = splitc(&[argument]);
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("splitc {}\n", splitscript::COMPILER_VERSION_TEXT)
        );
    }
}

#[test]
fn invalid_invocation_remains_a_usage_error() {
    let output = splitc(&[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("the required <INPUT.split> argument was not provided"));
    assert!(stderr.contains("Usage:"));
}

#[test]
fn fmt_uses_the_nearest_editorconfig() {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "splitscript-editorconfig-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join(".editorconfig"),
        "root = true\n[*.split]\nindent_style = tab\nend_of_line = crlf\ninsert_final_newline = false\n",
    )
    .unwrap();
    let input = directory.join("game.split");
    fs::write(
        &input,
        "state \"game.exe\"{}\nwhileAttached{if true{print(1)}}",
    )
    .unwrap();

    let output = splitc(&["fmt", input.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let formatted = fs::read(&input).unwrap();
    let text = String::from_utf8(formatted).unwrap();
    assert!(text.contains("\r\n\tif true {\r\n\t\tprint(1)"));
    assert!(!text.ends_with(['\r', '\n']));

    fs::remove_dir_all(directory).unwrap();
}
