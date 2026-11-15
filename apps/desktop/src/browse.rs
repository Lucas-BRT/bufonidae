//! The file browser: tick any mix of files and folders, in one place. The
//! system file dialog on Linux offers either files or folders, never both, so
//! the app has its own.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    Context, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window, div, px, uniform_list,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, StyledExt as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
};

use crate::backend::{Backend, DirEntryInfo, home_dir};

const ROW_HEIGHT: f32 = 36.0;

pub enum BrowseEvent {
    Cancelled,
    Chosen(Vec<PathBuf>),
}

pub struct BrowseForm {
    backend: Arc<Backend>,
    dir: PathBuf,
    entries: Vec<DirEntryInfo>,
    selected: BTreeSet<PathBuf>,
    show_hidden: bool,
    loading: bool,
    error: Option<String>,
}

impl EventEmitter<BrowseEvent> for BrowseForm {}

impl BrowseForm {
    pub fn new(backend: Arc<Backend>, cx: &mut Context<Self>) -> Self {
        let start = home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let mut form = Self {
            backend,
            dir: start.clone(),
            entries: Vec::new(),
            selected: BTreeSet::new(),
            show_hidden: false,
            loading: true,
            error: None,
        };
        form.open(start, cx);
        form
    }

    /// Shows `dir`. On failure the current folder stays and the error shows.
    fn open(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.loading = true;
        cx.notify();
        let job = self.backend.list_dir(dir.clone());
        cx.spawn(async move |this, cx| {
            let result = match job.await {
                Ok(result) => result,
                Err(join_error) => Err(join_error.to_string()),
            };
            this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(entries) => {
                        this.dir = dir;
                        this.entries = entries;
                        this.error = None;
                    }
                    Err(message) => this.error = Some(message),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn visible(&self) -> Vec<DirEntryInfo> {
        self.entries
            .iter()
            .filter(|e| self.show_hidden || !e.name.starts_with('.'))
            .cloned()
            .collect()
    }

    fn toggle(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.selected.remove(&path) {
            self.selected.insert(path);
        }
        cx.notify();
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(BrowseEvent::Cancelled);
    }

    fn choose(&mut self, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            cx.emit(BrowseEvent::Chosen(self.selected.iter().cloned().collect()));
        }
    }
}

impl Render for BrowseForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, danger, border, surface) = (
            theme.muted_foreground,
            theme.danger,
            theme.border,
            theme.popover,
        );
        let visible = self.visible();
        let count = visible.len();
        let parent = self.dir.parent().map(std::path::Path::to_path_buf);
        let selected_count = self.selected.len();

        let list = uniform_list(
            "browse-entries",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let visible = this.visible();
                let (muted, hover) = (cx.theme().muted_foreground, cx.theme().list_hover);
                range
                    .filter_map(|i| visible.get(i).cloned().map(|e| (i, e)))
                    .map(|(i, entry)| {
                        let path = entry.path.clone();
                        let chosen = this.selected.contains(&entry.path);
                        let icon = if entry.is_dir {
                            IconName::Folder
                        } else {
                            IconName::File
                        };
                        let row = div()
                            .id(("entry", i))
                            .h(px(ROW_HEIGHT))
                            .px_3()
                            .h_flex()
                            .gap_3()
                            .items_center()
                            .hover(move |s| s.bg(hover));

                        if entry.is_symlink {
                            // Shown, not selectable: links are never followed.
                            return row
                                .child(Icon::new(icon).text_color(muted))
                                .child(
                                    div()
                                        .flex_1()
                                        .truncate()
                                        .text_color(muted)
                                        .child(format!("{} (link, not stored)", entry.name)),
                                )
                                .into_any_element();
                        }
                        let checkbox =
                            Checkbox::new(("pick", i))
                                .checked(chosen)
                                .on_click(cx.listener({
                                    let path = path.clone();
                                    move |this, _: &bool, _, cx| this.toggle(path.clone(), cx)
                                }));
                        let name = div()
                            .id(("name", i))
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .cursor_pointer()
                            .child(entry.name.clone())
                            .on_click(cx.listener({
                                let (path, is_dir) = (path.clone(), entry.is_dir);
                                move |this, _, _, cx| {
                                    // A folder opens; a file toggles, so a click
                                    // anywhere on a file row picks it.
                                    if is_dir {
                                        this.open(path.clone(), cx);
                                    } else {
                                        this.toggle(path.clone(), cx);
                                    }
                                }
                            }));
                        row.child(checkbox)
                            .child(Icon::new(icon).text_color(muted))
                            .child(name)
                            .children(
                                entry
                                    .is_dir
                                    .then(|| Icon::new(IconName::ChevronRight).text_color(muted)),
                            )
                            .into_any_element()
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .h(px(300.0));

        let body = if let Some(message) = &self.error {
            div()
                .h(px(300.0))
                .h_flex()
                .items_start()
                .gap_2()
                .p_3()
                .text_sm()
                .text_color(danger)
                .child(Icon::new(IconName::TriangleAlert).size_4().flex_none())
                .child(div().flex_1().child(message.clone()))
                .into_any_element()
        } else if count == 0 {
            div()
                .h(px(300.0))
                .flex()
                .items_center()
                .justify_center()
                .text_color(muted)
                .child(if self.loading {
                    "Loading…"
                } else {
                    "This folder is empty."
                })
                .into_any_element()
        } else {
            list.into_any_element()
        };

        div()
            .v_flex()
            .w(px(560.0))
            .gap_3()
            .p_6()
            .rounded(px(14.0))
            .bg(surface)
            .border_1()
            .border_color(border)
            .shadow_lg()
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xl()
                            .font_semibold()
                            .child("Add files or folders"),
                    )
                    .child(
                        div().text_color(muted).child(
                            "Tick what you want to store. Click a folder's name to open it.",
                        ),
                    ),
            )
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("up")
                            .icon(IconName::ArrowUp)
                            .label("Up")
                            .disabled(parent.is_none())
                            .on_click(cx.listener({
                                let parent = parent.clone();
                                move |this, _, _, cx| {
                                    if let Some(parent) = &parent {
                                        this.open(parent.clone(), cx);
                                    }
                                }
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_sm()
                            .text_color(muted)
                            .child(self.dir.display().to_string()),
                    )
                    .child(
                        Checkbox::new("hidden")
                            .label("Show hidden")
                            .checked(self.show_hidden)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.show_hidden = *checked;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(border)
                    .overflow_hidden()
                    .child(body),
            )
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_color(muted).child(match selected_count {
                        0 => "Nothing selected".to_owned(),
                        n => format!("{n} selected"),
                    }))
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(
                                Button::new("cancel")
                                    .label("Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                            )
                            .child(
                                Button::new("choose")
                                    .primary()
                                    .label("Add selected")
                                    .disabled(selected_count == 0)
                                    .on_click(cx.listener(|this, _, _, cx| this.choose(cx))),
                            ),
                    ),
            )
    }
}
