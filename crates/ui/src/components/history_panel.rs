use gpui::*;
use gpui_component::{
    ActiveTheme as _,
    Disableable as _,
    Icon,
    IconName,
    Selectable as _,
    Sizable as _,
    StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    label::Label,
    list::ListItem,
    scroll::ScrollableElement as _,
    v_flex,
};

use crate::state::AppState;
use crate::utils::truncate_str;
use dbstudio_storage::types::QueryHistoryEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HistoryTab {
    History,
    Favorites,
}

pub struct HistoryPanel {
    history: Vec<QueryHistoryEntry>,
    favorites: Vec<crate::state::FavoriteEntry>,
    active_tab: HistoryTab,
    search_state: Entity<InputState>,
    search_text: String,
    _subscriptions: Vec<Subscription>,
}

impl HistoryPanel {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }

    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search history...")
                .clean_on_escape()
        });

        let _subscriptions = vec![
            cx.observe_global::<AppState>(move |this, cx| {
                let state = cx.global::<AppState>();
                this.history = state.query_history.clone();
                this.favorites = state.favorites.clone();
                cx.notify();
            }),
            cx.subscribe_in(&search_state, window, |this, _, event: &InputEvent, _window, cx| {
                if let InputEvent::Change = event {
                    this.search_text = this.search_state.read(cx).value().to_string();
                    cx.notify();
                }
            }),
        ];

        Self {
            history: cx.global::<AppState>().query_history.clone(),
            favorites: cx.global::<AppState>().favorites.clone(),
            active_tab: HistoryTab::History,
            search_state,
            search_text: String::new(),
            _subscriptions,
        }
    }

    fn filtered_history(&self) -> Vec<&QueryHistoryEntry> {
        if self.search_text.is_empty() {
            self.history.iter().collect()
        } else {
            let query = self.search_text.to_lowercase();
            self.history
                .iter()
                .filter(|entry| entry.sql.to_lowercase().contains(&query))
                .collect()
        }
    }

    fn filtered_favorites(&self) -> Vec<&crate::state::FavoriteEntry> {
        if self.search_text.is_empty() {
            self.favorites.iter().collect()
        } else {
            let query = self.search_text.to_lowercase();
            self.favorites
                .iter()
                .filter(|entry| {
                    entry.name.to_lowercase().contains(&query)
                        || entry.sql.to_lowercase().contains(&query)
                })
                .collect()
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
                    .disabled(self.history.is_empty() || self.active_tab == HistoryTab::Favorites)
                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                        crate::state::clear_history(cx);
                    })),
            );

        // Tab bar
        let tab_bar = h_flex()
            .id("history-tabs")
            .px_2()
            .py_1()
            .gap_1()
            .child(
                Button::new("tab-history")
                    .label("History")
                    .small()
                    .ghost()
                    .selected(self.active_tab == HistoryTab::History)
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.active_tab = HistoryTab::History;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("tab-favorites")
                    .label("Favorites")
                    .small()
                    .ghost()
                    .selected(self.active_tab == HistoryTab::Favorites)
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.active_tab = HistoryTab::Favorites;
                        cx.notify();
                    })),
            );

        let search_bar = h_flex()
            .id("history-search")
            .px_2()
            .py_1()
            .gap_2()
            .items_center()
            .child(
                Icon::new(IconName::Search)
                    .size_3_5()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(Input::new(&self.search_state).small());

        let (list, count_label) = if self.active_tab == HistoryTab::History {
            let filtered = self.filtered_history();
            let total = self.history.len();
            let filtered_count = filtered.len();

            let list = if filtered.is_empty() {
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .child(
                        Label::new(if self.search_text.is_empty() {
                            "No queries yet"
                        } else {
                            "No matching queries"
                        })
                        .text_sm()
                        .text_color(cx.theme().muted_foreground),
                    )
                    .into_any_element()
            } else {
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .gap_px()
                    .px_1()
                    .children(filtered.into_iter().enumerate().map(|(ix, entry)| {
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
                                    .child(Label::new(sql_preview).text_xs())
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

            let count_label = if self.search_text.is_empty() {
                format!("{} queries", total)
            } else {
                format!("{} of {} queries", filtered_count, total)
            };

            (list, count_label)
        } else {
            // Favorites tab
            let filtered = self.filtered_favorites();
            let total = self.favorites.len();
            let filtered_count = filtered.len();

            let list = if filtered.is_empty() {
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .child(
                        Label::new(if self.search_text.is_empty() {
                            "No favorites yet"
                        } else {
                            "No matching favorites"
                        })
                        .text_sm()
                        .text_color(cx.theme().muted_foreground),
                    )
                    .into_any_element()
            } else {
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .gap_px()
                    .px_1()
                    .children(filtered.into_iter().enumerate().map(|(ix, entry)| {
                        let sql_preview = truncate_str(&entry.sql, 60);

                        ListItem::new(ix)
                            .w_full()
                            .py_1()
                            .px_2()
                            .rounded(cx.theme().radius)
                            .cursor_pointer()
                            .hover(|this| this.bg(cx.theme().list_hover))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _window, _cx| {
                                // Load this favorite into the editor
                                let sql = this.favorites.get(ix).map(|f| f.sql.clone());
                                if let Some(_sql) = sql {
                                    // TODO: Load into editor
                                }
                            }))
                            .child(
                                v_flex()
                                    .gap_0p5()
                                    .child(Label::new(&entry.name).text_xs().font_bold())
                                    .child(Label::new(sql_preview).text_xs()),
                            )
                    }))
                    .into_any_element()
            };

            let count_label = if self.search_text.is_empty() {
                format!("{} favorites", total)
            } else {
                format!("{} of {} favorites", filtered_count, total)
            };

            (list, count_label)
        };

        v_flex()
            .id("history-panel")
            .size_full()
            .child(header)
            .child(tab_bar)
            .child(search_bar)
            .child(
                h_flex()
                    .px_2()
                    .py_0p5()
                    .child(
                        Label::new(count_label)
                            .text_xs()
                            .text_color(cx.theme().muted_foreground),
                    ),
            )
            .child(list)
    }
}
