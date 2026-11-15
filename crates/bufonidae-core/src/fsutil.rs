//! Small filesystem helpers shared by the config-like files.
//!
//! These files reveal the user's accounts and layout, so they are written
//! privately (mode 0600 inside a 0700 folder), atomically, and durably.

use std::fs;
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug)]
pub(crate) enum WriteError {
    Io(io::Error),
    /// The file is not what the caller last read: someone else wrote it.
    Changed,
}

impl From<io::Error> for WriteError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Creates `dir` and any missing parents with mode 0700.
fn private_dir(dir: &Path) -> io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

/// A temp name next to `path` that nobody can predict, so a planted file or
/// link at a guessed name cannot redirect the write.
fn unique_tmp(path: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(format!(
        ".{}.{nanos:08x}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    path.with_file_name(name)
}

/// Writes `text` to `path`: private, atomic (temp file + rename) and durable
/// (the file and then its folder are synced).
///
/// Refuses to replace a symbolic link: following one could write somewhere the
/// user never meant.
pub(crate) fn write_private(path: &Path, text: &str) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    private_dir(dir)?;
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{} is a symbolic link; refusing to replace it",
                path.display()
            ),
        ));
    }
    let tmp = unique_tmp(path);
    let result = (|| {
        // `create_new` fails if anything, even a link, is already there.
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        out.write_all(text.as_bytes())?;
        out.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
        return result;
    }
    // Without this, a crash right after the rename can still lose it.
    fs::File::open(dir)?.sync_all()
}

/// [`write_private`], but only if the file still holds `baseline` (what the
/// caller read; `None` means it did not exist). Two processes that each read,
/// change and save would otherwise silently overwrite one another.
///
/// The check and the write happen under an exclusive lock on a sibling
/// `.lock` file, so they cannot interleave with another writer.
pub(crate) fn write_private_checked(
    path: &Path,
    baseline: Option<&str>,
    text: &str,
) -> Result<(), WriteError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    private_dir(dir)?;
    let mut lock_name = path.file_name().unwrap_or_default().to_owned();
    lock_name.push(".lock");
    let lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path.with_file_name(lock_name))?;
    lock.lock()?;
    let current = match fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    if current.as_deref() != baseline {
        return Err(WriteError::Changed);
    }
    write_private(path, text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn writes_privately_into_a_private_folder_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b/file.toml");
        write_private(&path, "one").unwrap();
        write_private(&path, "two").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers, ["file.toml"], "no temp file is left behind");
    }

    #[test]
    fn a_planted_file_or_link_at_the_old_predictable_name_is_harmless() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.toml");
        let victim = dir.path().join("victim");
        fs::write(&victim, "precious").unwrap();
        std::os::unix::fs::symlink(&victim, dir.path().join("file.toml.tmp")).unwrap();
        write_private(&path, "data").unwrap();
        assert_eq!(fs::read_to_string(&victim).unwrap(), "precious");
        assert_eq!(fs::read_to_string(&path).unwrap(), "data");
    }

    #[test]
    fn refuses_to_replace_a_symbolic_link() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("elsewhere");
        fs::write(&target, "keep").unwrap();
        let link = dir.path().join("file.toml");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(write_private(&link, "data").is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep");
    }

    #[test]
    fn a_checked_write_detects_that_someone_else_wrote_in_between() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.toml");

        // Both writers read "nothing there yet".
        write_private_checked(&path, None, "first").unwrap();
        assert!(matches!(
            write_private_checked(&path, None, "second"),
            Err(WriteError::Changed)
        ));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "first",
            "no lost update"
        );

        // A writer that re-read succeeds.
        write_private_checked(&path, Some("first"), "second").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
    }

    #[test]
    fn racing_checked_writers_never_both_win() {
        let dir = tempfile::tempdir().unwrap();
        let path = std::sync::Arc::new(dir.path().join("file.toml"));
        write_private(&path, "start").unwrap();
        let wins = std::sync::Arc::new(AtomicU64::new(0));
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let (path, wins) = (path.clone(), wins.clone());
                std::thread::spawn(move || {
                    if write_private_checked(&path, Some("start"), &format!("writer {i}")).is_ok() {
                        wins.fetch_add(1, Ordering::SeqCst);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(wins.load(Ordering::SeqCst), 1, "exactly one writer wins");
    }
}
