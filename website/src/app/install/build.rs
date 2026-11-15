use topcoat::{
    Result,
    view::{View, component, view},
};

use crate::ui::{code::code_block, docs::doc_section};

#[component]
pub async fn section() -> Result<impl View> {
    Ok(view! {
        doc_section(
            id: "build",
            title: "Build",
            code_block(text: "git clone <the repository>\ncd bufonidae\ncargo build --release --locked")
            <p>
                "This makes " <code>"target/release/bufonidae"</code> " (the command line) and " <code>"target/release/bufonidae-desktop"</code> " (the desktop app)."
            </p>
            <p>
                "The desktop app and the command line share one index, and only one process can open it: close the app before using a terminal command that reads files, and the other way round."
            </p>
        )
    })
}
