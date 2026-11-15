use topcoat::{
    Result,
    view::{View, component, view},
};

use crate::ui::{code::code_block, docs::doc_section};

#[component]
pub async fn section() -> Result<impl View> {
    Ok(view! {
        doc_section(
            id: "accounts",
            title: "Connect accounts",
            code_block(text: "bufonidae account add personal --email you@example.com\nbufonidae account list")
            <p>
                "The password and the two-factor code are asked on the terminal, never taken as arguments, and the password goes to the system keyring. The account is signed in before anything is saved, so a wrong password stores nothing."
            </p>
            <p>
                "The desktop app does the same from its Accounts screen, and also shows how much space each account has."
            </p>
            <p class="text-muted-foreground">
                "Connect only accounts you own. Bufonidae never creates accounts, and using it to get around Mega's limits may get accounts suspended. The same Mega account cannot be connected twice."
            </p>
        )
    })
}
