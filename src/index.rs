use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};

use crate::markdown::collection::Collection;

use chrono::Utc;
use indoc::indoc;
use log::info;
use rusqlite::{Connection, Transaction};
use serde::Serialize;

pub struct Index {
    pub connection: Connection,
    pub collections: Vec<Box<dyn crate::markdown::collection::Collection>>,
}

const SCHEMA_VERSION: i64 = 3;

#[allow(dead_code)]
impl Index {
    pub fn schema_version(connection: &Connection) -> i64 {
        connection.query_row("SELECT version FROM application", [], |row| row.get(0)).unwrap_or(0)
    }

    pub fn ensure_schema_version(
        connection: &Connection,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if Self::schema_version(connection) < SCHEMA_VERSION {
            Self::create_schema(connection)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn create_schema(connection: &Connection) -> Result<(), Box<dyn std::error::Error>> {
        info!("Creating database schema");
        connection.execute("DROP TABLE IF EXISTS documents", ())?;
        connection.execute(
            indoc! {"
            CREATE TABLE documents (
                id INTEGER PRIMARY KEY,
                uri TEXT NOT NULL UNIQUE,
                type TEXT,
                title TEXT NOT NULL,
                markdown TEXT NOT NULL,
                created TIMESTAMP NOT NULL,
                modified TIMESTAMP NOT NULL,
                last_seen_at TIMESTAMP NOT NULL
            )"},
            (),
        )?;

        connection.execute("DROP TABLE IF EXISTS word_index", ())?;
        connection
        .execute(
            indoc! {"
            CREATE VIRTUAL TABLE IF NOT EXISTS word_index
            USING fts5(document_id UNINDEXED, title, text, tokenize = \"porter unicode61 remove_diacritics 1 tokenchars '-#'\")
            "},
            (),
        )?;

        connection.execute("DROP TABLE IF EXISTS application", ())?;
        connection.execute(
            indoc! {"
                CREATE TABLE application (
                    id INTEGER PRIMARY KEY,
                    version INTEGER NOT NULL
                )"
            },
            (),
        )?;

        connection.execute(
            indoc! {"
                INSERT INTO application (version) VALUES (?1)"
            },
            [SCHEMA_VERSION],
        )?;
        Ok(())
    }

    pub fn open(
        collections: Vec<Box<dyn crate::markdown::collection::Collection>>, connection: Connection,
    ) -> Index {
        Self::ensure_schema_version(&connection).expect("Failed to create database schema");
        Index { connection, collections }
    }

    pub fn open_in_memory(
        collections: Vec<Box<dyn crate::markdown::collection::Collection>>,
    ) -> Index {
        let connection = Connection::open_in_memory().expect("Failed to open in-memory database");
        Self::open(collections, connection)
    }

    pub fn open_from_file(
        collections: Vec<Box<dyn crate::markdown::collection::Collection>>, database_path: &Path,
    ) -> Index {
        let connection = Connection::open(database_path).unwrap_or_else(|_| {
            panic!("Failed to open database at {:?}", database_path.to_string_lossy())
        });
        Self::open(collections, connection)
    }

    pub fn size(&self) -> i64 {
        self.connection.query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0)).unwrap()
    }

    pub fn refresh(&mut self) -> Result<(), rusqlite::Error> {
        let tx = self.connection.transaction()?;
        Self::refresh_(&tx, &self.collections)?;
        tx.commit()?;
        Ok(())
    }

    fn refresh_(
        tx: &Transaction, collections: &Vec<Box<dyn Collection>>,
    ) -> Result<(), rusqlite::Error> {
        let timestamp = Utc::now();

        let mut update_unmodified_document = tx.prepare(indoc! {"
            UPDATE documents SET last_seen_at = ?1 WHERE uri = ?2 AND modified >= ?3
        "})?;

        let mut insert_into_documents = tx.prepare(indoc! {"
            INSERT INTO documents (uri, title, type, markdown, created, modified, last_seen_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(uri)
            DO UPDATE SET uri = excluded.uri, last_seen_at = excluded.last_seen_at
        "})?;

        let mut delete_from_word_index = tx.prepare(indoc! {"
            DELETE FROM word_index WHERE document_id = ?1
        "})?;

        let mut insert_into_word_index = tx.prepare(indoc! {"
            INSERT INTO word_index (document_id, title, text) VALUES (?1, ?2, ?3)
        "})?;

        for collection in collections {
            for document in &collection.documents() {
                if document.modified().is_none()
                    || update_unmodified_document.execute((
                        &timestamp,
                        &document.uri(),
                        &document.modified(),
                    ))? != 1
                {
                    insert_into_documents.execute((
                        &document.uri(),
                        &document.title(),
                        &document.doc_type(),
                        &document.markdown(),
                        &document.created(),
                        &document.modified(),
                        &timestamp,
                    ))?;

                    let id: u64 =
                        tx.query_row("SELECT last_insert_rowid()", [], |row| row.get(0))?;

                    delete_from_word_index.execute((id,))?;

                    let tags = match document.front_matter() {
                        Some(front_matter) => front_matter
                            .tags()
                            .map(|f| {
                                f.iter()
                                    .map(|tag| format!("#{tag}"))
                                    .collect::<Vec<String>>()
                                    .join(" ")
                            })
                            .unwrap_or("".to_string()),
                        None => "".to_string(),
                    };

                    let text =
                        format!("{} {} {}", &document.title().unwrap_or(""), document.text(), tags);
                    info!("{}", text);

                    insert_into_word_index.execute((id, document.title(), text))?;
                } else {
                    //println!("Document {} is up to date", document.uri());
                }
            }
        }

        info!("Deleting documents older than {}", timestamp);

        let mut delete_from_documents = tx.prepare(indoc! {"
            DELETE FROM documents WHERE last_seen_at < ?1
        "})?;
        delete_from_documents.execute([timestamp])?;

        let mut delete_from_word_index = tx.prepare(indoc! {"
            DELETE FROM word_index WHERE NOT EXISTS (SELECT 1 FROM documents WHERE documents.id = word_index.document_id)
        "})?;
        delete_from_word_index.execute([])?;

        Ok(())
    }

    pub fn search(&self, query: &str) -> Result<SearchResults, Box<dyn std::error::Error>> {
        info!("Searching for {}", query);

        let parts: Vec<String> = query.split(' ').map(|part| format!("\"{part}\"*")).collect();

        let mut match_word_index = self.connection.prepare(indoc! {"
            SELECT uri, documents.title, markdown, type, rank FROM documents
            JOIN word_index ON word_index.document_id = documents.id
            WHERE word_index MATCH ?1
        "})?;

        fn build_entry(row: &rusqlite::Row) -> Result<Entry, rusqlite::Error> {
            Ok(Entry::new(row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        }

        let match_title = format!("{{title}} : {}", parts.join(" "));
        let match_text = format!("{{text}} : {}", parts.join(" "));

        let title_rows = match_word_index.query_map([&match_title], build_entry)?;
        let mut title_results: Vec<Entry> = title_rows.map(|row| row.unwrap()).collect();

        let text_rows = match_word_index.query_map([&match_text], build_entry)?;
        let text_results: Vec<Entry> = text_rows.map(|row| row.unwrap()).collect();

        for result in text_results.into_iter() {
            if !title_results.contains(&result) {
                title_results.push(result);
            }
        }

        Ok(SearchResults { entries: title_results })
    }
}

trait OtherToSql {
    fn to_sql(&self) -> &str;
}

impl OtherToSql for &PathBuf {
    fn to_sql(&self) -> &str {
        self.to_str().unwrap()
    }
}

impl<T: OtherToSql> OtherToSql for Option<T> {
    fn to_sql(&self) -> &str {
        match self {
            Some(value) => value.to_sql(),
            None => "",
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Entry {
    title: String,
    url: String,
    #[serde(rename = "type")]
    doc_type: Option<String>,
    markdown: String,
}

impl Entry {
    pub fn new(url: String, title: String, markdown: String, doc_type: Option<String>) -> Entry {
        Entry { title, url, doc_type, markdown }
    }

    pub fn uri(&self) -> &str {
        &self.url
    }
}

impl Display for Entry {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.uri())
    }
}

pub struct SearchResults {
    entries: Vec<Entry>,
}

impl SearchResults {
    pub fn entries(&self) -> &Vec<Entry> {
        &self.entries
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    use crate::test::TestDir;

    #[test]
    fn ensure_schema_tests() -> Result<(), Box<dyn std::error::Error>> {
        let connection = Connection::open_in_memory()?;
        assert_eq!(true, Index::ensure_schema_version(&connection).unwrap());
        assert_eq!(false, Index::ensure_schema_version(&connection).unwrap());
        Ok(())
    }

    #[test]
    fn refresh_index_tests() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new();
        let mut index = Index::open_in_memory(vec![Box::new(dir.path().to_path_buf())]);

        assert_eq!(0, index.size());

        dir.write("document.md", "Initial document")?;
        index.refresh()?;

        assert_eq!(1, index.size());
        assert_eq!(1, index.search("Initial")?.len(), "indexed document should be found");
        assert_eq!(0, index.search("Unknown")?.len(), "unknown document should not be found");

        dir.write("document.md", "Updated document")?;
        index.refresh()?;

        assert_eq!(1, index.size());
        assert_eq!(0, index.search("Initial")?.len(), "original version should not be found");
        assert_eq!(1, index.search("Updated")?.len(), "updated version should be found");

        dir.delete("document.md")?;
        index.refresh()?;

        assert_eq!(0, index.size());
        assert_eq!(0, index.search("Updated")?.len(), "updated version should no longer be found");

        dir.write("one.md", "One")?;
        dir.write("two.md", "Two")?;
        dir.write("three.md", "Three")?;
        index.refresh()?;

        assert_eq!(3, index.size());
        assert_eq!(1, index.search("One")?.len(), "all documents should be searchable");
        assert_eq!(1, index.search("Two")?.len(), "all documents should be searchable");
        assert_eq!(1, index.search("Three")?.len(), "all documents should be searchable");
        Ok(())
    }

    #[test]
    fn search_result_tests() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new();
        let mut index = Index::open_in_memory(vec![Box::new(dir.path().to_path_buf())]);

        dir.write("root.md", "Root")?;
        dir.write("folder/child.md", "Child")?;
        index.refresh()?;

        assert_eq!(
            dir.url_for("root.md"),
            Url::parse(index.search("Root")?.entries()[0].uri()).unwrap()
        );
        assert_eq!(
            dir.url_for("folder/child.md"),
            Url::parse(index.search("Child")?.entries()[0].uri()).unwrap()
        );
        Ok(())
    }

    #[test]
    fn search_document_tests() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new();
        let mut index = Index::open_in_memory(vec![Box::new(dir.path().to_path_buf())]);

        dir.write("plain.md", "Very SIMPLE document")?;
        index.refresh()?;

        for query in ["very", "VeRy", "simple", "SIMPLE", "document"] {
            let results = index.search(query);
            assert_eq!(1, results?.len(), "match word");
        }

        for query in ["simple document", "very simple", "document simple very"] {
            let results = index.search(query);
            assert_eq!(1, results?.len(), "match multiple words");
        }

        for query in ["missing", "simple missing document", "very simple missing"] {
            let results = index.search(query);
            assert_eq!(0, results?.len(), "don't match missing word");
        }

        for query in ["v", "si", "doc"] {
            let results = index.search(query);
            assert_eq!(1, results?.len(), "match word prefixes ({query})");
        }
        Ok(())
    }

    #[test]
    fn search_inline_tags_tests() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new();
        let mut index = Index::open_in_memory(vec![Box::new(dir.path().to_path_buf())]);

        dir.write("doc.md", "Document with #inline tags #after")?;
        index.refresh()?;

        for query in ["#inline", "#after"] {
            let results = index.search(query);
            assert_eq!(1, results?.len(), "match qualified tags");
        }

        for query in ["inline", "after"] {
            let results = index.search(query);
            assert_eq!(0, results?.len(), "don't match unqualified tag");
        }

        Ok(())
    }

    #[test]
    fn search_front_matter_tags_tests() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new();
        let mut index = Index::open_in_memory(vec![Box::new(dir.path().to_path_buf())]);

        let content = indoc! {"
            ---
            tags: first, second
            ---
            Document with front matter tags
        "};

        dir.write("doc.md", content)?;
        index.refresh()?;

        for query in ["#first", "#second"] {
            let results = index.search(query);
            assert_eq!(1, results?.len(), "match qualified tags");
        }

        for query in ["first", "second"] {
            let results = index.search(query);
            assert_eq!(0, results?.len(), "don't match unqualified tag");
        }

        Ok(())
    }

    #[test]
    fn search_title_from_file_tests() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new();
        let mut index = Index::open_in_memory(vec![Box::new(dir.path().to_path_buf())]);

        dir.write("first.md", "")?;
        dir.write("folder/second.md", "")?;
        dir.write("folder/third.md", "third")?;
        index.refresh()?;

        let results = index.search("first")?;
        assert_eq!(1, results.len());
        assert_eq!(
            dir.url_for("first.md"),
            Url::parse(results.entries()[0].uri()).unwrap(),
            "file name"
        );

        let results = index.search("second")?;
        assert_eq!(1, results.len());
        assert_eq!(
            dir.url_for("folder/second.md"),
            Url::parse(results.entries()[0].uri()).unwrap(),
            "nested file name"
        );

        let results = index.search("third")?;
        assert_eq!(1, results.len(), "matching both title and text should only return one result");

        Ok(())
    }
}
