//! The Files screen: stored files as folders you can open, each with a
//! Download action.

use crate::interaction::{ControlFeedback as _, button, checkbox};

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, Window, div,
    px, uniform_list,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, StyledExt as _,
    button::ButtonVariants as _,
    input::Input,
    menu::{ContextMenuExt as _, PopupMenuItem},
};

use crate::app::{BufonidaeApp, centered};
use crate::format;
use crate::tree::{self, Row};

use bufonidae_core::config::{FileFilter, FileSort};

impl BufonidaeApp {
    pub(crate) fn files_screen(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let snapshot = self.snapshot().cloned().unwrap_or_default();
        let folder = self.folder.clone();
        let options = crate::preferences::current(cx);
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
        let mut filters = div().h_flex().gap_1();
        for (id, label, filter) in [
            ("all", "All", FileFilter::All),
            ("files-only", "Files", FileFilter::Files),
            ("folders-only", "Folders", FileFilter::Folders),
        ] {
            filters = filters.child(
                button(id)
                    .ghost()
                    .label(label)
                    .selected(self.file_filter == filter)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.file_filter = filter;
                        cx.notify();
                    })),
            );
        }
        let sort_label = match self.file_sort {
            FileSort::Name => "Name A–Z",
            FileSort::NameDescending => "Name Z–A",
            FileSort::Size => "Size ↑",
            FileSort::SizeDescending => "Size ↓",
        };
        let toolbar=div().v_flex().px_6().pb_3().gap_2()
            .child(div().h_flex().gap_2().child(div().flex_1().min_w(px(0.0)).child(Input::new(&self.search)))
                .child(button("cycle-sort").label(sort_label).tooltip("Change sort order").on_click(cx.listener(|this,_,_,cx|{this.file_sort=match this.file_sort {FileSort::Name=>FileSort::NameDescending,FileSort::NameDescending=>FileSort::Size,FileSort::Size=>FileSort::SizeDescending,FileSort::SizeDescending=>FileSort::Name};cx.notify();}))))
            .child(div().h_flex().flex_wrap().gap_2().child(filters)
                .child(button("select-visible").label(if all_visible {"Deselect visible"}else{"Select visible"}).disabled_feedback(visible_paths.is_empty()).on_click(cx.listener(move|this,_,_,cx|{for path in &visible_paths {if all_visible {this.selected_files.remove(path);}else{this.selected_files.insert(path.clone());}}cx.notify();})))
                .child(button("clear-selection").label(format!("Clear ({selected_count})")).disabled_feedback(selected_count==0).on_click(cx.listener(|this,_,_,cx|{this.selected_files.clear();cx.notify();})))
                .child(button("download-selected").primary().label(format!("Download selected ({selected_count})")).disabled_feedback(selected_count==0).on_click(cx.listener(|this,_,window,cx|this.download_selected(window,cx)))))
            .children(options.inline_help.then(||div().text_sm().text_color(cx.theme().muted_foreground).child("Search and filters affect this folder. Hidden selections are kept. Multiple selections retain virtual paths when downloaded.")));
        let theme = cx.theme();
        let (muted, border) = (theme.muted_foreground, theme.border);

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

        let needs_account = snapshot.accounts.is_empty();
        let add = button("add")
            .tooltip(crate::preferences::hint(
                cx,
                "Add files or connect an account",
                "Ctrl+O",
                false,
            ))
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
            let danger = cx.theme().danger;
            div()
                .h_flex()
                .items_center()
                .gap_3()
                .mx_6()
                .mb_3()
                .p_3()
                .rounded(px(10.0))
                .border_1()
                .border_color(danger)
                .child(
                    Icon::new(IconName::TriangleAlert)
                        .size_4()
                        .text_color(danger)
                        .flex_none(),
                )
                .child(div().flex_1().child(format!(
                    "{} {} an unfinished upload. Check your files to settle {}.",
                    snapshot.pending,
                    if snapshot.pending == 1 {
                        "copy is"
                    } else {
                        "copies are"
                    },
                    if snapshot.pending == 1 { "it" } else { "them" },
                )))
                .child(
                    button("settle-pending")
                        .label("Check files")
                        .on_click(cx.listener(|this, _, _, cx| this.go_check(cx))),
                )
        });

        if snapshot.files.is_empty() {
            return div()
                .v_flex()
                .size_full()
                .child(header)
                .children(notice)
                .child(centered(
                    cx,
                    Some(IconName::Inbox),
                    if needs_account { "Connect your first Mega account" } else { "Nothing stored yet" },
                    Some(if needs_account && !options.first_run_help {
                        "Connect an account to add files.".to_owned()
                    } else if needs_account {
                        "Connect accounts you already own. Passwords are kept in your Linux keyring, so unlock it first. After connecting, add files and choose how many copies to keep in Settings. Bufonidae is not a backup; keep your own copies and respect Mega's quota limits.".to_owned()
                    } else {
                        "Drop files or folders anywhere on this window, or use Add.".to_owned()
                    }),
                ));
        }

        let context_menus = options.context_menus;
        let list = uniform_list(
            "files",
            rows.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                range
                    .filter_map(|i| rows.get(i).cloned().map(|row| (i, row)))
                    .map(|(i, row)| {
                        let path = row.path().to_owned();
                        let selected = this.selected_files.contains(&path);
                        let toggle_path = path.clone();
                        let download_path = path.clone();
                        let open_path = path.clone();
                        let base = div()
                            .id(("file-row", i))
                            .w_full()
                            .h(px(row_height))
                            .px_6()
                            .h_flex()
                            .gap_2()
                            .items_center()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .hover(|style| style.bg(cx.theme().list_hover))
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
                                div().flex_1().min_w(px(0.0)).overflow_hidden().child(
                                    checkbox(gpui::SharedString::from(format!("select-{path}")))
                                        .w_full()
                                        .label(row.name().to_owned())
                                        .checked(selected)
                                        .on_click(cx.listener(move |this, _: &bool, _, cx| {
                                            if !this.selected_files.remove(&toggle_path) {
                                                this.selected_files.insert(toggle_path.clone());
                                            }
                                            cx.notify();
                                        })),
                                ),
                            )
                            .children(row.is_folder().then(|| {
                                button(("open", i))
                                    .ghost()
                                    .label("Open")
                                    .tooltip(format!("Open {}", row.name()))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.open_folder(open_path.clone(), cx)
                                    }))
                            }))
                            .child(
                                div()
                                    .w(px(84.0))
                                    .text_right()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format::bytes(row.bytes())),
                            )
                            .child(button(("download", i)).ghost().label("Download").on_click(
                                cx.listener(move |this, _, window, cx| {
                                    this.start_download(download_path.clone(), window, cx)
                                }),
                            ));
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
        .w_full()
        .flex_1()
        .min_h(px(0.0));

        div()
            .v_flex()
            .size_full()
            .child(header)
            .child(toolbar)
            .children(empty_results.then(|| {
                div()
                    .px_6()
                    .py_3()
                    .text_color(muted)
                    .child("No entries match this search and filter.")
            }))
            .children(notice)
            .child(div().h_px().bg(border))
            .child(list)
    }
}
