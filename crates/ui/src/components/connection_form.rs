use dbstudio_core::models::{DatabaseType, ConnectionConfig, SshAuthType, SshConfig};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _,
    Icon,
    IconName,
    IndexPath,
    StyledExt as _,
    Sizable as _,
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    form::{field, v_form},
    input::{Input, InputState},
    notification::NotificationType,
    select::{Select, SelectEvent, SelectItem, SelectState},
    switch::Switch,
    h_flex,
    v_flex,
};

use crate::state::{connect_config, delete_connection, save_connection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DbTypeOption(pub DatabaseType);

impl SelectItem for DbTypeOption {
    type Value = &'static str;

    fn title(&self) -> SharedString {
        self.0.display_name().into()
    }

    fn value(&self) -> &Self::Value {
        match self.0 {
            DatabaseType::SQLite => &"sqlite",
            DatabaseType::MySQL => &"mysql",
            DatabaseType::PostgreSQL => &"postgresql",
            DatabaseType::MSSQL => &"mssql",
            DatabaseType::Oracle => &"oracle",
        }
    }
}

fn all_types() -> Vec<DbTypeOption> {
    DatabaseType::all()
        .iter()
        .map(|t| DbTypeOption(*t))
        .collect()
}

/// Wrapper so we can implement `SelectItem` for the SSH auth choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SshAuthOption {
    Password,
    KeyFile,
}

impl SshAuthOption {
    fn label(&self) -> &'static str {
        match self {
            SshAuthOption::Password => "Password",
            SshAuthOption::KeyFile => "Private Key File",
        }
    }

    /// Map the persisted SSH auth type spelling onto the select option.
    fn from_wire(value: &str) -> Self {
        match SshAuthType::from_wire(value) {
            SshAuthType::KeyFile => SshAuthOption::KeyFile,
            SshAuthType::Password => SshAuthOption::Password,
        }
    }

    fn all() -> Vec<SshAuthOption> {
        vec![SshAuthOption::Password, SshAuthOption::KeyFile]
    }
}

impl SelectItem for SshAuthOption {
    type Value = &'static str;

    fn title(&self) -> SharedString {
        self.label().into()
    }

    fn value(&self) -> &Self::Value {
        match self {
            SshAuthOption::Password => &"password",
            SshAuthOption::KeyFile => &"key_file",
        }
    }
}

pub enum ConnectionFormEvent {
    Saved,
    Canceled,
}

impl EventEmitter<ConnectionFormEvent> for ConnectionForm {}

pub struct ConnectionForm {
    name: Entity<InputState>,
    host: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    database: Entity<InputState>,
    port: Entity<InputState>,
    db_type_select: Entity<SelectState<Vec<DbTypeOption>>>,
    db_type: DatabaseType,

    // SSH state
    ssh_enabled: bool,
    ssh_host: Entity<InputState>,
    ssh_port: Entity<InputState>,
    ssh_username: Entity<InputState>,
    ssh_auth_select: Entity<SelectState<Vec<SshAuthOption>>>,
    ssh_auth: SshAuthOption,
    ssh_key_path: Entity<InputState>,
    ssh_password: Entity<InputState>,
    ssh_key_passphrase: Entity<InputState>,
    extra_params: Entity<InputState>,

    active_connection: Option<dbstudio_storage::types::ConnectionInfo>,
    is_testing: bool,
}

impl ConnectionForm {
    pub fn view(
        connection: Option<dbstudio_storage::types::ConnectionInfo>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| Self::new(connection, window, cx))
    }

    /// Create a single-line text input with the given placeholder.
    fn text_input(
        window: &mut Window,
        cx: &mut Context<Self>,
        placeholder: &str,
        masked: bool,
    ) -> Entity<InputState> {
        let placeholder = placeholder.to_string();
        cx.new(|cx| {
            let input = InputState::new(window, cx)
                .placeholder(placeholder)
                .clean_on_escape();
            if masked {
                input.masked(true)
            } else {
                input
            }
        })
    }

    fn new(
        connection: Option<dbstudio_storage::types::ConnectionInfo>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = Self::text_input(window, cx, "Name", false);
        let host = Self::text_input(window, cx, "localhost", false);
        let username = Self::text_input(window, cx, "Username", false);
        let password = Self::text_input(window, cx, "Password", true);
        let database = Self::text_input(window, cx, "Database / file path", false);
        let port = Self::text_input(window, cx, "Port", false);

        let initial_type = connection
            .as_ref()
            .map(|c| c.db_type)
            .unwrap_or(DatabaseType::PostgreSQL);

        let types = all_types();
        let selected_index = types
            .iter()
            .position(|t| t.0 == initial_type)
            .unwrap_or(0);
        let db_type_select = cx.new(|cx| {
            SelectState::new(types, Some(IndexPath::new(selected_index)), window, cx)
        });
        cx.subscribe_in(&db_type_select, window, Self::on_db_type_change)
            .detach();

        // SSH inputs
        let ssh_host = Self::text_input(window, cx, "ssh.example.com", false);
        let ssh_port = Self::text_input(window, cx, "22", false);
        let ssh_username = Self::text_input(window, cx, "Username", false);
        let ssh_key_path = Self::text_input(window, cx, "/Users/you/.ssh/id_ed25519", false);

        let ssh_auth = connection
            .as_ref()
            .map(|c| SshAuthOption::from_wire(c.ssh_auth_type.as_deref().unwrap_or("")))
            .unwrap_or(SshAuthOption::Password);
        let ssh_auth_select = cx.new(|cx| {
            SelectState::new(
                SshAuthOption::all(),
                Some(IndexPath::new(match ssh_auth {
                    SshAuthOption::Password => 0,
                    SshAuthOption::KeyFile => 1,
                })),
                window,
                cx,
            )
        });
        cx.subscribe_in(&ssh_auth_select, window, Self::on_ssh_auth_change)
            .detach();
        let ssh_password = Self::text_input(window, cx, "SSH password", true);
        let ssh_key_passphrase = Self::text_input(window, cx, "Key passphrase", true);
        let extra_params =
            Self::text_input(window, cx, r#"{"ssl-mode":"require"}"#, false);

        let mut form = Self {
            name,
            host,
            username,
            password,
            database,
            port,
            db_type_select,
            db_type: initial_type,
            ssh_enabled: connection
                .as_ref()
                .map(|c| c.ssh_enabled)
                .unwrap_or(false),
            ssh_host,
            ssh_port,
            ssh_username,
            ssh_auth_select,
            ssh_auth,
            ssh_key_path,
            ssh_password,
            ssh_key_passphrase,
            extra_params,
            active_connection: connection.clone(),
            is_testing: false,
        };

        if let Some(c) = connection {
            form.populate_from(c, window, cx);
        } else {
            form
                .port
                .update(cx, |this, cx| this.set_value(initial_type.default_port().to_string(), window, cx));
        }
        form
    }

    fn on_db_type_change(
        &mut self,
        _: &Entity<SelectState<Vec<DbTypeOption>>>,
        event: &SelectEvent<Vec<DbTypeOption>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SelectEvent::Confirm(Some(value)) = event {
            let new_type = (*value).parse::<DatabaseType>().unwrap_or(DatabaseType::Oracle);
            let prev_default = self.db_type.default_port().to_string();
            let current = self.port.read(cx).value().to_string();
            if current.is_empty() || current == prev_default {
                self.port.update(cx, |this, cx| {
                    this.set_value(new_type.default_port().to_string(), window, cx)
                });
            }
            self.db_type = new_type;
            cx.notify();
        }
    }

    fn on_ssh_auth_change(
        &mut self,
        _: &Entity<SelectState<Vec<SshAuthOption>>>,
        event: &SelectEvent<Vec<SshAuthOption>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SelectEvent::Confirm(Some(value)) = event {
            self.ssh_auth = SshAuthOption::from_wire(value);
            cx.notify();
        }
    }

    fn populate_from(
        &mut self,
        connection: dbstudio_storage::types::ConnectionInfo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (input, value) in [
            (&self.name, connection.name.as_str()),
            (&self.host, connection.host.as_str()),
            (&self.username, connection.username.as_str()),
            (&self.database, connection.database.as_str()),
            (&self.port, connection.port.to_string().as_str()),
        ] {
            input.update(cx, |this, cx| this.set_value(value.to_string(), window, cx));
        }

        if let Some(params) = &connection.extra_params {
            self.extra_params.update(cx, |this, cx| {
                this.set_value(params.clone(), window, cx)
            });
        }

        if connection.ssh_enabled {
            self.ssh_enabled = true;
            for (input, value) in [
                (&self.ssh_host, connection.ssh_host.as_deref().unwrap_or_default()),
                (&self.ssh_username, connection.ssh_username.as_deref().unwrap_or_default()),
                (&self.ssh_key_path, connection.ssh_key_path.as_deref().unwrap_or_default()),
            ] {
                input.update(cx, |this, cx| this.set_value(value.to_string(), window, cx));
            }
            self.ssh_port.update(cx, |this, cx| {
                this.set_value(
                    connection.ssh_port.map(|p| p.to_string()).unwrap_or_default(),
                    window,
                    cx,
                )
            });
        }
    }

    fn get_config(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Option<ConnectionConfig> {
        let name = self.name.read(cx).value().to_string();
        let host = self.host.read(cx).value().to_string();
        let username = self.username.read(cx).value().to_string();
        let database = self.database.read(cx).value().to_string();
        let port_str = self.port.read(cx).value().to_string();

        if name.is_empty() || database.is_empty() {
            return None;
        }

        let port: u16 = if port_str.is_empty() || self.db_type == DatabaseType::SQLite {
            self.db_type.default_port()
        } else {
            port_str.parse().unwrap_or(self.db_type.default_port())
        };

        let id = self
            .active_connection
            .as_ref()
            .map(|c| c.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let created_at = self
            .active_connection
            .as_ref()
            .map(|c| c.created_at.clone())
            .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string());

        let ssh = self.build_ssh_config(cx);
        let extra_params = {
            let raw = self.extra_params.read(cx).value().trim().to_string();
            if raw.is_empty() {
                None
            } else {
                Some(raw)
            }
        };

        Some(ConnectionConfig {
            id,
            name,
            db_type: self.db_type,
            host,
            port,
            database,
            username,
            color: None,
            ssh_enabled: ssh.is_some(),
            ssh_host: ssh.as_ref().map(|s| s.host.clone()),
            ssh_port: ssh.as_ref().map(|s| s.port),
            ssh_username: ssh.as_ref().map(|s| s.username.clone()),
            ssh_auth_type: ssh.as_ref().map(|s| s.auth_type.as_str().to_string()),
            ssh_key_path: ssh.and_then(|s| s.key_path),
            extra_params,
            created_at,
            updated_at: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        })
    }

    fn build_ssh_config(&mut self, cx: &mut Context<Self>) -> Option<SshConfig> {
        if !self.ssh_enabled {
            return None;
        }

        let host = self.ssh_host.read(cx).value().to_string();
        let username = self.ssh_username.read(cx).value().to_string();
        let port: u16 = self
            .ssh_port
            .read(cx)
            .value()
            .parse()
            .unwrap_or(22);

        let (auth_type, key_path) = match self.ssh_auth {
            SshAuthOption::KeyFile => {
                let path = self.ssh_key_path.read(cx).value().to_string();
                if path.is_empty() {
                    return None;
                }
                (SshAuthType::KeyFile, Some(path))
            }
            SshAuthOption::Password => (SshAuthType::Password, None),
        };

        Some(SshConfig {
            enabled: true,
            host,
            port,
            username,
            auth_type,
            key_path,
            ssh_password: None,
            ssh_key_passphrase: None,
        })
    }

    fn on_save(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let password = self.password.read(cx).value().to_string();
        let ssh_password = self.ssh_password.read(cx).value().to_string();
        let ssh_key_passphrase = self.ssh_key_passphrase.read(cx).value().to_string();
        if let Some(config) = self.get_config(window, cx) {
            save_connection(&config, &password, &ssh_password, &ssh_key_passphrase, cx);
            cx.emit(ConnectionFormEvent::Saved);
        }
    }

    fn on_cancel(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(ConnectionFormEvent::Canceled);
    }

    fn on_delete(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(conn) = self.active_connection.clone() {
            delete_connection(conn.id, cx);
            cx.emit(ConnectionFormEvent::Saved);
        }
    }

    fn on_connect(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let password = self.password.read(cx).value().to_string();
        let ssh_password = self.ssh_password.read(cx).value().to_string();
        let ssh_key_passphrase = self.ssh_key_passphrase.read(cx).value().to_string();
        if let Some(config) = self.get_config(window, cx) {
            connect_config(&config, &password, &ssh_password, &ssh_key_passphrase, cx);
            cx.emit(ConnectionFormEvent::Saved);
        }
    }

    fn on_test_connection(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_testing {
            return;
        }
        let password = self.password.read(cx).value().to_string();
        let ssh_password = self.ssh_password.read(cx).value().to_string();
        let ssh_key_passphrase = self.ssh_key_passphrase.read(cx).value().to_string();
        let Some(config) = self.get_config(window, cx) else {
            return;
        };
        self.is_testing = true;
        cx.notify();

        let entity = cx.entity();
        cx.spawn_in(window, async move |_this, cx| {
            let result = async {
                let conn = dbstudio_db::connect(&config, &password, Some(&ssh_password), Some(&ssh_key_passphrase))
                    .await
                    .map_err(|e| e.to_string())?;
                conn.ping().await.map_err(|e| e.to_string())
            }
            .await;

            let _ = cx.update(|window, cx| {
                match result {
                    Ok(()) => {
                        window.push_notification(
                            (NotificationType::Success, "Connection successful!"),
                            cx,
                        );
                    }
                    Err(msg) => {
                        window.push_notification(
                            (NotificationType::Error, SharedString::from(msg)),
                            cx,
                        );
                    }
                }
                entity.update(cx, |form, cx| {
                    form.is_testing = false;
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn render_ssh_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let show_key_fields = matches!(self.ssh_auth, SshAuthOption::KeyFile);

        v_form()
            .columns(2)
            .small()
            .child(
                field()
                    .col_span(2)
                    .label_indent(false)
                    .child(
                        Switch::new("ssh-enabled")
                            .checked(self.ssh_enabled)
                            .label("Connect through SSH tunnel")
                            .on_click(cx.listener(|this, checked: &bool, _win, cx| {
                                this.ssh_enabled = *checked;
                                cx.notify();
                            })),
                    ),
            )
            .when(self.ssh_enabled, |f| {
                f.child(
                    field()
                        .label("SSH Host")
                        .child(Input::new(&self.ssh_host)),
                )
                .child(
                    field()
                        .label("SSH Port")
                        .child(Input::new(&self.ssh_port)),
                )
                .child(
                    field()
                        .col_span(2)
                        .label("SSH User")
                        .child(Input::new(&self.ssh_username)),
                )
                .child(
                    field()
                        .col_span(2)
                        .label("SSH Auth")
                        .child(Select::new(&self.ssh_auth_select)),
                )
                .when(show_key_fields, |inner| {
                    inner.child(
                        field()
                            .col_span(2)
                            .label("Private Key Path")
                            .child(Input::new(&self.ssh_key_path)),
                    )
                    .child(
                        field()
                            .col_span(2)
                            .label("Key Passphrase")
                            .child(Input::new(&self.ssh_key_passphrase)),
                    )
                })
                .when(!show_key_fields, |inner| {
                    inner.child(
                        field()
                            .col_span(2)
                            .label("SSH Password")
                            .child(Input::new(&self.ssh_password)),
                    )
                })
            })
    }
}

impl Render for ConnectionForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_edit = self.active_connection.is_some();

        let mut actions = h_flex()
            .gap_2()
            .mt_4()
            .child(
                Button::new("form-test")
                    .label("Test Connection")
                    .ghost()
                    .loading(self.is_testing)
                    .on_click(cx.listener(Self::on_test_connection)),
            )
            .child(
                Button::new("form-cancel")
                    .label("Cancel")
                    .ghost()
                    .on_click(cx.listener(Self::on_cancel)),
            )
            .child(
                Button::new("form-save")
                    .label(if is_edit { "Update" } else { "Save" })
                    .primary()
                    .on_click(cx.listener(Self::on_save)),
            );

        if is_edit {
            actions = actions
                .child(
                    Button::new("form-connect")
                        .label("Connect")
                        .primary()
                        .on_click(cx.listener(Self::on_connect)),
                )
                .child(
                    Button::new("form-delete")
                        .label("Delete")
                        .danger()
                        .ghost()
                        .on_click(cx.listener(Self::on_delete)),
                );
        }

        let db_label = if self.db_type == DatabaseType::SQLite {
            "SQLite file"
        } else {
            "Database"
        };
        let db_type_field = field()
            .label(db_label)
            .required(true)
            .child(Input::new(&self.database));

        v_flex()
            .id("connection-form")
            .w(px(520.0))
            .on_click(cx.listener(|_this, _: &ClickEvent, _window, cx| {
                cx.stop_propagation();
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                if event.keystroke.key == "escape" && !event.keystroke.modifiers.modified() {
                    this.on_cancel(&ClickEvent::default(), _window, cx);
                }
            }))
            .p_5()
            .rounded(cx.theme().radius)
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .shadow_lg()
            .child(
                h_flex()
                    .id("connection-form-header")
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .pb_4()
                    .mb_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .size_9()
                                    .flex_none()
                                    .rounded(cx.theme().radius)
                                    .items_center()
                                    .justify_center()
                                    .bg(cx.theme().info.opacity(0.15))
                                    .child(
                                        Icon::new(IconName::LayoutDashboard)
                                            .size_4()
                                            .text_color(cx.theme().info),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .child(
                                        div()
                                            .text_lg()
                                            .font_semibold()
                                            .child(if is_edit {
                                                "Edit Connection"
                                            } else {
                                                "New Connection"
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(if is_edit {
                                                "Modify connection details"
                                            } else {
                                                "Connect to a database server"
                                            }),
                                    ),
                            ),
                    )
                    .child(
                        Button::new("form-close")
                            .icon(Icon::new(IconName::Close).size_3_5())
                            .ghost()
                            .on_click(cx.listener(Self::on_cancel)),
                    ),
            )
            .child(
                v_form()
                    .columns(2)
                    .small()
                    .child(
                        field()
                            .col_span(2)
                            .label("Driver")
                            .required(true)
                            .child(Select::new(&self.db_type_select)),
                    )
                    .child(
                        field()
                            .col_span(2)
                            .label("Name")
                            .required(true)
                            .child(Input::new(&self.name)),
                    )
                    .child(
                        field()
                            .label("Host")
                            .child(Input::new(&self.host)),
                    )
                    .child(
                        field()
                            .label("Port")
                            .child(Input::new(&self.port)),
                    )
                    .child(
                        field()
                            .col_span(2)
                            .label("Username")
                            .child(Input::new(&self.username)),
                    )
                    .child(
                        field()
                            .col_span(2)
                            .label("Password")
                            .child(Input::new(&self.password)),
                    )
                    .child(db_type_field)
                    .when(self.db_type != DatabaseType::SQLite, |f| {
                        f.child(
                            field()
                                .col_span(2)
                                .label("Connection Options (JSON)")
                                .child(Input::new(&self.extra_params)),
                        )
                    }),
            )
            .child(div().mt_4().child(self.render_ssh_section(cx)))
            .child(actions)
    }
}