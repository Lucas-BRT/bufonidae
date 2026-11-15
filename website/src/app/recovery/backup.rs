use topcoat::{
    Result,
    view::{View, component, view},
};

use crate::ui::{code::code_block, docs::doc_section};

#[component]
pub async fn section() -> Result<impl View> {
    Ok(view! {
        doc_section(
            id: "backup",
            title: "Back up",
            <p>
                "Bufonidae keeps an index of which account holds which file, your redundancy settings, content hashes and the files it has lost. Export it:"
            </p>
            code_block(text: "bufonidae export backup.json")
            <p>
                "The file is plain JSON, written privately and atomically, and the command refuses to replace an existing file unless you pass " <code>"--force"</code> ". It holds no password, session or master key, but it does list your account e-mail addresses and folder structure, so keep it somewhere private."
            </p>
            <p class="text-muted-foreground">
                "Export with the desktop app closed: only one process can open the index. There is no scheduled backup."
            </p>
        )
    })
}
