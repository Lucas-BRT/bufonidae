# Compatibility record

What each part of the stack is tested with. Anything not listed is **untested**
and must not be advertised as supported.

| Component | Version | Where it is checked |
|---|---|---|
| Rust toolchain | 1.99.1 (`rust-toolchain.toml`) | CI and development |
| rclone | 1.76.0 | CI (integration tests on the `local` backend) and a real Mega account by hand |
| gpui | `=0.2.2` | CI (build) and by hand |
| gpui-component | `=0.5.1` (last release on the official gpui 0.2.x) | CI (build) and by hand |
| Turso (`turso` crate) | `=0.9.0-pre.5`, a beta | CI (tests) and by hand |
| Patched `xattr` | 0.2.3 in `vendor/xattr` | see `vendor/xattr/PATCHED.md` |

| Platform | State |
|---|---|
| Pop!_OS 24.04 (COSMIC, Wayland), x86_64 | developed and used by hand |
| Ubuntu 24.04 runner, x86_64 | CI builds and runs the tests; no display |
| Other distributions, X11, ARM64 | untested |

## Notes

- The Linux file dialog offers files or folders, never both, so the desktop app
  has its own file browser. Downloads use the system folder dialog.
- The rclone version is **enforced**: `Rcd::start` runs `rclone version` first and
  refuses anything older than 1.76.0 (the only tested release) or a pre-release
  build. For development only, `BUFONIDAE_ALLOW_UNTESTED_RCLONE=1` waives that.
  A test keeps the version in `ci.yml` and the one in the code identical.
- Mega hands out no content hash through rclone; integrity is size only.
