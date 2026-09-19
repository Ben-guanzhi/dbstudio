use gpui::*;
use gpui_component::{
    ActiveTheme as _,
    Disableable as _,
    Sizable as _,
    StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    label::Label,
    list::ListItem,
    scroll::ScrollableElement as _,
    v_flex,
};

use crate::state::AppState;
use crate::utils::truncate_str;
use dbstudio_storage::types::QueryHistoryEntry;

pub struct HistoryPanel {
    history: Vec<QueryHistoryEntry>,
    _subscriptions: Vec<Subscription>,
}

impl HistoryPanel {
    pub fn view(_window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let _subscriptions = vec![cx.observe_global::<AppState>(move |this, cx| {
            this.history = cx.global::<AppState>().query_history.clone();
            cx.notify();
        })];

        Self {
            history: cx.global::<AppState>().query_history.clone(),
            _subscriptions,
        }
    }
}

impl Render for HistoryPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = h_flex()
            .justify_between()
            .items_center()
            .px_2()
            .py_1()
            .child(Label::new("History").font_bold().text_base())
            .child(
                Button::new("history-clear")
                    .label("Clear")
                    .small()
                    .ghost()
                    .disabled(self.history.is_empty())
                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                        crate::state::clear_history(cx);
                    })),
            );

        let list = if self.history.is_empty() {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .child(
                    Label::new("No queries yet")
                        .text_sm()
                        .text_color(cx.theme().muted_foreground),
                )
                .into_any_element()
        } else {
            let items: Vec<QueryHistoryEntry> = self.history.clone();
            v_flex()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .gap_px()
                .px_1()
                .children(items.into_iter().enumerate().map(|(ix, entry)| {
                    let sql_preview = truncate_str(&entry.sql, 60);

                    let status_color = if entry.is_error {
                        gpui::red()
                    } else {
                        cx.theme().muted_foreground
                    };

                    let status_text = if entry.is_error {
                        "Error".to_string()
                    } else if let Some(rows) = entry.row_count {
                        format!("{} rows", rows)
                    } else {
                        "OK".to_string()
                    };

                    ListItem::new(ix)
                        .w_full()
                        .py_1()
                        .px_2()
                        .rounded(cx.theme().radius)
                        .cursor_pointer()
                        .hover(|this| this.bg(cx.theme().list_hover))
                        .child(
                            v_flex()
                                .gap_0p5()
                                .child(
                                    Label::new(sql_preview)
                                        .text_xs(),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            Label::new(status_text)
                                                .text_xs()
                                                .text_color(status_color),
                                        )
                                        .child(
                                            Label::new(format!("{} ms", entry.execution_time_ms))
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground),
                                        )
                                        .child(
                                            Label::new(&entry.executed_at)
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground),
                                        ),
                                ),
                        )
                }))
                .into_any_element()
        };

        v_flex()
            .id("history-panel")
            .size_full()
            .child(header)
            .child(list)
    }
}
