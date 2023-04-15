pub mod collection;
pub mod source;

use comrak::nodes::{Ast, NodeValue};
use comrak::{format_commonmark, Arena, ComrakOptions};
use std::cell::RefCell;
use std::collections::HashMap;
use std::marker::PhantomData;
use url::Url;

use chrono::{DateTime, Utc};

pub use crate::obsidian::Obsidian;
pub use collection::Collection;
use once_cell::sync::OnceCell;
use serde::{de::Visitor, Deserialize, Deserializer, Serialize};
pub use source::Source;

pub trait Dialect {
    fn parse<'a>(
        &self, arena: &'a Arena<comrak::arena_tree::Node<'a, RefCell<Ast>>>, source: &str,
    ) -> &'a comrak::arena_tree::Node<'a, RefCell<Ast>>;
}

pub trait DialectDocument<'a, T> {
    fn document(source: T) -> Document<'a>;
}

impl<'a, T: Dialect + Default + 'static, S: Source + 'static> DialectDocument<'a, S> for T {
    fn document(source: S) -> Document<'a> {
        Document { source: Box::new(source), dialect: Box::<T>::default(), ..Default::default() }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FrontMatter {
    title: Option<String>,
    #[serde(rename = "type")]
    doc_type: Option<String>,
    #[serde(default)]
    #[serde(deserialize_with = "FrontMatter::maybe_vec_of_strings")]
    tags: Option<Vec<String>>,
}

impl FrontMatter {
    // It would be fair to say I don't completely understand how this works.
    fn maybe_vec_of_strings<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct MaybeVecOfStrings(PhantomData<Option<Vec<String>>>);

        impl<'de> Visitor<'de> for MaybeVecOfStrings {
            type Value = Option<Vec<String>>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("string or array of strings")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Option<Vec<String>>, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut vec = Vec::new();
                while let Some(value) = seq.next_element()? {
                    vec.push(value);
                }
                Ok(Some(vec))
            }

            fn visit_str<E>(self, value: &str) -> Result<Option<Vec<String>>, E>
            where
                E: serde::de::Error,
            {
                let t: Vec<String> = value
                    .replace(',', " ")
                    .split(' ')
                    .filter_map(|s| {
                        let r = s.to_string();
                        if r.is_empty() {
                            None
                        } else {
                            Some(r)
                        }
                    })
                    .collect();
                Ok(Some(t))
            }
        }

        Ok(match deserializer.deserialize_any(MaybeVecOfStrings(PhantomData)) {
            Ok(result) => result,
            _ => None,
        })
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn doc_type(&self) -> Option<&str> {
        self.doc_type.as_deref()
    }

    pub fn tags(&self) -> Option<&[String]> {
        self.tags.as_deref()
    }
}

impl From<&str> for FrontMatter {
    fn from(source: &str) -> Self {
        serde_yaml::from_str(source).expect("Failed to parse front matter")
    }
}

impl From<&[u8]> for FrontMatter {
    fn from(source: &[u8]) -> Self {
        Self::from(String::from_utf8(source.into()).expect("Failed to parse front matter").as_str())
    }
}

impl Default for Box<dyn Dialect> {
    fn default() -> Self {
        Box::new(Obsidian {})
    }
}

trait DebuggableDialect: Dialect + std::fmt::Debug {}
#[derive(Default)]
pub struct Document<'a> {
    pub arena: Arena<comrak::arena_tree::Node<'a, RefCell<Ast>>>,
    pub root: OnceCell<Node<'a>>,
    pub front_matter: OnceCell<Option<FrontMatter>>,
    pub source: Box<dyn Source>,
    pub dialect: Box<dyn Dialect>,
}

impl Default for Box<dyn Source> {
    fn default() -> Self {
        Box::new("".to_string())
    }
}

impl<'a> Document<'a> {
    pub fn uri(&self) -> Url {
        self.source.url()
    }

    pub fn modified(&'a self) -> Option<DateTime<Utc>> {
        self.source.modified()
    }

    pub fn created(&'a self) -> Option<DateTime<Utc>> {
        self.source.created()
    }

    pub fn title(&'a self) -> Option<&str> {
        self.title_from_frontmatter().or(self.title_from_source())
    }

    pub fn content(&'a self) -> String {
        self.source.read()
    }

    pub fn markdown(&'a self) -> String {
        let mut output = Vec::new();
        format_commonmark(self.root().node, &ComrakOptions::default(), &mut output).unwrap();
        String::from_utf8(output).unwrap()
    }

    fn type_from_link(&'a self) -> Option<String> {
        let links = self.links();
        let type_links = &mut links
            .iter()
            .filter(|link| link.meta().is_some() && link.meta().unwrap().0 == "type");
        if let Some(link) = type_links.next() {
            let result = link.meta().unwrap().1.to_owned();
            Some(result)
        } else {
            None
        }
    }

    pub fn doc_type(&'a self) -> Option<String> {
        if let Some(frontmatter) = self.front_matter() {
            frontmatter.doc_type().map(|s| s.to_owned())
        } else {
            self.type_from_link()
        }
    }

    pub fn root(&'a self) -> &Node {
        if self.root.get().is_none() {
            self.init();
        };
        self.root.get().unwrap()
    }

    pub fn front_matter(&'a self) -> &Option<FrontMatter> {
        if self.front_matter.get().is_none() {
            self.init();
        };
        self.front_matter.get().unwrap()
    }

    pub fn init(&'a self) {
        self.root.get_or_init(|| Node { node: self.parse() });
        self.front_matter.get_or_init(|| {
            self.root().node.children().find_map(|child| {
                if let NodeValue::FrontMatter(data) = &child.data.borrow().value {
                    child.detach();
                    Some(FrontMatter::from(&data[4..(data.len() - 4)]))
                } else {
                    None
                }
            })
        });
    }

    pub fn links(&'a self) -> Vec<Link> {
        self.root().links()
    }

    pub fn text(&'a self) -> String {
        self.root().text()
    }

    fn parse(&'a self) -> &'a comrak::arena_tree::Node<'a, RefCell<Ast>> {
        self.dialect.parse(&self.arena, &self.source.read())
    }

    fn title_from_source(&self) -> Option<&str> {
        self.source.title()
    }

    fn title_from_frontmatter(&'a self) -> Option<&str> {
        if let Some(frontmatter) = self.front_matter() {
            frontmatter.title()
        } else {
            None
        }
    }

    fn tags_from_frontmatter(&'a self) -> Option<&[String]> {
        if let Some(frontmatter) = self.front_matter() {
            frontmatter.tags()
        } else {
            None
        }
    }
}

pub struct Node<'a> {
    pub node: &'a comrak::arena_tree::Node<'a, RefCell<Ast>>,
}

impl<'a> Node<'a> {
    pub fn text(&self) -> String {
        let mut text: Vec<u8> = vec![];
        let iter = self.node.descendants();
        for node in iter {
            match &node.data.borrow().value {
                NodeValue::Text(text_node) => text.extend(text_node),
                NodeValue::Code(code) => text.extend(code.literal.clone()),
                NodeValue::CodeBlock(block) => text.extend(block.literal.clone()),
                NodeValue::HtmlInline(html) => text.extend(html),
                NodeValue::HtmlBlock(html) => text.extend(html.literal.clone()),
                _ => (),
            }
        }
        String::from_utf8(text).expect("Unable to convert text to string")
    }

    pub fn links(&self) -> Vec<Link> {
        let mut links: Vec<Link> = vec![];
        let iter = self.node.descendants();
        for node in iter {
            if let NodeValue::Link(link) = &node.data.borrow().value {
                let text = Node { node }.text();
                let title = String::from_utf8(link.title.clone())
                    .expect("Unable to convert title to string");
                let url =
                    String::from_utf8(link.url.clone()).expect("Unable to convert url to string");

                links.push(Link::from(text, url, title));
            }
        }
        links
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Link {
    text: String,
    url: String,
    title: String,
}

impl Link {
    fn from(text: String, url: String, title: String) -> Self {
        Self { text, url, title }
    }

    fn meta(&self) -> Option<(String, String)> {
        if let Ok(url) = Url::parse(&self.url) {
            let query = url.query_pairs().collect::<HashMap<_, _>>();
            if let Some(key) = query.get("path") {
                let parts: Vec<&str> = key.splitn(2, '=').collect();
                if parts.len() > 1 {
                    Some((parts[0].to_owned(), parts[1].to_owned()))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            let parts: Vec<&str> = self.url.splitn(2, '=').collect();
            if parts.len() > 1 {
                Some((parts[0].to_owned(), parts[1].to_owned()))
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;

    use crate::test::TestDir;

    #[test]
    fn title_missing() {
        let document = Obsidian::document("");
        assert_eq!(None, document.title());
    }

    #[test]
    fn title_from_path() -> Result<(), Box<dyn std::error::Error>> {
        let test_dir = TestDir::new();
        let path = test_dir.write("folder/subfolder/title-from-path.md", "")?;

        let document = Obsidian::document(path);

        assert_eq!(Some("title-from-path"), document.title());
        Ok(())
    }

    #[test]
    fn title_from_frontmatter() -> Result<(), Box<dyn std::error::Error>> {
        let test_dir = TestDir::new();
        let source = indoc! {"
            ---
            title: Title from frontmatter
            ---
        "};

        let document = Obsidian::document(source);
        assert_eq!(
            Some("Title from frontmatter"),
            document.title(),
            "takes title from frontmatter when present"
        );

        let path = test_dir.write("folder/subfolder/title-from-path.md", source)?;

        let document = Obsidian::document(path);
        assert_eq!(
            Some("Title from frontmatter"),
            document.title(),
            "prefers title from frontmatter to path"
        );

        Ok(())
    }

    #[test]
    fn type_missing() {
        let document = Obsidian::document("");
        assert_eq!(None, document.doc_type());
    }

    #[test]
    fn type_from_frontmatter() -> Result<(), Box<dyn std::error::Error>> {
        let source = indoc! {"
            ---
            type: Person
            ---
        "};

        let document = Obsidian::document(source);
        assert_eq!(Some("Person".to_string()), document.doc_type());

        Ok(())
    }

    #[test]
    fn type_from_link() -> Result<(), Box<dyn std::error::Error>> {
        let source = indoc! {"
            [[type=person]]
        "};

        let document = Obsidian::document(source);
        assert_eq!(Some("person".to_string()), document.doc_type());

        let source = indoc! {"
            [[type=person|alias]]
        "};

        let document = Obsidian::document(source);
        assert_eq!(Some("person".to_string()), document.doc_type());

        Ok(())
    }

    #[test]
    fn text_from_standard_content() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            "Content",
            Obsidian::document(indoc! {"
                Content
            "})
            .text()
        );
        Ok(())
    }

    #[test]
    fn text_from_inline_code() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            "(defn hello)",
            Obsidian::document(indoc! {"
                `(defn hello)`
            "})
            .text()
        );
        Ok(())
    }

    #[test]
    fn text_from_code_block() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            "(defn hello)\n",
            Obsidian::document(indoc! {"
                ```clojure
                (defn hello)
                ```
            "})
            .text()
        );
        Ok(())
    }

    mod links {
        use super::*;
        use similar_asserts::assert_eq;

        #[test]
        fn markdown_link() {
            let document = Obsidian::document(indoc! {"
                [first](https://example.com/first)
            "});

            assert_eq!(1, document.links().len());
            assert_eq!(
                Link {
                    text: "first".to_string(),
                    url: "https://example.com/first".to_string(),
                    title: "".to_string()
                },
                document.links()[0]
            );
        }

        #[test]
        fn auto_link() {
            let document = Obsidian::document(indoc! {"
                https://example.com/second
            "});

            assert_eq!(1, document.links().len());
            assert_eq!(
                Link {
                    text: "https://example.com/second".to_string(),
                    url: "https://example.com/second".to_string(),
                    title: "".to_string()
                },
                document.links()[0]
            );
        }

        #[test]
        fn wiki_link() {
            let document = Obsidian::document(indoc! {"
                [[WikiLink]]
            "});

            assert_eq!(1, document.links().len());
            assert_eq!(
                Link {
                    text: "WikiLink".to_string(),
                    url: "obsidian://open?path=WikiLink".to_string(),
                    title: "".to_string()
                },
                document.links()[0]
            );
        }

        #[test]
        fn wiki_link_with_alias() {
            let document = Obsidian::document(indoc! {"
                Before [[WikiLink|Alias]] After
            "});

            assert_eq!(1, document.links().len());
            assert_eq!(
                Link {
                    text: "Alias".to_string(),
                    url: "obsidian://open?path=WikiLink".to_string(),
                    title: "".to_string()
                },
                document.links()[0]
            );
        }
    }

    #[test]
    fn links() {
        let document = Obsidian::document(indoc! {"
            [first](https://example.com/first)
            https://example.com/second
            [[WikiLink]]
            [[WikiLinkWithAlias|Alias]]
        "});

        similar_asserts::assert_eq!(4, document.links().len());
        similar_asserts::assert_eq!(
            Link {
                text: "first".to_string(),
                url: "https://example.com/first".to_string(),
                title: "".to_string()
            },
            document.links()[0]
        );
        assert_eq!(
            Link {
                text: "https://example.com/second".to_string(),
                url: "https://example.com/second".to_string(),
                title: "".to_string()
            },
            document.links()[1]
        );

        assert_eq!(
            Link {
                text: "WikiLink".to_string(),
                url: "obsidian://open?path=WikiLink".to_string(),
                title: "".to_string()
            },
            document.links()[2]
        );

        assert_eq!(
            Link {
                text: "Alias".to_string(),
                url: "obsidian://open?path=WikiLinkWithAlias".to_string(),
                title: "".to_string()
            },
            document.links()[3]
        );
    }

    mod front_matter {
        use super::*;

        #[test]
        fn tags_inline_array() {
            let front_matter = FrontMatter::from(indoc! {"
                tags: [tag1, tag2]
                anything: else
            "});

            assert!(front_matter.tags().is_some());
            assert_eq!(["tag1", "tag2"], front_matter.tags().unwrap()[..]);
        }

        #[test]
        fn tags_indented_array() {
            let front_matter = FrontMatter::from(indoc! {"
                tags:
                    - tag1
                    - tag2
                anything: else
            "});

            assert!(front_matter.tags().is_some());
            assert_eq!(["tag1", "tag2"], front_matter.tags().unwrap()[..]);
        }

        #[test]
        fn tags_string() {
            let front_matter = FrontMatter::from(indoc! {"
                tags: tag1, tag2
                anything: else
            "});

            assert!(front_matter.tags().is_some());
            assert_eq!(["tag1", "tag2"], front_matter.tags().unwrap()[..]);
        }

        #[test]
        fn empty_tags() {
            let front_matter = FrontMatter::from(indoc! {"
                tags:
                anything: else
            "});

            assert!(front_matter.tags().is_none());
        }

        #[test]
        fn no_tags() {
            let front_matter = FrontMatter::from(indoc! {"
                anything: else
            "});

            assert!(front_matter.tags().is_none());
        }
    }

    mod content {
        use super::*;

        #[test]
        fn content() {
            let document = Obsidian::document(indoc! {"
                # Title

                Content
            "});

            assert_eq!(
                indoc! {"
                    # Title

                    Content
                "},
                document.content()
            );
        }
    }

    mod markdown {
        use super::*;

        #[test]
        fn markdown() {
            let document = Obsidian::document(indoc! {"
                # Title

                Content
            "});

            assert_eq!(
                indoc! {"
                    # Title

                    Content
                "},
                document.markdown()
            );
        }

        #[test]
        fn removes_front_matter() {
            let document = Obsidian::document(indoc! {"
                ---
                some: front_matter
                more:
                    - front
                    - matter
                ---
                # Title

                Content
            "});

            assert_eq!(
                indoc! {"
                    # Title

                    Content
                "},
                document.markdown()
            );
        }

        #[test]
        fn normalizes_wiki_links() {
            let document = Obsidian::document(indoc! {"
                # Title

                [[Other Page]]
                [[Other Page|Alias]]
            "});

            assert_eq!(
                indoc! {"
                    # Title

                    [Other Page](obsidian://open?path=Other%20Page)
                    [Alias](obsidian://open?path=Other%20Page)
                "},
                document.markdown()
            );
        }
    }
}
