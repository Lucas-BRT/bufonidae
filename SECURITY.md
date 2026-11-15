# Security

## Reporting a problem

Do not open a public issue for a vulnerability. While this repository is
private, tell the maintainer directly. Once it is public, use GitHub's private
vulnerability reporting (Security tab, "Report a vulnerability").

## What Bufonidae protects

- **Passwords and Mega sessions** are kept in the desktop's Secret Service
  (GNOME Keyring, KWallet), never in a file, an export, the index or a log.
  If the keyring is locked, missing or refuses, the operation fails; there is
  no plaintext fallback.
- **The rclone daemon** (`rclone rcd`) listens only on `127.0.0.1` on a random
  port, with credentials made fresh for each run and passed in the environment,
  not on the command line. It never reads your `rclone.conf`, and accounts are
  defined in its memory only. The client does not use proxies and does not
  follow redirects. Every `RCLONE_*` variable of your own environment is
  removed from the daemon's, so your rclone settings cannot change what it does
  or switch on debug output.
- **Local files** (`accounts.toml`, `config.toml`, exports, the index) are
  created mode 0600 in a 0700 folder, written atomically and durably, and never
  replace a symbolic link.
- **Paths** read from the index, from a backup or from a remote listing are
  checked before use: nothing with `..`, an empty part, a leading `/` or NUL
  reaches the filesystem or rclone, a downloaded file never replaces an existing
  one, and a link in the destination's folder path is not followed.
- **What is shown** is escaped: file names and rclone's messages cannot inject
  terminal control sequences into command output, and errors from `config/*`
  calls are withheld because rclone echoes their input (which holds a password).

## What it does not protect

Be clear about these before relying on it:

- **A compromised user session.** Anything running as you can read the keyring,
  your files and the index, and can read the daemon's credentials from its
  process. The keyring protects against a stolen disk and other users, not
  against malware running as you. Root can read everything.
- **Metadata is plaintext.** The index, `accounts.toml` and exports list your
  account e-mail addresses, folder structure and file names unencrypted.
  Mega encrypts file *contents* end to end; this local metadata is not.
- **Staging is unencrypted.** `repair` copies a file through a temporary folder
  in the system temporary directory to make a new copy, and downloads are
  staged in a hidden folder inside the destination. They are removed when
  finished or cancelled, but a crash can leave one behind, and the disk may
  keep the data after deletion.
- **Memory.** The password exists briefly in this process and in rclone's.
  Text fields are cleared when a sign-in starts, but secrets are not
  zeroized, and a crash dump could contain them.
- **No integrity proof for old files.** Only files stored by this version have
  a recorded content hash. A copy is checked against it when it is read back,
  but Mega gives no hash, so damage is found on use, not by scanning.
- **Mega's terms.** Connecting accounts you own is fine; using this to evade
  Mega's quotas may get accounts suspended.

## Supply chain

Dependencies are pinned exactly and locked, GitHub Actions are pinned by commit
hash, the rclone binary used in CI is checked against a SHA-256, and `cargo
audit` runs weekly. `vendor/xattr` is a patched copy of a third-party crate;
see `vendor/xattr/PATCHED.md`.
