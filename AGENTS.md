# Agent instructions

- **Everything in this repository is written in English**: code, identifiers,
  comments, commit messages, documentation, issue and PR text, UI strings.
- The plan lives in `docs/PLAN.md`; keep it current when a decision changes.
- `bufonidae-core` must not depend on gpui or clap.
- No `unwrap` outside tests; typed errors (`thiserror`) in libraries.
- Pin every version (toolchain, tools in CI, actions by commit SHA). Bumps go
  in their own commit with the reason.
- rclone is the only transfer engine. Never automate Mega account creation.
- Before pushing: `cargo fmt --check`, `cargo clippy --locked --workspace
  --all-targets -- -D warnings`, `cargo test --locked --workspace`.
