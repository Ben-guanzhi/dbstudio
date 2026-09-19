use super::*;

impl ResultsPanel {
    pub(super) fn render_info(&self, cx: &Context<Self>) -> AnyElement {
        match self.result.as_deref() {
            Some(SqlResult::Query(query)) => div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!(
                    "{} rows · {} ms",
                    query.row_count, query.execution_time_ms
                ))
                .into_any_element(),
            _ => div().into_any_element(),
        }
    }

    pub(super) fn render_export_buttons(&self, has_query_results: bool, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_1()
            .child(self.render_export_button(
                "export-csv",
                "CSV",
                "Export as CSV",
                has_query_results,
                Self::export_csv,
                cx,
            ))
            .child(self.render_export_button(
                "export-json",
                "JSON",
                "Export as JSON",
                has_query_results,
                Self::export_json,
                cx,
            ))
    }

    /// A small ghost button with a file icon, disabled when there are no rows.
    fn render_export_button(
        &self,
        id: &'static str,
        label: &'static str,
        tooltip: &'static str,
        enabled: bool,
        action: fn(&Self, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) -> Button {
        Button::new(id)
            .label(label)
            .icon(Icon::new(IconName::FileText))
            .small()
            .ghost()
            .tooltip(tooltip)
            .disabled(!enabled)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| action(this, cx)))
    }

    pub(super) fn render_schema_view(&self, cx: &Context<Self>) -> AnyElement {
        if let Some(schema) = &self.selected_schema {
            v_flex()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .gap_2()
                .p_2()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            Icon::new(IconName::Frame)
                                .size_4()
                                .text_color(cx.theme().muted_foreground),
                        )
                        .child(
                            Label::new(&schema.table_name)
                                .text_base()
                                .font_bold(),
                        ),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child(section_header("Columns", cx))
                        .children(schema.columns.iter().map(|col| {
                            h_flex()
                                .gap_2()
                                .px_2()
                                .py_0p5()
                                .rounded(px(4.0))
                                .hover(|this| this.bg(cx.theme().list_hover))
                                .child(Label::new(&col.name).text_sm())
                                .child(muted_label(col.data_type.clone(), cx))
                                .when(col.is_primary_key, |this| {
                                    this.child(
                                        Label::new("PK")
                                            .text_xs()
                                            .text_color(cx.theme().accent_foreground)
                                            .bg(cx.theme().accent),
                                    )
                                })
                                .when(col.nullable, |this| {
                                    this.child(muted_label("NULL", cx))
                                })
                                .when(!col.nullable, |this| {
                                    this.child(muted_label("NOT NULL", cx))
                                })
                                .when_some(col.default_value.clone(), |this, def| {
                                    this.child(muted_label(format!("default: {}", def), cx))
                                })
                        })),
                )
                .when(!schema.indexes.is_empty(), |this| {
                    this.child(
                        v_flex()
                            .gap_1()
                            .mt_2()
                            .child(section_header("Indexes", cx))
                            .children(schema.indexes.iter().map(|idx| {
                                h_flex()
                                    .gap_2()
                                    .px_2()
                                    .py_0p5()
                                    .child(Label::new(&idx.name).text_sm())
                                    .child(muted_label(idx.columns.join(", "), cx))
                                    .when(idx.is_unique, |this| {
                                        this.child(muted_label("UNIQUE", cx))
                                    })
                                    .when(idx.is_primary, |this| {
                                        this.child(
                                            Label::new("PRIMARY")
                                                .text_xs()
                                                .text_color(cx.theme().accent_foreground)
                                                .bg(cx.theme().accent),
                                        )
                                    })
                            })),
                    )
                })
                .when(!schema.foreign_keys.is_empty(), |this| {
                    this.child(
                        v_flex()
                            .gap_1()
                            .mt_2()
                            .child(section_header("Foreign Keys", cx))
                            .children(schema.foreign_keys.iter().map(|fk| {
                                h_flex()
                                    .gap_2()
                                    .px_2()
                                    .py_0p5()
                                    .child(Label::new(&fk.name).text_sm())
                                    .child(muted_label(
                                        format!(
                                            "{}({}) -> {}({})",
                                            fk.source_table,
                                            fk.source_columns.join(", "),
                                            fk.target_table,
                                            fk.target_columns.join(", "),
                                        ),
                                        cx,
                                    ))
                            })),
                    )
                })
                .when_some(schema.create_sql.as_ref(), |this, sql| {
                    this.child(
                        v_flex()
                            .gap_1()
                            .mt_2()
                            .child(section_header("DDL", cx))
                            .child(
                                div()
                                    .p_2()
                                    .rounded(px(4.0))
                                    .bg(cx.theme().tiles)
                                    .child(
                                        Label::new(sql.clone())
                                            .text_xs()
                                            .font_family("monospace"),
                                    ),
                            ),
                    )
                })
                .into_any_element()
        } else {
            empty_hint("No schema information available", cx)
        }
    }

    pub(super) fn render_data_content(&self, has_table: bool, cx: &mut Context<Self>) -> AnyElement {
        match self.result.as_deref() {
            None => empty_hint(
                if has_table {
                    "Loading table data..."
                } else {
                    "Run a query to see results"
                },
                cx,
            ),
            Some(SqlResult::Error(err)) => render_error(cx, err).into_any_element(),
            Some(SqlResult::Modified(exec)) => render_modified(cx, exec).into_any_element(),
            Some(SqlResult::Query(_)) => div()
                .id("query-table-scroll")
                .relative()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .child(DataTable::new(&self.table).stripe(true))
                .into_any_element(),
        }
    }

    pub(super) fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_0()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(self.render_tab("tab-data", "Data", ResultsTab::Data, cx))
            .child(self.render_tab("tab-schema", "Schema", ResultsTab::Schema, cx))
    }

    fn render_tab(
        &self,
        id: &'static str,
        label: &'static str,
        tab: ResultsTab,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_active = self.active_tab == tab;

        div()
            .id(id)
            .px_3()
            .py_1()
            .text_sm()
            .cursor_pointer()
            .when(is_active, |this| {
                this.border_b_2()
                    .border_color(cx.theme().accent)
                    .text_color(cx.theme().accent_foreground)
                    .font_semibold()
            })
            .when(!is_active, |this| {
                this.text_color(cx.theme().muted_foreground)
                    .hover(|this| this.text_color(cx.theme().foreground))
            })
            .child(label)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.set_active_tab(tab, cx);
            }))
    }

    pub(super) fn render_toolbar(&self, has_table: bool, has_schema: bool, has_selection: bool, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_1()
            .items_center()
            .child(
                Button::new("refresh-table")
                    .icon(Icon::new(IconName::RotateCw).size_3_5())
                    .small()
                    .ghost()
                    .tooltip("Refresh")
                    .disabled(!has_table)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.refresh_table(cx);
                    })),
            )
            .child(
                Button::new("insert-row")
                    .icon(Icon::new(IconName::Plus).size_3_5())
                    .small()
                    .ghost()
                    .tooltip("Insert Row")
                    .disabled(!has_schema)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.open_insert_modal(window, cx);
                    })),
            )
            .child(
                Button::new("edit-row")
                    .icon(Icon::new(Icon::default().path("icons/pencil-line.svg")).size_3_5())
                    .small()
                    .ghost()
                    .tooltip("Edit Row")
                    .disabled(!has_selection || !has_schema)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.open_edit_modal(window, cx);
                    })),
            )
            .child(
                Button::new("delete-row")
                    .icon(Icon::new(IconName::Delete).size_3_5())
                    .small()
                    .ghost()
                    .tooltip("Delete Row")
                    .disabled(!has_selection)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.delete_row(cx);
                    })),
            )
    }

    pub(super) fn render_insert_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let modal_columns = self.insert_columns.clone();
        let modal_inputs = self.insert_inputs.clone();
        let is_editing = self.editing_row.is_some();
        let modal_title = if is_editing { "Edit Row" } else { "Insert Row" };
        let confirm_label = if is_editing { "Save" } else { "Insert" };

        div()
            .id("insert-modal-overlay")
            .absolute()
            .inset_0()
            .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                this.close_insert_modal(cx);
            }))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(gpui::black().opacity(0.4)),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .items_center()
                    .justify_center()
                    .child(
                        v_flex()
                            .id("insert-modal-content")
                            .w(px(480.0))
                            .max_h(px(600.0))
                            .p_4()
                            .gap_3()
                            .rounded(px(8.0))
                            .bg(cx.theme().background)
                            .border_1()
                            .border_color(cx.theme().border)
                            .on_click(|_e, _window, cx| {
                                cx.stop_propagation();
                            })
                            .child(
                                h_flex()
                                    .justify_between()
                                    .items_center()
                                    .child(
                                        Label::new(modal_title)
                                            .text_base()
                                            .font_bold(),
                                    )
                                    .child(
                                        Button::new("close-modal")
                                            .icon(Icon::new(IconName::Close).size_3_5())
                                            .ghost()
                                            .small()
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _, cx| {
                                                    this.close_insert_modal(cx);
                                                },
                                            )),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .overflow_y_scrollbar()
                                    .gap_2()
                                    .children(
                                        modal_columns
                                            .iter()
                                            .zip(modal_inputs.iter())
                                            .map(|(col, input)| {
                                                h_flex()
                                                    .gap_2()
                                                    .items_center()
                                                    .child(
                                                        div()
                                                            .w(px(120.0))
                                                            .child(
                                                                Label::new(&col.name)
                                                                    .text_sm(),
                                                            ),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .child(
                                                                Input::new(input),
                                                            ),
                                                    )
                                            }),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .justify_end()
                                    .gap_2()
                                    .child(
                                        Button::new("cancel-insert")
                                            .label("Cancel")
                                            .ghost()
                                            .small()
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _, cx| {
                                                    this.close_insert_modal(cx);
                                                },
                                            )),
                                    )
                                    .child(
                                        Button::new("confirm-insert")
                                            .label(confirm_label)
                                            .primary()
                                            .small()
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _, cx| {
                                                    this.confirm_insert(cx);
                                                },
                                            )),
                                    ),
                            ),
                    ),
            )
    }

}
