//! Canonical generated documentation views over compiler-owned catalogs.

mod bundled;
mod code;
mod entry;
mod intra_doc;
mod reference;
mod validation;

pub use entry::{DocumentedParameter, StandardLibraryDocumentation};
pub(crate) use intra_doc::strip_links as strip_intra_doc_links;
pub(crate) use reference::migration_topic_uri;
pub use reference::{DocumentationIndexEntry, DocumentationPage, DocumentationReference};
pub(crate) use reference::{language_item_uri, symbol_uri};
