use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    println!("cargo:rerun-if-changed=stdlib/standard.split");
    emit_build_identity();
    let source = fs::read_to_string("stdlib/standard.split")
        .expect("the bundled standard-library source must be readable");
    let library = splitscript_stdlib_loader::parse(&source).unwrap_or_else(|errors| {
        panic!(
            "the bundled standard-library source does not parse:\n{}",
            errors
                .into_iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    let generated_catalog =
        splitscript_stdlib_loader::generate_catalog(&library).unwrap_or_else(|errors| {
            panic!(
                "the bundled standard-library catalog is invalid:\n{}",
                errors
                    .into_iter()
                    .map(|error| error.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        });
    let generated_ids =
        splitscript_stdlib_loader::generate_ids(&library).unwrap_or_else(|errors| {
            panic!(
                "the bundled standard-library identities are invalid:\n{}",
                errors
                    .into_iter()
                    .map(|error| error.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        });
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo provides OUT_DIR"))
        .join("stdlib_ids.rs");
    fs::write(output, generated_ids).expect("generated standard-library IDs must be writable");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo provides OUT_DIR"))
        .join("stdlib_catalog.rs");
    fs::write(output, generated_catalog)
        .expect("generated standard-library catalog must be writable");
}

fn emit_build_identity() {
    println!("cargo:rerun-if-env-changed=SPLITSCRIPT_GIT_REVISION");

    let revision = match env::var("SPLITSCRIPT_GIT_REVISION") {
        Ok(revision) => Some(validate_revision(revision)),
        Err(env::VarError::NotPresent) => git_revision(),
        Err(env::VarError::NotUnicode(_)) => {
            panic!("SPLITSCRIPT_GIT_REVISION must be valid Unicode")
        }
    };
    if let Some(revision) = revision {
        println!("cargo:rustc-env=SPLITSCRIPT_GIT_REVISION={revision}");
        let short_revision = revision.get(..12).unwrap_or(&revision);
        println!(
            "cargo:rustc-env=SPLITSCRIPT_VERSION_TEXT={} ({short_revision})",
            env::var("CARGO_PKG_VERSION").expect("Cargo provides CARGO_PKG_VERSION")
        );
    } else {
        println!(
            "cargo:rustc-env=SPLITSCRIPT_VERSION_TEXT={}",
            env::var("CARGO_PKG_VERSION").expect("Cargo provides CARGO_PKG_VERSION")
        );
    }
}

fn validate_revision(value: String) -> String {
    let value = value.trim();
    assert!(
        (7..=64).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "SPLITSCRIPT_GIT_REVISION must be a 7- to 64-digit hexadecimal Git object ID"
    );
    value.to_owned()
}

fn git_revision() -> Option<String> {
    let git_dir = git_output(["rev-parse", "--git-dir"])?;
    let git_dir = Path::new(&git_dir);
    let git_dir = if git_dir.is_absolute() {
        git_dir.to_owned()
    } else {
        Path::new(&env::var("CARGO_MANIFEST_DIR").ok()?).join(git_dir)
    };
    println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
    println!(
        "cargo:rerun-if-changed={}",
        git_dir.join("packed-refs").display()
    );
    if let Ok(head) = fs::read_to_string(git_dir.join("HEAD"))
        && let Some(reference) = head.trim().strip_prefix("ref: ")
    {
        println!(
            "cargo:rerun-if-changed={}",
            git_dir.join(reference).display()
        );
    }
    git_output(["rev-parse", "--verify", "HEAD"])
}

fn git_output<const N: usize>(arguments: [&str; N]) -> Option<String> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").ok()?.replace('\\', "/");
    let output = Command::new("git")
        .args(["-c", &format!("safe.directory={manifest_dir}")])
        .args(arguments)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}
