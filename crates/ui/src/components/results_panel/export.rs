use super::*;

impl ResultsPanel {
    pub fn export_csv(&self, cx: &mut Context<Self>) {
        let query = match self.result.as_deref() {
            Some(SqlResult::Query(q)) => q.clone(),
            _ => return,
        };
        let path = export_path("csv");
        let path_display = path.display().to_string();

        let result = std::thread::spawn(move || -> std::io::Result<()> {
            let mut wtr = csv::Writer::from_path(&path)?;
            let headers: Vec<&str> = query.columns.iter().map(|c| c.name.as_str()).collect();
            wtr.write_record(&headers)?;
            for row in &query.rows {
                let cells: Vec<&str> = row.iter().map(|c| c.value.as_str()).collect();
                wtr.write_record(&cells)?;
            }
            wtr.flush()?;
            Ok(())
        });

        spawn_export_result(result, path_display, cx);
    }

    pub fn export_json(&self, cx: &mut Context<Self>) {
        let query = match self.result.as_deref() {
            Some(SqlResult::Query(q)) => q.clone(),
            _ => return,
        };
        let path = export_path("json");
        let path_display = path.display().to_string();

        let result = std::thread::spawn(move || -> std::io::Result<()> {
            let rows: Vec<serde_json::Value> = query
                .rows
                .iter()
                .map(|row| {
                    let mut map = serde_json::Map::new();
                    for (col, cell) in query.columns.iter().zip(row.iter()) {
                        if cell.is_null {
                            map.insert(col.name.clone(), serde_json::Value::Null);
                        } else {
                            map.insert(col.name.clone(), serde_json::Value::String(cell.value.clone()));
                        }
                    }
                    serde_json::Value::Object(map)
                })
                .collect();
            let json = serde_json::to_string_pretty(&rows)?;
            std::fs::write(&path, json)?;
            Ok(())
        });

        spawn_export_result(result, path_display, cx);
    }
}

