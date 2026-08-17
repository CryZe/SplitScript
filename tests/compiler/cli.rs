use std::process::Command;

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
        (&["docs"][..], "# SplitScript reference"),
        (
            &["docs", "asl.lifecycle.update"][..],
            "# update lifecycle block",
        ),
        (&["docs", "Process.read"][..], "# Process.read"),
    ] {
        let output = splitc(arguments);
        assert!(output.status.success(), "arguments: {arguments:?}");
        assert!(output.stderr.is_empty(), "arguments: {arguments:?}");
        assert!(
            String::from_utf8(output.stdout).unwrap().contains(expected),
            "arguments: {arguments:?}",
        );
    }

    let unknown = splitc(&["docs", "Process.r"]);
    assert_eq!(unknown.status.code(), Some(1));
    assert!(unknown.stdout.is_empty());
    let stderr = String::from_utf8(unknown.stderr).unwrap();
    assert!(stderr.contains("unknown documentation topic"));
    assert!(stderr.contains("Process.read"));
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
