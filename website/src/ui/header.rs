use topcoat::{
    Result,
    context::Cx,
    router::href,
    view::{StaticClass, View, attributes, class, component, view},
};

use super::container::container;
use crate::{
    app::install,
    components::button::{ButtonSize, ButtonVariant, button_variants},
    facts::REPO,
    site,
};

/// Visually mark the documentation section in the main menu.
const CURRENT: StaticClass =
    class!("data-[section-active=true]:bg-card data-[section-active=true]:text-primary");

#[component]
pub async fn site_header(cx: &Cx) -> Result<impl View> {
    let in_docs = site::current(cx).is_some_and(|page| page.section.is_some());
    let menu = || {
        class!(
            button_variants(ButtonVariant::Ghost, ButtonSize::Sm),
            CURRENT
        )
    };

    Ok(view! {
        <header class="border-b border-border print:hidden">
            container(
                attrs: attributes! { class="flex flex-wrap items-center justify-between gap-x-6 gap-y-2 py-4" },
                <a class="flex items-center gap-2.5 text-xl font-bold tracking-tight text-foreground no-underline" href=(href!(crate::app::page))>
                    <img src="/logo.svg" alt="" class="size-8">
                    "Bufonidae"
                </a>
                <nav aria-label="Main">
                    <ul class="m-0 flex list-none items-center gap-1 p-0">
                        <li class="m-0"><a class=(menu()) href=(href!(install::page)) data-section-active=(in_docs.then_some("true"))>"Docs"</a></li>
                        <li class="m-0"><a class=(menu()) href=(REPO)>"GitHub"</a></li>
                    </ul>
                </nav>
            )
        </header>
    })
}
