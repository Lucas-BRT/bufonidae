use topcoat::{
    Result,
    view::{View, component, view},
};

use crate::ui::docs::doc_section;

#[component]
pub async fn section() -> Result<impl View> {
    Ok(view! {
        doc_section(
            id: "protects",
            title: "What it protects",
            <ul>
                <li><strong>"Passwords and sessions"</strong> " live in the system keyring, never in a file, an export, the index or a log. If the keyring is locked or missing the operation fails; there is no plaintext fallback."</li>
                <li><strong>"The rclone daemon"</strong> " listens only on the loopback address on a random port, with credentials made fresh for each run and passed in the environment. It never reads your " <code>"rclone.conf"</code> ", ignores your own " <code>"RCLONE_*"</code> " settings, and uses no proxy."</li>
                <li><strong>"Local files"</strong> " are created private (mode 0600 in a 0700 folder), written atomically and durably, and never replace a symbolic link."</li>
                <li><strong>"Paths"</strong> " from the index, a backup or a remote listing are checked before use. A download never replaces an existing file and never follows a link in its destination folder."</li>
                <li><strong>"What is shown"</strong> " is escaped, so a file name cannot inject terminal control sequences into command output."</li>
            </ul>
        )
    })
}
