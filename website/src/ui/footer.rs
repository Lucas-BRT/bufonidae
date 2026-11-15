use topcoat::{
    Result,
    view::{View, attributes, component, view},
};

use super::container::container;
use crate::{
    components::badge::{BadgeVariant, badge},
    facts::{LICENSE, VERSION},
};

#[component]
pub async fn site_footer() -> Result<impl View> {
    Ok(view! {
        <footer class="border-t border-border py-6 text-sm text-muted-foreground print:hidden">
            container(
                attrs: attributes! { class="flex flex-wrap items-center justify-between gap-x-6 gap-y-3" },
                <span class="font-bold text-foreground">"Bufonidae " (VERSION)</span>
                <span class="flex flex-wrap items-center gap-3">
                    badge(variant: BadgeVariant::Outline, (LICENSE) " license")
                </span>
            )
        </footer>
    })
}
