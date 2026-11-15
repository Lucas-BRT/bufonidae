//! The Accounts screen: connected accounts and what each one holds.

use gpui::{Context, IntoElement, ParentElement as _, Styled as _, Window, div, px};
use gpui_component::{
    ActiveTheme as _, IconName, StyledExt as _,
    button::{Button, ButtonVariants as _},
};

use crate::app::{BufonidaeApp, centered};
use crate::format;

impl BufonidaeApp {
    pub(crate) fn accounts_screen(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let snapshot = self.snapshot().cloned().unwrap_or_default();
        let theme = cx.theme();
        let (muted, border, surface, accent_bg, accent_fg) = (
            theme.muted_foreground,
            theme.border,
            theme.popover,
            theme.accent,
            theme.accent_foreground,
        );

        let add_button = Button::new("add-account")
            .primary()
            .icon(IconName::Plus)
            .label("Add account")
            .on_click(cx.listener(|this, _, window, cx| this.open_add_account(window, cx)));

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
                    .child(div().text_2xl().font_semibold().child("Accounts"))
                    .child(
                        div()
                            .text_color(muted)
                            .child(match snapshot.accounts.len() {
                                0 => "None connected".to_owned(),
                                1 => "1 account".to_owned(),
                                n => format!("{n} accounts"),
                            }),
                    ),
            )
            .child(add_button);

        if snapshot.accounts.is_empty() {
            return div().v_flex().size_full().child(header).child(centered(
                cx,
                Some(IconName::CircleUser),
                "No accounts connected",
                Some("Connect a Mega account to start storing files.".to_owned()),
            ));
        }

        let rows = snapshot.accounts.iter().map(|account| {
            let (count, bytes) = snapshot.usage(&account.id);
            let initial = account
                .label
                .chars()
                .next()
                .map_or_else(String::new, |c| c.to_uppercase().to_string());
            div()
                .h_flex()
                .items_center()
                .gap_4()
                .p_4()
                .rounded(px(12.0))
                .bg(surface)
                .border_1()
                .border_color(border)
                .child(
                    div()
                        .size_10()
                        .rounded_full()
                        .bg(accent_bg)
                        .text_color(accent_fg)
                        .font_semibold()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(initial),
                )
                .child(
                    div()
                        .v_flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(div().font_medium().child(account.label.clone()))
                        .child(
                            div()
                                .truncate()
                                .text_sm()
                                .text_color(muted)
                                .child(account.email.clone()),
                        ),
                )
                .child(div().text_color(muted).child(format!(
                    "{} · {}",
                    format::files(count),
                    format::bytes(bytes)
                )))
        });

        div()
            .v_flex()
            .size_full()
            .child(header)
            .child(div().v_flex().gap_3().px_6().pb_6().children(rows))
    }
}
