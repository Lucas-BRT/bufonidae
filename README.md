# Bufonidae

Linux desktop app and CLI (Rust) that pools several Mega.nz accounts into one
logical directory and distributes files among them, using
[rclone](https://rclone.org) as the only transfer engine.

Status: early planning and scaffolding. See [`docs/PLAN.md`](docs/PLAN.md).

## Workspace

| Path | Purpose |
|------|---------|
| `crates/bufonidae-core` | placement, index, journal, quotas |
| `crates/bufonidae-rclone` | `rclone rcd` client and supervisor |
| `crates/bufonidae-cli` | `bufonidae` command |
| `apps/desktop` | gpui desktop app (MVP-2) |
| `website` | project website |

## Mega terms of service

Bufonidae only connects accounts you already own. It does not create accounts,
and it must not be used to circumvent Mega's quota limits; doing so may get
your accounts suspended.

## License

MIT, see [`LICENSE`](LICENSE).
