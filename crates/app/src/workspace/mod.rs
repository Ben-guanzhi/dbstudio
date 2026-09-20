use dbstudio_storage::types::ConnectionInfo;
use crate::{FormatSql, OpenCommandPalette, RedoLastEdit, ToggleAiPanel, ToggleComment, UndoLastEdit};
use dbstudio_ui::components::ai_panel::AiPanel;
use dbstudio_ui::components::connection_form::{ConnectionForm, ConnectionFormEvent};
use dbstudio_ui::components::connection_list::{ConnectionList, ConnectionListEvent};
use dbstudio_ui::components::command_palette::{CommandPalette, CommandPaletteEvent};
use dbstudio_ui::components::footer_bar::{FooterBar, FooterEvent};
use dbstudio_ui::components::header_bar::{HeaderBar, HeaderEvent};
use dbstudio_ui::components::history_panel::HistoryPanel;
use dbstudio_ui::components::results_panel::ResultsPanel;
use dbstudio_ui::components::sql_editor::{Editor, EditorEvent};
use dbstudio_ui::components::tables_tree::{TablesEvent, TablesTree};
use dbstudio_ui::state::{
    AppState, ConnectionStatus, connect, delete_connection, execute_query, export_database,
    import_database, load_table_schema, select_database,
};
use dbstudio_ui::utils::toolbar_divider;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _,
    Icon,
    IconName,
    Root,
    Sizable as _,
    StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    resizable::{resizable_panel, v_resizable},
    spinner::Spinner,
    v_flex,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Workspace {
    header: Entity<HeaderBar>,
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
        cx.subscribe_in(&form, window, |this, _entity, event, _window, cx| match event {
            ConnectionFormEvent::Saved | ConnectionFormEvent::Canceled => {
                this.form = None;
                this.show_form = false;
                this.inline_form = false;
                cx.notify();
            }
        })
        .detach();
        form
    }

    fn open_form(&mut self, editing: Option<ConnectionInfo>, window: &mut Window, cx: &mut Context<Self>) {
        self.form = Some(self.create_form(editing, window, cx));
        self.show_form = true;
        self.inline_form = false;
        cx.notify();
    }

    fn open_home_form(&mut self, editing: Option<ConnectionInfo>, window: &mut Window, cx: &mut Context<Self>) {
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
        load_table_schema(&table.name, table.schema.as_deref(), cx);
        let query = dbstudio_ui::state::build_select_query(&table.name, table.schema.as_deref(), cx);
        self.editor
            .update(cx, |this, cx| this.set_query(query.clone(), window, cx));
        self.results
            .update(cx, |this, cx| this.select_table(table.name.clone(), None, cx));
        execute_query(query, cx);
    }

    fn on_editor_event(
        &mut self,
        _: &Entity<Editor>,
        event: &EditorEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let EditorEvent::ExecuteQuery(sql) = event;
        execute_query(sql.clone(), cx);
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
                        execute_query(query, cx);
                    }
                } else {
                    execute_query(sql.clone(), cx);
                }
            }
            CommandPaletteEvent::SelectTable(table) => {
                let table = table.clone();
                let schema = cx
                    .global::<AppState>()
                    .tables()
                    .iter()
                    .find(|t| t.name == table)
                    .map(|t| t.schema.clone())
                    .flatten();
                load_table_schema(&table, schema.as_deref(), cx);
                let query =
                    dbstudio_ui::state::build_select_query(&table, schema.as_deref(), cx);
                self.editor.update(cx, |this, cx| {
                    this.set_query(query.clone(), window, cx);
                });
                self.results
                    .update(cx, |this, cx| this.select_table(table.clone(), None, cx));
                execute_query(query, cx);
            }
            CommandPaletteEvent::SelectDatabase(database) => {
                select_database(database, cx);
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
                cx.spawn(async move |_this, cx| {
                    let Some(path) = rx.await.ok().and_then(Result::ok).flatten() else {
                        return;
                    };
                    export_database(&path.to_string_lossy(), cx);
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
                cx.spawn(async move |_this, cx| {
                    let Some(paths) = rx.await.ok().and_then(Result::ok).flatten() else {
                        return;
                    };
                    let Some(path) = paths.first() else {
                        return;
                    };
                    import_database(&path.to_string_lossy(), cx);
                })
                .detach();
            }
        }
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

    fn on_connect_selected(&mut self, info: &ConnectionInfo, _window: &mut Window, cx: &mut Context<Self>) {
        connect(info, cx);
        self.selected_connection = None;
        self.connections.update(cx, |list, cx| list.set_selected(None, cx));
        cx.notify();
    }

    fn on_edit_selected(&mut self, info: &ConnectionInfo, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_connection = None;
        self.open_home_form(Some(info.clone()), window, cx);
    }

    fn on_delete_selected(&mut self, info: &ConnectionInfo, _window: &mut Window, cx: &mut Context<Self>) {
        delete_connection(info.id.clone(), cx);
        self.selected_connection = None;
        self.connections.update(cx, |list, cx| list.set_selected(None, cx));
        cx.notify();
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let header = HeaderBar::view(window, cx);
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

        let _subscriptions = vec![cx.observe_global::<AppState>(move |this, cx| {
            let state = cx.global::<AppState>();
            let active = state.active_connection().is_some();
            this.connecting = state.connection_state() == ConnectionStatus::Connecting;
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
            header,
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
            is_active: state.active_connection().is_some(),
            connecting: state.connection_state() == ConnectionStatus::Connecting,
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

    fn handle_url_connect(&mut self, url: dbstudio_ui::url_scheme::DbStudioUrl, cx: &mut Context<Self>) {
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
        
        connect(&info, cx);
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
                root = root
                    .child(
                        div()
                            .id("form-overlay")
                            .absolute()
                            .inset_0()
                            .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                this.show_form = false;
                                this.form = None;
                                cx.notify();
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
        .on_action(cx.listener(|this, _: &ToggleAiPanel, _window, cx| {
            this.toggle_ai_panel(cx);
        }))
        .on_action(cx.listener(|_this, _: &UndoLastEdit, _window, cx| {
            dbstudio_ui::state::undo_last_edit(cx);
        }))
        .on_action(cx.listener(|_this, _: &RedoLastEdit, _window, cx| {
            dbstudio_ui::state::redo_last_edit(cx);
        }))

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
                .hover(|style| style.bg(cx.theme().list_active).text_color(cx.theme().foreground))
                .active(|style| style.bg(cx.theme().list_active).text_color(cx.theme().foreground))
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
