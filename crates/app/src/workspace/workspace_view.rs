use super::*;
impl Workspace {
    pub(super) fn render_workspace(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let ws = cx.global::<AppState>().window_state(self.window_id);
        let show_tables = ws.show_tables;
        let show_history = ws.show_history;
        let show_ai = self.show_ai_panel;
        let sidebar = div()
            .id("left-pane")
            .flex()
            .flex_col()
            .h_full()
            .w(px(300.0))
            .flex_none()
            .overflow_hidden()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(self.tables.clone());
        let history_panel = div()
            .id("right-pane")
            .flex()
            .flex_col()
            .h_full()
            .w(px(300.0))
            .flex_none()
            .overflow_hidden()
            .border_l_1()
            .border_color(cx.theme().border)
            .child(self.history.clone());
        let ai_panel = div()
            .id("ai-pane")
            .flex()
            .flex_col()
            .h_full()
            .w(px(350.0))
            .flex_none()
            .overflow_hidden()
            .border_l_1()
            .border_color(cx.theme().border)
            .child(self.ai_panel.clone());
        let editor_pane = div()
            .id("editor-pane")
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .w_full()
            .min_h_0()
            .overflow_hidden()
            .child(
                v_resizable("editor-results-split")
                    .child(
                        resizable_panel()
                            .child(self.editor.clone())
                            .size(px(400.0))
                            .size_range(px(120.0)..px(800.0)),
                    )
                    .child(resizable_panel().child(self.results.clone())),
            );
        let main_area = div()
            .id("main-area")
            .flex()
            .flex_row()
            .flex_1()
            .h_full()
            .min_h_0()
            .when(show_tables, |this| this.child(sidebar))
            .child(editor_pane)
            .when(show_history, |this| this.child(history_panel))
            .when(show_ai, |this| this.child(ai_panel));

        let has_pending = cx
            .global::<AppState>()
            .window_state(self.window_id)
            .pending_dangerous_query
            .is_some();
        let pending_display = if let Some((ref sql, ref kind)) = cx
            .global::<AppState>()
            .window_state(self.window_id)
            .pending_dangerous_query
        {
            Some((sql.clone(), kind.label().to_string()))
        } else {
            None
        };

        v_flex()
            .id("workspace")
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .child(self.header.clone())
            .child(self.tabs.clone())
            .when(has_pending, |this| {
                if let Some((sql, kind_label)) = pending_display {
                    this.child(
                        h_flex()
                            .id("confirmation-bar")
                            .items_center()
                            .gap_3()
                            .px_4()
                            .py_2()
                            .bg(gpui::red().opacity(0.08))
                            .border_b_1()
                            .border_color(gpui::red().opacity(0.3))
                            .child(
                                Icon::new(IconName::TriangleAlert)
                                    .size_4()
                                    .text_color(gpui::red()),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_bold()
                                            .text_color(gpui::red())
                                            .child(format!("Confirm {} query", kind_label)),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .child(sql),
                                    ),
                            )
                            .child(
                                Button::new("confirm-execute")
                                    .label("Execute")
                                    .danger()
                                    .small()
                                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                        dbstudio_ui::state::confirm_dangerous_query(
                                            this.window_id,
                                            cx,
                                        );
                                    })),
                            )
                            .child(
                                Button::new("confirm-cancel")
                                    .label("Cancel")
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                        dbstudio_ui::state::reject_dangerous_query(
                                            this.window_id,
                                            cx,
                                        );
                                    })),
                            ),
                    )
                } else {
                    this
                }
            })
            .child(main_area)
            .child(self.footer.clone())
    }
}
