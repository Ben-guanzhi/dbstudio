use dbstudio_ui::state::Interaction;
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
        let segment = cx.global::<AppState>().active_object_for(self.window_id);
        let segment_bar = h_flex()
            .id("object-segment-bar")
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .flex_none()
            .border_b_1()
            .border_color(cx.theme().border)
            .children(ObjectSegment::ALL.into_iter().map(|seg| {
                let is_active = seg == segment;
                div()
                    .id(SharedString::from(format!("object-seg-{}", seg.label())))
                    .px_3()
                    .py_1()
                    .rounded(cx.theme().radius)
                    .text_xs()
                    .font_medium()
                    .when(is_active, |this| {
                        this.bg(cx.theme().accent)
                            .text_color(cx.theme().accent_foreground)
                    })
                    .when(!is_active, |this| {
                        this.text_color(cx.theme().muted_foreground)
                            .hover(|this| this.bg(cx.theme().list_hover))
                    })
                    .child(seg.label())
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        set_object_segment(seg, this.window_id, cx);
                        cx.notify();
                    }))
            }));
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
        // The center pane shows one object segment at a time: Data/Query share
        // the editor + results split, Structure and History take it over fully.
        let center_pane = match segment {
            ObjectSegment::Data | ObjectSegment::Query => editor_pane,
            ObjectSegment::Structure => div()
                .id("structure-pane")
                .flex()
                .flex_col()
                .flex_1()
                .h_full()
                .w_full()
                .min_h_0()
                .overflow_hidden()
                .child(self.structure.clone()),
            ObjectSegment::History => div()
                .id("history-pane")
                .flex()
                .flex_col()
                .flex_1()
                .h_full()
                .w_full()
                .min_h_0()
                .overflow_hidden()
                .child(self.history.clone()),
        };
        let center = div()
            .id("center-pane")
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .w_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .child(segment_bar)
            .child(center_pane);
        let main_area = div()
            .id("main-area")
            .flex()
            .flex_row()
            .flex_1()
            .h_full()
            .min_h_0()
            .when(show_tables, |this| this.child(sidebar))
            .child(center)
            .when(show_history, |this| this.child(history_panel))
            .when(show_ai, |this| this.child(ai_panel));

        let has_pending = cx
            .global::<AppState>()
            .window_state(self.window_id)
            .interaction
            .is_pending();
        let pending_display = match &cx
            .global::<AppState>()
            .window_state(self.window_id)
            .interaction
        {
            Interaction::ConfirmQuery { sql, kind } => {
                Some((sql.clone(), kind.label().to_string()))
            }
            Interaction::Nothing => None,
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
