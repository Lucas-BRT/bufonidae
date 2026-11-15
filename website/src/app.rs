//! The routes. A module is a route: this one is `/`, `app::install` is
//! `/install`. A page is a folder of its sections, each a component.

mod home;
pub mod install;
pub mod recovery;
pub mod security;

use topcoat::{
    Result,
    context::Cx,
    router::{
        Router, RouterBuilderDiscoverExt, Slot, error::NotFoundError, layout, module_router,
        not_found, page,
    },
    view::{View, error_boundary, view},
};

use crate::{
    site,
    ui::{document::document, not_found::not_found as not_found_page},
};

pub fn router() -> Router {
    module_router!().discover().build()
}

#[layout]
async fn root_layout(cx: &Cx, slot: Slot<'_>) -> Result<impl View> {
    let (title, description) =
        site::current(cx).map_or(site::NOT_FOUND, |entry| (entry.title, entry.description));

    Ok(view! {
        document(
            title: title,
            description: description,
            error_boundary(
                fallback: |error| {
                    if error.downcast_ref::<NotFoundError>().is_none() {
                        return Err(error);
                    }
                    Ok(
                        view! { not_found_page() },
                    )
                },
                // Any other error is rethrown for the router to answer.
                (slot)
            )
        )
    })
}

#[page]
pub async fn page() -> Result<impl View> {
    Ok(view! {
        home::hero::section()
        home::features::section()
        home::limits::section()
    })
}

// A URL matching no route would get a bare 404 that skips the layout.
not_found!();
