use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _,
    Disableable as _,
    Icon,
    IconName,
    Selectable as _,
    Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    label::Label,
    list::ListItem,
    scroll::ScrollableElement as _,
    v_flex,
};

use crate::state::AppState;

#[derive(Debug, Clone)]
struct AiMessage {
    role: MessageRole,
    content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AiTab {
    Chat,
    Explain,
}

pub struct AiPanel {
    chat_input: Entity<InputState>,
    messages: Vec<AiMessage>,
    active_tab: AiTab,
    is_loading: bool,
    current_sql: String,
    _subscriptions: Vec<Subscription>,
}

impl AiPanel {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }

    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let chat_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Ask about SQL...")
                .clean_on_escape()
        });

        let _subscriptions = vec![cx.observe_global::<AppState>(move |this, cx| {
            let state = cx.global::<AppState>();
            if let Some(session) = state.active_session() {
                this.current_sql = session.editor_text.clone();
            }
            cx.notify();
        })];

        Self {
            chat_input,
            messages: Vec::new(),
            active_tab: AiTab::Chat,
            is_loading: false,
            current_sql: String::new(),
            _subscriptions,
        }
    }

    fn send_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.chat_input.read(cx).value().to_string();
        if text.trim().is_empty() || self.is_loading {
            return;
        }
        self.messages.push(AiMessage {
            role: MessageRole::User,
            content: text.clone(),
        });
        self.chat_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
        self.is_loading = true;
        cx.notify();
        let response = self.mock_response(&text);
        self.messages.push(AiMessage {
            role: MessageRole::Assistant,
            content: response,
        });
        self.is_loading = false;
        cx.notify();
    }

    fn mock_response(&self, input: &str) -> String {
        let low = input.to_lowercase();
        if low.contains("explain") || low.contains("what does") {
            format!(
                "Current SQL in editor:\n```sql\n{}\n```\n\nThis query retrieves data from the database. Configure an AI provider in Settings for detailed analysis.",
                truncate_str(&self.current_sql, 200)
            )
        } else if low.contains("optimize") || low.contains("performance") {
            "Optimization tips:\n1. Add indexes on frequently queried columns\n2. Use LIMIT during development\n3. Avoid SELECT * in production\n4. Use EXPLAIN ANALYZE to measure performance".to_string()
        } else if low.contains("help") {
            "I can help with:\n- SQL Explanation\n- Query Optimization\n- Database Design\n- Debugging errors\n\nConfigure an AI provider in Settings for full functionality.".to_string()
        } else {
            format!("This is a placeholder response. Configure an AI provider in Settings for real AI assistance.\n\nYour question: {}", truncate_str(input, 100))
        }
    }

    fn explain_sql(&mut self, cx: &mut Context<Self>) {
        if self.current_sql.trim().is_empty() {
            return;
        }
        self.messages.push(AiMessage {
            role: MessageRole::User,
            content: format!("Explain this SQL:\n```sql\n{}\n```", self.current_sql),
        });
        self.is_loading = true;
        cx.notify();
        self.messages.push(AiMessage {
            role: MessageRole::Assistant,
            content: "**SQL Explanation:**\n\n1. Scans the table(s) in the FROM clause\n2. Filters rows via WHERE conditions\n3. Returns columns from SELECT list\n\n**Performance Notes:**\n- Add indexes on WHERE/JOIN columns\n- Avoid SELECT * in production\n\n*Configure an AI provider for detailed analysis.*".to_string(),
        });
        self.is_loading = false;
        cx.notify();
    }

    fn optimize_sql(&mut self, cx: &mut Context<Self>) {
        if self.current_sql.trim().is_empty() {
            return;
        }
        self.messages.push(AiMessage {
            role: MessageRole::User,
            content: format!("Optimize this SQL:\n```sql\n{}\n```", self.current_sql),
        });
        self.is_loading = true;
        cx.notify();
        self.messages.push(AiMessage {
            role: MessageRole::Assistant,
            content: "**Optimization Suggestions:**\n\n1. Add indexes on frequently queried columns\n2. Use LIMIT to restrict result sets\n3. Avoid subqueries where JOINs are more efficient\n4. Use EXPLAIN ANALYZE to measure actual performance\n\n*Configure an AI provider for rewrite suggestions.*".to_string(),
        });
        self.is_loading = false;
        cx.notify();
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

impl Render for AiPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = h_flex()
            .justify_between()
            .items_center()
            .px_2()
            .py_1()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(Icon::new(IconName::Bot).size_4())
                    .child(div().text_base().child("AI Assistant")),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("ai-explain")
                            .label("Explain")
                            .small()
                            .ghost()
                            .disabled(self.current_sql.trim().is_empty())
                            .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                this.explain_sql(cx);
                            })),
                    )
                    .child(
                        Button::new("ai-optimize")
                            .label("Optimize")
                            .small()
                            .ghost()
                            .disabled(self.current_sql.trim().is_empty())
                            .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                this.optimize_sql(cx);
                            })),
                    ),
            );

        let tab_bar = h_flex()
            .id("ai-tabs")
            .px_2()
            .py_1()
            .gap_1()
            .child(
                Button::new("tab-chat")
                    .label("Chat")
                    .small()
                    .ghost()
                    .selected(self.active_tab == AiTab::Chat)
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.active_tab = AiTab::Chat;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("tab-explain")
                    .label("Explain")
                    .small()
                    .ghost()
                    .selected(self.active_tab == AiTab::Explain)
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.active_tab = AiTab::Explain;
                        cx.notify();
                    })),
            );

        let messages_list = if self.messages.is_empty() {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    Icon::new(IconName::Bot)
                        .size_12()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    Label::new("Ask me anything about SQL or databases")
                        .text_sm()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    Label::new("Configure an AI provider in Settings for full functionality")
                        .text_xs()
                        .text_color(cx.theme().muted_foreground),
                )
                .into_any_element()
        } else {
            v_flex()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .gap_3()
                .px_2()
                .py_1()
                .children(self.messages.iter().enumerate().map(|(ix, msg)| {
                    let is_user = msg.role == MessageRole::User;
                    let bg = if is_user {
                        cx.theme().primary
                    } else {
                        cx.theme().background
                    };
                    let text_color = if is_user {
                        cx.theme().primary_foreground
                    } else {
                        cx.theme().foreground
                    };

                    ListItem::new(ix)
                        .w_full()
                        .py_2()
                        .px_3()
                        .rounded(cx.theme().radius)
                        .bg(bg)
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                        Label::new(if is_user { "You" } else { "AI" })
                                            .text_xs()
                                            .text_color(text_color),
                                )
                                .child(
                                    Label::new(msg.content.clone())
                                        .text_sm()
                                        .text_color(text_color),
                                ),
                        )
                }))
                .when(self.is_loading, |this| {
                    this.child(
                        h_flex()
                            .px_2()
                            .py_1()
                            .gap_2()
                            .child(
                                Icon::new(IconName::Loader)
                                    .size_4()
                                    .text_color(cx.theme().muted_foreground),
                            )
                            .child(
                                Label::new("Thinking...")
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground),
                            ),
                    )
                })
                .into_any_element()
        };

        let input_bar = h_flex()
            .id("ai-input")
            .px_2()
            .py_2()
            .gap_2()
            .items_center()
            .child(Input::new(&self.chat_input).flex_1())
            .child(
                Button::new("ai-send")
                    .icon(Icon::empty().path("icons/play.svg"))
                    .small()
                    .primary()
                    .ghost()
                    .disabled(self.is_loading)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.send_message(window, cx);
                    })),
            );

        v_flex()
            .id("ai-panel")
            .size_full()
            .child(header)
            .child(tab_bar)
            .child(messages_list)
            .child(input_bar)
    }
}
