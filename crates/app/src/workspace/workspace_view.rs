use super::*;
impl Workspace {
    pub(super) fn render_workspace(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let show_tables = cx.global::<AppState>().show_tables;
        let show_history = cx.global::<AppState>().show_history;
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
            .when(show_history, |this| this.child(history_panel));
        v_flex()
            .id("workspace")
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .child(self.header.clone())
            .child(main_area)
            .child(self.footer.clone())
    }
}
