use clap::{Parser, Subcommand};
use index::Index;
use log::{Level, Metadata, Record};
use rusqlite::Result;

use directories::*;

mod index;
mod markdown;
mod obsidian;

#[cfg(test)]
mod test;

#[derive(Parser, Debug)]
#[command(author, about, long_about = None)]

/// markdown-db helps with searching and navigating vaults of markdown documents
struct Cli {
    #[command(subcommand)]
    command: Commands,
    /// Use an in-memory database
    #[arg(long, global = true, env = "MARKDOWN_DB_IN_MEMORY", help_heading = "Database")]
    in_memory: bool,
    #[arg(short, long, global = true)]
    /// Use verbose output
    verbose: bool,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Search for documents matching a query
    Search(SearchArgs),
}

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
struct SearchArgs {
    /// Search query
    #[arg()]
    query: Option<String>,
}

struct SimpleLogger;

impl log::Log for SimpleLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Info
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            println!("{} - {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match &cli.command {
        Commands::Search(args) => search(&cli, args),
    }
}

fn index(cli: &Cli) -> Result<index::Index, Box<dyn std::error::Error>> {
    let collections = obsidian::vaults().unwrap();

    let mut index = if cli.in_memory {
        Index::open_in_memory(collections)
    } else {
        let database_path =
            ProjectDirs::from("net", "warmdot", "odb").unwrap().cache_dir().join("index.sqlite");
        std::fs::create_dir_all(database_path.parent().unwrap())?;
        Index::open_from_file(collections, database_path.as_path())
    };

    index.refresh()?;

    Ok(index)
}

fn search(cli: &Cli, args: &SearchArgs) -> Result<(), Box<dyn std::error::Error>> {
    let index = index(cli)?;

    if let Some(query) = &args.query {
        let results = index.search(query)?;
        println!(
            "{}",
            serde_json::to_string_pretty(results.entries())
                .expect("Failed to serialize results to JSON")
        );
        return Ok(());
    } else {
        println!("Index contains {} documents", index.size());
    }

    Ok(())
}
