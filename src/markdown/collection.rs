use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use super::{DialectDocument, Document, Obsidian};
use walkdir::WalkDir;

pub trait Collection {
    fn documents(&self) -> Vec<Document>;
}

fn documents<'a>(path: PathBuf) -> Vec<Document<'a>> {
    WalkDir::new(path)
        .into_iter()
        .filter(|entry| {
            entry
                .as_ref()
                .map(|entry| entry.path().extension() == Some(OsStr::new("md")))
                .unwrap_or(false)
        })
        .filter_map(|entry| entry.ok())
        .map(|entry| Obsidian::document(entry.path().to_path_buf()))
        .collect()
}

impl Collection for Path {
    fn documents(&self) -> Vec<Document> {
        documents(self.canonicalize().unwrap())
    }
}

impl Collection for PathBuf {
    fn documents(&self) -> Vec<Document> {
        documents(self.canonicalize().unwrap())
    }
}
