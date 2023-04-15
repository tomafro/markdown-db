use chrono::{DateTime, Utc};
use comrak::{nodes::Ast, Arena, ComrakOptions};
use directories::ProjectDirs;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::OsStr,
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

use crate::markdown::{self, Collection, Dialect, Document};

#[derive(Debug, Serialize, Deserialize)]
pub struct Vault {
    pub path: String,
}

pub struct Source {
    pub path: PathBuf,
}

impl markdown::Source for Source {
    fn read(&self) -> String {
        self.path.read()
    }

    fn url(&self) -> url::Url {
        url::Url::parse(
            format!("obsidian://open?path={}", urlencoding::encode(&self.path.to_string_lossy()))
                .as_str(),
        )
        .unwrap()
    }

    fn created(&self) -> Option<DateTime<Utc>> {
        self.path.created()
    }

    fn modified(&self) -> Option<DateTime<Utc>> {
        self.path.modified()
    }

    fn title(&self) -> Option<&str> {
        self.path.title()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub vaults: HashMap<String, Vault>,
}

impl Config {
    pub fn read() -> Result<Self, Box<dyn std::error::Error>> {
        let config_path =
            ProjectDirs::from("", "", "obsidian").unwrap().config_dir().join("obsidian.json");
        let config = std::fs::read_to_string(config_path)?;
        let config: Config = serde_json::from_str(&config)?;
        Ok(config)
    }
}

impl Collection for Vault {
    fn documents(&self) -> Vec<Document> {
        let path = Path::new(&self.path);
        WalkDir::new(path.canonicalize().unwrap())
            .into_iter()
            .filter(|entry| {
                entry
                    .as_ref()
                    .map(|entry| entry.path().extension() == Some(OsStr::new("md")))
                    .unwrap_or(false)
            })
            .filter_map(|entry| entry.ok())
            .map(|entry| Document {
                source: Box::new(Source { path: entry.path().to_path_buf() }),
                dialect: Box::new(Obsidian),
                ..Default::default()
            })
            .collect()
    }
}

pub fn vaults() -> Result<Vec<Box<dyn Collection>>, Box<dyn std::error::Error>> {
    let obsidian = Config::read()?;
    let collections = obsidian
        .vaults
        .values()
        .map(|v| Box::new(Vault { path: v.path.clone() }) as Box<dyn Collection>)
        .collect();
    Ok(collections)
}

#[derive(Default, Debug)]
pub struct Obsidian;

fn wiki_to_markdown_links(input: &str) -> String {
    let link_pattern = Regex::new(r"\[\[(?P<url>[^\]|]+)(\|(?P<alias>[^\]]+))?\]\]").unwrap();
    link_pattern
        .replace_all(input, |caps: &regex::Captures| {
            let url = &caps["url"];
            let alias = &caps.name("alias");
            if alias.is_none() {
                format!("[{}](obsidian://open?path={})", url, urlencoding::encode(url))
            } else {
                format!(
                    "[{}](obsidian://open?path={})",
                    alias.unwrap().as_str(),
                    urlencoding::encode(url)
                )
            }
        })
        .to_string()
}

impl Dialect for Obsidian {
    fn parse<'a>(
        &self, arena: &'a Arena<comrak::arena_tree::Node<'a, RefCell<Ast>>>, source: &str,
    ) -> &'a comrak::arena_tree::Node<'a, RefCell<Ast>> {
        let source = wiki_to_markdown_links(source);
        let options: ComrakOptions = ComrakOptions {
            extension: comrak::ComrakExtensionOptions {
                front_matter_delimiter: Some("---".to_owned()),
                autolink: true,
                ..Default::default()
            },
            ..Default::default()
        };
        comrak::parse_document(arena, &source, &options)
    }
}
