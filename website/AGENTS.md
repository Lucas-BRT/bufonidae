# Bufonidae website

Adds to [../AGENTS.md](../AGENTS.md).

Rust + Topcoat `=0.11.0`: server-rendered HTML, no JavaScript, one theme (warm
neutrals and a red accent, light or dark following the system, like the desktop
app). Run: `cargo run -p bufonidae-website` (`HOST=0.0.0.0` for a phone).
Release build: `cargo build --profile website -p bufonidae-website`; image:
`docker compose up -d --build` here.

## Content

- **Facts** (version, repository, licence, the Rust, rclone, gpui and Turso
  versions, the commands of the command line) are read by `build.rs` into
  `src/facts.rs` from the product's own files. Never type one into a page. If a
  source file changes shape, the build stops with a message.
- **Other copy** is handwritten: check it against `../docs/`, `../SECURITY.md`
  and the CLI; never invent a number or a command.
- Only `/` may sell; the other pages are documentation: plain titles, no
  slogans. Say what is not done as plainly as what is. The product is not
  released: never write as if it were.
- A command added to the command line needs a line in
  `src/app/install/commands.rs`, or `tests/routes.rs` fails.

## Code

- A page is `src/app/<page>.rs` with only its `#[page]`; its sections are
  components in `src/app/<page>/`, one per file.
- New page: the module, a row in `site::PAGES`, a row in `tests/routes.rs`.
- `src/ui/` holds site components; `src/components/` is Topcoat UI, copied with
  `topcoat ui add` (tracked in `components.toml`). Never add one that needs the
  JS runtime.
- Reusable markup is a `#[component]` with typed props. Classes are Tailwind
  utilities in Rust (`class="..."`, `class!(...)`), never built with `format!`.
- No icons set and no custom font: system fonts, no request leaves the site.
- `cargo fmt` always.

## Done

Tests check facts, commands and links against the product, never wording: do
not test a sentence or a heading. Any change to markup, CSS or layout-affecting
copy must also be looked at in a browser at 1280, 820 and 390 px; say which
pages and sizes you checked and which not.
