use topcoat::{
    Result,
    view::{View, component, view},
};

use crate::ui::docs::doc_section;

#[component]
pub async fn section() -> Result<impl View> {
    Ok(view! {
        doc_section(
            id: "limits",
            title: "What it does not protect",
            <ul>
                <li><strong>"A compromised user session."</strong> " Anything running as you can read the keyring, your files and the index. The keyring protects against a stolen disk and other users, not against malware running as you."</li>
                <li><strong>"Metadata."</strong> " The index, the account list and exports list your e-mail addresses, folder structure and file names unencrypted. Mega encrypts file contents end to end; this local metadata is not."</li>
                <li><strong>"Temporary copies."</strong> " Rebuilding a copy and downloading go through temporary files that are removed when finished or cancelled; a crash can leave one behind."</li>
                <li><strong>"Memory."</strong> " The password exists briefly in this process and in rclone's. It is cleared from the text fields when a sign-in starts but is not zeroized, and a crash dump could contain it."</li>
                <li><strong>"Old files."</strong> " Only files stored by this version have a recorded content hash, and Mega gives none, so damage is found when a file is read."</li>
            </ul>
        )
    })
}
