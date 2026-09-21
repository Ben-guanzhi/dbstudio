use crate::{
    DuplicateLine, FormatSql, MoveLineDown, MoveLineUp, OpenCommandPalette, RedoLastEdit,
    ToggleAiPanel, ToggleComment, UndoLastEdit,
};
use dbstudio_storage::types::ConnectionInfo;
use dbstudio_ui::components::ai_panel::AiPanel;
use dbstudio_ui::components::command_palette::{CommandPalette, CommandPaletteEvent};
use dbstudio_ui::components::connection_form::{ConnectionForm, ConnectionFormEvent};
use dbstudio_ui::components::connection_list::{ConnectionList, ConnectionListEvent};
use dbstudio_ui::components::footer_bar::{FooterBar, FooterEvent};
use dbstudio_ui::components::header_bar::{HeaderBar, HeaderEvent};
use dbstudio_ui::components::history_panel::{HistoryPanel, HistoryPanelEvent};
use dbstudio_ui::components::results_panel::ResultsPanel;
use dbstudio_ui::components::sql_editor::{Editor, EditorEvent};
use dbstudio_ui::components::tables_tree::{TablesEvent, TablesTree};
use dbstudio_ui::components::tabs::{
    TabClose, TabCloseOthers, TabDuplicateInNewWindow, TabsBar, TabsEvent,
};
use dbstudio_ui::state::{
    connect, delete_connection, execute_query, export_database, import_database, load_table_schema,
    select_database, AppState, ConnectionStatus,
};
use dbstudio_ui::utils::toolbar_divider;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    resizable::{resizable_panel, v_resizable},
    spinner::Spinner,
    v_flex, ActiveTheme as _, Icon, IconName, Root, Sizable as _, StyledExt as _,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Workspace {
    window_id: u64,
    header: Entity<HeaderBar>,
    tabs: Entity<TabsBar>,
    connections: Entity<ConnectionList>,
    tables: Entity<TablesTree>,
    editor: Entity<Editor>,
    results: Entity<ResultsPanel>,
    history: Entity<HistoryPanel>,
    ai_panel: Entity<AiPanel>,
    footer: Entity<FooterBar>,
    command_palette: Entity<CommandPalette>,
    selected_connection: Option<ConnectionInfo>,
    inline_form: bool,
    show_form: bool,
    show_ai_panel: bool,
    form: Option<Entity<ConnectionForm>>,
    is_active: bool,
    connecting: bool,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    fn create_form(
        &mut self,
        editing: Option<ConnectionInfo>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ConnectionForm> {
        let form = ConnectionForm::view(editing, window, cx);
        cx.subscribe_in(
            &form,
            window,
            |this, _entity, event, _window, cx| match event {
                ConnectionFormEvent::Saved | ConnectionFormEvent::Canceled => {
                    this.form = None;
                    this.show_form = false;
                    this.inline_form = false;
                    cx.notify();
                }
            },
        )
        .detach();
        form
    }

    fn open_form(
        &mut self,
        editing: Option<ConnectionInfo>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.form = Some(self.create_form(editing, window, cx));
        self.show_form = true;
        self.inline_form = false;
        cx.notify();
    }

    fn open_home_form(
        &mut self,
        editing: Option<ConnectionInfo>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected_connection = None;
        self.form = Some(self.create_form(editing, window, cx));
        self.inline_form = true;
        self.show_form = false;
        cx.notify();
    }

    fn on_header_event(
        &mut self,
        _: &Entity<HeaderBar>,
        event: &HeaderEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            HeaderEvent::ToggleTheme => crate::themes::toggle_color_mode(Some(window), cx),
            HeaderEvent::ToggleTables => {
                dbstudio_ui::state::toggle_tables(self.window_id, cx);
                cx.notify();
            }
            HeaderEvent::ToggleHistory => {
                dbstudio_ui::state::toggle_history(self.window_id, cx);
                cx.notify();
            }
            HeaderEvent::ToggleAi => self.toggle_ai_panel(cx),
            HeaderEvent::OpenPalette => self.open_command_palette(cx),
            HeaderEvent::NewWindow => crate::open_workspace_window(cx),
            HeaderEvent::NewConnection => {
                if self.is_active {
                    if !self.show_form {
                        self.open_form(None, window, cx);
                    }
                } else if !self.inline_form {
                    self.open_home_form(None, window, cx);
                }
            }
        }
    }

    fn on_connection_event(
        &mut self,
        _: &Entity<ConnectionList>,
        event: &ConnectionListEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ConnectionListEvent::Selected(info) = event;
        self.selected_connection = Some(info.clone());
        self.form = None;
        self.inline_form = false;
        self.show_form = false;
        cx.notify();
    }

    fn on_tables_event(
        &mut self,
        _: &Entity<TablesTree>,
        event: &TablesEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let TablesEvent::TableSelected(table) = event;
        let table = table.clone();
        load_table_schema(&table.name, table.schema.as_deref(), self.window_id, cx);
        let query = dbstudio_ui::state::build_select_query(
            &table.name,
            table.schema.as_deref(),
            self.window_id,
            cx,
        );
        self.editor
            .update(cx, |this, cx| this.set_query(query.clone(), window, cx));
        self.results.update(cx, |this, cx| {
            this.select_table(table.name.clone(), None, cx)
        });
        execute_query(query, self.window_id, cx);
    }

    fn on_editor_event(
        &mut self,
        _: &Entity<Editor>,
        event: &EditorEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let EditorEvent::ExecuteQuery(sql) = event;
        execute_query(sql.clone(), self.window_id, cx);
    }

    fn on_command_palette_event(
        &mut self,
        _: &Entity<CommandPalette>,
        event: &CommandPaletteEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            CommandPaletteEvent::Execute(sql) => {
                if sql.is_empty() {
                    let query = self
                        .editor
                        .read(cx)
                        .input_state
                        .read(cx)
                        .value()
                        .to_string();
                    if !query.trim().is_empty() {
                        execute_query(query, self.window_id, cx);
                    }
                } else {
                    execute_query(sql.clone(), self.window_id, cx);
                }
            }
            CommandPaletteEvent::SelectTable(table) => {
                let table = table.clone();
                let schema = cx
                    .global::<AppState>()
                    .tables_for(self.window_id)
                    .iter()
                    .find(|t| t.name == table)
                    .and_then(|t| t.schema.clone());
                load_table_schema(&table, schema.as_deref(), self.window_id, cx);
                let query = dbstudio_ui::state::build_select_query(
                    &table,
                    schema.as_deref(),
                    self.window_id,
                    cx,
                );
                self.editor
                    .update(cx, |this, cx| this.set_query(query.clone(), window, cx));
                self.results
                    .update(cx, |this, cx| this.select_table(table.clone(), None, cx));
                execute_query(query, self.window_id, cx);
            }
            CommandPaletteEvent::SelectDatabase(database) => {
                select_database(database, self.window_id, cx);
            }
            CommandPaletteEvent::RunCommand(cmd) => match cmd.as_str() {
                "editor-format" => {
                    self.editor.update(cx, |editor, cx| {
                        editor.format_sql(window, cx);
                    });
                }
                "toggle-ai" => {
                    self.toggle_ai_panel(cx);
                }
                "toggle-theme" => {
                    crate::themes::toggle_color_mode(Some(window), cx);
                }
                "export-csv" => {
                    self.results.update(cx, |this, cx| this.export_csv(cx));
                }
                "export-json" => {
                    self.results.update(cx, |this, cx| this.export_json(cx));
                }
                _ => {}
            },
            CommandPaletteEvent::Close => {
                self.command_palette.update(cx, |palette, cx| {
                    palette.close(cx);
                });
            }
        }
    }

    fn on_footer_event(
        &mut self,
        _: &Entity<FooterBar>,
        event: &FooterEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            FooterEvent::ExportDatabase => {
                // Native save dialog; the granted path is handled off the UI thread.
                let rx = cx.prompt_for_new_path(
                    &std::env::current_dir().unwrap_or_default(),
                    Some("database.sql"),
                );
                let window_id = self.window_id;
                cx.spawn(async move |_this, cx| {
                    let Some(path) = rx.await.ok().and_then(Result::ok).flatten() else {
                        return;
                    };
                    export_database(&path.to_string_lossy(), window_id, cx);
                })
                .detach();
            }
            FooterEvent::ImportDatabase => {
                let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: Some("Select a SQL dump to import".into()),
                });
                let window_id = self.window_id;
                cx.spawn(async move |_this, cx| {
                    let Some(paths) = rx.await.ok().and_then(Result::ok).flatten() else {
                        return;
                    };
                    let Some(path) = paths.first() else {
                        return;
                    };
                    import_database(&path.to_string_lossy(), window_id, cx);
                })
                .detach();
            }
        }
    }

    fn on_tabs_event(
        &mut self,
        _: &Entity<TabsBar>,
        event: &TabsEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TabsEvent::Select(id) => {
                dbstudio_ui::state::switch_session(*id, self.window_id, cx);
            }
            TabsEvent::Close(id) => {
                dbstudio_ui::state::close_session(*id, cx);
            }
            TabsEvent::CloseOthers(id) => {
                let sessions_to_close: Vec<u64> = cx
                    .global::<AppState>()
                    .sessions
                    .iter()
                    .filter(|s| s.id != *id)
                    .map(|s| s.id)
                    .collect();
                for sid in sessions_to_close {
                    dbstudio_ui::state::close_session(sid, cx);
                }
            }
            TabsEvent::DuplicateInNewWindow(id) => {
                let session_id = *id;
                crate::open_workspace_window(cx);
                // The new window starts with its own session; switch it to the duplicated one.
                let new_window_id = cx.global::<AppState>().windows.keys().max().copied();
                if let Some(new_wid) = new_window_id {
                    dbstudio_ui::state::switch_session(session_id, new_wid, cx);
                }
            }
            TabsEvent::NewTab => {
                if self.is_active {
                    if !self.show_form {
                        self.open_form(None, window, cx);
                    }
                } else if !self.inline_form {
                    self.open_home_form(None, window, cx);
                }
            }
        }
        cx.notify();
    }

    fn on_history_event(
        &mut self,
        _: &Entity<HistoryPanel>,
        event: &HistoryPanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let HistoryPanelEvent::LoadSql(sql) = event;
        let sql = sql.clone();
        dbstudio_ui::state::set_active_editor_text(&sql, self.window_id, cx);
        self.editor
            .update(cx, |this, cx| this.set_query(sql, window, cx));
    }

    pub fn open_command_palette(&mut self, cx: &mut Context<Self>) {
        self.command_palette.update(cx, |palette, cx| {
            palette.open(cx);
        });
    }

    pub fn toggle_ai_panel(&mut self, cx: &mut Context<Self>) {
        self.show_ai_panel = !self.show_ai_panel;
        cx.notify();
    }

    fn on_connect_selected(
        &mut self,
        info: &ConnectionInfo,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        connect(info, self.window_id, cx);
        self.selected_connection = None;
        self.connections
            .update(cx, |list, cx| list.set_selected(None, cx));
        cx.notify();
    }

    fn on_edit_selected(
        &mut self,
        info: &ConnectionInfo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected_connection = None;
        self.open_home_form(Some(info.clone()), window, cx);
    }

    fn on_delete_selected(
        &mut self,
        info: &ConnectionInfo,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        delete_connection(info.id.clone(), cx);
        self.selected_connection = None;
        self.connections
            .update(cx, |list, cx| list.set_selected(None, cx));
        cx.notify();
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let header = HeaderBar::view(window, cx);
        let tabs = TabsBar::view(window, cx);
        let connections = ConnectionList::view(window, cx);
        let tables = TablesTree::view(window, cx);
        let editor = Editor::view(window, cx);
        let results = ResultsPanel::view(window, cx);
        let history = HistoryPanel::view(window, cx);
        let ai_panel = AiPanel::view(window, cx);
        let footer = FooterBar::view(window, cx);
        let command_palette = CommandPalette::view(window, cx);

        cx.subscribe_in(&header, window, Self::on_header_event)
            .detach();
        cx.subscribe_in(&tabs, window, Self::on_tabs_event).detach();
        cx.subscribe_in(&connections, window, Self::on_connection_event)
            .detach();
        cx.subscribe_in(&tables, window, Self::on_tables_event)
            .detach();
        cx.subscribe_in(&editor, window, Self::on_editor_event)
            .detach();
        cx.subscribe_in(&command_palette, window, Self::on_command_palette_event)
            .detach();
        cx.subscribe_in(&footer, window, Self::on_footer_event)
            .detach();
        cx.subscribe_in(&history, window, Self::on_history_event)
            .detach();

        let window_id = window.window_handle().window_id().as_u64();
        let _subscriptions = vec![cx.observe_global::<AppState>(move |this, cx| {
            let state = cx.global::<AppState>();
            let active = state.active_connection_for(window_id).is_some();
            this.connecting = state.connection_state_for(window_id) == ConnectionStatus::Connecting;
            if active && !this.is_active {
                this.form = None;
                this.show_form = false;
                this.inline_form = false;
                this.selected_connection = None;
            }
            this.is_active = active;
            cx.notify();
        })];

        let state = cx.global::<AppState>();
        Self {
            window_id,
            header,
            tabs,
            connections,
            tables,
            editor,
            results,
            history,
            ai_panel,
            footer,
            command_palette,
            selected_connection: None,
            inline_form: false,
            show_form: false,
            show_ai_panel: false,
            form: None,
            is_active: state.active_connection_for(window_id).is_some(),
            connecting: state.connection_state_for(window_id) == ConnectionStatus::Connecting,
            _subscriptions,
        }
    }

    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|cx| Self::new(window, cx));

        // Check for URL scheme argument
        if let Some(url_str) = dbstudio_ui::url_scheme::find_url_in_args() {
            if let Some(url) = dbstudio_ui::url_scheme::DbStudioUrl::parse(&url_str) {
                if url.is_connect() {
                    entity.update(cx, |workspace, cx| {
                        workspace.handle_url_connect(url, cx);
                    });
                }
            }
        }

        entity
    }

    fn handle_url_connect(
        &mut self,
        url: dbstudio_ui::url_scheme::DbStudioUrl,
        cx: &mut Context<Self>,
    ) {
        use dbstudio_core::models::{ConnectionConfig, DatabaseType};

        let db_type = match url.db_type() {
            Some("sqlite") => DatabaseType::SQLite,
            Some("mysql") => DatabaseType::MySQL,
            Some("postgresql") => DatabaseType::PostgreSQL,
            Some("mssql") => DatabaseType::MSSQL,
            Some("oracle") => DatabaseType::Oracle,
            _ => return,
        };

        let _password = url.password().unwrap_or("").to_string();

        let mut config = ConnectionConfig::new(
            db_type,
            format!("{} Connection", url.db_type().unwrap_or("Unknown")),
        );

        if let Some(host) = url.host() {
            config.host = host.to_string();
        }
        if let Some(port) = url.port() {
            config.port = port;
        }
        if let Some(database) = url.database() {
            config.database = database.to_string();
        }
        if let Some(username) = url.username() {
            config.username = username.to_string();
        }

        // Create a ConnectionInfo and connect
        let info = ConnectionInfo {
            id: config.id.clone(),
            name: config.name.clone(),
            db_type: config.db_type,
            host: config.host.clone(),
            port: config.port,
            database: config.database.clone(),
            username: config.username.clone(),
            color: config.color.clone(),
            environment: config.environment,
            ssl_mode: config.ssl_mode,
            group: config.group.clone(),
            tags: config.tags.clone(),
            ssh_enabled: config.ssh_enabled,
            ssh_host: config.ssh_host.clone(),
            ssh_port: config.ssh_port,
            ssh_username: config.ssh_username.clone(),
            ssh_auth_type: config.ssh_auth_type.clone(),
            ssh_key_path: config.ssh_key_path.clone(),
            extra_params: config.extra_params.clone(),
            plugin_name: config.plugin_name.clone(),
            created_at: config.created_at.clone(),
            updated_at: config.updated_at.clone(),
        };

        connect(&info, self.window_id, cx);
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = if self.is_active {
            self.render_workspace(cx)
        } else {
            self.render_home(window, cx)
        };

        if self.show_form {
            if let Some(form) = &self.form {
                root = root.child(
                    div()
                        .id("form-overlay")
                        .absolute()
                        .inset_0()
                        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.show_form = false;
                            this.form = None;
                            cx.notify();
                        }))
                        .child(div().absolute().inset_0().bg(gpui::black().opacity(0.4)))
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .items_center()
                                .justify_center()
                                .child(form.clone()),
                        ),
                );
            }
        }

        root = root.child(self.command_palette.clone());

        root.on_action(cx.listener(|this, _: &OpenCommandPalette, _window, cx| {
            this.open_command_palette(cx);
        }))
        .on_action(cx.listener(|this, _: &FormatSql, window, cx| {
            this.editor.update(cx, |editor, cx| {
                editor.format_sql(window, cx);
            });
        }))
        .on_action(cx.listener(|this, _: &ToggleComment, window, cx| {
            this.editor.update(cx, |editor, cx| {
                editor.toggle_comment(window, cx);
            });
        }))
        .on_action(cx.listener(|this, _: &DuplicateLine, window, cx| {
            this.editor.update(cx, |editor, cx| {
                editor.duplicate_line(window, cx);
            });
        }))
        .on_action(cx.listener(|this, _: &MoveLineUp, window, cx| {
            this.editor.update(cx, |editor, cx| {
                editor.move_line_up(window, cx);
            });
        }))
        .on_action(cx.listener(|this, _: &MoveLineDown, window, cx| {
            this.editor.update(cx, |editor, cx| {
                editor.move_line_down(window, cx);
            });
        }))
        .on_action(cx.listener(|this, _: &ToggleAiPanel, _window, cx| {
            this.toggle_ai_panel(cx);
        }))
        .on_action(cx.listener(|this, _: &UndoLastEdit, _window, cx| {
            dbstudio_ui::state::undo_last_edit(this.window_id, cx);
        }))
        .on_action(cx.listener(|this, _: &RedoLastEdit, _window, cx| {
            dbstudio_ui::state::redo_last_edit(this.window_id, cx);
        }))
        .on_action(cx.listener(|_this, action: &TabClose, _window, cx| {
            dbstudio_ui::state::close_session(action.id, cx);
            cx.notify();
        }))
        .on_action(cx.listener(|_this, action: &TabCloseOthers, _window, cx| {
            let sessions_to_close: Vec<u64> = cx
                .global::<AppState>()
                .sessions
                .iter()
                .filter(|s| s.id != action.id)
                .map(|s| s.id)
                .collect();
            for sid in sessions_to_close {
                dbstudio_ui::state::close_session(sid, cx);
            }
            cx.notify();
        }))
        .on_action(
            cx.listener(|_this, action: &TabDuplicateInNewWindow, _window, cx| {
                let session_id = action.id;
                crate::open_workspace_window(cx);
                let new_window_id = cx.global::<AppState>().windows.keys().max().copied();
                if let Some(new_wid) = new_window_id {
                    dbstudio_ui::state::switch_session(session_id, new_wid, cx);
                }
                cx.notify();
            }),
        )
        .children(Root::render_notification_layer(window, cx))
    }
}

mod home_view;
mod workspace_view;

fn window_control_buttons(cx: &mut Context<Workspace>) -> impl IntoElement {
    h_flex()
        .items_center()
        .gap_0()
        .child(
            Button::new("home-minimize")
                .icon(Icon::new(IconName::WindowMinimize).size_3_5())
                .ghost()
                .small()
                .on_click(cx.listener(|_, _: &ClickEvent, window, _cx| {
                    window.minimize_window();
                })),
        )
        .child(
            div()
                .id("home-maximize")
                .flex()
                .flex_shrink_0()
                .w(px(34.0))
                .h_full()
                .justify_center()
                .content_center()
                .items_center()
                .text_color(cx.theme().foreground)
                .hover(|style| {
                    style
                        .bg(cx.theme().list_active)
                        .text_color(cx.theme().foreground)
                })
                .active(|style| {
                    style
                        .bg(cx.theme().list_active)
                        .text_color(cx.theme().foreground)
                })
                .window_control_area(WindowControlArea::Max)
                .child(Icon::new(IconName::WindowMaximize).size_3_5()),
        )
        .child(
            Button::new("home-close")
                .icon(Icon::new(IconName::WindowClose).size_3_5())
                .ghost()
                .small()
                .on_click(cx.listener(|_, _: &ClickEvent, _window, cx| {
                    cx.quit();
                })),
        )
}
