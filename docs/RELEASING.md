# Releasing

## Before tagging

1. `docs/PLAN.md` status matches reality, and nothing is advertised that is not
   implemented.
2. `docs/COMPATIBILITY.md` lists the exact versions that were tested, and
   `docs/QUALIFICATION.md` was run against exactly those, with the result
   recorded in its table. CI never talks to Mega, so this is the only live test.
3. CI is green on `main` (format, clippy, tests including the website's), and
   the weekly audit (`cargo audit`, `cargo deny check`) is clean, or every
   exception in `deny.toml` still has its reason.
4. Dependency, toolchain, gpui and rclone bumps went in their own commits. Do not
   upgrade storage (Turso), the UI stack (gpui) and rclone in the same release.
5. `CHANGELOG.md`: rename `[Unreleased]` to `[X.Y.Z] - date` and write the notes
   people will read. The release refuses to run without a section for the version.
6. `version` in the root `Cargo.toml` (and `Cargo.lock`) is `X.Y.Z`. The release
   refuses to run if the tag and this version differ.

## Trying the pipeline (no tag, no release)

```
gh workflow run release.yml --ref main
```

This runs the tests, the release build, the packaging and a smoke run of what was
packaged, and keeps the packages as a workflow artifact for three days. Nothing is
published and no tag is needed. Run it before a real release, and after changing
`packaging/`, `release.yml` or the toolchain.

## Releasing

```
git tag -a vX.Y.Z -m "Bufonidae X.Y.Z"
git push origin vX.Y.Z
```

The `Release` workflow re-runs the checks, builds, packages (a tarball and a
`.deb`, with the tested rclone inside), and creates a **draft pre-release** with
`SHA256SUMS`. It never publishes by itself: download the files from the draft,
install the `.deb` on a clean machine, and only then press Publish. On a public
repository it also attests the build provenance.

## What is not covered yet

- No AppImage and no Flatpak. The Flatpak sandbox needs the file browser, the
  Secret Service access and the subprocess use adapted first; an AppImage was
  not built because it could not be tested here.
- Packages are x86_64 only and built on Ubuntu 24.04, so they need glibc 2.39 or
  newer.
- Nothing is signed beyond the SHA-256 list and, when public, the attestation.
- There is no update check in the app: people reinstall.
