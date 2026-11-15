use topcoat::{
    Result,
    view::{View, component, view},
};

use crate::ui::docs::doc_section;

#[component]
pub async fn section() -> Result<impl View> {
    Ok(view! {
        doc_section(
            id: "desktop",
            title: "The desktop app",
            <ul>
                <li><strong>"Files"</strong> ": browse by folder, download a file or a folder, and drop files or folders anywhere on the window to store them. Large drops, and anything that includes your home folder, ask for confirmation."</li>
                <li><strong>"Accounts"</strong> ": space left on each account, sign in again, remove."</li>
                <li><strong>"Check"</strong> ": compares what Bufonidae remembers with what the accounts hold. It changes nothing until you press Apply fixes, and only Bufonidae's own record changes. Rebuild missing copies is a separate button."</li>
                <li><strong>"Settings"</strong> ": copies per file, with per-folder rules, and when to ask before a large upload."</li>
            </ul>
            <p>
                "Uploads and downloads can be cancelled. An upload cancelled midway leaves the file being sent unconfirmed; Check settles it."
            </p>
        )
    })
}
