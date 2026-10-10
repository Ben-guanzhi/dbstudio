use dbstudio_core::schema::TableInfo;
use gpui::*;
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    label::Label,
    list::ListItem,
    menu::PopupMenuItem,
    tree::{tree, TreeEntry, TreeItem, TreeState},
    v_flex, ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
};
use std::sync::Arc;

use crate::fold::contains_ignore_case;
use crate::state::{AppState, ObjectSegment};
use crate::utils::truncate_str;

#[derive(Debug, Clone)]
pub struct SelectedTable {
    pub name: String,
    pub schema: Option<String>,
    pub table_type: String,
}

pub enum TablesEvent {
    TableSelected(SelectedTable),
    /// Open `table` in one of the center pane's object segments.
    OpenObject {
        table: SelectedTable,
        segment: ObjectSegment,
    },
}

impl EventEmitter<TablesEvent> for TablesTree {}

actions!(tables_tree, [SelectItem]);

pub struct TablesTree {
    window_id: u64,
    tree_state: Entity<TreeState>,
    search_input: Entity<InputState>,
    selected_item: Option<TreeItem>,
    has_tables: bool,
    tables: Arc<Vec<TableInfo>>,
    search_query: String,
    _subscriptions: Vec<Subscription>,
}

impl TablesTree {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let window_id = window.window_handle().window_id().as_u64();
        cx.new(|cx| Self::new(window_id, window, cx))
    }

    fn new(window_id: u64, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tree_state = cx.new(|cx| TreeState::new(cx));
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search tables..."));

        let _subscriptions = vec![
            cx.observe_global::<AppState>(move |this, cx| {
                // This handler fires on every write to `AppState`, most of
                // which leave the table list alone; rebuilding the tree then
                // would re-clone and re-format every table for nothing. The
                // shared handle says "nothing could have moved" in O(1); when
                // the handle is new the content still decides, so a refresh
                // that returned the same schema leaves the tree alone.
                let changed = {
                    let state = cx.global::<AppState>();
                    let tables = state.tables_arc_for(this.window_id);
                    if Arc::ptr_eq(tables, &this.tables) {
                        false
                    } else {
                        let moved = tables.as_slice() != this.tables.as_slice();
                        this.tables = Arc::clone(tables);
                        this.has_tables = !this.tables.is_empty();
                        moved
                    }
                };
                if changed {
                    this.rebuild_items(cx);
                } else {
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &search_input,
                window,
                |this, _, event: &InputEvent, _window, cx| {
                    if let InputEvent::Change = event {
                        this.search_query = this.search_input.read(cx).value().to_string();
                        this.rebuild_items(cx);
                    }
                },
            ),
        ];

        Self {
            window_id,
            tree_state,
            search_input,
            selected_item: None,
            has_tables: !cx.global::<AppState>().tables_for(window_id).is_empty(),
            tables: Arc::clone(cx.global::<AppState>().tables_arc_for(window_id)),
            search_query: String::new(),
            _subscriptions,
        }
    }

    fn rebuild_items(&mut self, cx: &mut Context<Self>) {
        let query = self.search_query.trim().to_lowercase();
        let items = build_tree_items(&self.tables, &query);
        self.tree_state.update(cx, |state, cx| {
            state.set_items(items, cx);
            cx.notify();
        });
        cx.notify();
    }

    fn on_select_item(&mut self, _: &SelectItem, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(entry) = self.tree_state.read(cx).selected_entry() {
            let item = entry.item().clone();
            self.selected_item = Some(item.clone());
            self.emit_table_selected(&item, cx);
            cx.notify();
        }
    }

    /// Emit a `TableSelected` event for `item` (a `{schema}:{name}:{table_type}` id).
    ///
    /// Click handling must pass the clicked item here directly: reading
    /// `tree_state.selected_entry()` inside the row's own `on_click` would
    /// observe the *previous* selection, because the list updates its internal
    /// selection only after the row handler runs.
    fn emit_table_selected(&mut self, item: &TreeItem, cx: &mut Context<Self>) {
        let id = item.id.as_ref();
        let parts: Vec<&str> = id.rsplitn(3, ':').collect();
        if let [table_type, name, schema] = parts.as_slice() {
            cx.emit(TablesEvent::TableSelected(SelectedTable {
                name: name.to_string(),
                schema: if schema.is_empty() {
                    None
                } else {
                    Some(schema.to_string())
                },
                table_type: table_type.to_string(),
            }));
        }
    }

    fn render_tree_item(
        &self,
        ix: usize,
        entry: &TreeEntry,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> ListItem {
        let item = entry.item();
        let is_selected = selected;

        let is_folder = entry.is_folder();
        let is_view = item.id.contains(":view");
        let is_materialized = item.id.contains(":materialized_view");

        let name = truncate_str(item.label.as_str(), 23);
        let prefix = if is_folder {
            "SCHEMA"
        } else if is_view || is_materialized {
            "VIEW"
        } else {
            "TABLE"
        };

        let text_color = if is_selected {
            cx.theme().accent_foreground
        } else {
            cx.theme().foreground
        };

        let bg_color = if is_selected {
            cx.theme().list_active
        } else if ix.is_multiple_of(2) {
            cx.theme().colors.list
        } else {
            cx.theme().list_even
        };

        let icon = if is_folder {
            if entry.is_expanded() {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            }
        } else if is_view || is_materialized {
            IconName::Eye
        } else {
            IconName::Frame
        };

        ListItem::new(ix)
            .w_full()
            .py_1()
            .px_3()
            .pl(px(16.0) * entry.depth() + px(12.0))
            .bg(bg_color)
            .border_1()
            .border_color(if is_selected {
                cx.theme().list_active_border
            } else {
                bg_color
            })
            .rounded(cx.theme().radius)
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        h_flex()
                            .min_w_0()
                            .items_center()
                            .gap_2()
                            .text_color(text_color)
                            .child(Icon::new(icon).size_4().text_color(text_color.opacity(0.7)))
                            .child(Label::new(name).text_sm().whitespace_nowrap()),
                    )
                    .child(
                        Label::new(prefix)
                            .text_xs()
                            .text_color(text_color.opacity(0.6)),
                    ),
            )
            .on_click(cx.listener({
                let item = item.clone();
                move |this, _, _window, cx| {
                    this.selected_item = Some(item.clone());
                    this.emit_table_selected(&item, cx);
                    cx.notify();
                }
            }))
    }
}

/// Parse a leaf tree id (`{schema}:{name}:{table_type}`) into its parts.
///
/// Schema group ids (`schema:{label}`) and malformed ids return `None`.
fn parse_table_id(id: &str) -> Option<(String, String, String)> {
    let parts: Vec<&str> = id.rsplitn(3, ':').collect();
    match parts.as_slice() {
        [table_type, name, schema] => {
            Some((schema.to_string(), name.to_string(), table_type.to_string()))
        }
        _ => None,
    }
}

/// Quote `schema.name` (either part may be empty) for the active connection.
fn qualify_table(schema: &str, name: &str, window_id: u64, cx: &App) -> String {
    if schema.is_empty() {
        crate::state::quote_ident(name, window_id, cx)
    } else {
        format!(
            "{}.{}",
            crate::state::quote_ident(schema, window_id, cx),
            crate::state::quote_ident(name, window_id, cx)
        )
    }
}

/// Build an "Open …" menu item that emits [`TablesEvent::OpenObject`].
fn open_object_item(
    view: &Entity<TablesTree>,
    table: &SelectedTable,
    segment: ObjectSegment,
    label: &'static str,
) -> PopupMenuItem {
    let view = view.clone();
    let table = table.clone();
    PopupMenuItem::new(label).on_click(move |_, _, cx| {
        view.update(cx, |_, cx| {
            cx.emit(TablesEvent::OpenObject {
                table: table.clone(),
                segment,
            });
        });
    })
}

fn build_tree_items(tables: &[TableInfo], search: &str) -> Vec<TreeItem> {
    let mut schema_tables: std::collections::BTreeMap<String, Vec<&TableInfo>> = Default::default();
    for t in tables {
        if !search.is_empty() && !contains_ignore_case(&t.name, search) {
            continue;
        }
        let key = t.schema.clone().unwrap_or_default();
        schema_tables.entry(key).or_default().push(t);
    }

    schema_tables
        .into_iter()
        .map(|(schema, mut items)| {
            items.sort_by(|a, b| a.name.cmp(&b.name));
            let children: Vec<TreeItem> = items
                .into_iter()
                .map(|t| {
                    let id = format!("{}:{}:{}", schema, t.name, t.table_type);
                    TreeItem::new(id, t.name.clone())
                })
                .collect();
            let schema_label = if schema.is_empty() {
                "Tables".to_string()
            } else {
                schema
            };
            TreeItem::new(format!("schema:{}", schema_label), schema_label)
                .expanded(true)
                .children(children)
        })
        .collect()
}

impl Render for TablesTree {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let menu_view = view.clone();
        let window_id = self.window_id;

        let refresh_button = Button::new("tables-refresh")
            .label("Refresh")
            .small()
            .ghost()
            .disabled(!self.has_tables)
            .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                crate::state::refresh_tables(this.window_id, cx);
            }));

        let header = div()
            .h_flex()
            .justify_between()
            .items_center()
            .child(Label::new("Tables").font_bold().text_base())
            .child(refresh_button);

        v_flex()
            .id("tables-tree")
            .flex_1()
            .min_h_0()
            .p_2()
            .on_action(cx.listener(Self::on_select_item))
            .child(header)
            .child(
                div().h_flex().gap_1().px_1().pb_1().child(
                    Input::new(&self.search_input)
                        .w_full()
                        .small()
                        .prefix(Icon::new(IconName::Search).size_4())
                        .cleanable(true),
                ),
            )
            .child(
                tree(&self.tree_state, move |ix, entry, selected, _window, cx| {
                    view.update(cx, |this, cx| {
                        this.render_tree_item(ix, entry, selected, cx)
                    })
                })
                .p(px(4.0))
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_hidden()
                .border_1()
                .border_color(cx.theme().border)
                .rounded(cx.theme().radius)
                .context_menu(move |_ix, entry, menu, _window, _menu_cx| {
                    let Some((schema, name, table_type)) = parse_table_id(entry.item().id.as_ref())
                    else {
                        return menu;
                    };
                    let table = SelectedTable {
                        name: name.clone(),
                        schema: if schema.is_empty() {
                            None
                        } else {
                            Some(schema.clone())
                        },
                        table_type: table_type.clone(),
                    };
                    let mut menu = menu
                        .item(open_object_item(
                            &menu_view,
                            &table,
                            ObjectSegment::Data,
                            "Open Data",
                        ))
                        .item(open_object_item(
                            &menu_view,
                            &table,
                            ObjectSegment::Structure,
                            "Open Structure",
                        ))
                        .item(open_object_item(
                            &menu_view,
                            &table,
                            ObjectSegment::Query,
                            "Open Query",
                        ))
                        .separator()
                        .item({
                            let name = name.clone();
                            PopupMenuItem::new("Copy Name").on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(name.clone()));
                            })
                        })
                        .item(PopupMenuItem::new("Refresh").on_click(move |_, _, cx| {
                            crate::state::refresh_tables(window_id, cx);
                        }));
                    // Export entries: native save dialog, then dump off the UI
                    // thread (single table or the whole database).
                    menu = menu
                        .separator()
                        .item({
                            let e_name = name.clone();
                            let e_win = window_id;
                            PopupMenuItem::new("Export Table…").on_click(move |_, _, cx| {
                                let default_name = format!("{}.sql", e_name);
                                let rx = cx.prompt_for_new_path(
                                    &std::env::current_dir().unwrap_or_default(),
                                    Some(&default_name),
                                );
                                let table = e_name.clone();
                                let window_id = e_win;
                                cx.spawn(async move |cx| {
                                    let Some(path) = rx.await.ok().and_then(Result::ok).flatten()
                                    else {
                                        return;
                                    };
                                    crate::state::export_table(
                                        &path.to_string_lossy(),
                                        &table,
                                        window_id,
                                        cx,
                                    );
                                })
                                .detach();
                            })
                        })
                        .item({
                            let e_win = window_id;
                            PopupMenuItem::new("Export Database…").on_click(move |_, _, cx| {
                                let rx = cx.prompt_for_new_path(
                                    &std::env::current_dir().unwrap_or_default(),
                                    Some("database.sql"),
                                );
                                let window_id = e_win;
                                cx.spawn(async move |cx| {
                                    let Some(path) = rx.await.ok().and_then(Result::ok).flatten()
                                    else {
                                        return;
                                    };
                                    crate::state::export_database(
                                        &path.to_string_lossy(),
                                        window_id,
                                        cx,
                                    );
                                })
                                .detach();
                            })
                        });
                    // Destructive actions: base tables only, routed through
                    // execute_query so safe-mode + confirmation still apply.
                    if table_type == "table" || table_type.eq_ignore_ascii_case("BASE TABLE") {
                        let t_schema = schema.clone();
                        let t_name = name.clone();
                        let d_schema = schema;
                        let d_name = name;
                        menu = menu
                            .separator()
                            .item(PopupMenuItem::new("Truncate").on_click(move |_, _, cx| {
                                let target = qualify_table(&t_schema, &t_name, window_id, cx);
                                let db_type = cx
                                    .global::<AppState>()
                                    .active_connection_for(window_id)
                                    .map(|c| c.db_type());
                                // With no live connection there is no dialect to
                                // consult, and the standard form is sent.
                                let sql = match db_type {
                                    Some(t) => format!("{};", t.truncate_statement(&target)),
                                    None => format!("TRUNCATE TABLE {target};"),
                                };
                                crate::state::execute_query(sql, window_id, cx);
                            }))
                            .item(PopupMenuItem::new("Drop").on_click(move |_, _, cx| {
                                let target = qualify_table(&d_schema, &d_name, window_id, cx);
                                crate::state::execute_query(
                                    format!("DROP TABLE {target};"),
                                    window_id,
                                    cx,
                                );
                            }));
                    }
                    menu
                }),
            )
    }
}
