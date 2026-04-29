use std::path::PathBuf;

/// Represents the homegate data directory (default: `~/.homegate/`).
///
/// Contains `config.toml` and `pepper.txt`.
#[derive(Debug, Clone)]
pub struct DataDir(PathBuf);

impl DataDir {
    pub fn new(path: PathBuf) -> anyhow::Result<Self> {
        anyhow::ensure!(
            path.exists(),
            "Data directory '{}' does not exist",
            path.display()
        );
        anyhow::ensure!(path.is_dir(), "'{}' is not a directory", path.display());
        Ok(Self(path))
    }

    pub fn default_path() -> PathBuf {
        let home = std::env::var("HOME")
            .expect("Should be able to determine home directory - $HOME not set");
        PathBuf::from(home).join(".homegate")
    }

    pub fn config_file_path(&self) -> PathBuf {
        self.0.join("config.toml")
    }

    pub fn pepper_file_path(&self) -> PathBuf {
        self.0.join("pepper.txt")
    }
}
