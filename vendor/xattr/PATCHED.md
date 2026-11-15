# Patched copy of xattr 0.2.3

Why: `zed-async-tar` (via `gpui_http_client`, via `gpui`) depends on
`xattr = "0.2"`, which references `libc::ENOATTR`; recent `libc` versions no
longer export it on Linux, so the crate does not compile.

Change: `src/sys/mod.rs` uses `libc::ENODATA` (same value) on Linux/Android.

Remove this directory and the `[patch.crates-io]` entry in the root
`Cargo.toml` once gpui depends on an `xattr` that builds against current `libc`.
Upstream sources: https://crates.io/crates/xattr/0.2.3 (MIT OR Apache-2.0).
