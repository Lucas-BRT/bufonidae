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
it checks file **size** only, not content.

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
bufonidae redundancy show | set <single|mirror|mirror:N> [--folder <f>] | unset <f>
```

## Mega terms of service

Bufonidae only connects accounts you already own. It does not create accounts
and must not be used to get around Mega's quota limits; doing so may get your
accounts suspended.

## License

MIT, see [`LICENSE`](LICENSE).
