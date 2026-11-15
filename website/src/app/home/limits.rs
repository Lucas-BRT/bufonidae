use topcoat::{
    Result,
    view::{View, attributes, component, view},
};

use crate::{
    facts::{RCLONE, RUST},
    ui::container::container,
};

#[component]
pub async fn section() -> Result<impl View> {
    Ok(view! {
        container(
            attrs: attributes! { class="pt-4 pb-24" },
            <h2>"What it is not"</h2>
            <ul class="mt-5 text-muted-foreground">
                <li>"Not a mounted drive and not a two-way sync. It stores, lists, downloads and repairs."</li>
                <li>"Not a backup of its own. Mirroring survives losing an account, not Mega failing for everyone."</li>
                <li>"Integrity is checked against the hash recorded when a file was stored. Mega gives no hash, so damage shows when a file is read, not by scanning, and files stored before hashes existed have none."</li>
                <li>"Not for getting around Mega's quotas. Connect accounts you own; evading limits can get accounts suspended."</li>
            </ul>
            <p class="mt-6 text-muted-foreground">
                "Needs Linux, a Secret Service (GNOME Keyring or KWallet) and rclone " (RCLONE) ". Building it needs Rust " (RUST) "."
            </p>
        )
    })
}
