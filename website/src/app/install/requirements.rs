use topcoat::{
    Result,
    view::{View, component, view},
};

use crate::{
    facts::{GPUI, RCLONE, RUST, TURSO},
    ui::docs::doc_section,
};

#[component]
pub async fn section() -> Result<impl View> {
    Ok(view! {
        doc_section(
            id: "requirements",
            title: "Requirements",
            <ul>
                <li>"Linux on x86_64 with a Secret Service for passwords, such as GNOME Keyring or KWallet. Developed on Pop!_OS 24.04 with Wayland; other setups are untested."</li>
                <li>
                    <code>"rclone"</code> " on your " <code>"PATH"</code> ", version " (RCLONE) " or newer. Bufonidae checks the version when it starts and refuses an older or pre-release build."
                </li>
                <li>"To build: Rust " (RUST) " (pinned by the repository), and for the desktop app the packages " <code>"libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libfontconfig-dev libfreetype-dev"</code> "."</li>
            </ul>
            <p class="text-muted-foreground">
                "Tested with: rclone " (RCLONE) ", Rust " (RUST) ", gpui " (GPUI) ", Turso " (TURSO) " (a beta database). Anything else is untested and should not be assumed to work."
            </p>
        )
    })
}
