use std::path::{Path, PathBuf};

use tempfile::TempDir;
use url::Url;

pub struct TestDir {
    temp_dir: TempDir,
}

impl TestDir {
    pub fn new() -> Self {
        let temp_dir = TempDir::new().unwrap();
        Self { temp_dir }
    }

    pub fn path(&self) -> &Path {
        self.temp_dir.path()
    }

    pub fn url_for(&self, name: &str) -> Url {
        Url::from_file_path(self.path().join(name).canonicalize().unwrap()).unwrap()
    }

    pub fn write(&self, name: &str, contents: &str) -> std::io::Result<PathBuf> {
        std::thread::sleep(std::time::Duration::from_millis(5));
        let path = self.temp_dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, contents).map(|_| path)
    }

    pub fn delete(&self, name: &str) -> std::io::Result<()> {
        let path = self.temp_dir.path().join(name);
        std::fs::remove_file(path)
    }
}
