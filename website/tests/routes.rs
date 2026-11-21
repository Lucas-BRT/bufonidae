#![allow(clippy::unwrap_used)]
//! The site, tested through its router. These check facts, links and
//! structure against the product, never wording: do not test a sentence or a
//! heading.

use bufonidae_website::router;
use topcoat::router::{Body, to_bytes};

const PAGES: [&str; 4] = ["/", "/install", "/recovery", "/security"];
const DOCS: [&str; 3] = ["/install", "/recovery", "/security"];

async fn get(path: &str) -> (u16, String, String) {
    let request = http::Request::builder()
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let response = router().handle(request).await;
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(http::header::CONTENT_TYPE)
        .map(|value| value.to_str().unwrap().to_owned())
        .unwrap_or_default();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        content_type,
        String::from_utf8(bytes.to_vec()).unwrap(),
    )
}

async fn page(path: &str) -> String {
    let (status, content_type, body) = get(path).await;
    assert_eq!(status, 200, "{path}");
    assert!(
        content_type.starts_with("text/html"),
        "{path}: {content_type}"
    );
    body
}

/// The text of the product files the site takes its facts from, read again
/// here so that the test does not trust `build.rs`.
fn product(file: &str) -> String {
    std::fs::read_to_string(format!("{}/../{file}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[tokio::test]
async fn pages_are_accessible_documents() {
    for path in PAGES {
        let body = page(path).await;
        for needle in [
            "<html lang=\"en\">",
            "href=\"#main\"",
            "<main id=\"main\"",
            "<h1",
        ] {
            assert!(body.contains(needle), "{path} lacks {needle:?}");
        }
        assert_eq!(body.matches("<h1").count(), 1, "{path}: one h1");
        assert!(!body.contains("<script"), "{path}: no script");
        assert!(!body.contains("http://"), "{path}: no insecure link");
    }
}

#[tokio::test]
async fn every_doc_page_lists_all_the_docs() {
    for path in DOCS {
        let body = page(path).await;
        assert!(body.contains("aria-label=\"Documentation\""), "{path}");
        assert_eq!(body.matches(" aria-current=\"page\"").count(), 1, "{path}");
        assert!(
            body.contains(&format!("href=\"{path}\" aria-current=\"page\"")),
            "{path}"
        );
        for target in DOCS {
            assert!(
                body.contains(&format!("href=\"{target}\"")),
                "{path} does not link to {target}"
            );
        }
    }
}

#[tokio::test]
async fn every_internal_link_resolves() {
    for path in PAGES {
        let body = page(path).await;
        for part in body.split("href=\"").skip(1) {
            let target = part.split('"').next().unwrap();
            if target.starts_with('/') && !target.starts_with("//") {
                let route = target.split('#').next().unwrap();
                let (status, _, _) = get(route).await;
                assert_eq!(status, 200, "{path} links to {target}");
            }
        }
    }
}

#[tokio::test]
async fn the_facts_come_from_the_product() {
    let workspace = product("Cargo.toml");
    let toolchain = product("rust-toolchain.toml");
    let rclone = product("crates/bufonidae-rclone/src/lib.rs");

    let value = |text: &str, key: &str| -> String {
        text.lines()
            .find_map(|line| {
                let rest = line
                    .trim()
                    .strip_prefix(key)?
                    .trim_start()
                    .strip_prefix('=')?;
                Some(rest.trim().trim_matches('"').to_owned())
            })
            .unwrap_or_else(|| panic!("no {key}"))
    };
    let version = value(&workspace, "version");
    let rust = value(&toolchain, "channel");

    // The tested rclone version, read from the code that enforces it.
    let block = rclone
        .split("pub const TESTED_VERSION")
        .nth(1)
        .unwrap()
        .split("};")
        .next()
        .unwrap()
        .to_owned();
    let n = |field: &str| -> String {
        block
            .lines()
            .find_map(|l| {
                l.trim()
                    .strip_prefix(field)?
                    .trim()
                    .strip_suffix(',')
                    .map(str::to_owned)
            })
            .unwrap()
    };
    let rclone_version = format!("{}.{}.{}", n("major:"), n("minor:"), n("patch:"));

    let home = page("/").await;
    assert!(home.contains(&version), "home lacks the version {version}");
    let install = page("/install").await;
    assert!(
        install.contains(&rclone_version),
        "install lacks rclone {rclone_version}"
    );
    assert!(install.contains(&rust), "install lacks Rust {rust}");
    for pin in ["gpui", "turso"] {
        let line = workspace
            .lines()
            .find(|l| l.trim_start().starts_with(&format!("{pin} =")))
            .unwrap();
        let exact = line
            .split("version = \"=")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        assert!(install.contains(exact), "install lacks {pin} {exact}");
    }
}

#[tokio::test]
async fn every_command_of_the_cli_is_documented() {
    let cli = product("crates/bufonidae-cli/src/main.rs");
    let body = cli
        .split("enum Command {")
        .nth(1)
        .unwrap()
        .split("\n}")
        .next()
        .unwrap()
        .to_owned();
    let commands: Vec<String> = body
        .lines()
        .filter(|l| l.starts_with("    ") && !l.starts_with("     "))
        .filter_map(|l| {
            let name: String = l
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
    assert!(commands.len() >= 8, "found only {commands:?}");

    let install = page("/install").await;
    assert!(
        !install.contains("(undocumented)"),
        "a command has no description"
    );
    for command in commands {
        assert!(
            install.contains(&format!("bufonidae {command}")),
            "the install page does not list `bufonidae {command}`"
        );
    }
}

#[tokio::test]
async fn an_unknown_page_answers_404_inside_the_layout() {
    let (status, content_type, body) = get("/no-such-page").await;
    assert_eq!(status, 404);
    assert!(content_type.starts_with("text/html"));
    assert!(
        body.contains("<main id=\"main\""),
        "the 404 skips the layout"
    );
}

#[tokio::test]
async fn the_stylesheet_and_the_mark_are_served() {
    let (status, content_type, css) = get("/style.css").await;
    assert_eq!(status, 200);
    assert!(content_type.starts_with("text/css"));
    assert!(
        css.contains("--primary"),
        "the theme tokens are in the stylesheet"
    );
    let (status, content_type, svg) = get("/logo.svg").await;
    assert_eq!(status, 200);
    assert_eq!(content_type, "image/svg+xml");
    assert!(svg.contains("<svg"));
}
