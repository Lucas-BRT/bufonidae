# Agent instructions

- **Everything in this repository is written in English**: code, identifiers,
  comments, commit messages, documentation, issue and PR text, UI strings.
- The plan lives in `docs/PLAN.md`; keep it current when a decision changes.
- `bufonidae-core` must not depend on gpui or clap.
- No `unwrap` outside tests; typed errors (`thiserror`) in libraries.
- Pin every version (toolchain, tools in CI, actions by commit SHA). Bumps go
  in their own commit with the reason.
- rclone is the only transfer engine. Never automate Mega account creation.
- Branch from `develop` using `feat/<slug>` or `fix/<slug>`. Open PRs into
  `develop` and promote `develop` to `main` by PR.
- Use small Conventional Commits. Enable the tracked hooks with
  `git config --local core.hooksPath .githooks`.
- Before pushing, run the formatting, Clippy, and workspace tests in
  `.githooks/pre-push`. Run `cargo pedantic` for additional diagnostics and
  justify lint exceptions at the narrowest relevant scope.
