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

// --- Component ---

/// Trigger a browser file download with the given filename and content.
/// Gated to hydrate (browser) only — on SSR this is a no-op.
#[allow(unused_variables)]
fn trigger_download(filename: &str, mime: &str, content: &str) {
    #[cfg(feature = "hydrate")]
    {
        use wasm_bindgen::JsCast;

        let Some(window) = web_sys::window() else { return };
        let Some(document) = window.document() else { return };
        let Some(body) = document.body() else { return };

        let encoded = js_sys::encode_uri_component(content);
        let uri = format!("data:{};charset=utf-8,{}", mime, encoded);

        let Ok(el) = document.create_element("a") else { return };
        let _ = el.set_attribute("href", &uri);
        let _ = el.set_attribute("download", filename);
        let _ = el.set_attribute("style", "display:none");
        let _ = body.append_child(&el);

        if let Some(html_el) = el.dyn_ref::<web_sys::HtmlElement>() {
            html_el.click();
        }

        let _ = body.remove_child(&el);
    }
}

/// Load hidden-column preferences from localStorage.
/// Returns a `HashSet` of column keys that should be hidden.
fn load_hidden_cols(#[allow(unused_variables)] table_id: &str, columns: &[ColumnDef]) -> std::collections::HashSet<String> {
    #[cfg(feature = "hydrate")]
    {
        if let Some(storage) = web_sys::window()
            .and_then(|w| w.local_storage().ok())
            .flatten()
        {
            let key = format!("daimon_table_cols_{}", table_id);
            if let Ok(Some(json)) = storage.get_item(&key) {
                if let Ok(set) = serde_json::from_str::<std::collections::HashSet<String>>(&json) {
                    return set;
                }
            }
        }
    }

    // Fallback: hide columns marked default_hidden
    columns
        .iter()
        .filter(|c| c.default_hidden)
        .map(|c| c.key.to_string())
        .collect()
}

/// Persist hidden-column preferences to localStorage.
#[allow(unused_variables)]
fn save_hidden_cols(table_id: &str, hidden: &std::collections::HashSet<String>) {
    #[cfg(feature = "hydrate")]
    {
        if let Some(storage) = web_sys::window()
            .and_then(|w| w.local_storage().ok())
            .flatten()
        {
            let key = format!("daimon_table_cols_{}", table_id);
            if let Ok(json) = serde_json::to_string(hidden) {
                let _ = storage.set_item(&key, &json);
            }
        }
    }
}

/// Compute filtered + sorted rows from signals. Shared helper to avoid
/// duplicating the logic across multiple closure sites.
fn compute_processed<T: TableRow + Send + Sync>(
    stored_rows: StoredValue<Vec<T>>,
    all_columns: &StoredValue<Vec<ColumnDef>>,
    hidden_cols: &dyn Fn() -> std::collections::HashSet<String>,
    search_query: &dyn Fn() -> String,
    sort_col: &dyn Fn() -> Option<String>,
    sort_dir: &dyn Fn() -> Option<SortDir>,
) -> Vec<T> {
    let cols = all_columns.get_value();
    let hidden = hidden_cols();
    let vis_keys: Vec<&str> = cols
        .iter()
        .filter(|c| !hidden.contains(c.key))
        .map(|c| c.key)
        .collect();

    let query = search_query();
    let all = stored_rows.get_value();
    let mut filtered = filter_rows(&all, &query, &vis_keys);

    if let (Some(col), Some(dir)) = (sort_col(), sort_dir()) {
        let sort_type = cols
            .iter()
            .find(|c| c.key == col)
            .map(|c| c.sort_type)
            .unwrap_or(SortType::Text);
        sort_rows(&mut filtered, &col, dir, sort_type);
    }

    filtered
}

/// A generic sortable, searchable, paginated table component.
///
/// Renders data implementing `TableRow` with client-side search filtering,
/// column sorting (click header to cycle none -> asc -> desc -> none),
/// pagination, column visibility toggle, and CSV/JSON export.
///
/// # Props
/// - `rows` -- the data to display (immutable after initial load)
/// - `table_id` -- unique identifier for persisting column preferences in localStorage
#[component]
pub fn SortableTable<T: TableRow + Send + Sync>(
    rows: Vec<T>,
    #[prop(default = "default")] table_id: &'static str,
) -> impl IntoView {
    let all_columns_vec = T::columns();

    // --- Signals ---
    let (search_query, set_search_query) = signal(String::new());
    let (sort_col, set_sort_col) = signal(Option::<String>::None);
    let (sort_dir, set_sort_dir) = signal(Option::<SortDir>::None);
    let (current_page, set_current_page) = signal(0usize);
    let (page_size, set_page_size) = signal(25usize);
    let (show_col_menu, set_show_col_menu) = signal(false);

    // Column visibility: stored as a set of *hidden* column keys
    let initial_hidden = load_hidden_cols(table_id, &all_columns_vec);
    let (hidden_cols, set_hidden_cols) = signal(initial_hidden);

    // Rows are immutable after mount -- StoredValue avoids reactive overhead.
    // StoredValue is Copy, so closures can capture it without move issues.
    let stored_rows = StoredValue::new(rows);
    let stored_columns = StoredValue::new(all_columns_vec);

    // --- Derived helpers (all use Copy signals + StoredValue, so no move issues) ---

    let visible_columns = move || -> Vec<ColumnDef> {
        let hidden = hidden_cols.get();
        stored_columns
            .get_value()
            .into_iter()
            .filter(|c| !hidden.contains(c.key))
            .collect()
    };

    let processed_rows = move || -> Vec<T> {
        compute_processed(
            stored_rows,
            &stored_columns,
            &move || hidden_cols.get(),
            &move || search_query.get(),
            &move || sort_col.get(),
            &move || sort_dir.get(),
        )
    };

    let page_rows = move || -> Vec<T> {
        let rows = processed_rows();
        paginate(&rows, current_page.get(), page_size.get())
    };

    let total_filtered = move || processed_rows().len();
    let total_page_count = move || total_pages(total_filtered(), page_size.get());
    // Extracted memos so the view! macro doesn't see a `>=` operator inside
    // a `prop:disabled=move || ...` (Leptos 0.8 tokenizer treats `>` inside
    // attribute closures as an end-of-tag delimiter, leaking source text).
    let prev_disabled = Memo::new(move |_| current_page.get() == 0);
    let next_disabled = Memo::new(move |_| current_page.get() + 1 >= total_page_count());
    let page_label = Memo::new(move |_| {
        let cur = current_page.get() + 1;
        let total = total_page_count().max(1);
        format!("{cur} / {total}")
    });

    // --- Handlers ---

    // Reset page to 0 whenever search changes
    let on_search = move |ev: leptos::ev::Event| {
        set_search_query.set(event_target_value(&ev));
        set_current_page.set(0);
    };

    // Column sort: cycle none -> asc -> desc -> none
    let on_sort = move |col_key: String| {
        let current_col = sort_col.get();
        let current_dir = sort_dir.get();

        if current_col.as_deref() == Some(&col_key) {
            match current_dir {
                Some(SortDir::Asc) => set_sort_dir.set(Some(SortDir::Desc)),
                Some(SortDir::Desc) => {
                    set_sort_col.set(None);
                    set_sort_dir.set(None);
                }
                None => set_sort_dir.set(Some(SortDir::Asc)),
            }
        } else {
            set_sort_col.set(Some(col_key));
            set_sort_dir.set(Some(SortDir::Asc));
        }
        set_current_page.set(0);
    };

    let on_prev = move |_| {
        let p = current_page.get();
        if p > 0 {
            set_current_page.set(p - 1);
        }
    };

    let on_next = move |_| {
        let p = current_page.get();
        if p + 1 < total_page_count() {
            set_current_page.set(p + 1);
        }
    };

    let on_page_size = move |ev: leptos::ev::Event| {
        if let Ok(size) = event_target_value(&ev).parse::<usize>() {
            set_page_size.set(size);
            set_current_page.set(0);
        }
    };

    // Export handlers
    let on_export_csv = move |_| {
        let vis_cols = visible_columns();
        let rows = processed_rows();
        let csv = export_csv(&rows, &vis_cols);
        trigger_download("export.csv", "text/csv", &csv);
    };

    let on_export_json = move |_| {
        let vis_cols = visible_columns();
        let rows = processed_rows();
        let json = export_json(&rows, &vis_cols);
        trigger_download("export.json", "application/json", &json);
    };

    // Column toggle handler
    let toggle_col = move |col_key: String| {
        set_hidden_cols.update(|set| {
            if set.contains(&col_key) {
                set.remove(&col_key);
            } else {
                set.insert(col_key);
            }
        });
        save_hidden_cols(table_id, &hidden_cols.get());
        set_current_page.set(0);
    };

    // Row click navigation
    let navigate = leptos_router::hooks::use_navigate();

    view! {
        // --- Toolbar ---
        <div class="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-3 mb-4">
            <input
                type="text"
                placeholder="Search..."
                on:input=on_search
                class="w-full sm:w-64 px-3 py-1.5 text-sm bg-surface-secondary border border-border-primary rounded-md text-text-primary placeholder-text-muted focus:outline-none focus:border-accent-amber"
            />

            <div class="flex items-center gap-2 flex-wrap">
                // Columns dropdown
                <div class="relative">
                    <button
                        on:click=move |_| set_show_col_menu.update(|v| *v = !*v)
                        class="px-3 py-1.5 text-xs text-text-muted border border-border-primary rounded-md hover:text-text-secondary"
                    >
                        "Columns"
                    </button>

                    <Show when=move || show_col_menu.get()>
                        {
                            let cols = stored_columns.get_value();
                            view! {
                                <div class="absolute right-0 top-full mt-1 w-48 bg-surface-secondary border border-border-primary rounded-lg shadow-lg z-50 py-1 max-h-64 overflow-y-auto">
                                    {cols.into_iter().map(|col| {
                                        let key = col.key.to_string();
                                        let label = col.label.to_string();
                                        let key_for_check = key.clone();
                                        let key_for_toggle = key.clone();
                                        let toggle_col = toggle_col.clone();
                                        view! {
                                            <label class="flex items-center gap-2 px-3 py-1.5 text-sm text-text-secondary hover:bg-surface-tertiary cursor-pointer">
                                                <input
                                                    type="checkbox"
                                                    prop:checked=move || !hidden_cols.get().contains(&key_for_check)
                                                    on:change=move |_| {
                                                        let tc = toggle_col.clone();
                                                        tc(key_for_toggle.clone());
                                                    }
                                                    class="accent-accent-amber"
                                                />
                                                {label}
                                            </label>
                                        }
                                    }).collect_view()}
                                </div>
                            }
                        }
                    </Show>
                </div>

                // Export buttons
                <button
                    on:click=on_export_csv
                    class="px-3 py-1.5 text-xs text-text-muted border border-border-primary rounded-md hover:text-text-secondary"
                >
                    "CSV"
                </button>
                <button
                    on:click=on_export_json
                    class="px-3 py-1.5 text-xs text-text-muted border border-border-primary rounded-md hover:text-text-secondary"
                >
                    "JSON"
                </button>

                // Page size selector
                <select
                    on:change=on_page_size
                    class="px-2 py-1.5 text-xs text-text-muted bg-surface-secondary border border-border-primary rounded-md focus:outline-none focus:border-accent-amber"
                >
                    <option value="25" selected=true>"25"</option>
                    <option value="50">"50"</option>
                    <option value="100">"100"</option>
                </select>
            </div>
        </div>

        // --- Table ---
        <div class="overflow-x-auto">
            <table class="w-full text-sm">
                <thead>
                    <tr class="border-b border-border-primary text-text-muted text-[11px] uppercase tracking-wider">
                        {move || {
                            visible_columns().into_iter().map(|col| {
                                let key = col.key.to_string();
                                let label = col.label.to_string();
                                let sortable = col.sortable;
                                let col_key_static = col.key;
                                let on_sort = on_sort.clone();

                                let arrow = move || {
                                    let sc = sort_col.get();
                                    let sd = sort_dir.get();
                                    if sc.as_deref() == Some(col_key_static) {
                                        match sd {
                                            Some(SortDir::Asc) => " \u{2191}",
                                            Some(SortDir::Desc) => " \u{2193}",
                                            None => "",
                                        }
                                    } else {
                                        ""
                                    }
                                };

                                view! {
                                    <th
                                        class=format!(
                                            "text-left py-3 px-4 font-medium {}",
                                            if sortable { "cursor-pointer select-none hover:text-text-secondary" } else { "" }
                                        )
                                        on:click=move |_| {
                                            if sortable {
                                                let on_sort = on_sort.clone();
                                                on_sort(key.clone());
                                            }
                                        }
                                    >
                                        {label}
                                        <span class="text-accent-amber">{arrow}</span>
                                    </th>
                                }
                            }).collect_view()
                        }}
                    </tr>
                </thead>
                <tbody>
                    {move || {
                        let vis_cols = visible_columns();
                        let navigate = navigate.clone();

                        page_rows().into_iter().map(|row| {
                            let link = row.row_link();
                            let has_link = link.is_some();
                            let navigate = navigate.clone();

                            let cells = vis_cols.iter().map(|col| {
                                let cell = row.cell_view(col.key);
                                view! {
                                    <td class="py-3 px-4">{cell}</td>
                                }
                            }).collect_view();

                            view! {
                                <tr
                                    class=format!(
                                        "border-b border-border-primary/50 hover:bg-surface-tertiary/50{}",
                                        if has_link { " cursor-pointer" } else { "" }
                                    )
                                    on:click=move |_| {
                                        if let Some(ref href) = link {
                                            let nav = navigate.clone();
                                            nav(href, leptos_router::NavigateOptions::default());
                                        }
                                    }
                                >
                                    {cells}
                                </tr>
                            }
                        }).collect_view()
                    }}
                </tbody>
            </table>
        </div>

        // --- Pagination footer ---
        <div class="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-2 mt-4 text-xs text-text-muted">
            <span>
                {move || {
                    let total = total_filtered();
                    let size = page_size.get();
                    let page = current_page.get();
                    let start = if total == 0 { 0 } else { page * size + 1 };
                    let end = ((page + 1) * size).min(total);
                    format!("Showing {}\u{2013}{} of {} rows", start, end, total)
                }}
            </span>
            <div class="flex items-center gap-2">
                <button
                    on:click=on_prev
                    prop:disabled=prev_disabled
                    class="px-2 py-1 text-xs text-text-muted border border-border-primary rounded-md hover:text-text-secondary disabled:opacity-30 disabled:cursor-not-allowed"
                >
                    "Prev"
                </button>
                <span class="text-text-muted text-xs">
                    {move || page_label.get()}
                </span>
                <button
                    on:click=on_next
                    prop:disabled=next_disabled
                    class="px-2 py-1 text-xs text-text-muted border border-border-primary rounded-md hover:text-text-secondary disabled:opacity-30 disabled:cursor-not-allowed"
                >
                    "Next"
                </button>
            </div>
        </div>
    }
}
