//! The Files screen: stored files as folders you can open, each with a
//! Download action.

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, px, uniform_list,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, StyledExt as _,
    button::{Button, ButtonVariants as _},
};

use crate::app::{BufonidaeApp, centered};
use crate::format;
use crate::tree::{self, Row};

const ROW_HEIGHT: f32 = 52.0;

impl BufonidaeApp {
    pub(crate) fn files_screen(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let snapshot = self.snapshot().cloned().unwrap_or_default();
        let folder = self.folder.clone();
        let rows = tree::rows(&snapshot.files, &folder);
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
            let mut bar = div().h_flex().items_center().gap_1().child(
                Button::new("crumb-root")
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
                        Button::new(("crumb", i))
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

        let add = Button::new("add")
            .primary()
            .icon(IconName::Plus)
            .label("Add")
            .on_click(cx.listener(|this, _, window, cx| this.open_browser(window, cx)));

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

        if snapshot.files.is_empty() {
            return div().v_flex().size_full().child(header).child(centered(
                cx,
                Some(IconName::Inbox),
                "Nothing stored yet",
                Some("Drop files or folders anywhere on this window, or use Add.".to_owned()),
            ));
        }

        let accounts = snapshot.accounts;
        let list = uniform_list(
            "files",
            rows.len(),
            cx.processor(move |_this, range: std::ops::Range<usize>, _window, cx| {
                let theme = cx.theme();
                let (muted, border, hover, chip) = (
                    theme.muted_foreground,
                    theme.border,
                    theme.list_hover,
                    theme.muted,
                );
                range
                    .filter_map(|i| rows.get(i).map(|row| (i, row.clone())))
                    .map(|(i, row)| {
                        let download = {
                            let path = row.path().to_owned();
                            Button::new(("download", i))
                                .ghost()
                                .compact()
                                .icon(IconName::ArrowDown)
                                .tooltip("Download")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.start_download(path.clone(), window, cx);
                                }))
                        };
                        let base = div()
                            .id(("row", i))
                            .h(px(ROW_HEIGHT))
                            .px_6()
                            .h_flex()
                            .gap_3()
                            .items_center()
                            .border_b_1()
                            .border_color(border)
                            .hover(move |s| s.bg(hover));

                        match row {
                            Row::Folder {
                                name,
                                path,
                                files,
                                bytes,
                            } => base
                                .child(Icon::new(IconName::Folder).text_color(muted))
                                .child(
                                    div()
                                        .id(("open", i))
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .truncate()
                                        .cursor_pointer()
                                        .font_medium()
                                        .child(name)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.open_folder(path.clone(), cx);
                                        })),
                                )
                                .child(div().text_color(muted).child(format!(
                                    "{} · {}",
                                    format::files(files),
                                    format::bytes(bytes)
                                )))
                                .child(download)
                                .into_any_element(),
                            Row::File {
                                name,
                                size,
                                accounts: holders,
                                ..
                            } => {
                                let labels: Vec<String> = holders
                                    .iter()
                                    .map(|id| {
                                        accounts.iter().find(|a| &a.id == id).map_or_else(
                                            || "unknown".to_owned(),
                                            |a| a.label.clone(),
                                        )
                                    })
                                    .collect();
                                base.child(Icon::new(IconName::File).text_color(muted))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .truncate()
                                            .font_medium()
                                            .child(name),
                                    )
                                    .children(labels.into_iter().map(move |label| {
                                        div()
                                            .px_2()
                                            .py_0p5()
                                            .rounded(px(6.0))
                                            .bg(chip)
                                            .text_sm()
                                            .text_color(muted)
                                            .child(label)
                                    }))
                                    .child(
                                        div()
                                            .w(px(84.0))
                                            .text_right()
                                            .text_color(muted)
                                            .child(format::bytes(size)),
                                    )
                                    .child(download)
                                    .into_any_element()
                            }
                        }
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .flex_1();

        div()
            .v_flex()
            .size_full()
            .child(header)
            .child(div().h_px().bg(border))
            .child(list)
    }
}
