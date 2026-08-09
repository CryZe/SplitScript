use serde::Serialize;

/// Stable identity of the compiler that produced a result or artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompilerIdentity {
    pub version: &'static str,
    pub git_revision: Option<&'static str>,
}

pub const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COMPILER_GIT_REVISION: Option<&str> = option_env!("SPLITSCRIPT_GIT_REVISION");
pub const COMPILER_VERSION_TEXT: &str = env!("SPLITSCRIPT_VERSION_TEXT");

pub const fn compiler_identity() -> CompilerIdentity {
    CompilerIdentity {
        version: COMPILER_VERSION,
        git_revision: COMPILER_GIT_REVISION,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModuleMetadata {
    compiler: CompilerIdentity,
    target: &'static str,
    host_abi: &'static str,
}

pub(crate) fn module_metadata() -> Vec<u8> {
    serde_json::to_vec(&ModuleMetadata {
        compiler: compiler_identity(),
        target: "wasm-gc",
        host_abi: "livesplit-auto-splitting",
    })
    .expect("static compiler module metadata should serialize")
}
