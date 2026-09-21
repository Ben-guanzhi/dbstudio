use super::*;

impl ResultsPanel {
    /// Text filter row: keyword box plus a clear button.
    pub(super) fn render_text_filter_bar(
        &self,
        has_filter: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .gap_1()
            .items_center()
            .child(
                Icon::new(IconName::Search)
                    .size_3_5()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(Input::new(&self.filter_input).small().flex_1())
            .when(has_filter, |this| {
                this.child(
                    Button::new("clear-filter")
                        .icon(Icon::new(IconName::Close).size_3())
                        .small()
                        .ghost()
                        .on_click(cx.listener(|this, _: &ClickEvent, win, cx| {
                            this.filter_text.clear();
                            this.filter_input.update(cx, |input, cx| {
                                input.set_value(String::new(), win, cx);
                            });
                            this.apply_filter(cx);
                        })),
                )
            })
    }

    /// Type-aware per-column filter bar: active chips plus an "Add Filter"
    /// composer to append (column, operator, value) filters.
    pub(super) fn render_filter_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_columns = matches!(self.result.as_deref(), Some(SqlResult::Query(q)) if !q.columns.is_empty());
        if !has_columns {
            return div().into_any_element();
        }

        let mut row = h_flex()
            .gap_1()
            .items_center()
            .flex_wrap()
            .child(
                Button::new("add-column-filter")
                    .label("Filter")
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.toggle_filter_composer(window, cx);
                    })),
            )
            .children(self.column_filters.iter().enumerate().map(|(ix, filter)| {
                let label = format!(
                    "{} {} {}",
                    self.result_column_name(filter.column, cx),
                    filter.op.label(),
                    if filter.op.needs_value() { filter.value.as_str() } else { "" }
                );
                div()
                    .id(SharedString::from(format!("filter-chip-{ix}")))
                    .px_2()
                    .py_0p5()
                    .gap_1()
                    .items_center()
                    .rounded(px(4.0))
                    .bg(cx.theme().tiles)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(Label::new(label.trim()).text_xs())
                    .child(
                        Button::new(SharedString::from(format!("filter-chip-remove-{ix}")))
                            .icon(Icon::new(IconName::Close).size_3())
                            .small()
                            .ghost()
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.remove_column_filter(ix, cx);
                            })),
                    )
                    .into_any_element()
            }))
            .when(!self.column_filters.is_empty(), |this| {
                this.child(
                    Button::new("clear-column-filters")
                        .label("Clear")
                        .small()
                        .ghost()
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.clear_column_filters(cx);
                        })),
                )
            });

        if self.show_filter_composer {
            row = row.child(self.render_filter_composer(cx));
        }

        row.into_any_element()
    }

    fn render_filter_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let op_needs_value = self.filter_op.needs_value();
        h_flex()
            .gap_1()
            .items_center()
            .child(Select::new(&self.filter_col_select).small().placeholder("Column"))
            .child(Select::new(&self.filter_op_select).small().placeholder("Operator"))
            .when(op_needs_value, |this| {
                this.child(Input::new(&self.filter_value_input).small().w(px(160.0)))
            })
            .child(
                Button::new("apply-column-filter")
                    .label("Add")
                    .small()
                    .primary()
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.add_column_filter(window, cx);
                    })),
            )
            .child(
                Button::new("cancel-column-filter")
                    .label("Cancel")
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.show_filter_composer = false;
                        this.filter_value_input.update(cx, |input, cx| {
                            input.set_value(String::new(), window, cx);
                        });
                        cx.notify();
                    })),
            )
    }

    /// Name of the result column at the given data index.
    fn result_column_name(&self, col: usize, _cx: &Context<Self>) -> String {
        match self.result.as_deref() {
            Some(SqlResult::Query(q)) => q
                .columns
                .get(col)
                .map(|c| c.name.clone())
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    pub(super) fn render_info(&self, cx: &Context<Self>) -> AnyElement {
        match self.result.as_deref() {
            Some(SqlResult::Query(query)) => div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(if query.truncated {
                    format!(
                        "showing {} of {}+ rows · {} ms",
                        query.row_count, query.total_row_count, query.execution_time_ms
                    )
                } else {
                    format!(
                        "{} rows · {} ms",
                        query.row_count, query.execution_time_ms
                    )
                })
                .into_any_element(),
            _ => div().into_any_element(),
        }
    }

    /// "Load more" button, shown only when the active result was truncated at
    /// the row cap so the user can page through the remaining rows.
    pub(super) fn render_load_more_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let is_truncated = matches!(self.result.as_deref(), Some(SqlResult::Query(q)) if q.truncated);
        if !is_truncated {
            return div().into_any_element();
        }
        Button::new("load-more")
            .label("Load more")
            .small()
            .ghost()
            .tooltip("Fetch the next page of rows")
            .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                crate::state::load_more_rows(cx);
            }))
            .into_any_element()
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
        let session = cx.global::<AppState>().active_session();
        let pending_count = session.map(|s| s.pending_edits.len()).unwrap_or(0);
        let has_pending = pending_count > 0;
        let has_undo = session.map(|s| !s.undo_stack.is_empty()).unwrap_or(false);
        let has_redo = session.map(|s| !s.redo_stack.is_empty()).unwrap_or(false);

        h_flex()
            .gap_1()
            .items_center()
            .child(
                Button::new("undo-edit")
                    .icon(Icon::new(IconName::Undo).size_3_5())
                    .small()
                    .ghost()
                    .tooltip("Undo last applied edit")
                    .disabled(!has_undo)
                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                        crate::state::undo_last_edit(cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("redo-edit")
                    .icon(Icon::new(IconName::Redo).size_3_5())
                    .small()
                    .ghost()
                    .tooltip("Redo last undone edit")
                    .disabled(!has_redo)
                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                        crate::state::redo_last_edit(cx);
                        cx.notify();
                    })),
            )
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
            .when(has_pending, |this| {
                this.child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .ml_2()
                        .pl_2()
                        .border_l_1()
                        .border_color(cx.theme().border)
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().warning)
                                .child(format!("{} pending", pending_count)),
                        )
                        .child(
                            Button::new("review-pending")
                                .label("Review")
                                .small()
                                .ghost()
                                .tooltip("Review pending changes before applying")
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.toggle_review_modal(cx);
                                })),
                        )
                        .child(
                            Button::new("apply-pending")
                                .label("Apply")
                                .small()
                                .primary()
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.apply_all_pending(cx);
                                })),
                        )
                        .child(
                            Button::new("discard-pending")
                                .label("Discard")
                                .small()
                                .ghost()
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.discard_all_pending(cx);
                                })),
                        ),
                )
            })
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

    pub(super) fn render_review_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let edits = cx
            .global::<AppState>()
            .active_session()
            .map(|s| s.pending_edits.clone())
            .unwrap_or_default();
        let pending_label = format!("{} pending", edits.len());

        div()
            .id("review-modal-overlay")
            .absolute()
            .inset_0()
            .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                this.toggle_review_modal(cx);
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
                            .id("review-modal-content")
                            .w(px(640.0))
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
                                        h_flex()
                                            .gap_2()
                                            .items_center()
                                            .child(
                                                Icon::new(IconName::FileText)
                                                    .size_4()
                                                    .text_color(cx.theme().muted_foreground),
                                            )
                                            .child(
                                                Label::new("Pending Changes")
                                                    .text_base()
                                                    .font_bold(),
                                            ),
                                    )
                                    .child(
                                        Button::new("close-review")
                                            .icon(Icon::new(IconName::Close).size_3_5())
                                            .ghost()
                                            .small()
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _, cx| {
                                                    this.toggle_review_modal(cx);
                                                },
                                            )),
                                    ),
                            )
                            .child(
                                if edits.is_empty() {
                                    empty_hint("No pending changes", cx)
                                } else {
                                    v_flex()
                                        .flex_1()
                                        .overflow_y_scrollbar()
                                        .gap_2()
                                        .children(
                                            edits
                                                .iter()
                                                .enumerate()
                                                .map(|(ix, edit)| {
                                                    self.render_review_item(ix, edit, cx)
                                                }),
                                        )
                                        .into_any_element()
                                },
                            )
                            .when(!edits.is_empty(), |this| {
                                this.child(
                                    h_flex()
                                        .justify_end()
                                        .gap_2()
                                        .child(
                                            Button::new("review-discard")
                                                .label("Discard All")
                                                .ghost()
                                                .small()
                                                .on_click(cx.listener(
                                                    |this, _: &ClickEvent, _, cx| {
                                                        this.discard_all_pending(cx);
                                                    },
                                                )),
                                        )
                                        .child(
                                            Button::new("review-apply")
                                                .label(format!("Apply ({})", edits.len()))
                                                .primary()
                                                .small()
                                                .on_click(cx.listener(
                                                    |this, _: &ClickEvent, _, cx| {
                                                        this.apply_all_pending(cx);
                                                    },
                                                )),
                                        ),
                                )
                            })
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(pending_label),
                            ),
                    ),
            )
    }

    fn render_review_item(
        &self,
        index: usize,
        edit: &crate::state::guard::PendingWrite,
        cx: &Context<Self>,
    ) -> AnyElement {
        use crate::state::guard::DiffOp;

        let title = if let Some(diff) = &edit.diff {
            format!("{} {}  ·  {}", diff.op.label(), diff.table, diff.row_key)
        } else {
            format!("{}  ·  {}", edit.kind.label(), edit.label)
        };

        v_flex()
            .gap_1()
            .p_2()
            .rounded(px(4.0))
            .bg(cx.theme().tiles)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .text_color(cx.theme().foreground)
                    .child(title),
            )
            .child(
                match &edit.diff {
                    Some(diff) if !diff.cells.is_empty() => v_flex()
                        .gap_1()
                        .children(diff.cells.iter().map(|cell| {
                            let op = diff.op;
                            let old_disp = cell.old_value.as_deref().unwrap_or("NULL");
                            let new_disp = cell.new_value.as_deref().unwrap_or("NULL");
                            div()
                                .text_xs()
                                .flex()
                                .gap_1()
                                .child(
                                    div()
                                        .w(px(150.0))
                                        .text_color(cx.theme().muted_foreground)
                                        .child(cell.column.clone()),
                                )
                                .child(match op {
                                    DiffOp::Update => div()
                                        .flex()
                                        .gap_2()
                                        .child(div().text_color(gpui::red()).child(old_disp.to_string()))
                                        .child(
                                            div()
                                                .text_color(cx.theme().muted_foreground)
                                                .child("→"),
                                        )
                                        .child(
                                            div()
                                                .text_color(gpui::green())
                                                .child(new_disp.to_string()),
                                        ),
                                    DiffOp::Insert => div()
                                        .text_color(gpui::green())
                                        .child(format!("+ {}", new_disp)),
                                    DiffOp::Delete => div()
                                        .text_color(gpui::red())
                                        .child(format!("- {}", old_disp)),
                                })
                        })),
                    _ => div()
                        .text_xs()
                        .font_family("monospace")
                        .text_color(cx.theme().muted_foreground)
                        .child(edit.sql.clone()),
                },
            )
            .child(
                h_flex()
                    .justify_end()
                    .child(
                        Button::new(("review-apply-item", index))
                            .label("Apply")
                            .primary()
                            .small()
                            .on_click(cx.listener(
                                move |this, _: &ClickEvent, _, cx| {
                                    this.apply_pending(index, cx);
                                },
                            )),
                    ),
            )
            .into_any_element()
    }
}
