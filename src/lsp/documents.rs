//! Open-document ownership independent of JSON-RPC routing.

use std::collections::HashMap;

use crate::database::CompilerDatabase;

pub(super) struct Document {
    pub version: Option<i64>,
    pub database: CompilerDatabase,
}

#[derive(Default)]
pub(super) struct DocumentStore {
    documents: HashMap<String, Document>,
}

impl DocumentStore {
    pub fn open(&mut self, uri: String, version: Option<i64>, text: String) {
        let database = CompilerDatabase::with_source_name(uri.clone(), text);
        self.documents.insert(uri, Document { version, database });
    }

    pub fn change(&mut self, uri: &str, version: Option<i64>, text: String) -> bool {
        let Some(document) = self.documents.get_mut(uri) else {
            return false;
        };
        document.version = version;
        document.database.set_source(text);
        true
    }

    pub fn close(&mut self, uri: &str) -> bool {
        self.documents.remove(uri).is_some()
    }

    pub fn get_mut(&mut self, uri: &str) -> Option<&mut Document> {
        self.documents.get_mut(uri)
    }
}
