//! Install and use: requirements, building, accounts and the commands.

mod accounts;
mod build;
mod commands;
mod desktop;
mod requirements;
mod store;

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
            title: "Install and use",
            lede: "Linux on x86_64. There is no installer yet: you build it from source.",
            requirements::section()
            build::section()
            accounts::section()
            store::section()
            desktop::section()
            commands::section()
        )
    })
}
