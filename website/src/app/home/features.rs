use topcoat::{
    Result,
    view::{Child, View, attributes, component, view},
};

use crate::{
    components::card::{card, card_content, card_header, card_title},
    ui::container::container,
};

/// One thing it does.
#[component]
async fn feature(title: &str, #[default] child: Child<'_>) -> Result<impl View> {
    Ok(view! {
        card(
            card_header(card_title((title)))
            card_content(<p class="mb-0 text-sm text-muted-foreground">(child)</p>)
        )
    })
}

#[component]
pub async fn section() -> Result<impl View> {
    Ok(view! {
        container(
            attrs: attributes! { class="py-12" },
            <h2>"What it does"</h2>
            <div class="mt-6 grid gap-4 md:grid-cols-2">
                feature(
                    title: "Adds up the room on your accounts",
                    "Each file goes whole to the account with the most free space, so the free space of all accounts counts together. Files are not cut into pieces: they stay ordinary files on Mega."
                )
                feature(
                    title: "Copies on different accounts",
                    "Keep one copy, or mirror with two or more on different accounts, as a default and per folder. A file that cannot get every copy is not stored at all, rather than quietly left less protected."
                )
                feature(
                    title: "Checks what it stores",
                    "It records a content hash when it stores a file and checks every download and every rebuilt copy against it. It never overwrites a local file or a different file on Mega."
                )
                feature(
                    title: "Keeps passwords out of files",
                    "Passwords and sessions live in the system keyring. The backup it exports holds no secret."
                )
            </div>
        )
    })
}
