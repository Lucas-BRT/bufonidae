//! The Accounts screen: each connected account, how full it is, and the
//! actions on it (sign in again, remove).

use crate::interaction::{ControlFeedback as _, button};

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, StyledExt as _, button::ButtonVariants as _,
};

use crate::app::{BufonidaeApp, Health, centered};
use crate::backend::HealthStatus;
use crate::format;
use crate::ui::{self, Tone};

/// At or above these shares used, an account is almost full / nearly full.
const ALMOST_FULL_PERCENT: f32 = 90.0;
const FULL_PERCENT: f32 = 95.0;

impl BufonidaeApp {
    pub(crate) fn accounts_screen(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let snapshot = self.snapshot().cloned().unwrap_or_default();
        let theme = cx.theme();
        let (muted, danger, border, surface) = (
            theme.muted_foreground,
            theme.danger,
            theme.border,
            theme.popover,
        );

        let checking = matches!(self.health, Health::Checking);
        let add_button = button("add-account")
            .primary()
            .icon(IconName::Plus)
            .label("Add account")
            .on_click(cx.listener(|this, _, window, cx| this.open_add_account(window, cx)));
        let check_button = button("check-accounts")
            .icon(IconName::LoaderCircle)
            .label(if checking { "Checking…" } else { "Check" })
            .loading(checking && crate::motion::enabled(cx))
            .disabled_feedback(checking)
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
                ui::notice(
                    Tone::Danger,
                    format!("Could not check the accounts: {message}"),
                    None,
                    cx,
                )
                .mx_6()
                .mb_3(),
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
                    Some(space_line(*total, *used, *free, (muted, danger), (i, cx)))
                }
                (_, Some(HealthStatus::Unreachable(why))) => Some(
                    div()
                        .h_flex()
                        .items_start()
                        .gap_2()
                        .text_sm()
                        .text_color(danger)
                        .child(Icon::new(IconName::TriangleAlert).size_4().flex_none())
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .child(format!("Could not be reached: {why}")),
                        )
                        .into_any_element(),
                ),
                _ => None,
            };

            let (label, email, id) = (
                account.label.clone(),
                account.email.clone(),
                account.id.clone(),
            );
            let status = match (&self.health, self.health.of(&account.id)) {
                (Health::Checking, _) => Some(ui::chip(
                    Tone::Neutral,
                    Some(IconName::Loader),
                    "Checking",
                    cx,
                )),
                (_, Some(HealthStatus::Ok { total, used, free })) => Some(
                    if used_percent(*total, *used, *free).is_some_and(|p| p >= ALMOST_FULL_PERCENT)
                    {
                        ui::chip(Tone::Warning, None, "Almost full", cx)
                    } else {
                        ui::chip(Tone::Success, None, "Connected", cx)
                    },
                ),
                (_, Some(HealthStatus::Unreachable(_))) => {
                    Some(ui::chip(Tone::Danger, None, "Unreachable", cx))
                }
                _ => None,
            };
            let actions = div()
                .h_flex()
                .flex_none()
                .gap_4()
                .child(
                    button(("reauth", i))
                        .ghost()
                        .compact()
                        .label("Sign in again")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_reauth(label.clone(), email.clone(), window, cx);
                        })),
                )
                .child(
                    button(("remove", i))
                        .ghost()
                        .compact()
                        .text_color(danger)
                        .label("Remove")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_remove(&id, window, cx)
                        })),
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
                        .min_w(px(0.0))
                        .items_start()
                        .gap_4()
                        .child(
                            div()
                                .size_10()
                                .flex_none()
                                .rounded_full()
                                .bg(avatar_color(&account.id))
                                .text_color(gpui::white())
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
                                        .min_w(px(0.0))
                                        .text_sm()
                                        .text_color(muted)
                                        .child(account.email.clone()),
                                )
                                .child(div().text_sm().text_color(muted).child(format!(
                                    "{} · {}",
                                    format::files(count),
                                    format::bytes(bytes)
                                ))),
                        ),
                )
                .child(
                    div()
                        .h_flex()
                        .flex_wrap()
                        .gap_2()
                        .children(status)
                        .child(actions),
                )
                .children(health_line)
        });

        div()
            .v_flex()
            .size_full()
            .child(header)
            .children(banner)
            .child(
                div()
                    .id("accounts-body")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .child(div().v_flex().gap_3().px_6().pb_6().children(rows)),
            )
    }
}

/// The share used, as a percentage, when the account reports enough to say.
fn used_percent(total: Option<u64>, used: Option<u64>, free: Option<u64>) -> Option<f32> {
    let free = free.or_else(|| total.zip(used).map(|(t, u)| t.saturating_sub(u)))?;
    let total = total.filter(|&t| t > 0)?;
    #[allow(clippy::cast_precision_loss)]
    Some(((1.0 - free as f32 / total as f32) * 100.0).clamp(0.0, 100.0))
}

/// A stable hue per account, dark enough for white initials (above 7:1).
fn avatar_color(id: &str) -> gpui::Hsla {
    let hash = id.bytes().fold(2_166_136_261_u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(16_777_619)
    });
    #[allow(clippy::cast_precision_loss)]
    gpui::hsla((hash % 360) as f32 / 360.0, 0.45, 0.30, 1.0)
}

/// A usage bar and a sentence: "12.3 GB free of 20 GB". The bar turns warning
/// at 90% and danger at 95%; the icon and words say so as well.
fn space_line(
    total: Option<u64>,
    used: Option<u64>,
    free: Option<u64>,
    (muted, danger): (gpui::Hsla, gpui::Hsla),
    (index, cx): (usize, &gpui::App),
) -> gpui::AnyElement {
    // Mega reports used and total; free may be absent, so derive it.
    let free = free.or_else(|| total.zip(used).map(|(t, u)| t.saturating_sub(u)));
    let (Some(total), Some(free), Some(percent)) = (
        total.filter(|&t| t > 0),
        free,
        used_percent(total, used, free),
    ) else {
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
    let almost_full = percent >= ALMOST_FULL_PERCENT;
    let bar = if percent >= FULL_PERCENT {
        Some(Tone::Danger.accent(cx))
    } else if almost_full {
        Some(Tone::Warning.accent(cx))
    } else {
        None
    };
    div()
        .v_flex()
        .gap_1p5()
        .child(crate::motion::fill(("space-fill", index), percent, bar, cx))
        .child(
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .text_sm()
                .text_color(if percent >= FULL_PERCENT {
                    danger
                } else {
                    muted
                })
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
