# Bufonidae — plan

Linux desktop app (Rust) that aggregates N Mega.nz accounts into a single logical directory and distributes files among them, using `rclone` as the only transfer engine.

## 0. Repository conventions

- **Everything in the repository is written in English**: code, identifiers, comments, commit messages, documentation, issue/PR text, UI strings (i18n can come later, with English as the source language).
- MIT license. Private repository for now.

## 1. Architecture

```
bufonidae/                      (Cargo workspace, edition 2024, pinned toolchain)
├─ crates/
│  ├─ bufonidae-core/           placement, index, journal, quotas, policies (no UI I/O)
│  ├─ bufonidae-rclone/         `rclone rcd` client (JSON-RPC/HTTP), process supervisor
│  └─ bufonidae-cli/            `bufonidae` binary (clap), built on core
├─ apps/
│  └─ desktop/                  gpui + gpui-component, built on core
├─ website/                     Rust + Topcoat crate (SSR), Dockerfile + compose.yml
└─ docs/
```

Rule: core depends on neither gpui nor clap. CLI and desktop stay thin; all testable logic lives in core.

Flow: `drop` (GUI/CLI/watch folder) → **planner** (core) picks a destination account per item → **journal** (Turso) records the intent → `rclone rcd` executes (`operations/copyfile`, `sync/copy`) → verification → journal confirms → index updates.

## 2. Problems and solutions

| # | Problem | Solution |
|---|---------|----------|
| 1 | **Mega ToS** forbids circumventing quota limits; many free accounts may get banned. | Never automate account creation. Only connect accounts the user already owns. Explicit warning on first run. Documented in the README and website. |
| 2 | **rsync does not speak Mega.** | Dropped: rclone is the only transfer engine. Local staging is handled by the planner (size/mtime snapshot before and after, see #20). |
| 3 | **rclone's `mega` backend has no usable hashes or mtimes**: `--checksum` does not work, comparison is by size only. | Compute a local blake3 at upload time and store it in the index; verify integrity by size + indexed hash; `rclone check --size-only` as fallback. Confirm when pinning the rclone version. |
| 4 | **Per-account transfer (bandwidth) quota**, besides storage. Downloads fail when it runs out. | Model two quotas per account: free space (`operations/about`) and an estimated bandwidth budget over a sliding window. Reads pick the replica/account with bandwidth left; backoff on quota errors. |
| 5 | **Moving between accounts = download + upload** (no server-side cross-account move). Rebalancing is expensive. | Get placement right the first time. Rebalancing is manual/scheduled only, with a cost estimate shown first. |
| 6 | **Bad placement**: large files, folders with many small files, folders that should stay together. | Pluggable policies (`Placement` trait): `most-free-space` (default), `round-robin`, `keep-folder-together` (whole folder on one account if it fits), `spread` (per file). First-fit-decreasing bin packing for batch drops. Configurable minimum headroom per account (e.g. 5%). Default granularity: per file (decided), with keep-together as a per-folder option. |
| 7 | **File larger than any account's free space.** | Optional chunking: N MB parts on distinct accounts + manifest in the index. Phase 3; the MVP rejects with a clear error. Alternative: `rclone chunker` over `union` (to evaluate). |
| 8 | **Losing an account loses part of the data** (no redundancy). | RAID-style redundancy, optional and configurable per pool, overridable per folder (see §3). UI shows "data at risk" per account. Exportable index. |
| 9 | **Index drifts from reality** (someone edited via Mega's site, crash mid-transfer). | Source of truth = remote listing; the index is a cache. Write-ahead journal for recovery. A `reconcile` command/action (recursive `operations/list`) finds orphans, missing items and duplicates. |
| 10 | **Mega allows duplicate names** in the same folder. | The planner never creates duplicates; reconcile flags them (`rclone dedupe` only with confirmation). |
| 11 | **Rate limits / logins** (many simultaneous logins → errors, temporary blocks). | A single `rcd` instance manages all N accounts; sessions are reused; per-account and global concurrency semaphores; exponential backoff with jitter. |
| 12 | **Credentials**: `rclone.conf` only obscures passwords (obscure ≠ encryption). | Secrets in the Secret Service (GNOME Keyring/KWallet) via `keyring`/`oo7`. The rclone config is generated at runtime (0600 temp file in `XDG_RUNTIME_DIR`, or `config/create` over RC) and never persisted with passwords. 2FA: confirm support in the pinned rclone and expose a field in the UI. |
| 13 | **Metadata privacy**: the local index reveals folder structure. | Turso database in `XDG_DATA_HOME`, mode 0600; at-rest encryption out of MVP scope (Turso's is experimental). Mega is already E2EE; optional `crypt` on top is out of the MVP. |
| 14 | **External binary dependency** (rclone missing/wrong version). | Detect at startup (`rclone version`), pinned minimum version, actionable error. Not bundled in the MVP; packaging (AppImage/Flatpak) may include it. |
| 15 | **Orphaned `rcd` process / exposed port.** | Start with `--rc-addr 127.0.0.1:0` (ephemeral port) + random per-session `--rc-user/--rc-pass`; supervisor kills it on drop; `kill_on_drop`. |
| 16 | **gpui is unstable** (API changes without strict semver; ecosystem in flux). | Exact pin (zed git rev or crates.io version; confirm what is published at scaffold time). Thin view layer isolated in `apps/desktop`; gpui upgrades get their own commit. `gpui-component` for widgets (table, tree, input). |
| 17 | **gpui on Linux needs system libs** (Vulkan, Wayland/X11, xkbcommon, fontconfig…) and a GPU. | List in README and CI (apt). Check software-rendering fallback; document requirements. |
| 18 | **Testing without Mega** (real accounts in CI = secrets + flaky + ToS). | Fake accounts = rclone `local`/`memory` backend with simulated limits. `RemoteFs` trait in core with an `rclone` impl and a deterministic `fake` impl. Live Mega tests are manual/`--ignored`, never in CI. |
| 19 | **Long-running operations in the UI** (cancel, resume, progress). | Job queue in core with persisted states; progress via the rcd's `core/stats`/`job/status`; UI subscribes to an event channel. Resumable after a crash via the journal. |
| 20 | **Path safety**: dropped symlinks, `..`, invalid names, files changing during upload. | Canonicalize, explicit symlink policy (do not follow by default), snapshot size/mtime before and re-check after. |

## 3. Redundancy model (RAID-style, optional)

Configured per pool, overridable per folder:

- `single` (JBOD, default): one copy, no redundancy.
- `mirror` (RAID1): N copies on N distinct accounts. In MVP-1.
- `parity` (RAID5/6-like): erasure coding, k data + m parity parts on distinct accounts. Later phase.

Implemented: `single` and `mirror:N` (N = 2..8), set as a default and per folder (longest matching folder wins) with `bufonidae redundancy set|show|unset`; stored in `config.toml`. There is no separate "pool" object yet: the default level plays the pool-wide role. A level applies to files stored from then on; `bufonidae repair` brings existing files up to it, and also rebuilds copies lost with an account. A file that cannot get every copy (too few accounts or no room) is not stored at all, instead of being silently under-protected. `parity` is still future work.

The planner picks N distinct accounts per item; reads choose the healthiest copy (quota, errors); a degraded pool (account down) is shown in the UI with a repair action.

## 4. Data model (Turso, embedded)

- `account(id, label, remote_name, quota_total, quota_used, bw_window…, enabled)`
- `item(id, virtual_path, size, blake3, kind)`; `placement(item_id, account_id, remote_path, state)`
- `journal(id, op, payload, state, ts)` — write-ahead
- Virtual directory = set of accounts + policy; "3 accounts in the same directory" = one *pool*.

Implemented so far: `items(path, size)` and `placements(path, account_id, state)`, where `state` is `pending` (written before the upload, i.e. the write-ahead journal) or `done`.

**Why Turso, and what it costs** (decided by the project owner): it is a beta, pre-release Rust rewrite of SQLite (`turso` 0.9.0-pre.5). Consequences handled in the design:
- *Beta/data safety*: the index is a cache and the remote listing is the source of truth, so a corrupted index is rebuilt with `reconcile`, not lost data.
- *One process per database file*: the CLI and the desktop app cannot have the index open at the same time. The desktop app will need to own the index and the CLI to refuse (or talk to it) while the app runs. To be solved before MVP-2.
- *Pinned exactly*, bumped on purpose in its own commit; Dependabot will report newer pre-releases.

## 5. Status and release scope

### First-release guarantees (the contract)

- Bufonidae is a **logical directory over several accounts**, not a mounted filesystem and not a two-way sync. It stores, lists, downloads, and repairs; it does not watch folders.
- **Mirroring protects against losing an account**, not against Mega itself failing for everyone, and it is not a backup.
- "Every copy fits" is checked **before** a store starts. A runtime failure can still leave a file with fewer copies than wanted; `repair` completes it.
- Existing files are **never silently overwritten**: `get` never replaces a local file; `put` leaves a remote file in place and says so.
- Only regular files are stored. Symbolic links, special files and non-UTF-8 names are skipped and reported. Empty folders are not stored.
- Integrity today is **size only**. Content hashes (BLAKE3) are planned, not implemented. A same-size difference is not detected yet.
- The local index is **not disposable**: it holds redundancy intent and (later) hashes that a remote listing cannot recreate. Export and backup are release tasks.

### Status

| Area | State |
|---|---|
| Core: placement, mirror:N, put/get/ls, reconcile, repair, config | Done and tested with fake remotes; used against one real Mega account (login, upload, download, cancel) |
| Integrity | BLAKE3 recorded at store time and checked on every download and rebuilt copy; staged, verified, never-overwriting downloads; collisions and "different file already there" refused; a file edited mid-upload is not marked done |
| Metadata | Transactional index (schema v3), lost files remembered, reconcile quarantines ambiguity (duplicate names, disagreeing sizes), portable export/import, atomic private writes with lost-update detection |
| Resilience | One account that cannot sign in or does not answer is set aside and reported; the rest carry on |
| Security | rclone version enforced; daemon environment scrubbed, no proxy or redirects; central path validator; terminal-safe output; `SECURITY.md` |
| CLI | `account` (add, list, reauth, remove), `put`, `ls`, `get`, `reconcile`, `repair`, `redundancy`, `export`, `import` |
| Desktop | Files (folders, download, drop, cancel), Accounts (space, sign in again, remove), Check (reconcile and repair), Settings (copies, large-upload limit), unfinished-upload notice |
| Website | Four pages, facts read from the product, static container image checked running read-only |
| Packaging | Tarball and `.deb` with the tested rclone inside, reproducible; release workflow makes a **draft** pre-release |

### What is left

Honest list, most important first. Nothing here is hidden by the status above.

1. **Live qualification** (`docs/QUALIFICATION.md`): the checklist exists and has not been run. Only login, upload, download and cancel were tried on a real account; everything else is tested with fakes.
2. **Release pipeline**: exercised by hand-dispatch only (see `docs/RELEASING.md`); a real tag has never been pushed, nothing has been published, and `.deb` installation on a clean machine is untested.
3. **A durable job queue and one long-lived rclone** (each operation still starts its own rclone and signs in again). A crash mid-store is recovered by Check, not resumed.
4. **Rename and delete** inside the pool (they need an item identity separate from its path, and a journal for the operation).
5. **Desktop gaps**: selecting several files to download at once, keyboard navigation and screen-reader support (unverified in the pinned gpui), first-run onboarding, translations.
6. **Security gaps**: secrets are not zeroized; the keyring's locked, denied and missing cases are not tested; no structured log.
7. **Packaging gaps**: no AppImage (not testable here), no Flatpak (the sandbox needs the file browser and keyring access adapted), x86_64 and glibc 2.39 or newer only, no signatures beyond SHA-256 (and attestation on a public repository).
8. **Turso is a beta database.** Export/import is the way out; an importer into another database is not built.

### Deferred beyond the first release

`parity` (erasure coding), chunking of huge files, watch folders, mounting, automatic rebalancing, named pools, other providers, automatic duplicate cleanup, desktop/CLI IPC, full translations, ARM64, Flatpak.

## 6. Dependencies

**System**: `rclone` (minimum version pinned after testing the mega backend), Secret Service (gnome-keyring/kwallet), gpui libs: `libxkbcommon`, `wayland`, `libxcb`, Vulkan loader, `fontconfig`, `freetype`. Dev: `gh`, `docker`.

**Rust, core/rclone/cli** (versions resolved with `cargo add`/`cargo info` at scaffold time and frozen in `Cargo.lock`; shared deps in `[workspace.dependencies]`):
- `tokio` (rt, process, sync, time) — async, rcd supervisor
- `reqwest` (rustls, json; no openssl) — RC client
- `serde`, `serde_json`, `toml` — protocol and config
- `turso` (pinned exactly, pre-release) — index/journal
- `thiserror` (libs), `anyhow` (binaries only)
- `tracing`, `tracing-subscriber` — logging (not used yet)
- `clap` (derive) — CLI
- `directories` — XDG paths
- `keyring` or `oo7` — secrets
- `blake3` — local hashing (planned, not added yet)
- `notify` — watch folder (deferred)
- `uuid`, `bytesize`, `humantime`
- dev: `tempfile`, `proptest` (planner), `insta` (snapshots), `wiremock`/`axum` fake rcd, `assert_cmd`

**Desktop**: `gpui`, `gpui-component` (pinned together), `rfd` or gpui's own dialog for folder selection, `ashpd` if the XDG portal is needed.

**Website**: Topcoat (SSR), facts read from the product's own files at build time, multi-stage `Dockerfile` + `compose.yml`.

**Tooling**: `rustfmt`, `clippy`, `cargo-deny` (licenses/advisories/bans), `cargo-audit`, `cargo-machete`.

## 7. Best practices

- Pinned `rust-toolchain.toml` (1.99.1 today), `rustfmt.toml`, `[workspace.lints]` (`unsafe_code = "forbid"` in core, selective clippy `pedantic`).
- Inherited `[workspace.package]` (edition, license, rust-version).
- Typed errors in libraries; `anyhow` only at the edge. No `unwrap` outside tests.
- Synchronous core where possible; async only at the I/O boundary. Traits (`RemoteFs`, `Placement`, `Clock`) for deterministic tests.
- Profiles: `dev debug=0`, dev deps `debug=false`; `release strip=true, lto="thin", codegen-units=16`; size-tuned `[profile.website]`.
- Commits drafted with `pionus` and reviewed against the diff; dependency/toolchain bumps in separate commits with the reason.
- No secrets in logs; redact passwords/tokens in `Debug`.
- `SECURITY.md`; ToS notice in the README.

## 8. CI/CD

`.github/workflows/ci.yml`:
- `concurrency: ci-${{ github.workflow }}-${{ github.ref }}`, `cancel-in-progress: true`
- `permissions: contents: read`; `env: CARGO_TERM_COLOR=always, CARGO_INCREMENTAL=0, CARGO_NET_RETRY=10`
- `Swatinem/rust-cache@v2` pinned by SHA, same `shared-key` in every job
- `--locked` on every cargo call; **one** `cargo fmt --check`, **one** `cargo clippy --workspace --all-targets -- -D warnings`, **one** `cargo test --workspace`
- apt install of the gpui libs in the job
- Tools (`cargo-deny`, `cargo-audit`) cached by version, `cargo install --version X --locked` only on cache miss
- Third-party actions pinned by commit SHA

Other workflows:
- `audit.yml` — weekly: `cargo audit`, `cargo deny check`, report of outdated toolchain/tools (opens an issue)
- `dependabot.yml` — cargo + github-actions (weekly)
- `release.yml` — on `v*` tags, no cancel-in-progress: release builds of `cli` and `desktop`, tarball + `.deb`/AppImage, checksums, GitHub Release
- `website.yml` — Docker image build (no push on PRs)

## 9. Repository

Private `Lucas-BRT/bufonidae` on GitHub, default branch `main`, MIT `LICENSE`, README and `AGENTS.md` (including the English-only rule).

## 10. Decided

- rclone is the only transfer engine; rsync is out of scope.
- Default placement granularity: per file.
- Direct dependencies are pinned exactly (`=x.y.z`) and `Cargo.lock` is committed; Dependabot proposes bumps, each made on purpose in its own commit.
- The Linux file dialog offers files or folders, never both, so the app has its own browser.
- Flatpak is deferred; `.deb`, AppImage and tarballs ship first, with the tested rclone bundled privately.

## 11. Open questions

None at the moment.
