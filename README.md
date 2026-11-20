# Bufonidae

Linux desktop app and command line (Rust) that pools several Mega.nz accounts
into one logical directory and spreads files across them, using
[rclone](https://rclone.org) as the only transfer engine.

> **Status: development software, not released.** It works against a real Mega
> account for login, upload, download and cancel, but it has not had a
> qualification pass. Keep your own backups. See
> [`docs/PLAN.md`](docs/PLAN.md) for exactly what is and is not done.

## What it does

- Connects several Mega accounts you already own; passwords live in the system
  keyring (Secret Service), never in a file.
- Stores each file whole on the account with the most free room, or on several
  accounts when you ask for redundancy (`single`, or `mirror:N` for N copies on
  different accounts, per default and per folder).
- Lists, downloads, checks (`reconcile`) and rebuilds missing copies (`repair`).
- Desktop app with folder browsing, drag and drop, progress and cancel.

What it is not: a mounted drive, a two-way sync, or a backup of its own. Today
uploads record a local BLAKE3 hash, which downloads and repaired copies verify
when available. Remote checks compare **size** only; a same-size remote change
requires downloading a copy to verify it. Use `bufonidae verify` or the desktop
Check screen for an explicit deep content check (this consumes transfer bandwidth).

## Requirements

- Linux (developed on Pop!_OS 24.04, Wayland), a Secret Service such as
  GNOME Keyring or KWallet.
- `rclone` on your `PATH`. Tested with 1.76.0; see
  [`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md).
- To build: Rust (the version in `rust-toolchain.toml`) and, for the desktop
  app, the development packages `libxcb1-dev libxkbcommon-dev
  libxkbcommon-x11-dev libwayland-dev libfontconfig-dev libfreetype-dev`.

## Build and run

```
cargo build --release --locked
target/release/bufonidae --help          # command line
target/release/bufonidae-desktop         # desktop app
```

The desktop app and the command line share one index, and only one process can
open it at a time: close the app before using a terminal command, and the other
way round.

## Command line

```
bufonidae account add <label> --email <email>
bufonidae put <paths>... --to <folder> [--accounts a,b]
bufonidae ls [folder]
bufonidae get <path> [--out <dir>]
bufonidae reconcile [--prefix <folder>] [--apply]
bufonidae repair [--prefix <folder>]
bufonidae verify [--prefix <folder>]
bufonidae redundancy show | set <single|mirror|mirror:N> [--folder <f>] | unset <f>
```

## Desktop preferences

Settings provides density/text size, Files search and sorting defaults,
notification and sound policies, keyboard presets, context menus and guidance.
Desktop preferences use **Save desktop preferences** / **Discard unsaved changes**;
redundancy and the large-upload limit retain their immediate-save behavior.
Click the sidebar storage meter to open **Storage distribution** and **Sync**.
Review per-account before/after usage and file changes, then approve balance,
repair, mirror (2–8 copies), or account evacuation. The preview includes all
staged download/upload/verification legs, local disk peak, editable bandwidth
caps and modeled hours; provider quota/reset waits remain uncertain. Old copies
are retired only after replacement readback verification. Stop waits for the
current file; options apply to this run. Parity and block striping are not
implemented. Future upload redundancy remains configured in Settings.

The sidebar shows aggregated remote storage usage, available space and account
coverage, with an explicit refresh action. Quotas are checked after accounts
load; partial/unavailable reports are labeled. Commands and Help stay available
in the top utility bar. Files supports multiple selected paths; Help includes an optional guided tour,
and Commands opens a searchable action palette (`Ctrl+K` in the full preset).

Optional Linux notifications require `gdbus` and a session notification service;
sounds require `paplay` and the freedesktop sound theme. Settings has test buttons
and reports delivery errors. The app must remain running. Settings also offers
an optional native StatusNotifier tray icon: idle visibility, transfer-count
badge and primary-click behavior are configured independently. Its menu opens
the app, Transfers or Settings. GNOME may require an AppIndicator extension;
connection errors and Retry are available in Settings. Closing the window exits
the app. Counts refresh every two seconds; the badge caps at 99 and the tooltip
shows the exact count. Some desktop shells do not render overlay badges.

## More

- [`docs/QUALIFICATION.md`](docs/QUALIFICATION.md): the live-Mega checklist (not run yet).
- [`docs/RECOVERY.md`](docs/RECOVERY.md), [`SECURITY.md`](SECURITY.md),
  [`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md), [`docs/RELEASING.md`](docs/RELEASING.md),
  [`CHANGELOG.md`](CHANGELOG.md).

## Mega terms of service

Bufonidae only connects accounts you already own. It does not create accounts
and must not be used to get around Mega's quota limits; doing so may get your
accounts suspended.

## License

MIT, see [`LICENSE`](LICENSE).
