use super::*;

impl ResultsPanel {
    pub fn open_insert_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let columns = match &self.selected_schema {
            Some(s) => s.columns.clone(),
            None => return,
        };
        self.editing_row = None;
        self.edit_original_row = None;
        self.insert_columns = columns;
        self.insert_inputs = self.insert_columns.iter().map(|col| {
            cx.new(|cx| {
                let mut input = InputState::new(window, cx)
                    .placeholder(&col.data_type)
                    .clean_on_escape();
                if let Some(ref def) = col.default_value {
                    input = input.default_value(def.as_str());
                }
                input
            })
        }).collect();
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
        self.insert_inputs = self.insert_columns.iter().map(|col| {
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
        }).collect();
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
            None => { self.close_insert_modal(cx); return; }
        };
        let col_names: Vec<String> = self.insert_columns.iter().map(|c| c.name.clone()).collect();
        let values: Vec<String> = self.insert_inputs.iter().map(|input| {
            let val = input.read(cx).value().trim().to_string();
            if val.is_empty() {
                "NULL".to_string()
            } else {
                quote_string_literal(&val)
            }
        }).collect();

        let table_q = crate::state::quote_ident(&table, cx);
        let cols_q: Vec<String> = col_names.iter().map(|c| crate::state::quote_ident(c, cx)).collect();

        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({});",
            table_q,
            cols_q.join(", "),
            values.join(", ")
        );

        self.close_insert_modal(cx);
        crate::state::execute_query(sql, cx);
    }

    fn apply_edit(&mut self, cx: &mut Context<Self>) {
        let table = match &self.current_table {
            Some(t) => t.clone(),
            None => { self.close_insert_modal(cx); return; }
        };
        let original_row = match &self.edit_original_row {
            Some(r) => r.clone(),
            None => { self.close_insert_modal(cx); return; }
        };
        let query = match self.result.as_deref() {
            Some(SqlResult::Query(q)) => q.clone(),
            _ => { self.close_insert_modal(cx); return; }
        };

        let pk_columns: Vec<&ColumnInfo> = match &self.selected_schema {
            Some(s) => s.columns.iter().filter(|c| c.is_primary_key).collect(),
            None => Vec::new(),
        };
        if pk_columns.is_empty() {
            return;
        }

        let sets: Vec<String> = self.insert_columns.iter().zip(self.insert_inputs.iter()).map(|(col, input)| {
            let val = input.read(cx).value().trim().to_string();
            let col_q = crate::state::quote_ident(&col.name, cx);
            if val.is_empty() {
                format!("{} = NULL", col_q)
            } else {
                format!("{} = {}", col_q, quote_string_literal(&val))
            }
        }).collect();

        let conditions: Vec<String> = pk_columns
            .iter()
            .filter_map(|pk_col| {
                let col_ix = query.columns.iter().position(|c| c.name == pk_col.name)?;
                let cell = original_row.get(col_ix)?;
                let col_q = crate::state::quote_ident(&pk_col.name, cx);
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

        let table_q = crate::state::quote_ident(&table, cx);
        let sql = format!(
            "UPDATE {} SET {} WHERE {};",
            table_q,
            sets.join(", "),
            conditions.join(" AND ")
        );
        self.close_insert_modal(cx);
        crate::state::execute_query(sql, cx);
    }

    pub fn refresh_table(&mut self, cx: &mut Context<Self>) {
        let table = match &self.current_table {
            Some(t) => t.clone(),
            None => return,
        };
        let sql = crate::state::build_select_query(&table, None, cx);
        crate::state::execute_query(sql, cx);
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

        let pk_columns = match &self.selected_schema {
            Some(s) => {
                let pks: Vec<&ColumnInfo> = s.columns.iter().filter(|c| c.is_primary_key).collect();
                if pks.is_empty() {
                    return;
                }
                pks
            }
            None => return,
        };

        let table_entity = self.table.clone();
        let conditions: Vec<String> = pk_columns
            .iter()
            .filter_map(|pk_col| {
                let col_q = crate::state::quote_ident(&pk_col.name, cx);
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

        let table_q = crate::state::quote_ident(&table, cx);
        let sql = format!(
            "DELETE FROM {} WHERE {};",
            table_q,
            conditions.join(" AND ")
        );
        *self.selected_row_cell.borrow_mut() = None;
        crate::state::execute_query(sql, cx);
    }

    pub fn show_schema(&mut self, schema: TableSchema, cx: &mut Context<Self>) {
        self.selected_schema = Some(schema);
        cx.notify();
    }

}
