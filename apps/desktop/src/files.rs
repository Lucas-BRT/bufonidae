//! The Files screen: everything stored, with where each file lives.

use gpui::{
    Context, IntoElement, ParentElement as _, Styled as _, Window, div, prelude::*, px,
    uniform_list,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, StyledExt as _,
    button::{Button, ButtonVariants as _},
};

use crate::app::{BufonidaeApp, centered};
use crate::backend::Snapshot;
use crate::format;

const ROW_HEIGHT: f32 = 52.0;

impl BufonidaeApp {
    pub(crate) fn files_screen(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let snapshot = self.snapshot().cloned().unwrap_or_default();
        let theme = cx.theme();
        let (muted, border) = (theme.muted_foreground, theme.border);

        let add_files = Button::new("add-files")
            .primary()
            .icon(IconName::Plus)
            .label("Add files")
            .on_click(cx.listener(|this, _, window, cx| this.pick_files(window, cx)));

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
                    .child(div().text_2xl().font_semibold().child("Files"))
                    .child(div().text_color(muted).child(format!(
                        "{} · {}",
                        format::files(snapshot.files.len()),
                        format::bytes(snapshot.total_bytes())
                    ))),
            )
            .child(add_files);

        if snapshot.files.is_empty() {
            return div().v_flex().size_full().child(header).child(centered(
                cx,
                Some(IconName::Inbox),
                "Nothing stored yet",
                Some("Drop files or folders anywhere on this window, or use Add files.".to_owned()),
            ));
        }

        let Snapshot { files, accounts } = snapshot;
        let list = uniform_list(
            "files",
            files.len(),
            cx.processor(move |_this, range: std::ops::Range<usize>, _window, cx| {
                let theme = cx.theme();
                let (muted, border, hover, chip) = (
                    theme.muted_foreground,
                    theme.border,
                    theme.list_hover,
                    theme.muted,
                );
                range
                    .map(|i| {
                        let file = &files[i];
                        let (folder, name) = format::split_path(&file.path);
                        let holders: Vec<String> = file
                            .accounts
                            .iter()
                            .map(|id| {
                                accounts
                                    .iter()
                                    .find(|a| &a.id == id)
                                    .map_or_else(|| "unknown".to_owned(), |a| a.label.clone())
                            })
                            .collect();
                        div()
                            .id(("file", i))
                            .h(px(ROW_HEIGHT))
                            .px_6()
                            .h_flex()
                            .gap_3()
                            .items_center()
                            .border_b_1()
                            .border_color(border)
                            .hover(move |s| s.bg(hover))
                            .child(Icon::new(IconName::File).text_color(muted))
                            .child(
                                div()
                                    .v_flex()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .child(div().truncate().font_medium().child(name.to_owned()))
                                    .child(div().truncate().text_sm().text_color(muted).child(
                                        if folder.is_empty() {
                                            "/".to_owned()
                                        } else {
                                            folder.to_owned()
                                        },
                                    )),
                            )
                            .children(holders.into_iter().map(move |label| {
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
                                    .child(format::bytes(file.size)),
                            )
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
