use topcoat::{
    Result,
    view::{View, component, view},
};

use crate::ui::{code::code_block, docs::doc_section};

#[component]
pub async fn section() -> Result<impl View> {
    Ok(view! {
        doc_section(
            id: "restore",
            title: "Restore into a clean profile",
            code_block(text: "bufonidae import backup.json\nbufonidae account reauth <label>\nbufonidae reconcile")
            <p>
                "Import only works on an empty index and an empty account list, and checks the whole file before changing anything. Accounts come back with their old ids, so the restored index still points at them, but without passwords: sign each one in with " <code>"account reauth"</code> ". Then " <code>"reconcile"</code> " compares the restored index with what the accounts really hold."
            </p>
        )
    })
}
