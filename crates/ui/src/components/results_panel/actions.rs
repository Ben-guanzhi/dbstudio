use super::*;
use crate::state::guard::{CellDiff, DiffOp, PendingDiff, PendingWrite, WriteKind};
use crate::state::quote_ident;

/// Trimmed non-empty cell value, or `None` for SQL NULL.
pub(super) fn cell_value_str(input_val: &str) -> Option<String> {
    let t = input_val.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// Quote a cell value for SQL; empty -> NULL.
pub(super) fn quoted_or_null(val: Option<&str>) -> String {
    match val {
        Some(v) => quote_string_literal(v),
        None => "NULL".to_string(),
    }
}

/// Short human-readable row key, e.g. `id = 3`, built from PK conditions.
fn row_key(pk_conditions: &[String]) -> String {
    pk_conditions.join(" AND ")
}

impl ResultsPanel {
    pub fn open_insert_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let columns = match &self.selected_schema {
            Some(s) => s.columns.clone(),
            None => return,
        };
        self.editing_row = None;
        self.edit_original_row = None;
        self.insert_columns = columns;
        self.insert_inputs = self
            .insert_columns
            .iter()
            .map(|col| {
                cx.new(|cx| {
                    let mut input = InputState::new(window, cx)
                        .placeholder(&col.data_type)
                        .clean_on_escape();
                    if let Some(ref def) = col.default_value {
                        input = input.default_value(def.as_str());
                    }
                    input
                })
            })
            .collect();
        self.show_insert_modal = true;
        cx.notify();
    }

    pub fn open_edit_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let row_ix = match *self.selected_row_cell.borrow() {
            Some(ix) => ix,
            None => return,
        };
        let columns = match &self.selected_schema {
            Some(s) => s.columns.clone(),
            None => return,
        };
        let query = match self.result.as_deref() {
            Some(SqlResult::Query(q)) => q.clone(),
            _ => return,
        };
        // The selection index is in *display* coordinates (row may be sorted/
        // reordered in the table), so rebuild the row through the delegate
        // instead of indexing the original `QueryResult` rows directly.
        let row_cells: Vec<ResultCell> = {
            let table = self.table.clone();
            let delegate = table.read(cx).delegate();
            query
                .columns
                .iter()
                .map(|c| {
                    delegate
                        .cell_named(row_ix, &c.name)
                        .cloned()
                        .unwrap_or_else(ResultCell::null)
                })
                .collect()
        };
        self.editing_row = Some(row_ix);
        self.edit_original_row = Some(row_cells.clone());
        self.insert_columns = columns;
        self.insert_inputs = self
            .insert_columns
            .iter()
            .map(|col| {
                cx.new(|cx| {
                    let mut input = InputState::new(window, cx)
                        .placeholder(&col.data_type)
                        .clean_on_escape();
                    let col_ix = query.columns.iter().position(|c| c.name == col.name);
                    let cell = col_ix.and_then(|ix| row_cells.get(ix));
                    match cell {
                        Some(cell) if !cell.is_null => {
                            input = input.default_value(cell.value.as_str());
                        }
                        _ => {
                            if let Some(ref def) = col.default_value {
                                input = input.default_value(def.as_str());
                            }
                        }
                    }
                    input
                })
            })
            .collect();
        self.show_insert_modal = true;
        cx.notify();
    }

    pub fn close_insert_modal(&mut self, cx: &mut Context<Self>) {
        self.show_insert_modal = false;
        self.editing_row = None;
        self.edit_original_row = None;
        self.insert_inputs.clear();
        self.insert_columns.clear();
        cx.notify();
    }

    pub(super) fn confirm_insert(&mut self, cx: &mut Context<Self>) {
        if self.editing_row.is_some() {
            self.apply_edit(cx);
            return;
        }

        let table = match &self.current_table {
            Some(t) => t.clone(),
            None => {
                self.close_insert_modal(cx);
                return;
            }
        };

        let col_names: Vec<String> = self.insert_columns.iter().map(|c| c.name.clone()).collect();
        let values: Vec<String> = self
            .insert_inputs
            .iter()
            .map(|input| {
                cell_value_str(input.read(cx).value().as_str())
                    .map(|v| quote_string_literal(&v))
                    .unwrap_or_else(|| "NULL".to_string())
            })
            .collect();

        let table_q = quote_ident(&table, self.window_id, cx);
        let cols_q: Vec<String> = col_names
            .iter()
            .map(|c| quote_ident(c, self.window_id, cx))
            .collect();

        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({});",
            table_q,
            cols_q.join(", "),
            values.join(", ")
        );

        // Diff + inverse (DELETE by primary key, when the PK was supplied).
        let pk_conditions: Vec<String> = self
            .insert_columns
            .iter()
            .zip(self.insert_inputs.iter())
            .filter_map(|(col, input)| {
                if !col.is_primary_key {
                    return None;
                }
                let val = cell_value_str(input.read(cx).value().as_str());
                let col_q = quote_ident(&col.name, self.window_id, cx);
                match val {
                    Some(v) => Some(format!("{} = {}", col_q, quote_string_literal(&v))),
                    None => None,
                }
            })
            .collect();

        let cells: Vec<CellDiff> = self
            .insert_columns
            .iter()
            .zip(self.insert_inputs.iter())
            .map(|(col, input)| CellDiff {
                column: col.name.clone(),
                old_value: None,
                new_value: cell_value_str(input.read(cx).value().as_str()),
            })
            .collect();

        let diff = Some(PendingDiff {
            table: table.clone(),
            op: DiffOp::Insert,
            row_key: row_key(&pk_conditions),
            cells,
        });

        let inverse_sql = if pk_conditions.is_empty() {
            String::new()
        } else {
            format!(
                "DELETE FROM {} WHERE {};",
                table_q,
                pk_conditions.join(" AND ")
            )
        };

        self.close_insert_modal(cx);
        self.stage_edit(sql, inverse_sql, "INSERT".to_string(), diff, cx);
    }

    fn apply_edit(&mut self, cx: &mut Context<Self>) {
        let table = match &self.current_table {
            Some(t) => t.clone(),
            None => {
                self.close_insert_modal(cx);
                return;
            }
        };
        let schema = match &self.selected_schema {
            Some(s) => s.clone(),
            None => {
                self.close_insert_modal(cx);
                return;
            }
        };
        let original_row = match &self.edit_original_row {
            Some(r) => r.clone(),
            None => {
                self.close_insert_modal(cx);
                return;
            }
        };
        let query = match self.result.as_deref() {
            Some(SqlResult::Query(q)) => q.clone(),
            _ => {
                self.close_insert_modal(cx);
                return;
            }
        };

        let pk_columns: Vec<&ColumnInfo> =
            schema.columns.iter().filter(|c| c.is_primary_key).collect();
        if pk_columns.is_empty() {
            return;
        }
        let pk_names: Vec<String> = pk_columns.iter().map(|c| c.name.clone()).collect();

        // Old / new value for every schema column (None -> NULL).
        let mut old_vals: Vec<Option<String>> = Vec::new();
        let mut new_vals: Vec<Option<String>> = Vec::new();
        for (col, input) in self.insert_columns.iter().zip(self.insert_inputs.iter()) {
            let old = query
                .columns
                .iter()
                .position(|c| c.name == col.name)
                .and_then(|ix| original_row.get(ix))
                .filter(|c| !c.is_null)
                .map(|c| c.value.clone());
            new_vals.push(cell_value_str(input.read(cx).value().as_str()));
            old_vals.push(old);
        }

        let mut sets: Vec<String> = Vec::new();
        let mut cells: Vec<CellDiff> = Vec::new();
        let mut pk_conditions: Vec<String> = Vec::new();

        for ((col, old), new) in self
            .insert_columns
            .iter()
            .zip(old_vals.iter())
            .zip(new_vals.iter())
        {
            let col_q = quote_ident(&col.name, self.window_id, cx);
            if col.is_primary_key {
                match new {
                    Some(v) => {
                        pk_conditions.push(format!("{} = {}", col_q, quote_string_literal(v)))
                    }
                    None => pk_conditions.push(format!("{} IS NULL", col_q)),
                }
            }
            if old != new {
                sets.push(format!("{} = {}", col_q, quoted_or_null(new.as_deref())));
                cells.push(CellDiff {
                    column: col.name.clone(),
                    old_value: old.clone(),
                    new_value: new.clone(),
                });
            }
        }

        if sets.is_empty() {
            self.close_insert_modal(cx);
            return;
        }
        if pk_conditions.is_empty() {
            self.close_insert_modal(cx);
            return;
        }

        let table_q = quote_ident(&table, self.window_id, cx);
        let sql = format!(
            "UPDATE {} SET {} WHERE {};",
            table_q,
            sets.join(", "),
            pk_conditions.join(" AND ")
        );

        // Inverse restores original column values identified by the original PK.
        let mut inv_sets: Vec<String> = Vec::new();
        for (ix, col) in schema.columns.iter().enumerate() {
            if !pk_names.contains(&col.name) {
                if let Some(old) = old_vals.get(ix) {
                    let col_q = quote_ident(&col.name, self.window_id, cx);
                    inv_sets.push(format!("{} = {}", col_q, quoted_or_null(old.as_deref())));
                }
            }
        }
        let inverse_sql = if inv_sets.is_empty() {
            String::new()
        } else {
            format!(
                "UPDATE {} SET {} WHERE {};",
                table_q,
                inv_sets.join(", "),
                pk_conditions.join(" AND ")
            )
        };

        let diff = Some(PendingDiff {
            table: table.clone(),
            op: DiffOp::Update,
            row_key: row_key(&pk_conditions),
            cells,
        });

        self.close_insert_modal(cx);
        self.stage_edit(sql, inverse_sql, "UPDATE".to_string(), diff, cx);
    }

    /// Stage an edit SQL in the session's pending_edits buffer.
    pub(super) fn stage_edit(
        &mut self,
        sql: String,
        inverse_sql: String,
        label: String,
        diff: Option<PendingDiff>,
        cx: &mut Context<Self>,
    ) {
        let kind = if label.starts_with("INSERT") {
            WriteKind::Insert
        } else if label.starts_with("UPDATE") {
            WriteKind::Update
        } else if label.starts_with("DELETE") {
            WriteKind::Delete
        } else {
            WriteKind::OtherDdl
        };
        cx.update_global::<AppState, _>(|state, _cx| {
            if let Some(s) = state.active_session_mut_for(self.window_id) {
                s.pending_edits.push(PendingWrite {
                    sql,
                    kind,
                    label,
                    inverse_sql,
                    diff,
                });
            }
        });
        cx.notify();
    }

    pub fn refresh_table(&mut self, cx: &mut Context<Self>) {
        let table = match &self.current_table {
            Some(t) => t.clone(),
            None => return,
        };
        let sql = crate::state::build_select_query(&table, None, self.window_id, cx);
        crate::state::execute_query(sql, self.window_id, cx);
    }

    /// Apply all pending edits by executing each SQL via the raw query path.
    pub fn apply_all_pending(&mut self, cx: &mut Context<Self>) {
        let edits: Vec<(String, String, String)> = cx.update_global::<AppState, _>(|state, _cx| {
            match state.active_session_mut_for(self.window_id) {
                Some(s) => {
                    let records: Vec<(String, String, String)> = s
                        .pending_edits
                        .iter()
                        .map(|e| (e.sql.clone(), e.inverse_sql.clone(), e.label.clone()))
                        .collect();
                    s.pending_edits.clear();
                    records
                }
                None => Vec::new(),
            }
        });
        for (forward, inverse, label) in edits {
            cx.update_global::<AppState, _>(|state, _cx| {
                if let Some(s) = state.active_session_mut_for(self.window_id) {
                    s.undo_stack.push(crate::state::guard::EditRecord {
                        forward_sql: forward.clone(),
                        inverse_sql: inverse,
                        label: label.clone(),
                    });
                    s.redo_stack.clear();
                }
            });
            crate::state::execute_raw_query(forward, self.window_id, cx);
        }
        self.table.update(cx, |table, cx| {
            table.delegate_mut().clear_modified();
            cx.notify();
        });
        self.show_review_modal = false;
        cx.notify();
    }

    /// Apply a single pending edit, leaving the remaining edits queued.
    pub fn apply_pending(&mut self, index: usize, cx: &mut Context<Self>) {
        let removed = cx.update_global::<AppState, _>(|state, _cx| {
            match state.active_session_mut_for(self.window_id) {
                Some(s) => {
                    if index < s.pending_edits.len() {
                        let removed = s.pending_edits.remove(index);
                        s.undo_stack.push(crate::state::guard::EditRecord {
                            forward_sql: removed.sql.clone(),
                            inverse_sql: removed.inverse_sql.clone(),
                            label: removed.label.clone(),
                        });
                        s.redo_stack.clear();
                        Some(removed)
                    } else {
                        None
                    }
                }
                None => None,
            }
        });
        let Some(edit) = removed else {
            return;
        };
        crate::state::execute_raw_query(edit.sql, self.window_id, cx);
        let empty = cx.update_global::<AppState, _>(|state, _cx| {
            state
                .active_session_for(self.window_id)
                .map(|s| s.pending_edits.is_empty())
                .unwrap_or(true)
        });
        if empty {
            self.show_review_modal = false;
            self.table.update(cx, |table, cx| {
                table.delegate_mut().clear_modified();
                cx.notify();
            });
        }
        cx.notify();
    }

    /// Discard all pending edits.
    pub fn discard_all_pending(&mut self, cx: &mut Context<Self>) {
        cx.update_global::<AppState, _>(|state, _cx| {
            if let Some(s) = state.active_session_mut_for(self.window_id) {
                s.pending_edits.clear();
            }
        });
        self.table.update(cx, |table, cx| {
            table.delegate_mut().clear_modified();
            cx.notify();
        });
        self.show_review_modal = false;
        cx.notify();
    }

    pub(super) fn delete_row(&mut self, cx: &mut Context<Self>) {
        let row_ix = match *self.selected_row_cell.borrow() {
            Some(ix) => ix,
            None => return,
        };
        let table = match &self.current_table {
            Some(t) => t.clone(),
            None => return,
        };
        let schema = match &self.selected_schema {
            Some(s) => s.clone(),
            None => return,
        };

        let table_entity = self.table.clone();
        let pk_columns: Vec<&ColumnInfo> =
            schema.columns.iter().filter(|c| c.is_primary_key).collect();
        if pk_columns.is_empty() {
            return;
        }

        let conditions: Vec<String> = pk_columns
            .iter()
            .filter_map(|pk_col| {
                let col_q = crate::state::quote_ident(&pk_col.name, self.window_id, cx);
                let cell = table_entity
                    .read(cx)
                    .delegate()
                    .cell_named(row_ix, &pk_col.name)?;
                if cell.is_null {
                    Some(format!("{} IS NULL", col_q))
                } else {
                    Some(format!("{} = {}", col_q, quote_string_literal(&cell.value)))
                }
            })
            .collect();

        if conditions.is_empty() {
            return;
        }

        let table_q = crate::state::quote_ident(&table, self.window_id, cx);
        let sql = format!(
            "DELETE FROM {} WHERE {};",
            table_q,
            conditions.join(" AND ")
        );

        // Inverse INSERT + diff require the full old row (all schema columns).
        let all_present: bool = schema.columns.iter().all(|col| {
            table_entity
                .read(cx)
                .delegate()
                .cell_named(row_ix, &col.name)
                .is_some()
        });
        let inverse_sql = if all_present {
            let cols_q: Vec<String> = schema
                .columns
                .iter()
                .map(|c| crate::state::quote_ident(&c.name, self.window_id, cx))
                .collect();
            let vals: Vec<String> = schema
                .columns
                .iter()
                .map(|col| {
                    table_entity
                        .read(cx)
                        .delegate()
                        .cell_named(row_ix, &col.name)
                        .map(|c| {
                            if c.is_null {
                                "NULL".to_string()
                            } else {
                                quote_string_literal(&c.value)
                            }
                        })
                        .unwrap_or_else(|| "NULL".to_string())
                })
                .collect();
            format!(
                "INSERT INTO {} ({}) VALUES ({});",
                table_q,
                cols_q.join(", "),
                vals.join(", ")
            )
        } else {
            String::new()
        };

        let cells: Vec<CellDiff> = schema
            .columns
            .iter()
            .filter_map(|col| {
                table_entity
                    .read(cx)
                    .delegate()
                    .cell_named(row_ix, &col.name)
                    .map(|c| CellDiff {
                        column: col.name.clone(),
                        old_value: if c.is_null {
                            None
                        } else {
                            Some(c.value.clone())
                        },
                        new_value: None,
                    })
            })
            .collect();

        let diff = Some(PendingDiff {
            table: table.clone(),
            op: DiffOp::Delete,
            row_key: row_key(&conditions),
            cells,
        });

        *self.selected_row_cell.borrow_mut() = None;
        self.stage_edit(sql, inverse_sql, "DELETE".to_string(), diff, cx);
    }

    pub fn show_schema(&mut self, schema: TableSchema, cx: &mut Context<Self>) {
        self.selected_schema = Some(schema);
        cx.notify();
    }
}
