//! Thin layer over rclone: version detection now, `rclone rcd` client later.

use std::fmt;

pub mod rcd;

/// Minimum rclone version Bufonidae is tested against (to be confirmed
/// against the `mega` backend before the first release).
pub const MIN_VERSION: Version = Version {
    major: 1,
    minor: 65,
    patch: 0,
};

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
        assert!(v >= MIN_VERSION);
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
}
