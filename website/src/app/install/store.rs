use topcoat::{
    Result,
    view::{View, component, view},
};

use crate::ui::{code::code_block, docs::doc_section};

#[component]
pub async fn section() -> Result<impl View> {
    Ok(view! {
        doc_section(
            id: "store",
            title: "Store, list and download",
            code_block(text: "bufonidae put photos/ notes.txt --to Documents\nbufonidae ls Documents\nbufonidae get Documents/photos --out ~/Downloads")
            <p>
                "Files and folders go under the folder you name with " <code>"--to"</code> ". A folder keeps its own name. Symbolic links and special files are skipped and reported."
            </p>
            <p>
                "Nothing is overwritten. A file already stored with the same size is left alone; a different file under the same name is reported and not replaced. Two different local files that would get the same stored name are refused before anything is sent."
            </p>
            <p>
                "Choose how many copies each file gets with " <code>"bufonidae redundancy set mirror:2"</code> ", for everything or for one folder with " <code>"--folder"</code> ". A file that cannot get every copy is not stored."
            </p>
        )
    })
}
