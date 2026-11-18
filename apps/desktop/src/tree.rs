//! The stored files as a folder tree, derived from their flat paths.

use std::collections::BTreeMap;

use bufonidae_core::index::IndexedFile;

/// One row of a folder: a subfolder (with what it holds) or a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Folder {
        name: String,
        path: String,
        files: usize,
        bytes: u64,
    },
    File {
        name: String,
        path: String,
        size: u64,
        accounts: Vec<String>,
    },
}

impl Row {
    pub fn name(&self) -> &str {
        match self {
            Self::Folder { name, .. } | Self::File { name, .. } => name,
        }
    }
    pub fn bytes(&self) -> u64 {
        match self {
            Self::Folder { bytes, .. } => *bytes,
            Self::File { size, .. } => *size,
        }
    }
    pub fn is_folder(&self) -> bool {
        matches!(self, Self::Folder { .. })
    }

    pub fn path(&self) -> &str {
        match self {
            Self::Folder { path, .. } | Self::File { path, .. } => path,
        }
    }
}

fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    }
}

/// What `folder` directly contains: subfolders first, then files, each by
/// name ignoring case. `folder` is `""` for the top level.
/// Current-folder search/filter/sort projection; selection lives separately.
pub fn visible_rows(
    files: &[IndexedFile],
    folder: &str,
    query: &str,
    sort: bufonidae_core::config::FileSort,
    filter: bufonidae_core::config::FileFilter,
    options: &bufonidae_core::config::DesktopOptions,
) -> Vec<Row> {
    use bufonidae_core::config::{FileFilter, FileSort};
    let query = if options.case_sensitive {
        query.to_owned()
    } else {
        query.to_lowercase()
    };
    let mut visible = rows(files, folder);
    visible.retain(|row| {
        let name = if options.case_sensitive {
            row.name().to_owned()
        } else {
            row.name().to_lowercase()
        };
        name.contains(&query)
            && match filter {
                FileFilter::All => true,
                FileFilter::Files => !row.is_folder(),
                FileFilter::Folders => row.is_folder(),
            }
    });
    visible.sort_by(|a, b| {
        let group = if options.folders_first {
            b.is_folder().cmp(&a.is_folder())
        } else {
            std::cmp::Ordering::Equal
        };
        group
            .then_with(|| match sort {
                FileSort::Name => a.name().to_lowercase().cmp(&b.name().to_lowercase()),
                FileSort::NameDescending => b.name().to_lowercase().cmp(&a.name().to_lowercase()),
                FileSort::Size => a.bytes().cmp(&b.bytes()),
                FileSort::SizeDescending => b.bytes().cmp(&a.bytes()),
            })
            .then_with(|| a.path().cmp(b.path()))
    });
    visible
}

pub fn rows(files: &[IndexedFile], folder: &str) -> Vec<Row> {
    let prefix = if folder.is_empty() {
        String::new()
    } else {
        format!("{folder}/")
    };
    let mut folders: BTreeMap<String, (usize, u64)> = BTreeMap::new();
    let mut direct = Vec::new();
    for file in files {
        let Some(rest) = file.path.strip_prefix(&prefix) else {
            continue;
        };
        match rest.split_once('/') {
            Some((first, _)) => {
                let entry = folders.entry(first.to_owned()).or_default();
                entry.0 += 1;
                entry.1 += file.size;
            }
            None if !rest.is_empty() => direct.push(Row::File {
                name: rest.to_owned(),
                path: file.path.clone(),
                size: file.size,
                accounts: file.accounts.clone(),
            }),
            None => {}
        }
    }
    let mut out: Vec<Row> = folders
        .into_iter()
        .map(|(name, (files, bytes))| Row::Folder {
            path: join(folder, &name),
            name,
            files,
            bytes,
        })
        .collect();
    out.sort_by_key(|r| match r {
        Row::Folder { name, .. } => name.to_lowercase(),
        Row::File { .. } => String::new(),
    });
    direct.sort_by_key(|r| match r {
        Row::File { name, .. } => name.to_lowercase(),
        Row::Folder { .. } => String::new(),
    });
    out.extend(direct);
    out
}

/// `("Photos", "Photos")`, `("2026", "Photos/2026")` for `Photos/2026`.
pub fn crumbs(folder: &str) -> Vec<(String, String)> {
    let mut path = String::new();
    folder
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|segment| {
            path = join(&path, segment);
            (segment.to_owned(), path.clone())
        })
        .collect()
}

/// `true` when something is stored under `folder` (or it is the top level).
pub fn exists(files: &[IndexedFile], folder: &str) -> bool {
    folder.is_empty()
        || files.iter().any(|f| {
            f.path
                .strip_prefix(folder)
                .is_some_and(|r| r.starts_with('/'))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, size: u64) -> IndexedFile {
        IndexedFile {
            path: path.to_owned(),
            size,
            accounts: vec!["a".to_owned()],
            blake3: None,
        }
    }

    fn sample() -> Vec<IndexedFile> {
        vec![
            file("README.txt", 10),
            file("Photos/2026/a.jpg", 100),
            file("Photos/2026/b.jpg", 200),
            file("Photos/2025/c.jpg", 300),
            file("Photos/cover.png", 5),
            file("photos-old/x.bin", 1),
        ]
    }

    #[test]
    fn search_filter_and_sort_respect_saved_choices_and_folder_grouping() {
        use bufonidae_core::config::{DesktopOptions, FileFilter, FileSort};
        let files = vec![
            file("Zoo/child", 1),
            file("alpha.txt", 4),
            file("Beta.txt", 9),
            file("écho.txt", 2),
        ];
        let options = DesktopOptions::default();
        let names = |rows: Vec<Row>| {
            rows.iter()
                .map(|row| row.name().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(visible_rows(
                &files,
                "",
                "",
                FileSort::Name,
                FileFilter::All,
                &options
            )),
            ["Zoo", "alpha.txt", "Beta.txt", "écho.txt"]
        );
        assert_eq!(
            names(visible_rows(
                &files,
                "",
                "",
                FileSort::SizeDescending,
                FileFilter::Files,
                &options
            )),
            ["Beta.txt", "alpha.txt", "écho.txt"]
        );
        assert_eq!(
            names(visible_rows(
                &files,
                "",
                "beta",
                FileSort::Name,
                FileFilter::All,
                &options
            )),
            ["Beta.txt"]
        );
        let strict = DesktopOptions {
            case_sensitive: true,
            ..options
        };
        assert!(
            visible_rows(&files, "", "beta", FileSort::Name, FileFilter::All, &strict).is_empty()
        );
        assert_eq!(
            names(visible_rows(
                &files,
                "",
                "",
                FileSort::Name,
                FileFilter::Folders,
                &strict
            )),
            ["Zoo"]
        );
    }

    #[test]
    fn top_level_shows_folders_then_files() {
        let r = rows(&sample(), "");
        let names: Vec<_> = r
            .iter()
            .map(|r| match r {
                Row::Folder { name, .. } | Row::File { name, .. } => name.as_str(),
            })
            .collect();
        assert_eq!(names, ["Photos", "photos-old", "README.txt"]);
        assert_eq!(
            r[0],
            Row::Folder {
                name: "Photos".into(),
                path: "Photos".into(),
                files: 4,
                bytes: 605
            }
        );
    }

    #[test]
    fn a_folder_lists_its_direct_children_only() {
        let r = rows(&sample(), "Photos");
        assert_eq!(r.len(), 3, "2025, 2026 and cover.png; not photos-old");
        assert!(matches!(&r[0], Row::Folder { name, files: 1, .. } if name == "2025"));
        assert!(matches!(&r[1], Row::Folder { name, files: 2, bytes: 300, .. } if name == "2026"));
        assert!(matches!(&r[2], Row::File { path, .. } if path == "Photos/cover.png"));
    }

    #[test]
    fn crumbs_and_existence() {
        assert_eq!(
            crumbs("Photos/2026"),
            [
                ("Photos".to_owned(), "Photos".to_owned()),
                ("2026".to_owned(), "Photos/2026".to_owned())
            ]
        );
        assert!(crumbs("").is_empty());
        let files = sample();
        assert!(exists(&files, ""));
        assert!(exists(&files, "Photos/2026"));
        assert!(!exists(&files, "Photos/2027"));
        assert!(
            !exists(&files, "Phot"),
            "a prefix of a name is not a folder"
        );
    }
}
