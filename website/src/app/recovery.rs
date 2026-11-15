//! Backup and recovery: export, restore, and a damaged index.

mod backup;
mod damaged;
mod restore;

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
            title: "Backup and recovery",
            lede: "The local index is data worth backing up: a listing of your accounts cannot recreate it.",
            backup::section()
            restore::section()
            damaged::section()
        )
    })
}
