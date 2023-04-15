use chrono::{DateTime, Utc};
use comrak::{
    nodes::{Ast, NodeValue},
    Arena, ComrakOptions,
};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::OsStr,
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

use crate::markdown::{self, Collection, Dialect, Document, Node};

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

const MARKER: &[u8; 6] = b"\xF0\x9F\x94\x97!!";

impl Dialect for Obsidian {
    fn parse<'a>(
        &self, arena: &'a Arena<comrak::arena_tree::Node<'a, RefCell<Ast>>>, source: &str,
    ) -> &'a comrak::arena_tree::Node<'a, RefCell<Ast>> {
        let options: ComrakOptions = ComrakOptions {
            extension: comrak::ComrakExtensionOptions {
                front_matter_delimiter: Some("---".to_owned()),
                autolink: true,
                ..Default::default()
            },
            ..Default::default()
        };

        let result = comrak::parse_document_with_broken_link_callback(
            arena,
            source,
            &options,
            Some(&mut |link_ref: &[u8]| Some((MARKER.to_vec(), link_ref.to_owned()))),
        );

        let links = result.descendants().filter(|node| {
            if let NodeValue::Link(link) = &node.data.borrow().value {
                link.url == MARKER.to_vec()
            } else {
                false
            }
        });

        for node in links {
            let previous = node.previous_sibling();
            let next = node.next_sibling();

            if let Some(previous) = previous {
                if let NodeValue::Text(ref mut previous_text) = previous.data.borrow_mut().value {
                    if let Some(91) = previous_text.last() {
                        if let Some(next) = next {
                            if let NodeValue::Text(ref mut next_text) = next.data.borrow_mut().value
                            {
                                if let Some(93) = next_text.first() {
                                    previous_text.pop();
                                    next_text.remove(0);
                                }
                            }
                        }
                    }
                }
            }

            let text = Node { node }.text();
            if let &mut NodeValue::Link(ref mut link) = &mut node.data.borrow_mut().value {
                let mut parts: Vec<&str> = text.split('|').map(|s| s.trim()).collect();
                link.title = b"".to_vec();
                link.url =
                    format!("obsidian://open?path={}", parts.remove(0).to_owned()).into_bytes();

                if parts.len() > 0 {
                    node.children().for_each(|child| {
                        if let &mut NodeValue::Text(ref mut text) =
                            &mut child.data.borrow_mut().value
                        {
                            text.clear();
                            text.extend(parts.remove(0).to_owned().into_bytes());
                        }
                    });
                }
            }
        }

        result
    }
}
