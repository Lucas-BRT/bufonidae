//! The one place that decides whether a stored path is safe to use.
//!
//! Stored paths come from the local index, from a backup file and from remote
//! listings, none of which are trusted. A path that passes here can be joined
//! under a local folder, or given to rclone as a remote path, without leaving
//! that place.

/// `true` for a relative, `/`-separated path of real names.
///
/// Refused: empty paths, a leading `/`, empty parts (`a//b`), `.` and `..`
/// parts, and NUL. Everything else is allowed on purpose: real files may have
/// spaces, newlines, `:` or `\`, so display code escapes control characters
/// instead.
#[must_use]
pub fn is_safe_stored_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\0')
        && !path.starts_with('/')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_and_awkward_but_real_names() {
        for ok in [
            "a",
            "a/b/c.txt",
            "Fotos/são joão.jpg",
            "weird:name/with\\backslash",
            "new\nline/ok",
            ".hidden/.also",
            "a..b/c",
            "...",
        ] {
            assert!(is_safe_stored_path(ok), "{ok:?} should be allowed");
        }
    }

    #[test]
    fn refuses_anything_that_could_leave_its_place() {
        for bad in [
            "",
            "/abs",
            "a//b",
            "a/",
            "/",
            ".",
            "..",
            "../x",
            "a/../b",
            "a/./b",
            "a/..",
            "nul\0byte",
        ] {
            assert!(!is_safe_stored_path(bad), "{bad:?} should be refused");
        }
    }
}
