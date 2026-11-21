use std::{env, fs, path::Path};

use topcoat::tailwind::BuildConfig;

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=assets/style.css");
    let config = BuildConfig::new().input("styles.css");
    // The Docker build points this at a musl Tailwind CLI; elsewhere Topcoat
    // downloads its pinned glibc one.
    let config = if env::var_os("TAILWIND_CLI").is_some() {
        config.executable_env("TAILWIND_CLI")
    } else {
        config
    };
    config
        .render()
        .expect("Tailwind failed to build styles.css");
    facts();
}

/// The files of the product that the site takes its facts from. A change to
/// one rebuilds the site; one whose shape no longer parses stops the build
/// with a message, instead of the site going quietly out of date.
const SOURCES: [&str; 4] = [
    "Cargo.toml",
    "rust-toolchain.toml",
    "crates/bufonidae-rclone/src/lib.rs",
    "crates/bufonidae-cli/src/main.rs",
];

/// Writes `facts.rs` to `OUT_DIR`.
fn facts() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let read = |name: &str| {
        println!("cargo:rerun-if-changed=../{name}");
        fs::read_to_string(root.join(name)).unwrap_or_else(|e| panic!("cannot read {name}: {e}"))
    };
    let [workspace, toolchain, rclone, cli] = SOURCES.map(read);

    let version = quoted_value(&workspace, "version").expect("Cargo.toml has no version");
    let repository = quoted_value(&workspace, "repository").expect("Cargo.toml has no repository");
    let license = quoted_value(&workspace, "license").expect("Cargo.toml has no license");
    let rust = quoted_value(&toolchain, "channel").expect("rust-toolchain.toml has no channel");
    let gpui = pinned(&workspace, "gpui").expect("Cargo.toml does not pin gpui");
    let turso = pinned(&workspace, "turso").expect("Cargo.toml does not pin turso");

    let mut out = String::new();
    out += &format!("pub const VERSION: &str = {version:?};\n");
    out += &format!("pub const REPO: &str = {repository:?};\n");
    out += &format!("pub const LICENSE: &str = {license:?};\n");
    out += &format!("pub const RUST: &str = {rust:?};\n");
    out += &format!("pub const GPUI: &str = {gpui:?};\n");
    out += &format!("pub const TURSO: &str = {turso:?};\n");
    out += &format!("pub const RCLONE: &str = {:?};\n", tested_rclone(&rclone));
    out += &format!("pub const COMMANDS: &[&str] = &{:?};\n", commands(&cli));

    let dest = Path::new(&env::var_os("OUT_DIR").expect("OUT_DIR")).join("facts.rs");
    fs::write(dest, out).expect("cannot write facts.rs");
}

/// The value of the first `key = "value"` line.
fn quoted_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let rest = line
            .trim()
            .strip_prefix(key)?
            .trim_start()
            .strip_prefix('=')?;
        let rest = rest.trim().strip_prefix('"')?;
        Some(rest.split('"').next()?.to_owned())
    })
}

/// The exact version a dependency is pinned to (`name = { version = "=1.2.3"`).
fn pinned(text: &str, name: &str) -> Option<String> {
    let line = text
        .lines()
        .find(|line| line.trim_start().starts_with(&format!("{name} =")))?;
    let after = line.split("version = \"=").nth(1)?;
    Some(after.split('"').next()?.to_owned())
}

/// `major.minor.patch` of `TESTED_VERSION` in the rclone crate.
fn tested_rclone(source: &str) -> String {
    let block = source
        .split("pub const TESTED_VERSION")
        .nth(1)
        .and_then(|rest| rest.split("};").next())
        .expect("bufonidae-rclone has no TESTED_VERSION");
    let number = |field: &str| {
        block
            .lines()
            .find_map(|line| line.trim().strip_prefix(field)?.trim().strip_suffix(','))
            .unwrap_or_else(|| panic!("TESTED_VERSION has no {field}"))
            .trim()
            .to_owned()
    };
    format!(
        "{}.{}.{}",
        number("major:"),
        number("minor:"),
        number("patch:")
    )
}

/// The subcommands of the command line, from `enum Command` (lowercase).
fn commands(source: &str) -> Vec<String> {
    let body = source
        .split("enum Command {")
        .nth(1)
        .and_then(|rest| rest.split("\n}").next())
        .expect("the CLI has no `enum Command`");
    let found: Vec<String> = body
        .lines()
        .filter(|line| line.starts_with("    ") && !line.starts_with("     "))
        .filter_map(|line| {
            let name: String = line
                .trim()
                .chars()
                .take_while(|c| c.is_alphanumeric())
                .collect();
            name.chars()
                .next()
                .filter(char::is_ascii_uppercase)
                .map(|_| name.to_lowercase())
        })
        .collect();
    assert!(!found.is_empty(), "no subcommands found in the CLI");
    found
}
