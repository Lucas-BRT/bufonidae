use crate::interaction::{ControlFeedback as _, button};
use gpui::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Window, div,
};
use gpui_component::{
    Selectable as _, StyledExt as _,
    button::ButtonVariants as _,
    input::{Input, InputEvent, InputState},
};

#[derive(Clone, Copy)]
pub enum Command {
    Files,
    Accounts,
    Check,
    Transfers,
    Settings,
    Storage,
    Add,
    Search,
    DownloadSelection,
    Refresh,
    Help,
}
impl Command {
    fn label(self) -> &'static str {
        match self {
            Self::Files => "Open Files",
            Self::Accounts => "Open Accounts",
            Self::Check => "Open Check",
            Self::Transfers => "Open Transfers",
            Self::Settings => "Open Settings",
            Self::Storage => "Open storage distribution and Sync",
            Self::Add => "Add files or folders",
            Self::Search => "Search the current folder",
            Self::DownloadSelection => "Download selected paths",
            Self::Refresh => "Refresh current screen",
            Self::Help => "Open help",
        }
    }
}
pub enum CommandEvent {
    Chosen(Command),
    Closed,
}
pub struct CommandsForm {
    query: Entity<InputState>,
    help: bool,
    highlight: usize,
    guide_step: Option<usize>,
}
impl EventEmitter<CommandEvent> for CommandsForm {}
impl CommandsForm {
    pub fn new(help: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search commands"));
        cx.subscribe(&query, |this, _, event: &InputEvent, cx| match event {
            InputEvent::Change => {
                this.highlight = 0;
                cx.notify();
            }
            InputEvent::PressEnter { .. } => {
                if let Some(command) = this.filtered(cx).get(this.highlight) {
                    cx.emit(CommandEvent::Chosen(*command));
                }
            }
            _ => {}
        })
        .detach();
        if !help {
            query.update(cx, |field, cx| field.focus(window, cx));
        }
        Self {
            query,
            help,
            highlight: 0,
            guide_step: None,
        }
    }
    fn filtered(&self, cx: &Context<Self>) -> Vec<Command> {
        let query = self.query.read(cx).value().to_lowercase();
        [
            Command::Files,
            Command::Accounts,
            Command::Check,
            Command::Transfers,
            Command::Settings,
            Command::Storage,
            Command::Add,
            Command::Search,
            Command::DownloadSelection,
            Command::Refresh,
            Command::Help,
        ]
        .into_iter()
        .filter(|command| command.label().to_lowercase().contains(&query))
        .collect()
    }
    pub fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(CommandEvent::Closed);
    }
}
impl Render for CommandsForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut content = crate::ui::Dialog::new(560.0, window, cx).child(
            div().text_xl().font_semibold().child(if self.help {
                "Bufonidae help"
            } else {
                "Commands"
            }),
        );
        if self.help && self.guide_step.is_some() {
            let steps = [
                (
                    "Connect accounts",
                    "Start with Mega accounts you already own. Open Accounts and connect one; unlock your keyring if requested. No accounts are created automatically.",
                    Command::Accounts,
                ),
                (
                    "Choose protection",
                    "In Settings choose copies per file. Two copies need two distinct accounts with enough space. More copies consume more storage; keep an independent backup.",
                    Command::Settings,
                ),
                (
                    "Add a small file",
                    "Use Add to choose a file and a destination. Review the plan before storing. The dialog shows waiting, connecting and live transfer progress.",
                    Command::Add,
                ),
                (
                    "Find and download",
                    "Use Files search, filters and checkboxes. Multiple selections preserve virtual paths. Download never overwrites a local file. Transfers keeps the result and any error details.",
                    Command::Files,
                ),
                (
                    "Check and recover",
                    "Run Check after an interruption. Deep verification downloads copies and consumes bandwidth. Repair rebuilds missing copies. A mirror protects against account loss, not every kind of loss.",
                    Command::Check,
                ),
            ];
            let step = self.guide_step.unwrap_or(0).min(steps.len() - 1);
            let (title, text, command) = steps[step];
            content = content
                .child(div().font_semibold().child(format!(
                    "Step {} of {} · {title}",
                    step + 1,
                    steps.len()
                )))
                .child(div().text_sm().child(text))
                .child(button("tour-open-screen").label(command.label()).on_click(
                    cx.listener(move |_, _, _, cx| cx.emit(CommandEvent::Chosen(command))),
                ))
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            button("tour-back")
                                .label("Previous")
                                .disabled_feedback(step == 0)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.guide_step =
                                        this.guide_step.map(|step| step.saturating_sub(1));
                                    cx.notify();
                                })),
                        )
                        .child(
                            button("tour-next")
                                .primary()
                                .label(if step + 1 == steps.len() {
                                    "Finish tour"
                                } else {
                                    "Next"
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.guide_step = if step + 1 == steps.len() {
                                        None
                                    } else {
                                        Some(step + 1)
                                    };
                                    cx.notify();
                                })),
                        ),
                );
        } else if self.help {
            for (title, text) in [
                (
                    "Accounts",
                    "Connect Mega accounts you already own. Passwords stay in your system keyring. Unlock the keyring if sign-in cannot access credentials. Respect Mega's quota limits.",
                ),
                (
                    "Store files",
                    "Use Add to select files and folders, or drop them into the window. Choose the destination and review the upload. Settings controls how many account copies each file needs. This is not a mounted drive or automatic sync.",
                ),
                (
                    "Find and download",
                    "Search and filter the open folder; use checkboxes for multiple paths. Select visible affects the current results, while Clear removes all selections. Multiple-path downloads preserve virtual paths and parent selections subsume descendants. Existing local files are never replaced.",
                ),
                (
                    "Interrupted or damaged copies",
                    "Open Transfers to inspect failed/interrupted jobs. Review uploads before retrying. Check compares remote listings; Download and verify contents consumes bandwidth to compare hashes. Repair rebuilds missing copies. Keep a separate backup and export the index using the CLI.",
                ),
                (
                    "Notifications and privacy",
                    "Settings offers independent completion, failure and cancellation notifications, durations and sound policies. Paths are omitted unless you opt in. Desktop do-not-disturb can suppress notifications. The app must remain running.",
                ),
                (
                    "Preferences",
                    "Choose density and base text size independently. Desktop preferences use Save and Discard; redundancy and the upload threshold save immediately. Disable optional explanations without hiding errors or safety confirmations.",
                ),
            ] {
                content = content
                    .child(div().font_semibold().child(title))
                    .child(div().text_sm().child(text));
            }
            if crate::preferences::current(cx).shortcut_hints {
                content=content.child(div().font_semibold().child("Keyboard reference")).child(div().text_sm().child("Ctrl+1…5: screens · Ctrl+F: search · Ctrl+O: Add · Ctrl+K: commands · F1: help · F5: refresh · Alt+Up: parent folder. Command search supports Up/Down and Enter; dialogs support Tab, Shift+Tab and Escape. Settings selects All, Navigation only or Off for application shortcuts."));
            }
            content = content.child(
                button("start-guided-tour")
                    .primary()
                    .label("Start guided tour")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.guide_step = Some(0);
                        cx.notify();
                    })),
            );
        } else {
            let commands = self.filtered(cx);
            content = content.child(Input::new(&self.query));
            if commands.is_empty() {
                content = content.child(div().text_sm().child("No commands match this search."));
            }
            for (i, command) in commands.into_iter().enumerate() {
                content = content.child(
                    button(("command", i))
                        .ghost()
                        .w_full()
                        .justify_start()
                        .label(command.label())
                        .selected(i == self.highlight)
                        .on_click(
                            cx.listener(move |_, _, _, cx| cx.emit(CommandEvent::Chosen(command))),
                        ),
                );
            }
        }
        div()
            .child(
                content.footer(
                    div().h_flex().justify_end().child(
                        button("close-commands")
                            .label("Close")
                            .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                    ),
                ),
            )
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if !this.help && matches!(event.keystroke.key.as_str(), "up" | "down") {
                    let count = this.filtered(cx).len();
                    if count > 0 {
                        this.highlight = if event.keystroke.key == "up" {
                            (this.highlight + count - 1) % count
                        } else {
                            (this.highlight + 1) % count
                        };
                        cx.notify();
                    }
                    cx.stop_propagation();
                }
            }))
    }
}
