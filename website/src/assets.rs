//! The files the site serves besides its pages: the stylesheet and the mark.
//! Both are part of the binary.

use http::header::{CACHE_CONTROL, CONTENT_TYPE};
use topcoat::{Result, router::response::IntoResponse, router::route};

/// Built by `build.rs`: Tailwind's utilities plus `assets/style.css`.
const STYLE: &str = include_str!(concat!(env!("OUT_DIR"), "/tailwind.css"));
const LOGO: &str = include_str!("../assets/logo.svg");

/// Debug builds never cache, so each edit shows on reload.
const CACHE: &str = if cfg!(debug_assertions) {
    "no-store"
} else {
    "public, max-age=3600"
};

#[route(GET "/style.css")]
async fn style() -> Result<impl IntoResponse> {
    Ok((
        [
            (CONTENT_TYPE, "text/css; charset=utf-8"),
            (CACHE_CONTROL, CACHE),
        ],
        STYLE,
    ))
}

/// The mark: favicon and header logo.
#[route(GET "/logo.svg")]
async fn logo() -> Result<impl IntoResponse> {
    Ok((
        [(CONTENT_TYPE, "image/svg+xml"), (CACHE_CONTROL, CACHE)],
        LOGO,
    ))
}
