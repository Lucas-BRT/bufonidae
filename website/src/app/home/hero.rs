use topcoat::{
    Result,
    router::href,
    view::{View, attributes, component, view},
};

use crate::{
    app::install,
    components::{
        badge::{BadgeVariant, badge},
        button::{ButtonSize, ButtonVariant, button_variants},
    },
    facts::{REPO, VERSION},
    ui::container::container,
};

#[component]
pub async fn section() -> Result<impl View> {
    Ok(view! {
        container(
            attrs: attributes! { class="pt-14 pb-12" },
            <p class="mb-5">
                badge(variant: BadgeVariant::Outline, "Development software, version " (VERSION))
            </p>
            <h1 class="max-w-[22ch]">"One folder over several Mega accounts"</h1>
            <p class="mt-5 text-lg text-muted-foreground">
                "Bufonidae stores each file whole on the Mega account with the most room, and can keep copies on different accounts. It uses rclone to talk to Mega. A Linux desktop app and a command line."
            </p>
            <p class="text-muted-foreground">
                "It is not released yet. It has worked against a real Mega account for signing in, uploading, downloading and cancelling, but it has not had a full qualification pass. Keep your own backups."
            </p>
            <div class="mt-8 flex flex-wrap gap-3">
                <a class=(button_variants(ButtonVariant::Primary, ButtonSize::Lg)) href=(href!(install::page))>"Install and use"</a>
                <a class=(button_variants(ButtonVariant::Outline, ButtonSize::Lg)) href=(REPO)>"Source on GitHub"</a>
            </div>
        )
    })
}
