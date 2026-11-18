use topcoat::{
    Result,
    view::{View, component, view},
};

use crate::{facts::COMMANDS, ui::docs::doc_section};

/// What each command does. A command added to the command line without a line
/// here is shown as undocumented, and a test fails.
fn describe(command: &str) -> &'static str {
    match command {
        "put" => "Store files and folders, spreading files across the accounts.",
        "ls" => "List what is stored, from the local index (no network).",
        "get" => "Download a file or a folder.",
        "reconcile" => {
            "Compare the index with the accounts; a dry run unless --apply, and only the index changes."
        }
        "repair" => "Rebuild missing copies of files that have fewer than the redundancy asks for.",
        "verify" => {
            "Download every indexed replica and verify its recorded BLAKE3 hash; files without a hash are reported separately."
        }
        "redundancy" => "Show or set how many copies each file gets (show, set, unset).",
        "export" => "Write a portable backup (no passwords) to a JSON file.",
        "import" => "Restore a backup into an empty profile.",
        "account" => "Add, list, sign in again (reauth) or remove an account.",
        _ => "(undocumented)",
    }
}

#[component]
pub async fn section() -> Result<impl View> {
    Ok(view! {
        doc_section(
            id: "commands",
            title: "Commands",
            <dl class="m-0 grid gap-x-6 gap-y-3 sm:grid-cols-[10rem_minmax(0,1fr)]">
                for command in COMMANDS {
                    <dt class="font-medium"><code>"bufonidae " (*command)</code></dt>
                    <dd class="m-0 text-muted-foreground">(describe(command))</dd>
                }
            </dl>
            <p class="mt-6 text-muted-foreground">
                "Run any command with " <code>"--help"</code> " for its options."
            </p>
        )
    })
}
