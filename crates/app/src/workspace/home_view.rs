use super::*;

impl Workspace {
    pub(super) fn render_home(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let sidebar = v_flex()
            .id("home-sidebar")
            .min_w(px(300.0))
            .flex_none()
            .h_full()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .id("home-connections")
                    .flex_1()
                    .gap_2()
                    .p_2()
                    .items_start()
                    .child(
                        div()
                            .pl_1()
                            .w_full()
                            .flex()
                            .flex_row()
                            .justify_between()
                            .items_center()
                            .child(div().text_sm().font_bold().child("Connections"))
                            .child(
                                Button::new("home-new")
                                    .icon(Icon::new(IconName::Plus).size_3_5())
                                    .tooltip("New Connection")
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.open_home_form(None, window, cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .p(px(8.0))
                            .flex_1()
                            .w_full()
                            .min_h_0()
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded(cx.theme().radius)
                            .child(self.connections.clone()),
                    ),
            );

        let mut main = div()
            .id("home-main")
            .flex_1()
            .min_h_0()
            .bg(cx.theme().tiles)
            .p_4();

        if self.connecting {
            main = main
                .flex()
                .items_center()
                .justify_center()
                .child(
                    v_flex()
                        .items_center()
                        .gap_2()
                        .child(Spinner::new().color(cx.theme().muted_foreground))
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("Connecting..."),
                        ),
                );
        } else if self.inline_form {
            if let Some(form) = &self.form {
                main = main.child(
                    div().w(px(560.0)).mt_8().mx_auto().child(form.clone()),
                );
            } else {
                main = self.render_home_welcome(main, cx);
            }
        } else if let Some(conn) = &self.selected_connection {
            main = main
                .flex()
                .items_center()
                .justify_center()
                .child(
                    v_flex()
                        .gap_1()
                        .items_center()
                        .child(div().text_xl().child(conn.name.clone()))
                        .child(
                            div()
                                .text_lg()
                                .text_color(cx.theme().muted_foreground)
                                .child(if conn.host.is_empty() {
                                    conn.database.clone()
                                } else {
                                    format!(
                                        "{}@{}:{}/{}",
                                        conn.username, conn.host, conn.port, conn.database
                                    )
                                }),
                        )
                        .child(
                            h_flex()
                                .justify_center()
                                .gap_1()
                                .child(
                                    Button::new("detail-delete")
                                        .label("Delete")
                                        .ghost()
                                        .small()
                                        .on_click(cx.listener({
                                            let info = conn.clone();
                                            move |this, _: &ClickEvent, window, cx| {
                                                this.on_delete_selected(&info, window, cx)
                                            }
                                        })),
                                )
                                .child(
                                    Button::new("detail-edit")
                                        .label("Edit")
                                        .ghost()
                                        .small()
                                        .on_click(cx.listener({
                                            let info = conn.clone();
                                            move |this, _: &ClickEvent, window, cx| {
                                                this.on_edit_selected(&info, window, cx)
                                            }
                                        })),
                                )
                                .child(
                                    Button::new("detail-connect")
                                        .label("Connect")
                                        .primary()
                                        .small()
                                        .on_click(cx.listener({
                                            let info = conn.clone();
                                            move |this, _: &ClickEvent, window, cx| {
                                                this.on_connect_selected(&info, window, cx)
                                            }
                                        })),
                                ),
                        ),
                );
        } else {
            main = self.render_home_welcome(main, cx);
        }

        v_flex()
            .id("home-page")
            .size_full()
            .bg(cx.theme().background)
            .child(
                div()
                    .id("home-header")
                    .flex()
                    .h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .px_2()
                    .h(px(36.0))
                    .bg(cx.theme().title_bar)
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .child(
                                Button::new("home-header-new")
                                    .icon(Icon::new(IconName::Plus).size_3_5())
                                    .tooltip("New Connection")
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.open_home_form(None, window, cx);
                                    })),
                            )
                            .child(toolbar_divider(cx))
                            .child(
                                Button::new("home-theme")
                                    .icon(Icon::new(IconName::Sun).size_3_5())
                                    .ghost()
                                    .small()
                                    .tooltip("Toggle Theme")
                                    .on_click(cx.listener(|_, _: &ClickEvent, window, cx| {
                                        crate::themes::toggle_color_mode(Some(window), cx);
                                    })),
                            )
                            .child(
                                Button::new("home-github")
                                    .icon(Icon::new(IconName::Github).size_3_5())
                                    .ghost()
                                    .small()
                                    .tooltip("GitHub")
                                    .on_click(cx.listener(|_, _: &ClickEvent, _window, _cx| {})),
                            ),
                    )
                    .child(window_control_buttons(cx)),
            )
            .child(
                h_flex()
                    .id("home-content")
                    .flex_1()
                    .min_h_0()
                    .child(sidebar)
                    .child(main),
            )
    }

    pub(super) fn render_home_welcome(&self, main: Stateful<Div>, cx: &mut Context<Self>) -> Stateful<Div> {
        main.flex()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .child(div().text_lg().font_semibold().child(dbstudio_core::APP_NAME))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("Create or select a connection"),
                    )
                    .child(
                        div()
                            .mt_2()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_xs()
                            .opacity(0.6)
                            .child(format!("Version: {}", VERSION)),
                    )
                    .child(
                        div()
                            .mt_1()
                            .flex()
                            .justify_center()
                            .child(Icon::new(IconName::Heart).size_3()),
                    ),
            )
    }

}
