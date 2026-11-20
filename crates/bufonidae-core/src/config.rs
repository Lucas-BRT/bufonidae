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
        "desktop settings: text size must be 12..20 px and notification duration 1..30 seconds"
    )]
    InvalidDesktop,
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
    #[error(
        "config file {0} was changed by another Bufonidae process while this one was working; \
         run the command again"
    )]
    Changed(PathBuf),
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileSort {
    #[default]
    Name,
    NameDescending,
    Size,
    SizeDescending,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileFilter {
    #[default]
    All,
    Files,
    Folders,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutMode {
    #[default]
    Full,
    Navigation,
    Off,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundMode {
    #[default]
    Off,
    Errors,
    All,
}

/// Device-local desktop preferences; exported with the rest of the settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopOptions {
    pub compact: bool,
    pub text_size: u8,
    pub sort: FileSort,
    pub filter: FileFilter,
    pub case_sensitive: bool,
    pub folders_first: bool,
    pub notify_success: bool,
    pub notify_failure: bool,
    pub notify_cancel: bool,
    pub notification_seconds: u8,
    pub notification_names: bool,
    pub sound: SoundMode,
    pub shortcuts: ShortcutMode,
    pub context_menus: bool,
    pub inline_help: bool,
    pub shortcut_hints: bool,
    pub first_run_help: bool,
    pub tray_enabled: bool,
    pub tray_when_idle: bool,
    pub tray_badge: bool,
    pub tray_open_transfers: bool,
}
impl Default for DesktopOptions {
    fn default() -> Self {
        Self {
            compact: false,
            text_size: 14,
            sort: FileSort::Name,
            filter: FileFilter::All,
            case_sensitive: false,
            folders_first: true,
            notify_success: true,
            notify_failure: true,
            notify_cancel: false,
            notification_seconds: 5,
            notification_names: false,
            sound: SoundMode::Off,
            shortcuts: ShortcutMode::Full,
            context_menus: true,
            inline_help: true,
            shortcut_hints: true,
            first_run_help: true,
            tray_enabled: false,
            tray_when_idle: true,
            tray_badge: true,
            tray_open_transfers: true,
        }
    }
}
impl DesktopOptions {
    pub fn validate(&self) -> Result<(), Error> {
        if !(12..=20).contains(&self.text_size) || !(1..=30).contains(&self.notification_seconds) {
            return Err(Error::InvalidDesktop);
        }
        Ok(())
    }
}

/// Redundancy settings. `copies` = number of copies on distinct accounts:
/// 1 is `single` (JBOD, no redundancy), N >= 2 is `mirror:N`.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Disable application motion; never disable textual state feedback.
    #[serde(default)]
    reduced_motion: bool,
    #[serde(default)]
    desktop: DesktopOptions,
    #[serde(skip)]
    path: PathBuf,
    /// The file's text when it was last read or written (`None`: it did not
    /// exist). A save refuses to go on if the file is no longer this.
    #[serde(skip)]
    baseline: Option<String>,
}

/// Two configs are equal when their settings are, wherever each was loaded
/// from.
impl PartialEq for Config {
    fn eq(&self, other: &Self) -> bool {
        self.default_copies == other.default_copies
            && self.folders == other.folders
            && self.large_upload_bytes == other.large_upload_bytes
            && self.reduced_motion == other.reduced_motion
            && self.desktop == other.desktop
    }
}

impl Eq for Config {}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_copies: 1,
            folders: BTreeMap::new(),
            large_upload_bytes: DEFAULT_LARGE_UPLOAD_BYTES,
            reduced_motion: false,
            desktop: DesktopOptions::default(),
            path: PathBuf::new(),
            baseline: None,
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
        let (mut config, baseline) = match fs::read_to_string(&path) {
            Ok(text) => {
                let config = toml::from_str::<Self>(&text).map_err(|source| Error::Parse {
                    path: path.clone(),
                    source,
                })?;
                (config, Some(text))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Self::default(), None),
            Err(source) => return Err(Error::Io { path, source }),
        };
        // A hand-edited file must not smuggle in an impossible level.
        let valid = |n: u8| (1..=MAX_COPIES).contains(&n);
        if !valid(config.default_copies) || !config.folders.values().all(|&n| valid(n)) {
            return Err(Error::InvalidLevel(format!("in {}", path.display())));
        }
        config.desktop.validate()?;
        config.path = path;
        config.baseline = baseline;
        Ok(config)
    }

    #[must_use]
    pub fn default_copies(&self) -> u8 {
        self.default_copies
    }

    /// Replaces every setting with those of `other` (an exported copy) and
    /// saves. `other` is validated like a hand-edited file would be.
    pub fn restore_from(&mut self, other: &Self) -> Result<(), Error> {
        let valid = |n: u8| (1..=MAX_COPIES).contains(&n);
        if !valid(other.default_copies) || !other.folders.values().all(|&n| valid(n)) {
            return Err(Error::InvalidLevel("in the imported settings".to_owned()));
        }
        other.desktop.validate()?;
        let mut folders = BTreeMap::new();
        for (folder, &copies) in &other.folders {
            folders.insert(normalize_folder(folder)?, copies);
        }
        self.changing(|c| {
            c.default_copies = other.default_copies;
            c.folders = folders;
            c.large_upload_bytes = other.large_upload_bytes;
            c.reduced_motion = other.reduced_motion;
            c.desktop = other.desktop.clone();
        })
    }

    #[must_use]
    pub fn large_upload_bytes(&self) -> u64 {
        self.large_upload_bytes
    }

    #[must_use]
    pub fn reduced_motion(&self) -> bool {
        self.reduced_motion
    }

    pub fn set_reduced_motion(&mut self, reduced: bool) -> Result<(), Error> {
        self.changing(|config| config.reduced_motion = reduced)
    }

    pub fn desktop(&self) -> &DesktopOptions {
        &self.desktop
    }

    pub fn set_desktop(&mut self, options: DesktopOptions) -> Result<(), Error> {
        options.validate()?;
        self.changing(|config| config.desktop = options)
    }

    /// Sets the confirmation threshold and saves.
    pub fn set_large_upload_bytes(&mut self, bytes: u64) -> Result<(), Error> {
        self.changing(|c| c.large_upload_bytes = bytes)
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
        let folder = folder.map(normalize_folder).transpose()?;
        self.changing(|c| match folder {
            None => c.default_copies = copies,
            Some(f) => {
                c.folders.insert(f, copies);
            }
        })
    }

    /// Removes a folder override and saves.
    pub fn unset(&mut self, folder: &str) -> Result<(), Error> {
        let key = normalize_folder(folder)?;
        if !self.folders.contains_key(&key) {
            return Err(Error::NoOverride(key));
        }
        self.changing(|c| {
            c.folders.remove(&key);
        })
    }

    /// Applies `change`, saves, and puts the old values back if saving fails,
    /// so memory never differs from what is on disk.
    fn changing(&mut self, change: impl FnOnce(&mut Self)) -> Result<(), Error> {
        let before = (
            self.default_copies,
            self.folders.clone(),
            self.large_upload_bytes,
            self.reduced_motion,
            self.desktop.clone(),
        );
        change(self);
        let result = self.save();
        if result.is_err() {
            (
                self.default_copies,
                self.folders,
                self.large_upload_bytes,
                self.reduced_motion,
                self.desktop,
            ) = before;
        }
        result
    }

    fn save(&mut self) -> Result<(), Error> {
        let text = toml::to_string_pretty(&*self)?;
        crate::fsutil::write_private_checked(&self.path, self.baseline.as_deref(), &text).map_err(
            |e| match e {
                crate::fsutil::WriteError::Changed => Error::Changed(self.path.clone()),
                crate::fsutil::WriteError::Io(source) => Error::Io {
                    path: self.path.clone(),
                    source,
                },
            },
        )?;
        self.baseline = Some(text);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_preferences_round_trip_and_reject_invalid_values_without_saving() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::load(&path).unwrap();
        let options = DesktopOptions {
            compact: true,
            text_size: 18,
            sort: FileSort::SizeDescending,
            filter: FileFilter::Files,
            notify_cancel: true,
            sound: SoundMode::Errors,
            shortcuts: ShortcutMode::Navigation,
            tray_enabled: true,
            tray_when_idle: false,
            tray_badge: false,
            tray_open_transfers: false,
            ..DesktopOptions::default()
        };
        config.set_desktop(options.clone()).unwrap();
        assert_eq!(Config::load(&path).unwrap().desktop(), &options);
        let before = std::fs::read_to_string(&path).unwrap();
        let invalid = DesktopOptions {
            text_size: 0,
            ..options.clone()
        };
        assert!(config.set_desktop(invalid).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        assert_eq!(config.desktop(), &options);
        let mut restored = Config::load(dir.path().join("restored.toml")).unwrap();
        restored.restore_from(&config).unwrap();
        assert_eq!(restored.desktop(), &options);
        std::fs::write(&path, "default_copies=1\n").unwrap();
        assert!(config.set_desktop(DesktopOptions::default()).is_err());
        assert_eq!(config.desktop(), &options);
    }

    #[test]
    fn legacy_settings_gain_motion_preference_without_changing_redundancy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "default_copies = 2\nlarge_upload_bytes = 42\n").unwrap();
        let mut config = Config::load(&path).unwrap();
        assert!(!config.reduced_motion());
        config.set_reduced_motion(true).unwrap();
        let saved = Config::load(&path).unwrap();
        assert!(saved.reduced_motion());
        assert_eq!(saved.default_copies(), 2);
        assert_eq!(saved.large_upload_bytes(), 42);
        // An external edit must not produce a false in-memory success.
        std::fs::write(&path, "default_copies = 3\n").unwrap();
        assert!(config.set_reduced_motion(false).is_err());
        assert!(config.reduced_motion());
        let mut restored = Config::load(dir.path().join("restored.toml")).unwrap();
        restored.restore_from(&saved).unwrap();
        assert!(restored.reduced_motion());
        assert_eq!(restored.default_copies(), 2);
    }

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

    #[test]
    fn two_writers_cannot_silently_overwrite_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut first = Config::load(&path).unwrap();
        let mut second = Config::load(&path).unwrap();

        first.set(None, 2).unwrap();
        // `second` still believes the file does not exist.
        assert!(matches!(second.set(None, 3), Err(Error::Changed(_))));
        assert_eq!(second.default_copies(), 1, "memory was put back");
        assert_eq!(
            Config::load(&path).unwrap().default_copies(),
            2,
            "no lost update"
        );

        // After re-reading, it can go ahead.
        let mut second = Config::load(&path).unwrap();
        second.set(None, 3).unwrap();
        assert_eq!(Config::load(&path).unwrap().default_copies(), 3);
    }

    #[test]
    fn a_failed_save_leaves_memory_matching_the_disk() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("cfg");
        let path = sub.join("config.toml");
        let mut c = Config::load(&path).unwrap();
        c.set(Some("Docs"), 2).unwrap();

        // Make the folder read-only so the next save fails.
        fs::set_permissions(&sub, fs::Permissions::from_mode(0o500)).unwrap();
        let result = c.set(Some("Pics"), 3);
        let unset = c.unset("Docs");
        fs::set_permissions(&sub, fs::Permissions::from_mode(0o700)).unwrap();

        assert!(result.is_err() && unset.is_err());
        assert_eq!(c.copies_for("Pics/x"), 1, "the failed set was undone");
        assert_eq!(c.copies_for("Docs/x"), 2, "the failed unset was undone");
        assert_eq!(Config::load(&path).unwrap().copies_for("Docs/x"), 2);
    }
}
