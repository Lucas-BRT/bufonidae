# Changelog

All notable changes are listed here, newest first. Versions follow
[Semantic Versioning](https://semver.org); while the version is 0.x, anything
may change between releases.

## [Unreleased]

Nothing has been released yet. What exists today:

- Command line: `account`, `put`, `ls`, `get`, `reconcile`, `repair`,
  `redundancy`, `export`, `import`.
- Desktop app: Files (folders, download, drag and drop), Accounts (space, sign
  in again, remove), Check, Settings (copies per file, large-upload limit).
- Placement per file across accounts, with optional `mirror:N` copies per
  default and per folder.
- Content hashes recorded when a file is stored and checked on every download
  and rebuilt copy; nothing local or remote is overwritten.
- Portable, secret-free backup (`export` / `import`) and a recovery guide.
- The rclone version is checked at start; the tested release is 1.76.0.
