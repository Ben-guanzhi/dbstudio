//! The Structure object tab: renders the selected table's schema (columns,
//! indexes, foreign keys, DDL) as the center-pane sibling of the Data grid.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Icon, IconName, StyledExt as _, h_flex, label::Label, v_flex,
};

use crate::state::AppState;

/// Read-only schema inspector for the session's current table object.
pub struct StructureView {
    window_id: u64,
    _subscriptions: Vec<Subscription>,
}

impl StructureView {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let window_id = window.window_handle().window_id().as_u64();
        cx.new(|cx| {
            let _subscriptions = vec![cx.observe_global::<AppState>(|_this, cx| cx.notify())];
            Self {
                window_id,
                _subscriptions,
            }
        })
    }

    fn empty_state(message: &'static str, cx: &Context<Self>) -> AnyElement {
        div()
            .id("structure-empty")
            .flex_1()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(message)
            .into_any_element()
    }

    fn section_title(title: &'static str, cx: &Context<Self>) -> Label {
        Label::new(title)
            .text_sm()
            .font_semibold()
            .text_color(cx.theme().muted_foreground)
    }

    fn muted(text: impl Into<SharedString>, cx: &Context<Self>) -> Label {
        Label::new(text)
            .text_xs()
            .text_color(cx.theme().muted_foreground)
    }

    fn chip(text: impl Into<SharedString>, cx: &Context<Self>) -> Div {
        div()
            .px_1()
            .py_1()
            .rounded(cx.theme().radius)
            .bg(cx.theme().tiles)
            .border_1()
            .border_color(cx.theme().border)
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(Label::new(text))
    }
}

impl Render for StructureView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = cx.global::<AppState>();
        let Some((table, schema_prefix)) = state.current_object_for(self.window_id) else {
            return Self::empty_state("Select a table to view its structure.", cx);
        };
        let Some(schema) = state
            .table_schemas_for(self.window_id)
            .get(&table)
            .cloned()
        else {
            return Self::empty_state("Loading schema...", cx);
        };

        let header = h_flex()
            .items_center()
            .gap_2()
            .child(
                Icon::new(IconName::Frame)
                    .size_4()
                    .text_color(cx.theme().accent),
            )
            .child(
                Label::new(table.clone())
                    .text_base()
                    .font_semibold()
                    .text_color(cx.theme().foreground),
            )
            .children(
                schema_prefix
                    .filter(|s| !s.is_empty())
                    .map(|s| Self::muted(format!("({s})"), cx)),
            );

        let columns = if schema.columns.is_empty() {
            Self::muted("No columns.", cx).into_any_element()
        } else {
            v_flex()
                .gap_1()
                .children(schema.columns.iter().enumerate().map(|(ix, col)| {
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded(cx.theme().radius)
                        .when(ix % 2 == 0, |this| this.bg(cx.theme().list_even))
                        .child(
                            div().w(px(200.0)).min_w_0().child(
                                Label::new(col.name.clone())
                                    .text_sm()
                                    .font_medium()
                                    .text_color(cx.theme().foreground),
                            ),
                        )
                        .child(div().w(px(150.0)).child(Self::muted(col.data_type.clone(), cx)))
                        .when(col.is_primary_key, |this| this.child(Self::chip("PK", cx)))
                        .when(!col.nullable, |this| this.child(Self::chip("NOT NULL", cx)))
                        .when_some(col.default_value.as_deref(), |this, default| {
                            this.child(Self::chip(format!("DEFAULT {default}"), cx))
                        })
                        .when_some(col.comment.as_deref(), |this, comment| {
                            this.child(Self::muted(comment, cx))
                        })
                }))
                .into_any_element()
        };

        let indexes = if schema.indexes.is_empty() {
            Self::muted("No indexes.", cx).into_any_element()
        } else {
            v_flex()
                .gap_1()
                .children(schema.indexes.iter().map(|index| {
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .child(Label::new(index.name.clone()).text_sm())
                        .child(Self::muted(index.columns.join(", "), cx))
                        .when(index.is_primary, |this| this.child(Self::chip("PRIMARY", cx)))
                        .when(index.is_unique, |this| this.child(Self::chip("UNIQUE", cx)))
                        .when_some(index.index_type.as_deref(), |this, kind| {
                            this.child(Self::muted(kind, cx))
                        })
                }))
                .into_any_element()
        };

        let foreign_keys = if schema.foreign_keys.is_empty() {
            Self::muted("No foreign keys.", cx).into_any_element()
        } else {
            v_flex()
                .gap_1()
                .children(schema.foreign_keys.iter().map(|fk| {
                    let target = format!("{}({})", fk.target_table, fk.target_columns.join(", "));
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .child(Label::new(fk.name.clone()).text_sm())
                        .child(Self::muted(fk.source_columns.join(", "), cx))
                        .child(
                            Label::new("→")
                                .text_xs()
                                .text_color(cx.theme().muted_foreground),
                        )
                        .child(Label::new(target).text_sm().font_medium())
                        .when_some(fk.on_delete.as_deref(), |this, action| {
                            this.child(Self::chip(format!("ON DELETE {action}"), cx))
                        })
                        .when_some(fk.on_update.as_deref(), |this, action| {
                            this.child(Self::chip(format!("ON UPDATE {action}"), cx))
                        })
                }))
                .into_any_element()
        };

        v_flex()
            .id("structure-view")
            .flex_1()
            .min_h_0()
            .w_full()
            .p_4()
            .gap_2()
            .overflow_y_scroll()
            .child(header)
            .child(Self::section_title("Columns", cx))
            .child(columns)
            .child(Self::section_title("Indexes", cx))
            .child(indexes)
            .child(Self::section_title("Foreign Keys", cx))
            .child(foreign_keys)
            .child(Self::section_title("DDL", cx))
            .child(
                schema
                    .create_sql
                    .as_deref()
                    .map(|ddl| {
                        div()
                            .id("structure-ddl")
                            .w_full()
                            .p_2()
                            .rounded(cx.theme().radius)
                            .bg(cx.theme().tiles)
                            .border_1()
                            .border_color(cx.theme().border)
                            .text_xs()
                            .font_family("monospace")
                            .text_color(cx.theme().foreground)
                            .child(ddl.to_string())
                            .into_any_element()
                    })
                    .unwrap_or_else(|| Self::muted("No DDL available.", cx).into_any_element()),
            )
            .into_any_element()
    }
}