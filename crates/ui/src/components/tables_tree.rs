use dbstudio_core::schema::TableInfo;
use gpui::*;
use gpui_component::{
    ActiveTheme as _,
    Disableable as _,
    Icon,
    IconName,
    Sizable as _,
    StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    label::Label,
    list::ListItem,
    tree::{TreeEntry, TreeItem, TreeState, tree},
    v_flex,
};

use crate::state::AppState;
use crate::utils::truncate_str;

#[derive(Debug, Clone)]
pub struct SelectedTable {
    pub name: String,
    pub schema: Option<String>,
    pub table_type: String,
}

pub enum TablesEvent {
    TableSelected(SelectedTable),
}

impl EventEmitter<TablesEvent> for TablesTree {}

actions!(tables_tree, [SelectItem]);

pub struct TablesTree {
    window_id: u64,
    tree_state: Entity<TreeState>,
    search_input: Entity<InputState>,
    selected_item: Option<TreeItem>,
    has_tables: bool,
    tables: Vec<TableInfo>,
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
        let search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search tables...")
        });

        let _subscriptions = vec![
            cx.observe_global::<AppState>(move |this, cx| {
                let tables = cx.global::<AppState>().tables_for(this.window_id).to_vec();
                this.tables = tables;
                this.has_tables = !this.tables.is_empty();
                this.rebuild_items(cx);
            }),
            cx.subscribe_in(&search_input, window, |this, _, event: &InputEvent, _window, cx| {
                if let InputEvent::Change = event {
                    this.search_query = this.search_input.read(cx).value().to_string();
                    this.rebuild_items(cx);
                }
            }),
        ];

        Self {
            window_id,
            tree_state,
            search_input,
            selected_item: None,
            has_tables: !cx.global::<AppState>().tables_for(window_id).is_empty(),
            tables: cx.global::<AppState>().tables_for(window_id).to_vec(),
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

    fn on_select_item(
        &mut self,
        _: &SelectItem,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    fn render_tree_item(&self, ix: usize, entry: &TreeEntry, selected: bool, cx: &mut Context<Self>) -> ListItem {
        let item = entry.item();
        let is_selected = selected;

        let is_folder = entry.is_folder();
        let is_view = item.id.contains(":view");
        let is_materialized = item.id.contains(":materialized_view");

        let name = truncate_str(item.label.as_str(), 23);
        let prefix = if is_folder { "SCHEMA" } else if is_view || is_materialized { "VIEW" } else { "TABLE" };

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

fn build_tree_items(tables: &[TableInfo], search: &str) -> Vec<TreeItem> {
    let mut schema_tables: std::collections::BTreeMap<String, Vec<&TableInfo>> = Default::default();
    for t in tables {
        if !search.is_empty() && !t.name.to_lowercase().contains(search) {
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
            let schema_label = if schema.is_empty() { "Tables".to_string() } else { schema };
            TreeItem::new(format!("schema:{}", schema_label), schema_label)
                .expanded(true)
                .children(children)
        })
        .collect()
}

impl Render for TablesTree {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();

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
                div()
                    .h_flex()
                    .gap_1()
                    .px_1()
                    .pb_1()
                    .child(
                        Input::new(&self.search_input)
                            .w_full()
                            .small()
                            .prefix(Icon::new(IconName::Search).size_4())
                            .cleanable(true),
                    ),
            )
            .child(
                tree(&self.tree_state, move |ix, entry, selected, _window, cx| {
                    view.update(cx, |this, cx| this.render_tree_item(ix, entry, selected, cx))
                })
                .p(px(4.0))
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_hidden()
                .border_1()
                .border_color(cx.theme().border)
                .rounded(cx.theme().radius),
            )
    }
}
