//! The Files screen: stored files as folders you can open, each with a
//! Download action.

use crate::interaction::{ControlFeedback as _, button, checkbox};
use crate::ui::{self, Tone};

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, ScrollStrategy, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, px, uniform_list,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, StyledExt as _,
    button::ButtonVariants as _,
    input::Input,
    menu::{ContextMenuExt as _, PopupMenuItem},
    tooltip::Tooltip,
};

use crate::app::{BufonidaeApp, centered};
use crate::format;
use crate::tree::{self, Row};

use bufonidae_core::config::{FileFilter, FileSort};

/// Width of the "Open" slot, kept for files too so sizes stay aligned.
const OPEN_SLOT: f32 = 64.0;
const TYPE_SLOT: f32 = 72.0;
const SIZE_SLOT: f32 = 100.0;
const ACTION_SLOT: f32 = 36.0;
/// Height of the column-head row, the same with or without a selection.
const LIST_HEAD: f32 = 44.0;

impl BufonidaeApp {
    pub(crate) fn clear_search_and_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search
            .update(cx, |field, cx| field.set_value("", window, cx));
        self.file_filter = FileFilter::All;
        self.active_row = None;
        self.search.update(cx, |field, cx| field.focus(window, cx));
        cx.notify();
    }

    fn toggle_selected(&mut self, path: &str) {
        if !self.selected_files.remove(path) {
            self.selected_files.insert(path.to_owned());
        }
    }

    pub(crate) fn files_screen(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let snapshot = self.snapshot().cloned().unwrap_or_default();
        let folder = self.folder.clone();
        let options = crate::preferences::current(cx);
        let needs_account = snapshot.accounts.is_empty();
        if needs_account && snapshot.files.is_empty() {
            let muted = cx.theme().muted_foreground;
            return div()
                .v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .p_6()
                .child(Icon::new(IconName::CircleUser).size_8().text_color(muted))
                .child(
                    div()
                        .text_2xl()
                        .font_semibold()
                        .text_center()
                        .child("Connect your first Mega account"),
                )
                .children(options.first_run_help.then(|| {
                    div()
                        .max_w(px(420.0))
                        .text_center()
                        .text_color(muted)
                        .child("Use a Mega account you already own. Unlock your system keyring before signing in.")
                }))
                .child(
                    button("connect-first-account")
                        .primary()
                        .label("Connect account")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_add_account(window, cx);
                        })),
                );
        }
        let row_height = if options.compact { 40.0 } else { 52.0 };
        let query = self.search.read(cx).value().to_string();
        let rows = tree::visible_rows(
            &snapshot.files,
            &folder,
            &query,
            self.file_sort,
            self.file_filter,
            &options,
        );
        let visible_paths = rows
            .iter()
            .map(|row| row.path().to_owned())
            .collect::<Vec<_>>();
        let empty_results = rows.is_empty();
        let selected_count = self.selected_files.len();
        let all_visible = !rows.is_empty()
            && visible_paths
                .iter()
                .all(|path| self.selected_files.contains(path));
        if let Some(active) = self.active_row
            && active >= rows.len()
        {
            self.active_row = rows.len().checked_sub(1);
        }
        let active_row = self.active_row;

        let theme = cx.theme();
        let (muted, border, foreground, background) = (
            theme.muted_foreground,
            theme.border,
            theme.foreground,
            theme.background,
        );

        // Filter: a segmented control whose selected segment is inverted, so
        // the state survives grayscale.
        let mut filters = div()
            .h_flex()
            .flex_none()
            .gap_1()
            .p_1()
            .rounded(px(10.0))
            .border_1()
            .border_color(cx.theme().input);
        for (id, label, filter) in [
            ("all", "All", FileFilter::All),
            ("files-only", "Files", FileFilter::Files),
            ("folders-only", "Folders", FileFilter::Folders),
        ] {
            let chosen = self.file_filter == filter;
            let segment = button(id)
                .ghost()
                .compact()
                .label(label)
                .selected(chosen)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.file_filter = filter;
                    this.active_row = None;
                    cx.notify();
                }));
            filters = filters.child(if chosen {
                segment.bg(foreground).text_color(background)
            } else {
                segment
            });
        }

        let toolbar = div()
            .v_flex()
            .px_6()
            .pb_3()
            .gap_2()
            .child(
                div()
                    .h_flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(200.0))
                            .child(Input::new(&self.search)),
                    )
                    .child(filters),
            )
            .children(options.inline_help.then(|| {
                div().text_sm().text_color(muted).child(
                    "Search and filters affect this folder. Hidden selections are kept. \
                     Multiple selections retain virtual paths when downloaded.",
                )
            }));

        // Selection actions replace the column heads in the same row, so the
        // list never moves when something is selected.
        if (selected_count > 0) != self.selection_bar_visible {
            self.selection_bar_visible = selected_count > 0;
            if self.selection_bar_visible {
                self.selection_bar_generation += 1;
            }
        }
        let selection_actions = (selected_count > 0).then(|| {
            crate::motion::fade(
                div()
                    .h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .font_semibold()
                            .child(format!("{selected_count} selected")),
                    )
                    .child(button("clear-selection").compact().label("Clear").on_click(
                        cx.listener(|this, _, _, cx| {
                            this.selected_files.clear();
                            cx.notify();
                        }),
                    ))
                    .child(
                        button("download-selected")
                            .compact()
                            .primary()
                            .label(format!("Download selected ({selected_count})"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.download_selected(window, cx)
                            })),
                    ),
                ("selection-bar", self.selection_bar_generation),
                false,
                0.0,
                cx,
            )
        });

        // What the current folder holds, counting everything below it.
        let (file_count, byte_count) = rows.iter().fold((0, 0), |(n, b), row| match row {
            Row::Folder { files, bytes, .. } => (n + files, b + bytes),
            Row::File { size, .. } => (n + 1, b + size),
        });

        let crumbs = tree::crumbs(&folder);
        let title = crumbs
            .last()
            .map_or_else(|| "Files".to_owned(), |(name, _)| name.clone());

        let breadcrumb = (!crumbs.is_empty()).then(|| {
            let mut bar = div().h_flex().flex_wrap().items_center().gap_1().child(
                button("crumb-root")
                    .ghost()
                    .compact()
                    .label("Files")
                    .on_click(cx.listener(|this, _, _, cx| this.open_folder(String::new(), cx))),
            );
            for (i, (name, path)) in crumbs.iter().enumerate() {
                bar = bar.child(Icon::new(IconName::ChevronRight).size_4().text_color(muted));
                if i + 1 == crumbs.len() {
                    bar = bar.child(div().px_2().text_sm().child(name.clone()));
                } else {
                    let path = path.clone();
                    bar = bar.child(
                        button(("crumb", i))
                            .ghost()
                            .compact()
                            .label(name.clone())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_folder(path.clone(), cx);
                            })),
                    );
                }
            }
            bar
        });

        let add = button("add")
            .primary()
            .icon(IconName::Plus)
            .label(if needs_account {
                "Connect account"
            } else {
                "Add"
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                if needs_account {
                    this.open_add_account(window, cx);
                } else {
                    this.open_browser(window, cx);
                }
            }));
        let add = if needs_account {
            add
        } else {
            add.tooltip(crate::preferences::hint(cx, "Add files", "Ctrl+O", false))
        };

        let header = div()
            .h_flex()
            .items_start()
            .justify_between()
            .gap_4()
            .px_6()
            .pt_6()
            .pb_4()
            .child(
                div()
                    .v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap_1()
                    .children(breadcrumb)
                    .child(div().text_2xl().font_semibold().truncate().child(title))
                    .child(div().text_color(muted).child(format!(
                        "{} · {}",
                        format::files(file_count),
                        format::bytes(byte_count)
                    ))),
            )
            .child(add);

        // A store that was interrupted leaves copies unconfirmed; saying so
        // here means the user hears about it without having to look.
        let notice = (snapshot.pending > 0).then(|| {
            ui::notice(
                Tone::Warning,
                format!(
                    "{} {} an unfinished upload. Check your files to settle {}.",
                    snapshot.pending,
                    if snapshot.pending == 1 {
                        "copy is"
                    } else {
                        "copies are"
                    },
                    if snapshot.pending == 1 { "it" } else { "them" },
                ),
                Some(
                    button("settle-pending")
                        .label("Check files")
                        .on_click(cx.listener(|this, _, _, cx| this.go_check(cx)))
                        .into_any_element(),
                ),
                cx,
            )
            .mx_6()
            .mb_3()
        });

        if snapshot.files.is_empty() {
            let body = centered(
                cx,
                Some(IconName::Inbox),
                "Nothing stored yet",
                Some("Drop files or folders anywhere on this window, or use Add.".to_owned()),
            );
            let action = button("empty-primary")
                .primary()
                .icon(IconName::Plus)
                .label("Add files")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.open_browser(window, cx);
                }));
            return div()
                .v_flex()
                .size_full()
                .child(header)
                .children(notice)
                .child(
                    body.child(action).child(
                        div()
                            .mt_2()
                            .w_full()
                            .max_w(px(420.0))
                            .py_6()
                            .rounded(px(12.0))
                            .border_2()
                            .border_dashed()
                            .border_color(border)
                            .text_color(muted)
                            .h_flex()
                            .justify_center()
                            .child("Drop files or folders here"),
                    ),
                );
        }

        // Column heads: sorting lives here, so the current order is visible.
        let sort = self.file_sort;
        let by_name = matches!(sort, FileSort::Name | FileSort::NameDescending);
        let sort_head = |id: &'static str, label: &'static str, active: bool, descending: bool| {
            let spoken = if active {
                format!(
                    "Sort by {}, currently {}",
                    label.to_lowercase(),
                    if descending {
                        "descending"
                    } else {
                        "ascending"
                    }
                )
            } else {
                format!("Sort by {}", label.to_lowercase())
            };
            button(id)
                .ghost()
                .compact()
                .label(label)
                .tooltip(spoken)
                .children(active.then(|| {
                    Icon::new(if descending {
                        IconName::ArrowDown
                    } else {
                        IconName::ArrowUp
                    })
                    .size_3p5()
                }))
        };
        let name_head = sort_head(
            "sort-name",
            "Name",
            by_name,
            sort == FileSort::NameDescending,
        )
        .on_click(cx.listener(|this, _, _, cx| {
            this.file_sort = if this.file_sort == FileSort::Name {
                FileSort::NameDescending
            } else {
                FileSort::Name
            };
            cx.notify();
        }));
        let size_head = sort_head(
            "sort-size",
            "Size",
            !by_name,
            sort == FileSort::SizeDescending,
        )
        .on_click(cx.listener(|this, _, _, cx| {
            this.file_sort = if this.file_sort == FileSort::Size {
                FileSort::SizeDescending
            } else {
                FileSort::Size
            };
            cx.notify();
        }));
        let select_visible = visible_paths.clone();
        let list_head = div()
            .h_flex()
            .w_full()
            .flex_none()
            .h(px(LIST_HEAD))
            .px_6()
            .gap_2()
            .items_center()
            .border_t_1()
            .border_color(border)
            .child(
                checkbox("select-visible")
                    .checked(all_visible)
                    .tooltip(move |window, cx| {
                        Tooltip::new(if all_visible {
                            "Deselect everything shown"
                        } else {
                            "Select everything shown"
                        })
                        .build(window, cx)
                    })
                    .disabled_feedback(select_visible.is_empty())
                    .on_click(cx.listener(move |this, _: &bool, _, cx| {
                        for path in &select_visible {
                            if all_visible {
                                this.selected_files.remove(path);
                            } else {
                                this.selected_files.insert(path.clone());
                            }
                        }
                        cx.notify();
                    })),
            )
            .child(if let Some(actions) = selection_actions {
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(actions)
                    .into_any_element()
            } else {
                div()
                    .h_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap_2()
                    .items_center()
                    .child(div().w(px(16.0)).flex_none())
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .h_flex()
                            .ml(px(-10.0))
                            .child(name_head),
                    )
                    .child(div().w(px(OPEN_SLOT)).flex_none())
                    .child(
                        div()
                            .w(px(TYPE_SLOT))
                            .flex_none()
                            .text_sm()
                            .text_color(muted)
                            .child("Type"),
                    )
                    .child(
                        div()
                            .w(px(SIZE_SLOT))
                            .flex_none()
                            .h_flex()
                            .justify_end()
                            .mr(px(-8.0))
                            .child(size_head),
                    )
                    .child(div().w(px(ACTION_SLOT)).flex_none())
                    .into_any_element()
            });

        let context_menus = options.context_menus;
        let reduced = !crate::motion::enabled(cx);
        let keyed = rows
            .iter()
            .map(|row| (row.path().to_owned(), row.is_folder()))
            .collect::<Vec<_>>();
        let list = uniform_list(
            "files",
            rows.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                range
                    .filter_map(|i| rows.get(i).cloned().map(|row| (i, row)))
                    .map(|(i, row)| {
                        let path = row.path().to_owned();
                        let selected = this.selected_files.contains(&path);
                        let focused = active_row == Some(i);
                        let toggle_path = path.clone();
                        let download_path = path.clone();
                        let open_path = path.clone();
                        let click_path = path.clone();
                        let prefix = format!("{path}/");
                        let arrived = this
                            .fresh
                            .iter()
                            .any(|f| *f == path || f.starts_with(&prefix));
                        let flash = (arrived && !reduced)
                            .then(|| {
                                crate::motion::flash(
                                    SharedString::from(format!(
                                        "fresh-{}-{i}",
                                        this.fresh_generation()
                                    )),
                                    cx,
                                )
                            })
                            .flatten();
                        let is_folder = row.is_folder();
                        let name = row.name().to_owned();
                        let tip = name.clone();
                        let theme = cx.theme();
                        let (list_active, list_hover, accent, ring) = (
                            theme.list_active,
                            theme.list_hover,
                            theme.list_active_border,
                            theme.ring,
                        );
                        let base = div()
                            .id(("file-row", i))
                            .relative()
                            .w_full()
                            .h(px(row_height))
                            .px_6()
                            .h_flex()
                            .gap_2()
                            .items_center()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .when_selected(selected, list_active)
                            .hover(move |style| {
                                style.bg(if selected { list_active } else { list_hover })
                            })
                            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                                this.active_row = Some(i);
                                if event.click_count() >= 2 {
                                    if is_folder {
                                        this.open_folder(click_path.clone(), cx);
                                        return;
                                    }
                                } else {
                                    this.toggle_selected(&click_path);
                                }
                                cx.notify();
                            }))
                            .children(flash)
                            // The accent bar marks selection by shape, not hue.
                            .children(selected.then(|| {
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_0()
                                    .bottom_0()
                                    .w(px(3.0))
                                    .bg(accent)
                            }))
                            .children(
                                (focused && this.files_focus.is_focused(_window)).then(|| {
                                    div().absolute().inset_0().border_2().border_color(ring)
                                }),
                            )
                            .child(
                                checkbox(SharedString::from(format!("select-{path}")))
                                    .checked(selected)
                                    .on_click(cx.listener(move |this, _: &bool, _, cx| {
                                        this.toggle_selected(&toggle_path);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Icon::new(if row.is_folder() {
                                    IconName::Folder
                                } else {
                                    IconName::File
                                })
                                .flex_none()
                                .text_color(cx.theme().muted_foreground),
                            )
                            .child(
                                div()
                                    .id(("file-name", i))
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .h_flex()
                                    .gap_2()
                                    .items_center()
                                    .tooltip(move |window, cx| {
                                        let tip = tip.clone();
                                        Tooltip::element(move |_, _| {
                                            div().w(px(300.0)).min_w(px(0.0)).child(tip.clone())
                                        })
                                        .build(window, cx)
                                    })
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .child(format::middle_truncate(&name, 34)),
                                    )
                                    .children(
                                        (arrived && reduced)
                                            .then(|| ui::chip(Tone::Neutral, None, "New", cx)),
                                    ),
                            )
                            .child(div().w(px(OPEN_SLOT)).flex_none().children(
                                row.is_folder().then(|| {
                                    button(("open", i))
                                        .ghost()
                                        .label("Open")
                                        .tooltip("Open folder")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.open_folder(open_path.clone(), cx)
                                        }))
                                }),
                            ))
                            .child(
                                div()
                                    .w(px(TYPE_SLOT))
                                    .flex_none()
                                    .truncate()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format::file_type(&name, is_folder)),
                            )
                            .child(
                                div()
                                    .w(px(SIZE_SLOT))
                                    .flex_none()
                                    .text_right()
                                    .whitespace_nowrap()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format::bytes(row.bytes())),
                            )
                            .child(
                                div().w(px(ACTION_SLOT)).flex_none().child(
                                    button(("download", i))
                                        .ghost()
                                        .icon(IconName::ArrowDown)
                                        .tooltip("Download")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            cx.stop_propagation();
                                            this.start_download(download_path.clone(), window, cx)
                                        })),
                                ),
                            );
                        if context_menus {
                            let app = cx.entity().downgrade();
                            base.context_menu(move |menu, _, _| {
                                let app = app.clone();
                                let download = path.clone();
                                let copy = path.clone();
                                menu.item(PopupMenuItem::new("Download").on_click(
                                    move |_, window, cx| {
                                        app.update(cx, |this, cx| {
                                            this.start_download(download.clone(), window, cx)
                                        })
                                        .ok();
                                    },
                                ))
                                .item(
                                    PopupMenuItem::new("Copy virtual path").on_click(
                                        move |_, _, cx| {
                                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                                copy.clone(),
                                            ))
                                        },
                                    ),
                                )
                            })
                            .into_any_element()
                        } else {
                            base.into_any_element()
                        }
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.files_scroll.clone())
        .w_full()
        .flex_1()
        .min_h(px(0.0));

        // Up/Down/Home/End move the highlighted row, Space selects it, Enter
        // opens a folder or starts a download.
        let last = keyed.len().saturating_sub(1);
        let keys = cx.listener(move |this, event: &KeyDownEvent, window, cx| {
            let current = this.active_row;
            let next = match event.keystroke.key.as_str() {
                "down" => Some(current.map_or(0, |i| (i + 1).min(last))),
                "up" => Some(current.map_or(0, |i| i.saturating_sub(1))),
                "home" => Some(0),
                "end" => Some(last),
                "pagedown" => Some(current.map_or(0, |i| (i + 8).min(last))),
                "pageup" => Some(current.map_or(0, |i| i.saturating_sub(8))),
                "space" | "enter" => {
                    let Some((path, folder)) = current.and_then(|i| keyed.get(i)) else {
                        return;
                    };
                    if event.keystroke.key == "space" {
                        this.toggle_selected(path);
                    } else if *folder {
                        this.open_folder(path.clone(), cx);
                    } else {
                        this.start_download(path.clone(), window, cx);
                    }
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                _ => None,
            };
            if let Some(next) = next
                && !keyed.is_empty()
            {
                this.active_row = Some(next);
                // Scroll only when the row would leave the viewport.
                let (top, capacity) = {
                    let state = this.files_scroll.0.borrow();
                    let height = state.base_handle.bounds().size.height;
                    (
                        state.base_handle.logical_scroll_top().0,
                        (((height / px(row_height)).floor() as usize).max(1)),
                    )
                };
                if next < top {
                    this.files_scroll.scroll_to_item(next, ScrollStrategy::Top);
                } else if next >= top + capacity {
                    this.files_scroll
                        .scroll_to_item(next, ScrollStrategy::Bottom);
                }
                cx.stop_propagation();
                cx.notify();
            }
        });

        let list_area: AnyElement =
            if empty_results {
                centered(
                    cx,
                    Some(IconName::Search),
                    "No entries match",
                    Some("Nothing in this folder matches the search and filter.".to_owned()),
                )
                .child(
                    button("clear-filters")
                        .label("Clear search and filters")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.clear_search_and_filters(window, cx)
                        })),
                )
                .into_any_element()
            } else {
                div()
                    .id("files-list")
                    .track_focus(&self.files_focus)
                    .on_key_down(keys)
                    .v_flex()
                    .w_full()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(list)
                    .into_any_element()
            };

        div()
            .v_flex()
            .size_full()
            .child(header)
            .child(toolbar)
            .children(notice)
            .child(list_head)
            .child(div().h_px().bg(border))
            .child(list_area)
    }
}

/// Paints a selected row's tint; a plain row is left alone.
trait SelectedTint: Sized {
    fn when_selected(self, selected: bool, tint: gpui::Hsla) -> Self;
}

impl<T: gpui::Styled> SelectedTint for T {
    fn when_selected(self, selected: bool, tint: gpui::Hsla) -> Self {
        if selected { self.bg(tint) } else { self }
    }
}
