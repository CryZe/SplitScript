use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=stdlib/standard.split");
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
