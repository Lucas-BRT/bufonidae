# Releasing

Checklist for a release. Nothing here is automated yet: the release workflow is
milestone 8 in `docs/PLAN.md`.

## Before tagging

1. `docs/PLAN.md` status matches reality, and no feature is advertised that is
   not implemented (hashes, quotas, rename/delete).
2. `docs/COMPATIBILITY.md` lists the exact versions that were tested, and the
   manual live-Mega checklist was run against them.
3. `cargo fmt --check`, `cargo clippy --locked --workspace --all-targets -- -D
   warnings`, `cargo test --locked --workspace` pass; CI is green on `main`.
4. `cargo audit` and (once added) `cargo deny check` are clean, or every
   exception is written down with a reason.
5. Dependency, toolchain, gpui and rclone bumps went in their own commits.
6. `CHANGELOG.md` has the release notes (added with the release workflow).

## Never together

Do not upgrade storage (Turso), the UI stack (gpui) and rclone in the same
release.
