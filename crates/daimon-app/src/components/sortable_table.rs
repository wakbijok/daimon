use leptos::prelude::*;

// --- Types ---

#[derive(Clone, Debug)]
pub struct ColumnDef {
    pub key: &'static str,
    pub label: &'static str,
    pub sortable: bool,
    pub default_hidden: bool,
    pub sort_type: SortType,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SortType {
    Text,
    Numeric,
    Percentage,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SortDir {
    Asc,
    Desc,
}

// --- Trait ---

pub trait TableRow: Clone + 'static {
    fn columns() -> Vec<ColumnDef>;
    fn cell_value(&self, col: &str) -> String;
    fn cell_view(&self, col: &str) -> AnyView;
    fn row_key(&self) -> String;
    fn row_link(&self) -> Option<String> {
        None
    }
}

// --- Pure logic functions ---

/// Sort rows by column value
pub fn sort_rows<T: TableRow>(rows: &mut [T], col: &str, dir: SortDir, sort_type: SortType) {
    rows.sort_by(|a, b| {
        let va = a.cell_value(col);
        let vb = b.cell_value(col);
        let ordering = match sort_type {
            SortType::Numeric | SortType::Percentage => {
                let na: f64 = va.parse().unwrap_or(0.0);
                let nb: f64 = vb.parse().unwrap_or(0.0);
                na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
            }
            SortType::Text => va.to_lowercase().cmp(&vb.to_lowercase()),
        };
        match dir {
            SortDir::Asc => ordering,
            SortDir::Desc => ordering.reverse(),
        }
    });
}

/// Filter rows by search query across visible columns
pub fn filter_rows<T: TableRow>(rows: &[T], query: &str, visible_cols: &[&str]) -> Vec<T> {
    if query.is_empty() {
        return rows.to_vec();
    }
    let q = query.to_lowercase();
    rows.iter()
        .filter(|row| {
            visible_cols
                .iter()
                .any(|col| row.cell_value(col).to_lowercase().contains(&q))
        })
        .cloned()
        .collect()
}

/// Get a page slice from rows
pub fn paginate<T: Clone>(rows: &[T], page: usize, page_size: usize) -> Vec<T> {
    let start = page * page_size;
    if start >= rows.len() {
        return vec![];
    }
    let end = (start + page_size).min(rows.len());
    rows[start..end].to_vec()
}

/// Total number of pages
pub fn total_pages(row_count: usize, page_size: usize) -> usize {
    if page_size == 0 {
        return 0;
    }
    (row_count + page_size - 1) / page_size
}

/// Export rows to CSV string
pub fn export_csv<T: TableRow>(rows: &[T], columns: &[ColumnDef]) -> String {
    let mut csv = columns
        .iter()
        .map(|c| c.label)
        .collect::<Vec<_>>()
        .join(",");
    csv.push('\n');
    for row in rows {
        let line = columns
            .iter()
            .map(|c| {
                let val = row.cell_value(c.key);
                if val.contains(',') || val.contains('"') {
                    format!("\"{}\"", val.replace('"', "\"\""))
                } else {
                    val
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        csv.push_str(&line);
        csv.push('\n');
    }
    csv
}

/// Export rows to JSON string
pub fn export_json<T: TableRow>(rows: &[T], columns: &[ColumnDef]) -> String {
    let objects: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let mut map = serde_json::Map::new();
            for col in columns {
                map.insert(
                    col.key.to_string(),
                    serde_json::Value::String(row.cell_value(col.key)),
                );
            }
            serde_json::Value::Object(map)
        })
        .collect();
    serde_json::to_string_pretty(&objects).unwrap_or_default()
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct TestRow {
        name: String,
        value: f64,
    }

    impl TableRow for TestRow {
        fn columns() -> Vec<ColumnDef> {
            vec![
                ColumnDef {
                    key: "name",
                    label: "Name",
                    sortable: true,
                    default_hidden: false,
                    sort_type: SortType::Text,
                },
                ColumnDef {
                    key: "value",
                    label: "Value",
                    sortable: true,
                    default_hidden: false,
                    sort_type: SortType::Numeric,
                },
            ]
        }

        fn cell_value(&self, col: &str) -> String {
            match col {
                "name" => self.name.clone(),
                "value" => self.value.to_string(),
                _ => String::new(),
            }
        }

        fn cell_view(&self, _col: &str) -> AnyView {
            ().into_any()
        }

        fn row_key(&self) -> String {
            self.name.clone()
        }
    }

    fn sample_rows() -> Vec<TestRow> {
        vec![
            TestRow { name: "charlie".to_string(), value: 30.0 },
            TestRow { name: "alice".to_string(),   value: 10.0 },
            TestRow { name: "bob".to_string(),      value: 20.0 },
        ]
    }

    #[test]
    fn sort_text_asc() {
        let mut rows = sample_rows();
        sort_rows(&mut rows, "name", SortDir::Asc, SortType::Text);
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["alice", "bob", "charlie"]);
    }

    #[test]
    fn sort_numeric_desc() {
        let mut rows = sample_rows();
        sort_rows(&mut rows, "value", SortDir::Desc, SortType::Numeric);
        let values: Vec<f64> = rows.iter().map(|r| r.value).collect();
        assert_eq!(values, vec![30.0, 20.0, 10.0]);
    }

    #[test]
    fn sort_text_case_insensitive() {
        let mut rows = vec![
            TestRow { name: "Banana".to_string(), value: 2.0 },
            TestRow { name: "apple".to_string(),  value: 1.0 },
            TestRow { name: "Cherry".to_string(), value: 3.0 },
        ];
        sort_rows(&mut rows, "name", SortDir::Asc, SortType::Text);
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["apple", "Banana", "Cherry"]);
    }

    #[test]
    fn filter_matches_any_column() {
        let rows = sample_rows();
        let result = filter_rows(&rows, "bob", &["name", "value"]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "bob");
    }

    #[test]
    fn filter_case_insensitive() {
        let rows = sample_rows();
        let result = filter_rows(&rows, "ALICE", &["name", "value"]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "alice");
    }

    #[test]
    fn filter_empty_query_returns_all() {
        let rows = sample_rows();
        let result = filter_rows(&rows, "", &["name", "value"]);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn paginate_first_page() {
        let rows = sample_rows();
        let page = paginate(&rows, 0, 2);
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].name, "charlie");
        assert_eq!(page[1].name, "alice");
    }

    #[test]
    fn paginate_last_page_partial() {
        let rows = sample_rows();
        let page = paginate(&rows, 1, 2);
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].name, "bob");
    }

    #[test]
    fn paginate_beyond_range() {
        let rows = sample_rows();
        let page = paginate(&rows, 5, 2);
        assert!(page.is_empty());
    }

    #[test]
    fn total_pages_calculation() {
        assert_eq!(total_pages(0, 25), 0);
        assert_eq!(total_pages(25, 25), 1);
        assert_eq!(total_pages(26, 25), 2);
        assert_eq!(total_pages(100, 25), 4);
    }

    #[test]
    fn export_csv_basic() {
        let rows = vec![
            TestRow { name: "alice".to_string(), value: 10.0 },
            TestRow { name: "bob".to_string(),   value: 20.0 },
        ];
        let cols = TestRow::columns();
        let csv = export_csv(&rows, &cols);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "Name,Value");
        assert!(lines[1].starts_with("alice,"));
        assert!(lines[2].starts_with("bob,"));
    }

    #[test]
    fn export_csv_escapes_commas() {
        let rows = vec![TestRow {
            name: "alice, jr".to_string(),
            value: 10.0,
        }];
        let cols = TestRow::columns();
        let csv = export_csv(&rows, &cols);
        let data_line = csv.lines().nth(1).unwrap();
        assert!(data_line.starts_with("\"alice, jr\""));
    }
}
