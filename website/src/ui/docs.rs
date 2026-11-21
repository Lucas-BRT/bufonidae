use topcoat::{
    Result,
    context::Cx,
    router::href,
    view::{Child, StaticClass, View, attributes, class, component, view},
};

use super::container::container;
use crate::{
    components::button::{ButtonSize, ButtonVariant, button_variants},
    site::{self, Section},
};

const GROUP: StaticClass = class!(
    "mb-1 hidden px-3 font-mono text-xs tracking-[0.12em] text-muted-foreground uppercase lg:block"
);
const LIST: StaticClass = class!("m-0 flex list-none flex-wrap gap-x-1 p-0 lg:mb-5 lg:block");
const LINK: StaticClass = class!(
    "block rounded-lg px-3 py-1.5 text-muted-foreground no-underline hover:text-foreground",
    "aria-[current=page]:bg-card aria-[current=page]:text-primary",
);

/// The documentation layout: the side list of the pages next to the page
/// itself. On narrow screens the list becomes a strip of links above it.
#[component]
pub async fn docs(
    cx: &Cx,
    title: &str,
    lede: &str,
    #[default] child: Child<'_>,
) -> Result<impl View> {
    Ok(view! {
        container(
            attrs: attributes! { class="grid gap-5 pt-5 pb-24 lg:grid-cols-[12rem_minmax(0,1fr)] lg:gap-14 lg:pt-14" },
            <nav class="flex flex-wrap gap-1 lg:sticky lg:top-6 lg:block lg:self-start" aria-label="Documentation">
                for section in Section::ALL {
                    <p class=(GROUP)>(section.title())</p>
                    <ul class=(LIST)>
                        for page in site::pages_in(section) {
                            let link = href!(page.target);
                            let current = link.is_current(cx);
                            <li class="m-0"><a class=(LINK) href=(link) aria-current=(current.then_some("page"))>(page.label)</a></li>
                        }
                    </ul>
                }
            </nav>
            <article class="min-w-0 max-w-[46rem]">
                <header>
                    <h1>(title)</h1>
                    <p class="mt-4 text-lg text-muted-foreground">(lede)</p>
                </header>
                (child)
                if let Some(next) = site::next(cx) {
                    <nav class="mt-14 flex justify-end" aria-label="Next page">
                        <a class=(button_variants(ButtonVariant::Outline, ButtonSize::Lg)) href=(href!(next.target))>"Next: " (next.label)</a>
                    </nav>
                }
            </article>
        )
    })
}

/// A section of a documentation page: its `h2`, named by `id`, and its
/// content.
#[component]
pub async fn doc_section(id: &str, title: &str, #[default] child: Child<'_>) -> Result<impl View> {
    Ok(view! {
        <section aria-labelledby=(id)>
            <h2 id=(id) class="mt-12 mb-4 text-2xl">(title)</h2>
            (child)
        </section>
    })
}
