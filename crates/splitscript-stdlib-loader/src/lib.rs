//! Build-time catalog generation for SplitScript's privileged standard-library
//! source.
//!
//! Syntax and parsing live in `splitscript-syntax`; this crate only translates
//! the parsed declaration tree into Rust catalog data and stable identities.

mod generate;
mod validation;

pub use generate::{generate_catalog, generate_ids};
pub use splitscript_syntax::standard_library::*;
