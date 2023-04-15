use std::path::PathBuf;

use chrono::{DateTime, Utc};
use url::Url;

use base64::Engine;

pub trait Source {
    fn read(&self) -> String;
    fn url(&self) -> Url;

    fn title(&self) -> Option<&str> {
        None
    }

    fn path(&self) -> Option<&PathBuf> {
        None
    }

    fn created(&self) -> Option<DateTime<Utc>> {
        None
    }

    fn modified(&self) -> Option<DateTime<Utc>> {
        None
    }
}

impl Source for String {
    fn read(&self) -> String {
        self.clone()
    }

    fn url(&self) -> Url {
        Url::parse(&format!(
            "data:text/plain;base64,{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self)
        ))
        .unwrap()
    }
}

impl Source for &str {
    fn read(&self) -> String {
        self.to_string()
    }

    fn url(&self) -> Url {
        Url::parse(&format!(
            "data:text/plain;base64,{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self)
        ))
        .unwrap()
    }
}

impl Source for PathBuf {
    fn read(&self) -> String {
        std::fs::read_to_string(self).unwrap()
    }

    fn url(&self) -> Url {
        Url::from_file_path(self).unwrap()
    }

    fn title(&self) -> Option<&str> {
        self.file_stem().and_then(|s| s.to_str())
    }

    fn path(&self) -> Option<&PathBuf> {
        Some(self)
    }

    fn created(&self) -> Option<DateTime<Utc>> {
        self.metadata().ok().and_then(|m| m.created().ok()).map(|t| t.into())
    }

    fn modified(&self) -> Option<DateTime<Utc>> {
        self.metadata().ok().and_then(|m| m.modified().ok()).map(|t| t.into())
    }
}
