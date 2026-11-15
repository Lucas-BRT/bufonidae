use topcoat::{
    Result,
    view::{View, component, view},
};

/// A block of shell commands or output, as plain text. Scrolls sideways
/// instead of wrapping, so a command is never broken across lines.
#[component]
pub async fn code_block(text: &str) -> Result<impl View> {
    Ok(view! {
        <pre class="overflow-x-auto rounded-lg border border-border bg-card p-4 text-sm" tabindex="0"><code>(text)</code></pre>
    })
}
