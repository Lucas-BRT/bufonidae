//! The Accounts screen: each connected account, how full it is, and the
//! actions on it (sign in again, remove).

use gpui::{Context, IntoElement, ParentElement as _, Styled as _, Window, div, px};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, StyledExt as _,
    button::{Button, ButtonVariants as _},
    progress::Progress,
};

use crate::app::{BufonidaeApp, Health, centered};
use crate::backend::HealthStatus;
use crate::format;

/// Below this share free, an account is called almost full.
const ALMOST_FULL: f32 = 0.10;

impl BufonidaeApp {
    pub(crate) fn accounts_screen(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let snapshot = self.snapshot().cloned().unwrap_or_default();
        let theme = cx.theme();
        let (muted, danger, border, surface, accent_bg, accent_fg) = (
            theme.muted_foreground,
            theme.danger,
            theme.border,
            theme.popover,
            theme.accent,
            theme.accent_foreground,
        );

        let checking = matches!(self.health, Health::Checking);
        let add_button = Button::new("add-account")
            .primary()
            .icon(IconName::Plus)
            .label("Add account")
            .on_click(cx.listener(|this, _, window, cx| this.open_add_account(window, cx)));
        let check_button = Button::new("check-accounts")
            .icon(IconName::LoaderCircle)
            .label(if checking { "Checking…" } else { "Check" })
            .loading(checking)
            .tooltip("Ask each account how much space it has")
            .on_click(cx.listener(|this, _, _, cx| this.check_health(cx)));

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
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .children((!snapshot.accounts.is_empty()).then_some(check_button))
                    .child(add_button),
            );

        if snapshot.accounts.is_empty() {
            return div().v_flex().size_full().child(header).child(centered(
                cx,
                Some(IconName::CircleUser),
                "No accounts connected",
                Some("Connect a Mega account to start storing files.".to_owned()),
            ));
        }

        let banner = match &self.health {
            Health::Failed(message) => Some(
                div()
                    .h_flex()
                    .items_start()
                    .gap_2()
                    .px_6()
                    .pb_3()
                    .text_color(danger)
                    .child(Icon::new(IconName::TriangleAlert).size_4().flex_none())
                    .child(
                        div()
                            .flex_1()
                            .child(format!("Could not check the accounts: {message}")),
                    ),
            ),
            _ => None,
        };

        let rows = snapshot.accounts.iter().enumerate().map(|(i, account)| {
            let (count, bytes) = snapshot.usage(&account.id);
            let initial = account
                .label
                .chars()
                .next()
                .map_or_else(String::new, |c| c.to_uppercase().to_string());

            // How full it is, or why we do not know.
            let health_line = match (&self.health, self.health.of(&account.id)) {
                (Health::Checking, _) => Some(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child("Checking space…")
                        .into_any_element(),
                ),
                (_, Some(HealthStatus::Ok { total, used, free })) => {
                    Some(space_line(*total, *used, *free, muted, danger))
                }
                (_, Some(HealthStatus::Unreachable(why))) => Some(
                    div()
                        .h_flex()
                        .items_start()
                        .gap_2()
                        .text_sm()
                        .text_color(danger)
                        .child(Icon::new(IconName::TriangleAlert).size_4().flex_none())
                        .child(div().flex_1().child(format!("Could not be reached: {why}")))
                        .into_any_element(),
                ),
                _ => None,
            };

            let (label, email, id) = (
                account.label.clone(),
                account.email.clone(),
                account.id.clone(),
            );
            let actions = div()
                .h_flex()
                .gap_1()
                .child(
                    Button::new(("reauth", i))
                        .ghost()
                        .compact()
                        .label("Sign in again")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_reauth(label.clone(), email.clone(), window, cx);
                        })),
                )
                .child(
                    Button::new(("remove", i))
                        .ghost()
                        .compact()
                        .label("Remove")
                        .on_click(cx.listener(move |this, _, _, cx| this.open_remove(&id, cx))),
                );

            div()
                .v_flex()
                .gap_3()
                .p_4()
                .rounded(px(12.0))
                .bg(surface)
                .border_1()
                .border_color(border)
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_4()
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
                        .child(actions),
                )
                .children(health_line)
        });

        div()
            .v_flex()
            .size_full()
            .child(header)
            .children(banner)
            .child(div().v_flex().gap_3().px_6().pb_6().children(rows))
    }
}

/// A usage bar and a sentence: "12.3 GB free of 20 GB".
fn space_line(
    total: Option<u64>,
    used: Option<u64>,
    free: Option<u64>,
    muted: gpui::Hsla,
    danger: gpui::Hsla,
) -> gpui::AnyElement {
    // Mega reports used and total; free may be absent, so derive it.
    let free = free.or_else(|| total.zip(used).map(|(t, u)| t.saturating_sub(u)));
    let (Some(total), Some(free)) = (total.filter(|&t| t > 0), free) else {
        let text = match used {
            Some(used) => format!("{} used (the total is not reported)", format::bytes(used)),
            None => "Space is not reported by this account".to_owned(),
        };
        return div()
            .text_sm()
            .text_color(muted)
            .child(text)
            .into_any_element();
    };
    #[allow(clippy::cast_precision_loss)]
    let share_free = free as f32 / total as f32;
    let used_percent = ((1.0 - share_free) * 100.0).clamp(0.0, 100.0);
    let almost_full = share_free < ALMOST_FULL;
    div()
        .v_flex()
        .gap_1p5()
        .child(Progress::new().value(used_percent))
        .child(
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .text_sm()
                .text_color(if almost_full { danger } else { muted })
                .children(
                    almost_full.then(|| Icon::new(IconName::TriangleAlert).size_4().flex_none()),
                )
                .child(format!(
                    "{} free of {}{}",
                    format::bytes(free),
                    format::bytes(total),
                    if almost_full { " · almost full" } else { "" }
                )),
        )
        .into_any_element()
}
