//! The Check screen: compares what the app remembers with what the accounts
//! really hold, applies the safe fixes on request, and rebuilds missing
//! copies. Nothing changes until the user presses a button that says so.

use crate::interaction::{ControlFeedback as _, button};

use bufonidae_core::repair;
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, StyledExt as _, button::ButtonVariants as _,
};

use crate::app::BufonidaeApp;
use crate::backend::CheckOutcome;
use crate::format;

/// Lines listed per problem before "and N more".
const LISTED: usize = 6;

pub(crate) enum CheckState {
    Idle,
    /// What it is doing, in words.
    Running(&'static str),
    Result {
        outcome: Box<CheckOutcome>,
        /// Whether the fixes in the report were written (otherwise: a dry run).
        applied: bool,
        /// Present right after a repair.
        repaired: Option<Box<repair::Report>>,
    },
    Failed(String),
    Verified(Box<bufonidae_core::verify::Report>),
    VerificationCancelled,
}

fn clean(text: &str) -> String {
    bufonidae_rclone::escape_control(text)
}

/// A titled group of lines, capped.
fn group(
    title: String,
    icon: IconName,
    color: gpui::Hsla,
    muted: gpui::Hsla,
    lines: Vec<String>,
    note: Option<&'static str>,
) -> gpui::AnyElement {
    let more = lines.len().saturating_sub(LISTED);
    div()
        .v_flex()
        .gap_1p5()
        .child(
            div()
                .h_flex()
                .items_start()
                .gap_2()
                .text_color(color)
                .font_medium()
                .child(Icon::new(icon).size_4().flex_none())
                .child(div().flex_1().child(title)),
        )
        .children(note.map(|n| div().pl_6().text_sm().text_color(muted).child(n)))
        .children(lines.into_iter().take(LISTED).map(|line| {
            div()
                .pl_6()
                .truncate()
                .text_sm()
                .text_color(muted)
                .child(line)
        }))
        .children((more > 0).then(|| {
            div()
                .pl_6()
                .text_sm()
                .text_color(muted)
                .child(format!("and {more} more"))
        }))
        .into_any_element()
}

impl BufonidaeApp {
    pub(crate) fn check_screen(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, danger, success, border, surface) = (
            theme.muted_foreground,
            theme.danger,
            theme.success,
            theme.border,
            theme.popover,
        );
        let running = matches!(self.check, CheckState::Running(_));

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
                    .child(div().text_2xl().font_semibold().child("Check files"))
                    .child(div().text_color(muted).child(
                        "Compares what Bufonidae remembers with what your accounts really hold.",
                    )),
            )
            .child(
                button("check-now")
                    .primary()
                    .icon(IconName::CircleCheck)
                    .label("Check now")
                    .loading(running && crate::motion::enabled(cx))
                    .disabled_feedback(running)
                    .on_click(cx.listener(|this, _, _, cx| this.run_check(false, cx))),
            );

        let body = match &self.check {
            CheckState::Idle => div()
                .v_flex()
                .gap_2()
                .p_6()
                .text_color(muted)
                .child("Nothing is changed by a check. If it finds something it can fix, you decide.")
                .child("Run it after an interrupted store, after removing or adding an account, or if a download was refused.")
                .into_any_element(),
            CheckState::Running(what) => div()
                .p_6()
                .text_color(muted)
                .child(crate::motion::activity(if self.verification_stopping { "Cancelling verification — waiting for it to stop" } else { what }, cx))
                .into_any_element(),
            CheckState::Failed(message) => div()
                .h_flex()
                .items_start()
                .gap_2()
                .p_6()
                .text_color(danger)
                .child(Icon::new(IconName::TriangleAlert).size_4().flex_none())
                .child(div().flex_1().child(message.clone()))
                .into_any_element(),
            CheckState::VerificationCancelled => div().p_6().child("Content verification cancelled. No remote files or trusted hashes were changed.").into_any_element(),
            CheckState::Verified(report) => div().v_flex().p_6().gap_4()
                .child(format!("{} copies verified · {} · {} without a hash reference · {} failures", report.verified, format::bytes(report.bytes), report.without_hash.len(), report.failed.len()))
                .child(group("Copies that could not be verified".into(), IconName::TriangleAlert, danger, muted,
                    report.failed.iter().map(|(path, account, reason)| format!("{} · {}: {}", clean(path), clean(account), clean(reason))).collect(), None))
                .child(group("Files without a trusted hash".into(), IconName::Info, muted, muted,
                    report.without_hash.iter().map(|path| clean(path)).collect(), Some("Matching size alone does not establish content integrity.")))
                .into_any_element(),
            CheckState::Result {
                outcome,
                applied,
                repaired,
            } => {
                let r = &outcome.report;
                let gaps = &outcome.gaps;
                let lost: Vec<&repair::Gap> = gaps.iter().filter(|g| g.holders.is_empty()).collect();
                let under: Vec<&repair::Gap> = gaps.iter().filter(|g| !g.holders.is_empty()).collect();
                let all_good = r.fixable() == 0 && r.unresolved() == 0 && gaps.is_empty();
                let mut sections: Vec<gpui::AnyElement> = Vec::new();

                if let Some(done) = repaired {
                    sections.push(group(
                        format!("{} rebuilt", plural_copies(done.repaired.len())),
                        IconName::CircleCheck,
                        success,
                        muted,
                        done.repaired
                            .iter()
                            .map(|(p, a)| format!("{} → {}", clean(p), clean(a)))
                            .collect(),
                        None,
                    ));
                    for (path, reason) in &done.failed {
                        sections.push(group(
                            format!("Could not rebuild {}", clean(path)),
                            IconName::CircleX,
                            danger,
                            muted,
                            vec![clean(reason)],
                            None,
                        ));
                    }
                }
                if all_good {
                    sections.push(group(
                        format!("Everything matches ({} checked)", format::files(r.consistent)),
                        IconName::CircleCheck,
                        success,
                        muted,
                        Vec::new(),
                        None,
                    ));
                }
                if r.fixable() > 0 {
                    let verb = |done: &'static str, would: &'static str| {
                        if *applied { done } else { would }
                    };
                    let mut lines = Vec::new();
                    for (p, a) in &r.confirmed {
                        lines.push(format!("{} {} ({})", verb("Confirmed", "Would confirm"), clean(p), clean(a)));
                    }
                    for (p, a) in &r.dropped {
                        lines.push(format!("{} {} ({}): not there any more", verb("Forgot", "Would forget"), clean(p), clean(a)));
                    }
                    for (p, a, size) in &r.imported {
                        lines.push(format!("{} {} ({}, {})", verb("Added", "Would add"), clean(p), clean(a), format::bytes(*size)));
                    }
                    sections.push(group(
                        if *applied {
                            format!("{} fixed in the index", r.fixable())
                        } else {
                            format!("{} can be fixed in the index", r.fixable())
                        },
                        IconName::Info,
                        muted,
                        muted,
                        lines,
                        Some("Only Bufonidae's own record changes. Nothing is changed or deleted in your Mega accounts."),
                    ));
                }
                for (path, a, want, got) in &r.size_mismatch {
                    sections.push(group(
                        format!("{} ({}) has a different size", clean(path), clean(a)),
                        IconName::TriangleAlert,
                        danger,
                        muted,
                        vec![format!("remembered {}, the account has {}", format::bytes(*want), format::bytes(*got))],
                        None,
                    ));
                }
                if !r.partial.is_empty() {
                    sections.push(group(
                        format!("{} look partly uploaded", format::files(r.partial.len())),
                        IconName::TriangleAlert,
                        danger,
                        muted,
                        r.partial.iter().map(|(p, a, want, got)| format!("{} ({}): expected {}, found {}", clean(p), clean(a), format::bytes(*want), format::bytes(*got))).collect(),
                        Some("Left in place. Store the file again or remove the partial copy on mega.nz."),
                    ));
                }
                if !r.conflicts.is_empty() {
                    sections.push(group(
                        format!("{} disagree between accounts", format::files(r.conflicts.len())),
                        IconName::TriangleAlert,
                        danger,
                        muted,
                        r.conflicts.iter().map(|(p, a, want, got)| format!("{} ({}): others are {}, this is {}", clean(p), clean(a), format::bytes(*want), format::bytes(*got))).collect(),
                        Some("Not added, because the right one is unknown. Remove the wrong copy on mega.nz."),
                    ));
                }
                if !r.duplicates.is_empty() {
                    sections.push(group(
                        format!("{} exist twice in one account", format::files(r.duplicates.len())),
                        IconName::TriangleAlert,
                        danger,
                        muted,
                        r.duplicates.iter().map(|(p, a)| format!("{} ({})", clean(p), clean(a))).collect(),
                        Some("Left alone, because which one is yours cannot be told. Remove one on mega.nz."),
                    ));
                }
                for (label, why) in &r.unreachable {
                    sections.push(group(
                        format!("Account {} was not used", clean(label)),
                        IconName::TriangleAlert,
                        danger,
                        muted,
                        vec![clean(why)],
                        Some("Its files were not touched."),
                    ));
                }
                if !lost.is_empty() {
                    sections.push(group(
                        format!("{} lost", format::files(lost.len())),
                        IconName::CircleX,
                        danger,
                        muted,
                        lost.iter().map(|g| clean(&g.path)).collect(),
                        Some("Every copy is gone from the accounts that are connected."),
                    ));
                }
                if !under.is_empty() {
                    sections.push(group(
                        format!("{} with fewer copies than wanted", format::files(under.len())),
                        IconName::TriangleAlert,
                        danger,
                        muted,
                        under.iter().map(|g| format!("{}: {} of {}", clean(&g.path), g.holders.len(), g.wanted)).collect(),
                        None,
                    ));
                }

                let mut actions = div().h_flex().gap_2();
                if r.fixable() > 0 && !*applied {
                    actions = actions.child(
                        button("apply-fixes")
                            .primary()
                            .label("Apply fixes")
                            .disabled_feedback(running)
                            .on_click(cx.listener(|this, _, _, cx| this.run_check(true, cx))),
                    );
                }
                if !under.is_empty() {
                    actions = actions.child(
                        button("repair")
                            .primary()
                            .label("Rebuild missing copies")
                            .disabled_feedback(running)
                            .on_click(cx.listener(|this, _, _, cx| this.run_repair(cx))),
                    );
                }
                let has_actions = (r.fixable() > 0 && !*applied) || !under.is_empty();

                div()
                    .v_flex()
                    .gap_4()
                    .p_6()
                    .child(
                        div()
                            .v_flex()
                            .gap_4()
                            .p_5()
                            .rounded(px(12.0))
                            .bg(surface)
                            .border_1()
                            .border_color(border)
                            .children(sections),
                    )
                    .children(has_actions.then_some(actions))
                    .into_any_element()
            }
        };

        div()
            .v_flex()
            .size_full()
            .child(header)
            .child(div().h_px().bg(border))
            .child({
                let bytes = self.snapshot().map(|s| s.files.iter().filter(|f| f.blake3.is_some()).fold(0u64, |total, file| total.saturating_add(file.size.saturating_mul(file.accounts.len() as u64)))).unwrap_or(0);
                div().h_flex().flex_wrap().gap_3().px_6().py_3()
                    .child(div().text_sm().text_color(muted).child(format!("Deep check downloads up to {} of indexed copies. Files without a hash are reported separately.", format::bytes(bytes))))
                    .child(button("deep-check").label("Download and verify contents").disabled_feedback(running)
                        .on_click(cx.listener(|this, _, _, cx| this.run_deep_check(cx))))
                    .children((self.verification_cancel.is_some() || self.verification_stopping).then(|| button("cancel-verification").label(if self.verification_stopping { "Cancelling…" } else { "Cancel verification" }).disabled_feedback(self.verification_stopping)
                        .on_click(cx.listener(|this, _, _, cx| { if let Some(cancel) = this.verification_cancel.take() { let _ = cancel.send(()); this.verification_stopping = true; cx.notify(); } }))))
            })
            .child(
                div()
                    .id("check-body")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(body),
            )
    }
}

fn plural_copies(n: usize) -> String {
    format!("{n} cop{}", if n == 1 { "y" } else { "ies" })
}
