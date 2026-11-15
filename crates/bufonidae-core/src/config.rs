//! User settings: how many copies of a file to keep (RAID-1 style mirroring),
//! as a default and per folder.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::get::is_under;

pub const MAX_COPIES: u8 = 8;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "invalid level {0:?}: use `single`, `mirror` (2 copies) or `mirror:N` with N from 2 to {MAX_COPIES}"
    )]
    InvalidLevel(String),
    #[error("invalid folder {0:?}: must be relative, non-empty and free of '..'")]
    InvalidFolder(String),
    #[error("no folder override for {0:?}")]
    NoOverride(String),
    #[error("could not determine the configuration directory")]
    NoConfigDir,
    #[error("config file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("config file {path} is corrupt: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("could not serialize the config: {0}")]
    Serialize(#[from] toml::ser::Error),
}

fn one() -> u8 {
    1
}

/// Ask for confirmation before storing more than this, by default (1 GB).
pub const DEFAULT_LARGE_UPLOAD_BYTES: u64 = 1_000_000_000;

fn default_large_upload() -> u64 {
    DEFAULT_LARGE_UPLOAD_BYTES
}

/// Redundancy settings. `copies` = number of copies on distinct accounts:
/// 1 is `single` (JBOD, no redundancy), N >= 2 is `mirror:N`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "one")]
    default_copies: u8,
    /// Folder (relative path) -> copies; the longest matching folder wins.
    #[serde(default)]
    folders: BTreeMap<String, u8>,
    /// The desktop app asks for confirmation before storing more than this
    /// many bytes in one go. 0 means always ask.
    #[serde(default = "default_large_upload")]
    large_upload_bytes: u64,
    #[serde(skip)]
    path: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_copies: 1,
            folders: BTreeMap::new(),
            large_upload_bytes: DEFAULT_LARGE_UPLOAD_BYTES,
            path: PathBuf::new(),
        }
    }
}

/// `single` -> 1, `mirror` -> 2, `mirror:N` -> N.
pub fn parse_level(level: &str) -> Result<u8, Error> {
    let bad = || Error::InvalidLevel(level.to_owned());
    match level {
        "single" => Ok(1),
        "mirror" => Ok(2),
        _ => {
            let n: u8 = level
                .strip_prefix("mirror:")
                .and_then(|n| n.parse().ok())
                .ok_or_else(bad)?;
            if (2..=MAX_COPIES).contains(&n) {
                Ok(n)
            } else {
                Err(bad())
            }
        }
    }
}

/// Inverse of [`parse_level`], for display.
#[must_use]
pub fn describe(copies: u8) -> String {
    if copies <= 1 {
        "single".to_owned()
    } else {
        format!("mirror:{copies}")
    }
}

fn normalize_folder(folder: &str) -> Result<String, Error> {
    let trimmed = folder.trim_matches('/');
    let ok = !trimmed.is_empty()
        && !folder.starts_with('/')
        && Path::new(trimmed)
            .components()
            .all(|c| matches!(c, Component::Normal(_)));
    if ok {
        Ok(trimmed.to_owned())
    } else {
        Err(Error::InvalidFolder(folder.to_owned()))
    }
}

impl Config {
    /// `$XDG_CONFIG_HOME/bufonidae/config.toml`.
    pub fn default_path() -> Result<PathBuf, Error> {
        let dirs = directories::ProjectDirs::from("", "", "bufonidae").ok_or(Error::NoConfigDir)?;
        Ok(dirs.config_dir().join("config.toml"))
    }

    /// Loads the config, or the defaults when the file does not exist.
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, Error> {
        let path = path.into();
        let mut config = match fs::read_to_string(&path) {
            Ok(text) => toml::from_str::<Self>(&text).map_err(|source| Error::Parse {
                path: path.clone(),
                source,
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(source) => return Err(Error::Io { path, source }),
        };
        // A hand-edited file must not smuggle in an impossible level.
        let valid = |n: u8| (1..=MAX_COPIES).contains(&n);
        if !valid(config.default_copies) || !config.folders.values().all(|&n| valid(n)) {
            return Err(Error::InvalidLevel(format!("in {}", path.display())));
        }
        config.path = path;
        Ok(config)
    }

    #[must_use]
    pub fn default_copies(&self) -> u8 {
        self.default_copies
    }

    #[must_use]
    pub fn large_upload_bytes(&self) -> u64 {
        self.large_upload_bytes
    }

    /// Sets the confirmation threshold and saves.
    pub fn set_large_upload_bytes(&mut self, bytes: u64) -> Result<(), Error> {
        self.large_upload_bytes = bytes;
        self.save()
    }

    #[must_use]
    pub fn folders(&self) -> &BTreeMap<String, u8> {
        &self.folders
    }

    /// Copies wanted for the file at `path`: the longest matching folder
    /// override, else the default.
    #[must_use]
    pub fn copies_for(&self, path: &str) -> u8 {
        self.folders
            .iter()
            .filter(|(folder, _)| is_under(path, folder))
            .max_by_key(|(folder, _)| folder.len())
            .map_or(self.default_copies, |(_, &n)| n)
    }

    /// Sets the default (`folder` is `None`) or a folder override, and saves.
    pub fn set(&mut self, folder: Option<&str>, copies: u8) -> Result<(), Error> {
        match folder {
            None => self.default_copies = copies,
            Some(f) => {
                self.folders.insert(normalize_folder(f)?, copies);
            }
        }
        self.save()
    }

    /// Removes a folder override and saves.
    pub fn unset(&mut self, folder: &str) -> Result<(), Error> {
        let key = normalize_folder(folder)?;
        self.folders.remove(&key).ok_or(Error::NoOverride(key))?;
        self.save()
    }

    fn save(&self) -> Result<(), Error> {
        let text = toml::to_string_pretty(self)?;
        crate::fsutil::write_private(&self.path, &text).map_err(|source| Error::Io {
            path: self.path.clone(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_levels() {
        assert_eq!(parse_level("single").unwrap(), 1);
        assert_eq!(parse_level("mirror").unwrap(), 2);
        assert_eq!(parse_level("mirror:3").unwrap(), 3);
        for bad in ["", "mirror:1", "mirror:9", "mirror:x", "raid5", "Single"] {
            assert!(parse_level(bad).is_err(), "{bad}");
        }
        assert_eq!(describe(1), "single");
        assert_eq!(describe(3), "mirror:3");
    }

    #[test]
    fn longest_folder_wins_and_respects_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Config::load(dir.path().join("c/config.toml")).unwrap();
        assert_eq!(c.copies_for("anything/x"), 1);
        c.set(None, 2).unwrap();
        c.set(Some("Docs"), 3).unwrap();
        c.set(Some("Docs/Scratch/"), 1).unwrap();
        assert_eq!(c.copies_for("Pics/a"), 2, "default");
        assert_eq!(c.copies_for("Docs/a"), 3);
        assert_eq!(c.copies_for("Docs/Scratch/a"), 1, "longest wins");
        assert_eq!(c.copies_for("Docs2/a"), 2, "not under Docs");
    }

    #[test]
    fn persists_privately_and_unset_works() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut c = Config::load(&path).unwrap();
        c.set(Some("Docs"), 2).unwrap();
        let again = Config::load(&path).unwrap();
        assert_eq!(again.copies_for("Docs/x"), 2);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let mut again = again;
        again.unset("Docs").unwrap();
        assert_eq!(Config::load(&path).unwrap().copies_for("Docs/x"), 1);
        assert!(matches!(again.unset("Docs"), Err(Error::NoOverride(_))));
    }

    #[test]
    fn rejects_bad_folders_and_hand_edited_levels() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut c = Config::load(&path).unwrap();
        for bad in ["", "/", "/abs", "a/../b", ".."] {
            assert!(
                matches!(c.set(Some(bad), 2), Err(Error::InvalidFolder(_))),
                "{bad}"
            );
        }
        fs::write(&path, "default_copies = 0\n").unwrap();
        assert!(Config::load(&path).is_err());
        fs::write(&path, "default_copies = 200\n").unwrap();
        assert!(Config::load(&path).is_err());
    }

    #[test]
    fn large_upload_threshold_defaults_to_one_gb_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut c = Config::load(&path).unwrap();
        assert_eq!(c.large_upload_bytes(), 1_000_000_000);

        c.set_large_upload_bytes(5_000_000_000).unwrap();
        assert_eq!(
            Config::load(&path).unwrap().large_upload_bytes(),
            5_000_000_000
        );

        // An older file without the key still loads, with the default.
        fs::write(&path, "default_copies = 2\n").unwrap();
        let old = Config::load(&path).unwrap();
        assert_eq!(old.large_upload_bytes(), 1_000_000_000);
        assert_eq!(old.default_copies(), 2);
    }
}
