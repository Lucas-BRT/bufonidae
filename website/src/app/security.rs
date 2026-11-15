//! Security and limits: what is protected, and what is not.

mod limits;
mod protects;
mod terms;

use topcoat::{
    Result,
    router::page,
    view::{View, view},
};

use crate::ui::docs::docs;

#[page]
pub async fn page() -> Result<impl View> {
    Ok(view! {
        docs(
            title: "Security and limits",
            lede: "What Bufonidae protects, and where its protection ends.",
            protects::section()
            limits::section()
            terms::section()
        )
    })
}
