use topcoat::{
    Result,
    view::{View, component, view},
};

use crate::{facts::REPO, ui::docs::doc_section};

#[component]
pub async fn section() -> Result<impl View> {
    Ok(view! {
        doc_section(
            id: "terms",
            title: "Mega's terms, and reporting a problem",
            <p>
                "Bufonidae only connects accounts you already own. It does not create accounts, and it must not be used to get around Mega's quota limits: doing so may get your accounts suspended."
            </p>
            <p>
                "To report a vulnerability, do not open a public issue; use GitHub's private vulnerability reporting on the " <a href=(REPO)>"repository"</a> "."
            </p>
        )
    })
}
