//! Thin layer over rclone: version detection now, `rclone rcd` client later.

use std::fmt;

pub mod rcd;

/// The rclone release Bufonidae is tested against: CI runs its integration
/// tests with it and the live Mega checks were made with it.
pub const TESTED_VERSION: Version = Version {
    major: 1,
    minor: 76,
    patch: 0,
};

/// The oldest rclone Bufonidae will start. Only the tested release is known to
/// work, so this is not lowered on a guess.
pub const MIN_VERSION: Version = TESTED_VERSION;

/// Set this environment variable (to anything non-empty) to run with an
/// rclone that is older than [`MIN_VERSION`] or a pre-release. For
/// development only: behaviour is unverified.
pub const ALLOW_UNTESTED_ENV: &str = "BUFONIDAE_ALLOW_UNTESTED_RCLONE";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not parse rclone version from output: {0:?}")]
    UnparsableVersion(String),
    #[error(
        "rclone {found} is older than the {min} Bufonidae is tested with; install {min} or newer \
         (set {ALLOW_UNTESTED_ENV}=1 to try anyway, unsupported)"
    )]
    UnsupportedVersion { found: Version, min: Version },
    #[error(
        "rclone {0} is a pre-release build, which Bufonidae does not support \
         (set {ALLOW_UNTESTED_ENV}=1 to try anyway, unsupported)"
    )]
    Prerelease(String),
    #[error("failed to spawn rclone: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("rclone rcd did not start: {0}")]
    Startup(String),
    #[error("request to rclone rcd failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("rclone rcd returned an error for `{method}` (HTTP {status}): {message}")]
    Rpc {
        method: String,
        status: u16,
        message: String,
    },
}

/// Longest diagnostic text kept from rclone, in characters.
const MAX_DIAGNOSTIC: usize = 1000;

/// Makes text from rclone (or from a file name) safe to show: control
/// characters, including the escape that starts terminal sequences, become a
/// visible `\u{..}` form, and the length is capped. rclone's messages and the
/// names of the user's files are untrusted data.
#[must_use]
pub fn escape_control(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(MAX_DIAGNOSTIC));
    for (count, c) in text.chars().enumerate() {
        if count >= MAX_DIAGNOSTIC {
            out.push('…');
            break;
        }
        if c.is_control() {
            out.push_str(&format!("\\u{{{:x}}}", u32::from(c)));
        } else {
            out.push(c);
        }
    }
    out
}

/// Checks the output of `rclone version` against [`MIN_VERSION`]. Pre-release
/// builds are refused too; `allow_untested` waives both checks.
pub fn check_supported(output: &str, allow_untested: bool) -> Result<Version, Error> {
    let found = parse_version(output)?;
    if allow_untested {
        return Ok(found);
    }
    let first_line = output.lines().next().unwrap_or_default();
    if first_line.contains('-') || first_line.contains('+') {
        return Err(Error::Prerelease(first_line.trim().to_owned()));
    }
    if found < MIN_VERSION {
        return Err(Error::UnsupportedVersion {
            found,
            min: MIN_VERSION,
        });
    }
    Ok(found)
}

impl Error {
    /// `true` only for rclone's "that directory does not exist" answer
    /// (HTTP 404, `directory not found`). Anything else, including a missing
    /// remote or a login failure, is a real error and never an empty folder.
    #[must_use]
    pub fn is_directory_not_found(&self) -> bool {
        matches!(
            self,
            Self::Rpc { status: 404, message, .. } if message.ends_with("directory not found")
        )
    }
}

/// Parses the first line of `rclone version`, e.g. `rclone v1.68.2`.
pub fn parse_version(output: &str) -> Result<Version, Error> {
    let bad = || Error::UnparsableVersion(output.lines().next().unwrap_or_default().to_owned());
    let token = output
        .lines()
        .next()
        .and_then(|l| l.strip_prefix("rclone v"))
        .ok_or_else(bad)?;
    // Drop suffixes such as `-beta.123.abcdef`.
    let core = token.split(['-', '+', ' ']).next().ok_or_else(bad)?;
    let mut parts = core.split('.').map(str::parse::<u32>);
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(Ok(major)), Some(Ok(minor)), Some(Ok(patch)), None) => Ok(Version {
            major,
            minor,
            patch,
        }),
        _ => Err(bad()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_release_version() {
        let v = parse_version("rclone v1.68.2\n- os/version: ubuntu 24.04\n").unwrap();
        assert_eq!(v.to_string(), "1.68.2");
    }

    #[test]
    fn parses_beta_version() {
        let v = parse_version("rclone v1.69.0-beta.8000.abcdef\n").unwrap();
        assert_eq!(v.to_string(), "1.69.0");
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_version("command not found").is_err());
        assert!(parse_version("").is_err());
        assert!(parse_version("rclone v1.2\n").is_err());
    }

    fn rpc(status: u16, message: &str) -> Error {
        Error::Rpc {
            method: "operations/list".into(),
            status,
            message: message.into(),
        }
    }

    #[test]
    fn only_a_real_directory_not_found_counts_as_one() {
        assert!(rpc(404, "error in ListJSON: directory not found").is_directory_not_found());
        // Same words from a different failure, or the wrong status, must not.
        assert!(!rpc(500, "error in ListJSON: directory not found").is_directory_not_found());
        assert!(!rpc(404, "object not found").is_directory_not_found());
        assert!(
            !rpc(500, "didn't find section in config file (\"acct-x\")").is_directory_not_found()
        );
        assert!(!rpc(500, "couldn't login: not found").is_directory_not_found());
        assert!(!Error::Startup("directory not found".into()).is_directory_not_found());
    }

    #[test]
    fn old_and_pre_release_versions_are_refused_unless_allowed() {
        assert!(check_supported("rclone v1.76.0\n", false).is_ok());
        assert!(check_supported("rclone v1.80.1\n", false).is_ok());
        assert!(matches!(
            check_supported("rclone v1.75.9\n", false),
            Err(Error::UnsupportedVersion { .. })
        ));
        assert!(matches!(
            check_supported("rclone v1.60.1-DEV\n", false),
            Err(Error::Prerelease(_))
        ));
        assert!(matches!(
            check_supported("rclone v1.77.0-beta.9.abc\n", false),
            Err(Error::Prerelease(_))
        ));
        assert!(matches!(
            check_supported("not rclone", false),
            Err(Error::UnparsableVersion(_))
        ));
        // The development override waives both checks, but not parsing.
        assert!(check_supported("rclone v1.60.1-DEV\n", true).is_ok());
        assert!(check_supported("garbage", true).is_err());
    }

    /// The version CI downloads and the version the code enforces must be the
    /// same, or CI would test one rclone and ship another.
    #[test]
    fn ci_downloads_the_rclone_version_the_code_expects() {
        let ci = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../.github/workflows/ci.yml"
        ))
        .unwrap();
        let line = ci
            .lines()
            .find(|l| l.trim_start().starts_with("RCLONE_VERSION:"))
            .unwrap();
        assert!(
            line.contains(&format!("\"{TESTED_VERSION}\"")),
            "ci.yml says {line:?}, the code expects {TESTED_VERSION}"
        );
    }

    #[test]
    fn control_characters_cannot_reach_a_terminal() {
        let hostile = "ok\x1b[2J\x1b]0;pwned\x07 line\nbreak\r\0end";
        let shown = escape_control(hostile);
        assert!(!shown.chars().any(char::is_control), "{shown:?}");
        assert!(shown.contains("\\u{1b}"));
        assert!(shown.starts_with("ok") && shown.ends_with("end"));
        // Ordinary text, including non-ASCII, is untouched.
        assert_eq!(escape_control("fotos/são joão.jpg"), "fotos/são joão.jpg");
        // Length is capped.
        let long = "x".repeat(5000);
        assert!(escape_control(&long).chars().count() <= MAX_DIAGNOSTIC + 1);
    }
}
