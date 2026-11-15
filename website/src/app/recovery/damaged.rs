use topcoat::{
    Result,
    view::{View, component, view},
};

use crate::ui::docs::doc_section;

#[component]
pub async fn section() -> Result<impl View> {
    Ok(view! {
        doc_section(
            id: "damaged",
            title: "If the index is damaged",
            <p>
                "Opening a damaged index fails with an error, and the file is left exactly as it was. Do not delete it. Move it aside (the " <code>"index.db"</code> " files in " <code>"~/.local/share/bufonidae"</code> ", including the " <code>"-wal"</code> " file), then either restore a backup, or start empty and run " <code>"bufonidae reconcile --apply"</code> " to rebuild what the accounts hold."
            </p>
            <p>
                "Rebuilding from the accounts alone loses what a listing cannot show: the record of lost files, the content hashes, and any file whose copies disagree. Your redundancy rules live in " <code>"config.toml"</code> ", not in the index."
            </p>
            <p class="text-muted-foreground">
                "Not built yet: an importer into another database. The JSON export is the portable format, so it is how data would move if Turso, a beta database, ever had to be replaced."
            </p>
        )
    })
}
